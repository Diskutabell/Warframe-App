//! warframe.market prices (public v2 API, no login), with a disk cache and a polite request queue.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::{Duration, Instant};

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{self, Settings};
use crate::{net, now};

const API: &str = "https://api.warframe.market/v2";
/// WFCD has no art for individual prime parts; warframe.market has a 128 px glyph for most.
pub const ASSETS: &str = "https://warframe.market/static/assets/";
/// warframe.market asks for at most 3 requests a second; this is shared by every caller.
const MIN_INTERVAL: Duration = Duration::from_millis(400);
const ITEMS_MAX_AGE: f64 = 86400.0;

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
    queued: HashSet<String>,
}

pub struct Market {
    settings: Arc<RwLock<Settings>>,
    items: RwLock<Items>,
    prices: Mutex<HashMap<String, Price>>,
    queue: Mutex<Queue>,
    wake: Condvar,
    next_slot: Mutex<Instant>,
    dirty: AtomicBool,
    pub error: Mutex<Option<String>>,
}

fn prices_file() -> PathBuf {
    config::dirs().cache.join("prices.json")
}

fn items_file() -> PathBuf {
    config::dirs().cache.join("wfm_items.json")
}

impl Market {
    pub fn new(settings: Arc<RwLock<Settings>>) -> Arc<Market> {
        let market = Arc::new(Market {
            settings,
            items: RwLock::new(Items::default()),
            prices: Mutex::new(config::read_json(&prices_file()).unwrap_or_default()),
            queue: Mutex::new(Queue::default()),
            wake: Condvar::new(),
            next_slot: Mutex::new(Instant::now()),
            dirty: AtomicBool::new(false),
            error: Mutex::new(None),
        });
        let worker = Arc::clone(&market);
        std::thread::Builder::new()
            .name("market".into())
            .spawn(move || worker.run())
            .expect("spawning the market thread");
        market
    }

    fn headers(&self) -> [(&'static str, String); 3] {
        let platform = self.settings.read().unwrap_or_else(|e| e.into_inner()).platform.as_str().to_string();
        [("Language", "en".into()), ("Platform", platform), ("Crossplay", "true".into())]
    }

    fn get(&self, path: &str) -> anyhow::Result<Value> {
        self.wait_for_slot();
        let headers = self.headers();
        let refs: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        net::get_json(&format!("{API}{path}"), &refs, Duration::from_secs(20))
    }

    /// Blocks until this caller may send the next request.
    fn wait_for_slot(&self) {
        let wait = {
            let mut next = self.next_slot.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let start = (*next).max(now);
            *next = start + MIN_INTERVAL;
            start - now
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }

    fn set_error(&self, e: Option<String>) {
        *self.error.lock().unwrap_or_else(|e| e.into_inner()) = e;
    }

    /// The item list (names, ducats, art, game references), refreshed once a day.
    pub fn load_items(&self) {
        let mut cached: Option<Value> = config::read_json(&items_file());
        let stale = cached.as_ref().is_none_or(|c| now() - c["t"].as_f64().unwrap_or(0.0) > ITEMS_MAX_AGE);
        if stale {
            match self.get("/items") {
                Ok(v) => {
                    let fresh = json!({"t": now(), "data": v["data"]});
                    let _ = config::write_json(&items_file(), &fresh);
                    cached = Some(fresh);
                }
                Err(e) => self.set_error(Some(format!("warframe.market item list unavailable: {e}"))),
            }
        }
        let mut items = Items::default();
        for it in cached.as_ref().and_then(|c| c["data"].as_array()).into_iter().flatten() {
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
        *self.items.write().unwrap_or_else(|e| e.into_inner()) = items;
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
        self.settings.read().unwrap_or_else(|e| e.into_inner()).price_cache_minutes as f64 * 60.0
    }

    /// The cached price (lowest online sell order), queueing a refresh when it is stale.
    pub fn price(&self, slug: Option<&str>, request: bool) -> Option<Price> {
        let slug = slug?;
        let p = self.prices.lock().unwrap_or_else(|e| e.into_inner()).get(slug).cloned();
        if request && p.as_ref().is_none_or(|p| now() - p.t > self.ttl()) {
            self.request(slug, false);
        }
        p
    }

    pub fn cached(&self, slug: &str) -> Option<Price> {
        self.prices.lock().unwrap_or_else(|e| e.into_inner()).get(slug).cloned()
    }

    pub fn request(&self, slug: &str, urgent: bool) {
        let mut q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if q.queued.contains(slug) && !urgent {
            return;
        }
        q.queued.insert(slug.to_string());
        if urgent {
            q.urgent.push_back(slug.to_string())
        } else {
            q.normal.push_back(slug.to_string())
        }
        self.wake.notify_one();
    }

    pub fn pending(&self) -> usize {
        let q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        q.urgent.len() + q.normal.len()
    }

    /// Prices for a handful of slugs right now (the reward screen), within `timeout`. Anything
    /// fetched in the last 15 minutes is reused.
    pub fn fetch_now(&self, slugs: &[Option<String>], timeout: Duration) -> HashMap<String, Price> {
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
            if let Some(p) = self.fetch(slug) {
                out.insert(slug.clone(), p);
            }
        }
        out
    }

    fn fetch(&self, slug: &str) -> Option<Price> {
        let entry = match self.get(&format!("/orders/item/{slug}/top")) {
            Ok(v) => {
                let plat = |side: &str| -> Vec<i64> {
                    v["data"][side].as_array().into_iter().flatten().filter_map(|o| o["platinum"].as_i64()).collect()
                };
                let mut sells = plat("sell");
                sells.sort_unstable();
                let buys = plat("buy");
                let top3 = &sells[..sells.len().min(3)];
                Price {
                    sell: sells.first().copied(),
                    sell_avg: (!top3.is_empty())
                        .then(|| crate::difflib::py_round(top3.iter().sum::<i64>() as f64 / top3.len() as f64, 1)),
                    buy: buys.iter().max().copied(),
                    t: now(),
                    err: None,
                }
            }
            Err(e) => match e.downcast_ref::<net::HttpStatus>() {
                // A 404 is an answer ("no such item"); remember it so it is not asked again soon.
                Some(status) => Price { sell: None, sell_avg: None, buy: None, t: now(), err: Some(status.0) },
                None => {
                    self.set_error(Some(format!("warframe.market unreachable: {e}")));
                    return self.cached(slug);
                }
            },
        };
        self.set_error(None);
        self.prices.lock().unwrap_or_else(|e| e.into_inner()).insert(slug.to_string(), entry.clone());
        self.dirty.store(true, Ordering::Relaxed);
        Some(entry)
    }

    fn save_if_dirty(&self) {
        if self.dirty.swap(false, Ordering::Relaxed) {
            let snapshot = self.prices.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let _ = config::write_json(&prices_file(), &snapshot);
        }
    }

    fn run(&self) {
        let mut last_save = Instant::now();
        loop {
            let slug = {
                let q = self.queue.lock().unwrap_or_else(|e| e.into_inner());
                let (mut q, _) = self
                    .wake
                    .wait_timeout_while(q, Duration::from_secs(5), |q| q.urgent.is_empty() && q.normal.is_empty())
                    .unwrap_or_else(|e| e.into_inner());
                let slug = q.urgent.pop_front().or_else(|| q.normal.pop_front());
                if let Some(s) = &slug {
                    q.queued.remove(s);
                }
                slug
            };
            if let Some(slug) = slug {
                let fresh = self.cached(&slug).is_some_and(|p| now() - p.t <= self.ttl());
                if !fresh {
                    self.fetch(&slug);
                }
            }
            if last_save.elapsed() > Duration::from_secs(10) {
                self.save_if_dirty();
                last_save = Instant::now();
            }
        }
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
/// `StrunWraithBarrel`, `IronframeBlueprint`). The file names were checked against the site
/// (2026-10-10); it answers 200 with an HTML page for a glyph it does not have.
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
}
