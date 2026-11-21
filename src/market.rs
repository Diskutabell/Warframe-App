//! warframe.market prices (public v2 API, no login), with a disk cache and a polite request queue.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::bus::{Bus, Topic};
use crate::config::{self, Platform};
use crate::db::Db;
use crate::services::settings::SettingsStore;
use crate::{net, now};

const API: &str = "https://api.warframe.market/v2";
/// WFCD has no art for individual prime parts; warframe.market has a 128 px glyph for most.
pub const ASSETS: &str = "https://warframe.market/static/assets/";
/// warframe.market asks for at most 3 requests a second; this is shared by every caller.
const MIN_INTERVAL: Duration = Duration::from_millis(400);
const ITEMS_MAX_AGE: f64 = 86400.0;
/// Riven auctions are only searchable through the older API.
const API_V1: &str = "https://api.warframe.market/v1";
const SITE: &str = "https://warframe.market/";
/// The site's list of weapons that take rivens changes with new weapons only.
const RIVEN_WEAPONS_MAX_AGE: f64 = 7.0 * 86400.0;

/// What warframe.market lists for rivens like one that was read: the same weapon with at least
/// the same positive stats, for a fixed price.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct RivenListings {
    pub count: usize,
    pub lowest: Option<i64>,
    pub median: Option<i64>,
    /// The cheapest of those whose seller is in the game right now.
    #[serde(rename = "lowestOnline")]
    pub lowest_online: Option<i64>,
    /// The same search on the site.
    pub url: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Price {
    pub sell: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sell_avg: Option<f64>,
    pub buy: Option<i64>,
    pub t: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub err: Option<u16>,
}

#[derive(Clone, Debug)]
pub struct MarketItem {
    pub name: Option<String>,
    pub ducats: Option<i64>,
    pub thumb: Option<String>,
}

#[derive(Default)]
struct Items {
    /// In the API's own order, which search results fall back on for ties.
    by_slug: IndexMap<String, MarketItem>,
    /// Lower-case display name -> slug.
    by_name: HashMap<String, String>,
    /// The game's own uniqueName ("gameRef") -> slug: exact, unlike name matching.
    by_ref: HashMap<String, String>,
}

#[derive(Default)]
struct Queue {
    urgent: VecDeque<String>,
    normal: VecDeque<String>,
    queued_urgent: HashSet<String>,
    queued_normal: HashSet<String>,
}

impl Queue {
    fn push(&mut self, slug: &str, urgent: bool) -> bool {
        if !valid_slug(slug) {
            return false;
        }
        if urgent {
            if self.queued_urgent.contains(slug) {
                return false;
            }
            if self.queued_normal.remove(slug) {
                self.normal.retain(|s| s != slug);
            }
            self.queued_urgent.insert(slug.to_string());
            self.urgent.push_back(slug.to_string());
            true
        } else {
            if self.queued_urgent.contains(slug) || self.queued_normal.contains(slug) {
                return false;
            }
            self.queued_normal.insert(slug.to_string());
            self.normal.push_back(slug.to_string());
            true
        }
    }

    fn pop(&mut self) -> Option<String> {
        if let Some(s) = self.urgent.pop_front() {
            self.queued_urgent.remove(&s);
            Some(s)
        } else if let Some(s) = self.normal.pop_front() {
            self.queued_normal.remove(&s);
            Some(s)
        } else {
            None
        }
    }
}

struct PriceCache {
    platform: Platform,
    entries: HashMap<String, Price>,
}

pub struct Market {
    settings: Arc<SettingsStore>,
    /// Where the prices are kept (with a sparse history).
    db: Arc<Db>,
    /// Fresh prices are announced here, so the pages showing them can redraw.
    bus: Bus,
    items: RwLock<Items>,
    prices: Mutex<PriceCache>,
    save_lock: Mutex<()>,
    riven_weapons: RwLock<Option<(f64, Vec<Value>)>>,
    /// One refresh of the riven weapon list at a time.
    riven_weapons_fetch: tokio::sync::Mutex<()>,
    queue: Mutex<Queue>,
    /// Wakes the worker when a slug is queued.
    wake: Notify,
    next_slot: Mutex<Instant>,
    /// Prices fetched since the last save, by slug (all of the cache's platform).
    dirty: Mutex<HashSet<String>>,
    pub error: Mutex<Option<String>>,
}

/// warframe.market slugs (items, riven weapons, attributes) are lowercase ASCII, digits and `_`.
fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 128
        && slug.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn items_file() -> PathBuf {
    config::dirs().cache.join("wfm_items.json")
}

fn valid_item_list(data: &Value) -> bool {
    data.as_array().is_some_and(|items| !items.is_empty() && items.iter().any(|item| item["slug"].as_str().is_some()))
}

fn price_from_orders(value: &Value, time: f64) -> anyhow::Result<Price> {
    let data = &value["data"];
    let orders = |side: &str| -> anyhow::Result<Vec<i64>> {
        let list = data[side].as_array().ok_or_else(|| anyhow::anyhow!("missing {side} orders"))?;
        list.iter()
            .map(|order| {
                order["platinum"].as_i64().filter(|p| *p >= 0).ok_or_else(|| anyhow::anyhow!("invalid {side} price"))
            })
            .collect()
    };
    let mut sells = orders("sell")?;
    let buys = orders("buy")?;
    sells.sort_unstable();
    let top3 = &sells[..sells.len().min(3)];
    Ok(Price {
        sell: sells.first().copied(),
        sell_avg: (!top3.is_empty())
            .then(|| crate::difflib::py_round(top3.iter().map(|p| *p as f64).sum::<f64>() / top3.len() as f64, 1)),
        buy: buys.iter().max().copied(),
        t: time,
        err: None,
    })
}

/// Indexes warframe.market's item list by slug, name and game reference.
fn index_items(cached: &Value) -> Items {
    let mut items = Items::default();
    for it in cached["data"].as_array().into_iter().flatten() {
        let Some(slug) = it["slug"].as_str() else { continue };
        let en = &it["i18n"]["en"];
        let name = en["name"].as_str().map(str::to_owned);
        // subIcon is the per-slot glyph (neuroptics, chassis, barrel...). thumb is often the
        // same picture for every part of a set, so the glyph is what tells them apart.
        let art = en["subIcon"].as_str().or_else(|| en["thumb"].as_str());
        items.by_slug.insert(
            slug.to_string(),
            MarketItem {
                name: name.clone(),
                ducats: it["ducats"].as_i64(),
                thumb: art.map(|a| format!("{ASSETS}{a}")),
            },
        );
        if let Some(n) = name {
            items.by_name.insert(n.to_lowercase(), slug.to_string());
        }
        if let Some(r) = it["gameRef"].as_str() {
            items.by_ref.entry(r.to_string()).or_insert_with(|| slug.to_string());
        }
    }
    items
}

fn reserve_slot(next: &Mutex<Instant>, deadline: Option<Instant>) -> anyhow::Result<Duration> {
    let mut next = next.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let start = (*next).max(now);
    if deadline.is_some_and(|end| start >= end) {
        anyhow::bail!("market request deadline expired");
    }
    *next = start + MIN_INTERVAL;
    Ok(start - now)
}

impl Market {
    /// The market with the prices cached for the current platform. Nothing is fetched until
    /// `run` works the queue.
    pub fn new(settings: Arc<SettingsStore>, db: Arc<Db>, bus: Bus) -> Arc<Market> {
        let platform = settings.get().platform;
        let entries = db.load_prices(platform).unwrap_or_else(|e| {
            eprintln!("could not read the kept prices: {e:#}");
            HashMap::new()
        });
        Arc::new(Market {
            settings,
            db,
            bus,
            items: RwLock::new(Items::default()),
            prices: Mutex::new(PriceCache { platform, entries }),
            save_lock: Mutex::new(()),
            riven_weapons: RwLock::new(None),
            riven_weapons_fetch: tokio::sync::Mutex::new(()),
            queue: Mutex::new(Queue::default()),
            wake: Notify::new(),
            next_slot: Mutex::new(Instant::now()),
            dirty: Mutex::new(HashSet::new()),
            error: Mutex::new(None),
        })
    }

    /// A market with default settings and nothing cached, for tests of what reads prices.
    #[cfg(test)]
    pub fn for_tests() -> Arc<Market> {
        let bus = Bus::new();
        let settings = Arc::new(SettingsStore::new(crate::config::Settings::default(), &bus));
        Arc::new(Market {
            settings,
            db: Arc::new(Db::memory()),
            bus,
            items: RwLock::new(Items::default()),
            prices: Mutex::new(PriceCache { platform: Platform::Pc, entries: HashMap::new() }),
            save_lock: Mutex::new(()),
            riven_weapons: RwLock::new(None),
            riven_weapons_fetch: tokio::sync::Mutex::new(()),
            queue: Mutex::new(Queue::default()),
            wake: Notify::new(),
            next_slot: Mutex::new(Instant::now()),
            dirty: Mutex::new(HashSet::new()),
            error: Mutex::new(None),
        })
    }

    fn platform(&self) -> Platform {
        self.settings.get().platform
    }

    /// Switches the in-memory price map when the user changes platforms in Settings.
    fn ensure_platform(&self) -> Platform {
        let current = self.platform();
        let old = {
            let p = self.prices.lock().unwrap_or_else(|e| e.into_inner());
            if p.platform == current {
                return current;
            }
            p.platform
        };
        let loaded: HashMap<String, Price> = self.db.load_prices(current).unwrap_or_default();
        // The swap and taking the old platform's unsaved prices are one step (prices, then dirty,
        // as everywhere): a price fetched for the old platform meanwhile is saved with it.
        let _guard = self.save_lock.lock().unwrap_or_else(|e| e.into_inner());
        let unsaved = {
            let mut p = self.prices.lock().unwrap_or_else(|e| e.into_inner());
            if p.platform != old || current != self.platform() {
                return current;
            }
            let slugs = std::mem::take(&mut *self.dirty.lock().unwrap_or_else(|e| e.into_inner()));
            let unsaved: Vec<(String, Price)> =
                slugs.iter().filter_map(|s| p.entries.get(s).map(|e| (s.clone(), e.clone()))).collect();
            p.platform = current;
            p.entries = loaded;
            unsaved
        };
        if !unsaved.is_empty() {
            if let Err(e) = self.db.save_prices(old, &unsaved) {
                eprintln!("could not save market prices: {e:#}");
            }
        }
        current
    }

    fn headers_for(platform: Platform) -> [(&'static str, &'static str); 3] {
        [("Language", "en"), ("Platform", platform.as_str()), ("Crossplay", "true")]
    }

    async fn get(&self, path: &str, deadline: Option<Instant>) -> anyhow::Result<Value> {
        self.get_for(self.platform(), path, deadline).await
    }

    async fn get_for(&self, platform: Platform, path: &str, deadline: Option<Instant>) -> anyhow::Result<Value> {
        self.wait_for_slot(deadline).await?;
        let headers = Self::headers_for(platform);
        let timeout = match deadline {
            Some(end) => end
                .checked_duration_since(Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or_else(|| anyhow::anyhow!("market request deadline expired"))?
                .min(Duration::from_secs(20)),
            None => Duration::from_secs(20),
        };
        net::get_json(&format!("{API}{path}"), &headers, timeout).await
    }

    /// Waits until this caller may send the next request.
    async fn wait_for_slot(&self, deadline: Option<Instant>) -> anyhow::Result<()> {
        let wait = reserve_slot(&self.next_slot, deadline)?;
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        Ok(())
    }

    fn set_error(&self, e: Option<String>) {
        let mut error = self.error.lock().unwrap_or_else(|e| e.into_inner());
        // The summary shows the market's error. Only a change between working and failing is
        // worth a fresh summary: while the market is down every failed request has a message of
        // its own, and pushing each would recompute the summary a few times a second.
        let flipped = error.is_some() != e.is_some();
        *error = e;
        if flipped {
            self.bus.state_changed();
        }
    }

    /// The item list (names, ducats, art, game references), refreshed once a day.
    pub async fn load_items(&self) {
        let mut cached: Option<Value> =
            tokio::task::spawn_blocking(|| config::read_json(&items_file())).await.ok().flatten();
        let stale = cached
            .as_ref()
            .is_none_or(|c| !valid_item_list(&c["data"]) || now() - c["t"].as_f64().unwrap_or(0.0) > ITEMS_MAX_AGE);
        if stale {
            match self.get("/items", None).await {
                Ok(v) if valid_item_list(&v["data"]) => {
                    let fresh = json!({"t": now(), "data": v["data"]});
                    let saved = {
                        let fresh = fresh.clone();
                        tokio::task::spawn_blocking(move || config::write_json(&items_file(), &fresh)).await
                    };
                    if let Ok(Err(e)) = saved {
                        eprintln!("could not cache warframe.market item list: {e:#}");
                    }
                    cached = Some(fresh);
                    self.set_error(None);
                }
                Ok(_) => {
                    self.set_error(Some("warframe.market returned an invalid item list; using cached data".into()))
                }
                Err(e) => self.set_error(Some(format!("warframe.market item list unavailable: {e}"))),
            }
        }
        let Some(cached) = cached.filter(|c| valid_item_list(&c["data"])) else { return };
        let items = tokio::task::spawn_blocking(move || index_items(&cached)).await;
        if let Ok(items) = items {
            *self.items.write().unwrap_or_else(|e| e.into_inner()) = items;
        }
        // Pages show names and art from the list.
        self.bus.signal(Topic::Prices);
    }

    pub fn ducats(&self, slug: Option<&str>) -> Option<i64> {
        let items = self.items.read().unwrap_or_else(|e| e.into_inner());
        slug.and_then(|s| items.by_slug.get(s)).and_then(|i| i.ducats)
    }

    /// The item's name as warframe.market lists it.
    pub fn name(&self, slug: &str) -> Option<String> {
        let items = self.items.read().unwrap_or_else(|e| e.into_inner());
        items.by_slug.get(slug).and_then(|i| i.name.clone())
    }

    pub fn image(&self, slug: Option<&str>) -> Option<String> {
        let items = self.items.read().unwrap_or_else(|e| e.into_inner());
        slug.and_then(|s| items.by_slug.get(s)).and_then(|i| i.thumb.clone())
    }

    /// The market slug for a game item: its uniqueName first (exact), then WFCD's slug, then the
    /// display name, then the name with " blueprint" (warframe parts trade as blueprints).
    pub fn slug_for(&self, uid: Option<&str>, slug: Option<&str>, name: Option<&str>) -> Option<String> {
        let items = self.items.read().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = uid.and_then(|u| items.by_ref.get(u)) {
            return Some(s.clone());
        }
        if let Some(s) = slug.filter(|s| items.by_slug.is_empty() || items.by_slug.contains_key(*s)) {
            return Some(s.to_string());
        }
        if let Some(n) = name.map(str::to_lowercase) {
            if let Some(s) = items.by_name.get(&n).or_else(|| items.by_name.get(&format!("{n} blueprint"))) {
                return Some(s.clone());
            }
        }
        slug.map(str::to_owned)
    }

    fn ttl(&self) -> f64 {
        self.settings.get().price_cache_minutes as f64 * 60.0
    }

    /// The cached price (lowest online sell order), queueing a refresh when it is stale.
    pub fn price(&self, slug: Option<&str>, request: bool) -> Option<Price> {
        let slug = slug?;
        self.ensure_platform();
        let p = self.prices.lock().unwrap_or_else(|e| e.into_inner()).entries.get(slug).cloned();
        if request && p.as_ref().is_none_or(|p| now() - p.t > self.ttl()) {
            self.request(slug, false);
        }
        p
    }

    pub fn cached(&self, slug: &str) -> Option<Price> {
        self.ensure_platform();
        self.prices.lock().unwrap_or_else(|e| e.into_inner()).entries.get(slug).cloned()
    }

    pub fn request(&self, slug: &str, urgent: bool) {
        let queued = self.queue.lock().unwrap_or_else(|e| e.into_inner()).push(slug, urgent);
        if queued {
            self.wake.notify_one();
        }
    }

    pub fn pending(&self) -> usize {
        let q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.urgent.len() + q.normal.len()
    }

    /// Prices for a handful of slugs right now (the reward screen), within `timeout`. Anything
    /// fetched in the last 15 minutes is reused.
    pub async fn fetch_now(&self, slugs: &[Option<String>], timeout: Duration) -> HashMap<String, Price> {
        let deadline = Instant::now() + timeout;
        let mut out = HashMap::new();
        for slug in slugs.iter().flatten() {
            if let Some(p) = self.cached(slug).filter(|p| now() - p.t < 900.0) {
                out.insert(slug.clone(), p);
                continue;
            }
            if Instant::now() > deadline {
                break;
            }
            if let Some(p) = self.fetch(slug, Some(deadline)).await {
                out.insert(slug.clone(), p);
            }
        }
        out
    }

    async fn fetch(&self, slug: &str, deadline: Option<Instant>) -> Option<Price> {
        if !valid_slug(slug) {
            return None;
        }
        let platform = self.ensure_platform();
        let entry = match self.get_for(platform, &format!("/orders/item/{slug}/top"), deadline).await {
            Ok(v) => match price_from_orders(&v, now()) {
                Ok(price) => price,
                Err(e) => {
                    self.set_error(Some(format!("warframe.market returned invalid orders for {slug}: {e:#}")));
                    return self.cached(slug);
                }
            },
            Err(e) => match e.downcast_ref::<net::HttpStatus>() {
                // A 404 is an answer ("no such item"); remember it so it is not asked again soon.
                Some(net::HttpStatus(404)) => Price { sell: None, sell_avg: None, buy: None, t: now(), err: Some(404) },
                _ => {
                    self.set_error(Some(format!("warframe.market price unavailable for {slug}: {e}")));
                    return self.cached(slug);
                }
            },
        };
        self.set_error(None);
        let stored = {
            let mut cache = self.prices.lock().unwrap_or_else(|e| e.into_inner());
            let stored = cache.platform == platform;
            if stored {
                cache.entries.insert(slug.to_string(), entry.clone());
                self.dirty.lock().unwrap_or_else(|e| e.into_inner()).insert(slug.to_string());
            }
            stored
        };
        if stored {
            self.bus.publish(Topic::Prices, &json!({"slugs": [slug]}));
        }
        Some(entry)
    }

    /// Keeps the prices fetched since the last save. Blocking (the database).
    pub fn save_if_dirty(&self) {
        let _guard = self.save_lock.lock().unwrap_or_else(|e| e.into_inner());
        let slugs = std::mem::take(&mut *self.dirty.lock().unwrap_or_else(|e| e.into_inner()));
        if slugs.is_empty() {
            return;
        }
        let (platform, changed) = {
            let cache = self.prices.lock().unwrap_or_else(|e| e.into_inner());
            let changed: Vec<(String, Price)> =
                slugs.iter().filter_map(|s| cache.entries.get(s).map(|p| (s.clone(), p.clone()))).collect();
            (cache.platform, changed)
        };
        if let Err(e) = self.db.save_prices(platform, &changed) {
            // Tried again at the next save.
            self.dirty.lock().unwrap_or_else(|e| e.into_inner()).extend(slugs);
            eprintln!("could not save market prices: {e:#}");
        }
    }

    /// Writes the prices fetched since the last save, on the blocking pool.
    async fn save_soon(self: &Arc<Self>) {
        let me = Arc::clone(self);
        let _ = tokio::task::spawn_blocking(move || me.save_if_dirty()).await;
    }

    /// Works the queue (one request per slot of the shared rate limit) until `stop`, saving new
    /// prices every 10 s and once more at the end.
    pub async fn run(self: Arc<Self>, stop: CancellationToken) {
        let mut last_save = Instant::now();
        loop {
            let next = self.queue.lock().unwrap_or_else(|e| e.into_inner()).pop();
            match next {
                // Offline nothing can be fetched: the cached prices are all there is.
                Some(_) if net::offline() => {}
                Some(slug) => {
                    let fresh = self.cached(&slug).is_some_and(|p| now() - p.t <= self.ttl());
                    if !fresh {
                        tokio::select! {
                            _ = stop.cancelled() => break,
                            _ = self.fetch(&slug, None) => {}
                        }
                    }
                }
                None => {
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = self.wake.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(10)) => {}
                    }
                }
            }
            if last_save.elapsed() > Duration::from_secs(10) {
                self.save_soon().await;
                last_save = Instant::now();
            }
        }
        self.save_soon().await;
    }

    /// The site's slug for a weapon's rivens, by the weapon's uniqueName, else its name.
    async fn riven_weapon_slug(&self, uid: &str, name: &str) -> Option<String> {
        let find_in = |weapons: &[Value]| {
            let by = |f: &dyn Fn(&Value) -> bool| weapons.iter().find(|w| f(w)).and_then(|w| w["slug"].as_str());
            by(&|w| w["gameRef"].as_str() == Some(uid))
                .or_else(|| by(&|w| w["i18n"]["en"]["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name))))
                .map(str::to_owned)
        };
        if let Some((t, weapons)) = self.riven_weapons.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if !weapons.is_empty() && now() - *t < RIVEN_WEAPONS_MAX_AGE {
                return find_in(weapons);
            }
        }
        let _refresh = self.riven_weapons_fetch.lock().await;
        if let Some((t, weapons)) = self.riven_weapons.read().unwrap_or_else(|e| e.into_inner()).as_ref() {
            if !weapons.is_empty() && now() - *t < RIVEN_WEAPONS_MAX_AGE {
                return find_in(weapons);
            }
        }
        let file = config::dirs().cache.join("wfm_riven_weapons.json");
        let in_memory = self.riven_weapons.read().unwrap_or_else(|e| e.into_inner()).clone();
        let mut cached = match in_memory {
            Some(found) => Some(found),
            None => {
                let file = file.clone();
                let read = move || -> Option<(f64, Vec<Value>)> {
                    let raw: Value = config::read_json(&file)?;
                    let weapons = raw["data"].as_array()?.clone();
                    Some((raw["t"].as_f64().unwrap_or(0.0), weapons))
                };
                tokio::task::spawn_blocking(read).await.ok().flatten()
            }
        };
        let fresh = cached.as_ref().is_some_and(|(t, w)| !w.is_empty() && now() - *t < RIVEN_WEAPONS_MAX_AGE);
        if !fresh {
            match self.get("/riven/weapons", None).await {
                Ok(v) if v["data"].as_array().is_some_and(|a| !a.is_empty()) => {
                    let fetched = now();
                    let weapons = v["data"].as_array().cloned().unwrap_or_default();
                    let saved = json!({"t": fetched, "data": weapons});
                    let _ = tokio::task::spawn_blocking(move || config::write_json(&file, &saved)).await;
                    cached = Some((fetched, weapons));
                }
                Ok(_) => eprintln!("warframe.market sent no riven weapons"),
                Err(e) => eprintln!("warframe.market riven weapons unavailable: {e:#}"),
            }
        }
        if let Some(entry) = &cached {
            *self.riven_weapons.write().unwrap_or_else(|e| e.into_inner()) = Some(entry.clone());
        }
        let (_, weapons) = cached?;
        find_in(&weapons)
    }

    /// Rivens for the same weapon with at least these positives (warframe.market's attribute
    /// slugs), sold at a fixed price. One search per riven read, through the shared rate limit.
    pub async fn riven_listings(&self, uid: &str, name: &str, positives: &[&str]) -> anyhow::Result<RivenListings> {
        let weapon = self
            .riven_weapon_slug(uid, name)
            .await
            .ok_or_else(|| anyhow::anyhow!("warframe.market lists no rivens for {name}"))?;
        // Attribute slugs are warframe.market's own and may hold a "/" ("base_damage_/_melee_damage"):
        // each must be one of the stat table's.
        let known = |slug: &str| crate::rivens::STATS.iter().any(|s| crate::rivens::market_slug(s.name) == Some(slug));
        if !valid_slug(&weapon) || !positives.iter().all(|s| known(s)) {
            anyhow::bail!("warframe.market riven query contained an invalid slug");
        }
        let stats = positives.join(",");
        let query = format!("type=riven&weapon_url_name={weapon}&positive_stats={stats}&sort_by=price_asc");
        self.wait_for_slot(None).await?;
        let headers = Self::headers_for(self.platform());
        let found = net::get_json(
            &format!("{API_V1}/auctions/search?{query}&buyout_policy=direct"),
            &headers,
            Duration::from_secs(20),
        )
        .await?;
        let auctions = found["payload"]["auctions"].as_array().cloned().unwrap_or_default();
        let live = |a: &&Value| a["visible"].as_bool() != Some(false) && a["closed"].as_bool() != Some(true);
        let price = |a: &Value| a["buyout_price"].as_i64().filter(|p| *p > 0);
        let mut prices: Vec<i64> = auctions.iter().filter(live).filter_map(price).collect();
        prices.sort_unstable();
        let lowest_online = auctions
            .iter()
            .filter(live)
            .filter(|a| a["owner"]["status"].as_str() == Some("ingame"))
            .filter_map(price)
            .min();
        Ok(RivenListings {
            count: prices.len(),
            lowest: prices.first().copied(),
            median: (!prices.is_empty()).then(|| prices[prices.len() / 2]),
            lowest_online,
            url: format!("{SITE}auctions/search?{query}&polarity=any"),
        })
    }

    /// Market items whose name contains `q`, names starting with it first, shortest first.
    pub fn search(&self, q: &str) -> Vec<(String, String, Option<i64>)> {
        let items = self.items.read().unwrap_or_else(|e| e.into_inner());
        let mut hits: Vec<(&String, &MarketItem)> = items
            .by_slug
            .iter()
            .filter(|(_, it)| it.name.as_deref().is_some_and(|n| n.to_lowercase().contains(q)))
            .collect();
        hits.sort_by_key(|(_, it)| {
            let n = it.name.as_deref().unwrap_or("");
            (!n.to_lowercase().starts_with(q), n.chars().count())
        });
        hits.into_iter()
            .take(15)
            .map(|(slug, it)| (slug.clone(), it.name.clone().unwrap_or_default(), it.ducats))
            .collect()
    }
}

/// A generic slot glyph (chassis, barrel, blueprint...) for a part warframe.market does not list,
/// which is every part that cannot be traded. `category` is the WFCD category of the item the part
/// builds; the slot is read off the end of the part's uid (`TenguHelmetComponent`,
/// `StrunWraithBarrel`, `IronframeBlueprint`). The site answers 200 with an HTML page for a glyph
/// it does not have.
pub fn slot_glyph(category: &str, uid: &str) -> Option<String> {
    const WEAPON: &[&str] = &[
        "aegis", "barrel", "blade", "chassis", "disc", "engine", "gauntlet", "grip", "guard", "handle", "head",
        "heatsink", "hilt", "limb", "link", "motor", "receiver", "rivet", "stock", "string",
    ];
    let (folder, slots): (&str, &[&str]) = match category {
        "Warframes" => ("warframe", &["chassis", "helmet", "systems"]),
        "Archwing" => ("archwing", &["chassis", "systems", "wings"]),
        "Sentinels" => ("sentinel", &["carapace", "cerebrum", "systems"]),
        "Primary" | "Secondary" | "Melee" | "Arch-Gun" | "Arch-Melee" | "SentinelWeapons" => ("weapon", WEAPON),
        _ => ("", &[]),
    };
    let last = uid.rsplit('/').next().unwrap_or("").to_ascii_lowercase();
    let blueprint = last.ends_with("blueprint");
    let stem = last.trim_end_matches("blueprint").trim_end_matches("component");
    match slots.iter().find(|s| stem.ends_with(**s)) {
        Some(slot) => Some(format!("{ASSETS}sub_icons/{folder}/generic_{slot}_128x128.png")),
        None if blueprint => Some(format!("{ASSETS}sub_icons/blueprint_128x128.png")),
        None => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn item_list_rejects_empty_or_unexpected_responses() {
        assert!(valid_item_list(&json!([{"slug": "ash_prime_set"}])));
        assert!(!valid_item_list(&json!([])));
        assert!(!valid_item_list(&json!([{"name": "Ash Prime"}])));
        assert!(!valid_item_list(&json!({"error": "temporarily unavailable"})));
        assert!(!valid_item_list(&Value::Null));
    }

    #[test]
    fn orders_require_valid_prices_but_allow_empty_sides() {
        let price = price_from_orders(
            &json!({"data": {"sell": [{"platinum": 20}, {"platinum": 10}], "buy": [{"platinum": 7}]}}),
            42.0,
        )
        .unwrap();
        assert_eq!((price.sell, price.sell_avg, price.buy, price.t), (Some(10), Some(15.0), Some(7), 42.0));
        let empty = price_from_orders(&json!({"data": {"sell": [], "buy": []}}), 42.0).unwrap();
        assert_eq!((empty.sell, empty.buy), (None, None));
        for invalid in [
            json!({"data": {}}),
            json!({"data": {"sell": [{}], "buy": []}}),
            json!({"data": {"sell": [], "buy": [{"platinum": -1}]}}),
        ] {
            assert!(price_from_orders(&invalid, 42.0).is_err());
        }
    }

    #[test]
    fn expired_market_deadline_does_not_consume_a_request_slot() {
        let reserved = Instant::now() + Duration::from_secs(5);
        let next = Mutex::new(reserved);
        assert!(reserve_slot(&next, Some(reserved)).is_err());
        assert_eq!(*next.lock().unwrap(), reserved);
        assert!(reserve_slot(&next, Some(reserved + Duration::from_secs(1))).is_ok());
        assert_eq!(*next.lock().unwrap(), reserved + MIN_INTERVAL);
    }

    #[test]
    fn slot_glyphs_for_parts_the_market_does_not_list() {
        let g = |c, u: &str| slot_glyph(c, u).map(|s| s.trim_start_matches(ASSETS).to_string());
        let wf = "/Lotus/Types/Recipes/WarframeRecipes/";
        assert_eq!(
            g("Warframes", &format!("{wf}IronframeHelmetComponent")).unwrap(),
            "sub_icons/warframe/generic_helmet_128x128.png"
        );
        assert_eq!(g("Warframes", &format!("{wf}IronframeBlueprint")).unwrap(), "sub_icons/blueprint_128x128.png");
        assert_eq!(
            g("Primary", "/Lotus/Types/Recipes/Weapons/WeaponParts/StrunWraithBarrel").unwrap(),
            "sub_icons/weapon/generic_barrel_128x128.png"
        );
        assert_eq!(
            g("Archwing", "/Lotus/Types/Recipes/ArchwingRecipes/StandardArchwingWingsComponent").unwrap(),
            "sub_icons/archwing/generic_wings_128x128.png"
        );
        // A shared clan-research part has no slot of its own: the caller falls back to the item's art.
        assert_eq!(g("Primary", "/Lotus/Types/Items/Research/EnergyComponent"), None);
    }

    #[test]
    fn urgent_requests_promote_without_duplicating_and_validate_slugs() {
        let mut q = Queue::default();
        assert!(q.push("ash_prime_set", false));
        assert!(!q.push("ash_prime_set", false));
        assert!(q.push("ash_prime_set", true), "promotes normal to urgent");
        assert!(!q.push("ash_prime_set", true), "no duplicate urgent entry");
        assert!(!q.push("ash_prime_set", false), "already urgent");
        assert!(!q.push("../evil", true));
        assert!(!q.push("foo&bar=1", false));
        assert_eq!(q.urgent.len(), 1);
        assert_eq!(q.normal.len(), 0);
        assert_eq!(q.pop().as_deref(), Some("ash_prime_set"));
        assert_eq!(q.pop(), None);
        assert!(q.queued_urgent.is_empty() && q.queued_normal.is_empty());
    }
}
