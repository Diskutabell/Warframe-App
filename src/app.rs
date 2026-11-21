//! The engine: every service wired together, the background work started and stopped, and what
//! the API answers with when it takes more than one service.
//!
//! Services own their state (`services/`); `App` only routes between them: EE.log's events to
//! the accounts, the scans and the memory scan; a changed setting to whatever it concerns; and a
//! page's request to the services it needs.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::bus::Bus;
use crate::config::Settings;
use crate::db::{Db, Legacy};
use crate::difflib::py_round;
use crate::eelog::{self, LogEvent};
use crate::market::Market;
use crate::rivens::{self, Graded};
use crate::services::accounts::AccountService;
use crate::services::catalog::CatalogStore;
use crate::services::gamelog::LogService;
use crate::services::lock;
use crate::services::memscan::MemscanService;
use crate::services::notifications::Notifications;
use crate::services::scans::{RewardResult, RivenResult, ScanService};
use crate::services::settings::SettingsStore;
use crate::services::shell::{Shell, UiEvent};
use crate::services::world::WorldService;
use crate::watch::Wakeups;
use crate::{UserError, arbitration, compositor, net, now, ocr, standing, tasks, update, world};

/// What the summary says about the helper programs is looked up again after this long.
const TOOLS_MAX_AGE: f64 = 30.0;
/// A watched inventory file is also looked at this often, in case a change notification is lost.
const INVENTORY_FILE_LOOK: Duration = Duration::from_secs(20);

pub struct App {
    pub bus: Bus,
    /// The app's own data (`<data>/warframeproject.db`).
    pub db: Arc<Db>,
    /// Cancelled when the app quits: background work stops and the server shuts down.
    pub stop: CancellationToken,
    pub settings: Arc<SettingsStore>,
    pub shell: Arc<Shell>,
    pub catalog: Arc<CatalogStore>,
    pub market: Arc<Market>,
    pub accounts: Arc<AccountService>,
    pub world: Arc<WorldService>,
    pub log: Arc<LogService>,
    pub scans: Arc<ScanService>,
    pub memscan: Arc<MemscanService>,
    notifications: Arc<Notifications>,
    /// The background work that must finish before the app ends (it saves what it holds).
    tasks: TaskTracker,
    /// For work started from threads outside the runtime (the Windows tray).
    runtime: Handle,
    /// A newer release, once the check at startup has found one (`update::check`).
    update: Mutex<Option<Value>>,
    tools: Mutex<(f64, Value)>,
    /// The watched inventory file and the token that stops its watcher.
    inventory_watch: Mutex<Option<CancellationToken>>,
    /// When this run of the app started: the reward session counts from here.
    started: f64,
    /// The port the API listens on in this run (a changed setting takes effect at the next start).
    port: u16,
}

/// Runs blocking work on the blocking pool; a panic in it (a bug) becomes an error.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|e| anyhow!("{e}"))
}

impl App {
    /// The engine with its saved data loaded. Must be called inside the runtime; nothing runs
    /// until `start`. Fails only when the app's data cannot be opened: then it must not run (it
    /// would start empty and could overwrite what is there).
    pub fn new(settings: Settings, port: u16) -> Result<Arc<App>> {
        let dirs = crate::config::dirs();
        let (db, notes) =
            Db::open(&dirs.data.join("warframeproject.db"), Legacy { data: &dirs.data, cache: &dirs.cache })
                .context("the app's data could not be opened")?;
        Self::with_db(settings, port, Arc::new(db), notes)
    }

    fn with_db(settings: Settings, port: u16, db: Arc<Db>, notes: Vec<String>) -> Result<Arc<App>> {
        let bus = Bus::new();
        let stop = CancellationToken::new();
        let settings = Arc::new(SettingsStore::new(settings, &bus));
        let shell = Arc::new(Shell::default());
        let catalog = Arc::new(CatalogStore::new(&bus));
        let market = Market::new(Arc::clone(&settings), Arc::clone(&db), bus.clone());
        let accounts = AccountService::new(
            Arc::clone(&db),
            notes,
            Arc::clone(&settings),
            Arc::clone(&catalog),
            Arc::clone(&market),
            &bus,
            stop.clone(),
        )?;
        let world = Arc::new(WorldService::new(Arc::clone(&settings), &bus));
        let log = Arc::new(LogService::new(Arc::clone(&settings)));
        let scans = Arc::new(ScanService::new(
            Arc::clone(&db),
            Arc::clone(&settings),
            Arc::clone(&catalog),
            Arc::clone(&accounts),
            Arc::clone(&market),
            Arc::clone(&shell),
            Arc::clone(&log),
            &bus,
        ));
        let memscan = Arc::new(MemscanService::new(Arc::clone(&settings), Arc::clone(&accounts), &bus, stop.clone()));
        let notifications =
            Arc::new(Notifications::new(Arc::clone(&settings), Arc::clone(&world), Arc::clone(&accounts)));
        Ok(Arc::new(App {
            bus,
            db,
            stop,
            settings,
            shell,
            catalog,
            market,
            accounts,
            world,
            log,
            scans,
            memscan,
            notifications,
            tasks: TaskTracker::new(),
            runtime: Handle::current(),
            update: Mutex::new(None),
            tools: Mutex::new((f64::NEG_INFINITY, Value::Null)),
            inventory_watch: Mutex::new(None),
            started: now(),
            port,
        }))
    }

    pub fn settings(&self) -> Settings {
        self.settings.get()
    }

    /// The interface scales the reward cards are looked for at (see `ScanService::ui_scales`).
    pub fn ui_scales(&self) -> Vec<f64> {
        self.scans.ui_scales()
    }

    // ------------------------------------------------------------ lifecycle

    /// Starts the background work: game data, the log, prices, the worldstate, notifications,
    /// profiles, the summary pushed to the UI, and one look for a newer version.
    pub fn start(self: &Arc<Self>) {
        let stop = self.stop.clone();

        let me = Arc::clone(self);
        let quit = stop.clone();
        self.tasks.spawn(async move {
            tokio::select! {
                _ = quit.cancelled() => {}
                _ = me.load_data(false) => {}
            }
        });
        self.tasks.spawn(Arc::clone(&self.market).run(stop.clone()));
        self.tasks.spawn(Arc::clone(&self.world).run(stop.clone()));
        self.tasks.spawn(Arc::clone(&self.notifications).run(stop.clone()));
        let log = Arc::clone(&self.log);
        self.tasks.spawn(Arc::clone(&self.accounts).run_profile_checks(move || log.game_active()));
        let me = Arc::clone(self);
        self.tasks.spawn(self.bus.clone().run_summary(stop.clone(), move || me.summary()));

        let (tx, rx) = mpsc::unbounded_channel();
        self.log.start(tx, stop.clone());
        self.tasks.spawn(Arc::clone(self).dispatch_log(rx));

        // One look at GitHub per start; `--offline` asks nobody.
        if !net::offline() {
            let me = Arc::clone(self);
            let quit = stop.clone();
            self.tasks.spawn(async move {
                let found = tokio::select! {
                    _ = quit.cancelled() => return,
                    found = update::check() => found,
                };
                match found {
                    Ok(found) => {
                        *lock(&me.update) = found;
                        me.bus.state_changed();
                    }
                    Err(e) => eprintln!("could not check for a newer version: {e:#}"),
                }
            });
        }
        self.memscan.at_startup();
        // The first look waits for the log to be read back: an import goes to the account shown.
        self.watch_inventory_file(Duration::from_secs(10));
    }

    /// Stops the background work and waits (briefly) for it to write what it holds.
    pub async fn shutdown(&self, grace: Duration) {
        self.stop.cancel();
        self.tasks.close();
        if tokio::time::timeout(grace, self.tasks.wait()).await.is_err() {
            eprintln!("background work did not stop in time");
        }
        let (market, db) = (Arc::clone(&self.market), Arc::clone(&self.db));
        let _ = blocking(move || {
            market.save_if_dirty();
            ocr::flush_archives(Duration::from_secs(5));
            if let Err(e) = db.checkpoint() {
                eprintln!("could not tidy the database: {e:#}");
            }
        })
        .await;
    }

    /// Downloads (or revalidates) the game data, then warframe.market's item list.
    async fn load_data(self: &Arc<Self>, force: bool) -> Option<String> {
        let failed = self.catalog.load(force).await.err();
        self.market.load_items().await;
        match &failed {
            None => self.accounts.event("Game data loaded", "info"),
            Some(e) => self.accounts.event(e, "warn"),
        }
        failed
    }

    /// The `Data > Reload` button: everything downloaded again.
    pub async fn reload_data(self: &Arc<Self>) -> Result<()> {
        match self.load_data(true).await {
            Some(e) => Err(UserError::new(e).into()),
            None => Ok(()),
        }
    }

    // ------------------------------------------------------------ the log

    /// Hands EE.log's events, in order, to whatever they concern.
    async fn dispatch_log(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<LogEvent>) {
        loop {
            let ev = tokio::select! {
                _ = self.stop.cancelled() => return,
                ev = rx.recv() => match ev {
                    Some(ev) => ev,
                    None => return,
                },
            };
            let me = Arc::clone(&self);
            // Account changes write files: off the async threads, one event after the other.
            if blocking(move || me.on_log(ev)).await.is_err() {
                eprintln!("a log event hit a bug; the log is followed on");
            }
        }
    }

    fn on_log(self: &Arc<Self>, ev: LogEvent) {
        // Every log event changes what the summary shows about the log (its last lines, whether
        // the game is running), and most change an account as well.
        self.bus.state_changed();
        match ev {
            LogEvent::Name(name, live) => {
                self.accounts.on_login(&name, live, self.log.modified());
                if live {
                    // The inventory can be overwritten shortly after it arrives.
                    self.memscan.schedule(Duration::from_secs(3), 2);
                }
            }
            LogEvent::Replayed => {
                self.accounts.follow_game(false, self.log.modified());
                self.memscan.after_replay();
            }
            LogEvent::Account(id, name) => self.accounts.on_account_line(&id, name.as_deref()),
            LogEvent::Candidate(id, sure, line) => self.accounts.on_candidate(id, sure, line),
            LogEvent::Mastery(lo, hi) => self.accounts.on_mastery(lo, hi),
            LogEvent::Duviri(offers) => self.accounts.on_duviri(offers),
            LogEvent::RewardScreen => self.scans.on_reward_screen(),
            LogEvent::RewardScreenClosed => self.scans.on_reward_screen_closed(),
            LogEvent::Sync => self.memscan.schedule(Duration::from_secs(3), 2),
            LogEvent::Active => {}
        }
    }

    // ------------------------------------------------------------ the inventory file

    /// Imports the inventory file set in Settings now (after `delay`) and whenever it changes.
    /// A new path replaces the old watcher; an empty one stops it.
    fn watch_inventory_file(self: &Arc<Self>, delay: Duration) {
        let path = self.settings.get().inventory_json_path;
        let mut current = lock(&self.inventory_watch);
        if let Some(old) = current.take() {
            old.cancel();
        }
        if path.is_empty() {
            return;
        }
        let stop = self.stop.child_token();
        *current = Some(stop.clone());
        let accounts = Arc::clone(&self.accounts);
        let watch = move || {
            std::thread::sleep(delay);
            let path = PathBuf::from(path);
            let wakeups = Wakeups::new(&path);
            let mut seen: Option<SystemTime> = None;
            while !stop.is_cancelled() {
                if let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) {
                    if seen.is_none_or(|s| modified > s) {
                        seen = Some(modified);
                        accounts.import_inventory_file(&path);
                    }
                }
                // A tool may write the file in several pieces: wait for the last one.
                wakeups.wait(INVENTORY_FILE_LOOK, Duration::from_millis(500));
            }
        };
        if let Err(e) = std::thread::Builder::new().name("inventory-file".into()).spawn(watch) {
            self.accounts.event(&format!("Could not watch the inventory file: {e}"), "warn");
        }
    }

    // ------------------------------------------------------------ settings

    /// Applies a patch from the UI and does what the change asks for.
    pub async fn update_settings(self: &Arc<Self>, patch: Value) -> Result<()> {
        let store = Arc::clone(&self.settings);
        let changed = blocking(move || store.update(&patch)).await??;
        let (before, after) = (changed.before, changed.after);
        if after.minimize_to_tray != before.minimize_to_tray {
            self.shell.send(UiEvent::TrayEnabled(after.minimize_to_tray));
        }
        if after.memory_scan_enabled != before.memory_scan_enabled {
            self.memscan.switched(after.memory_scan_enabled);
        }
        if after.inventory_json_path != before.inventory_json_path {
            self.watch_inventory_file(Duration::ZERO);
        }
        if after.screenshot_tool != before.screenshot_tool {
            self.tools_changed();
        }
        if after.platform != before.platform {
            // Another platform's world and prices: fetch the world now; prices follow per page.
            let world = Arc::clone(&self.world);
            tokio::spawn(async move { world.get().await });
        }
        Ok(())
    }

    // ------------------------------------------------------------ the summary

    /// The helper programs may have changed (the bundled OCR unpacked, another screenshot tool
    /// chosen): look again for the next summary.
    pub fn tools_changed(&self) {
        lock(&self.tools).0 = f64::NEG_INFINITY;
        self.bus.state_changed();
    }

    fn tools(&self) -> Value {
        let mut cache = lock(&self.tools);
        if now() - cache.0 > TOOLS_MAX_AGE {
            let t = ocr::detect_tools(self.settings.get().screenshot_tool);
            cache.1 = json!({"tesseract": t.tesseract, "screenshot": t.screenshot, "notify-send": t.notify_send});
            cache.0 = now();
        }
        cache.1.clone()
    }

    /// Everything the shell of the UI shows (`/api/state`, and pushed on every change).
    /// Blocking: walks the catalog and reads the reward history.
    pub fn summary(&self) -> Value {
        let v = self.accounts.view();
        let (mastered, total) = v.mastered_count();
        let inv = v.inv.as_deref();
        let currencies = inv.map_or(json!({}), |i| serde_json::to_value(i.currencies()).unwrap_or_default());
        let log = self.log.path();
        let recent = self.log.recent(8);
        let active = self.log.game_active();
        let a = self.accounts.summary();
        let (screens, best_value) = self.db.reward_totals(self.started, a.active.as_deref()).unwrap_or_default();
        let settings = self.settings.get();
        json!({
            "ready": v.cat.loaded_at.is_some(), "dataError": self.catalog.error(),
            "version": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS, "desktop": self.shell.present(),
            "minimizeToTray": settings.minimize_to_tray,
            "update": lock(&self.update).clone(),
            "account": a.account,
            "accounts": a.accounts,
            "masteryRank": a.mastery_rank,
            // The top of the badge's band (the badge for MR 16-18 is one icon), when that is all we know.
            "masteryRankMax": a.mastery_rank_max,
            "mastered": mastered, "total": total,
            "mastery": v.mastery_progress(),
            "currencies": currencies, "foundry": v.pending_foundry(),
            "collection": v.collection(),
            "pinned": v.pinned(),
            "tasks": tasks::summary(&v),
            "session": {"since": self.started, "screens": screens, "bestValue": best_value},
            "sources": {
                "eeLog": {"path": log, "active": active, "searching": eelog::searching() && log.is_none()},
                "profile": {"fetched": v.profile.as_ref().map(|p| p.fetched),
                            "source": v.profile.as_ref().and_then(|p| p.source.clone()),
                            "error": a.profile_error, "items": v.profile.as_ref().map_or(0, |p| p.xp.len())},
                "inventory": {"source": inv.and_then(|i| i.source.clone()), "imported": inv.map(|i| i.imported),
                              "synced": inv.and_then(|i| i.synced_at), "stacks": inv.map_or(0, |i| i.counts.len())},
                "manual": {"counts": v.manual.counts.len(), "mastered": v.manual.mastered.len()},
                "memscan": self.memscan.status_json(),
                "tools": self.tools(),
                "market": {"error": lock(&self.market.error).clone(), "queued": self.market.pending()},
            },
            "events": a.events,
            "logLines": recent,
        })
    }

    // ------------------------------------------------------------ pages that need several services

    /// Spare prime parts, valued; "source" says whether the counts come from an inventory.
    pub fn trade(&self) -> Value {
        let v = self.accounts.view();
        let source = if v.inv.is_some() { "inventory" } else { "manual" };
        json!({"rows": v.trade(), "source": source})
    }

    /// The open fissures, each with the relics owned for its tier and the best one to bring.
    pub async fn fissures(self: &Arc<Self>) -> Result<Value> {
        let (raw, err) = self.world.get().await;
        let me = Arc::clone(self);
        blocking(move || {
            let mut data = world::fissures(raw.as_deref());
            let picks = me.accounts.view().relic_picks();
            let best_overall = picks
                .values()
                .map(|p| &p["best"])
                .filter(|b| !b.is_null())
                .max_by(|a, b| {
                    let key = |v: &Value| (v["needs"].as_i64().unwrap_or(0), v["ev"].as_f64().unwrap_or(0.0));
                    key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
                })
                .cloned();
            // Omnia fissures take any relic.
            let all = json!({
                "owned": picks.values().map(|p| p["owned"].as_i64().unwrap_or(0)).sum::<i64>(),
                "kinds": picks.values().map(|p| p["kinds"].as_i64().unwrap_or(0)).sum::<i64>(),
                "best": best_overall,
            });
            for f in &mut data {
                let tier = f["tier"].as_str().unwrap_or("").to_string();
                f["relics"] =
                    if tier == "Omnia" { all.clone() } else { picks.get(&tier).cloned().unwrap_or(Value::Null) };
                f["watch"] = json!(world::watch_key(f));
            }
            json!({"fissures": data, "error": err, "watch": me.settings.get().fissure_watch})
        })
        .await
    }

    /// Everything the World page shows.
    pub async fn world_view(self: &Arc<Self>) -> Result<Value> {
        let (raw, err) = self.world.get().await;
        let me = Arc::clone(self);
        blocking(move || {
            let mut w = world::summary(raw.as_deref(), err);
            let view = me.accounts.view();
            world::add_art(&mut w, &view.cat);
            // The schedule is known for years ahead; it does not come with the world state.
            if w.is_object() {
                w["arbitration"] = arbitration::summary(&view.cat, now());
                // What Duviri offers the account shown, as the game last logged it.
                let spiral = raw.as_deref().and_then(world::spiral_began);
                w["duviri"] =
                    me.accounts.duviri_offers().map_or(Value::Null, |o| world::duviri_picks(&view, &o, spiral));
            }
            // Where the account shown stands in the running Nightwave season.
            if let Some(tag) = raw.as_deref().and_then(world::nightwave_tag) {
                let s = standing::summary(&view, me.accounts.active_rank(), Some(&tag));
                let season = s["syndicates"].as_array().and_then(|rows| rows.iter().find(|r| r["kind"] == "nightwave"));
                if let (Some(season), true) = (season, w["nightwave"].is_object()) {
                    w["nightwave"]["mine"] = season.clone();
                }
            }
            w
        })
        .await
    }

    /// Syndicate standing of the account shown.
    pub async fn standing(self: &Arc<Self>) -> Result<Value> {
        let (raw, _) = self.world.get().await;
        let me = Arc::clone(self);
        blocking(move || {
            let tag = raw.as_deref().and_then(world::nightwave_tag);
            let mut s = standing::summary(&me.accounts.view(), me.accounts.active_rank(), tag.as_deref());
            s["account"] = json!(me.accounts.active_name());
            s
        })
        .await
    }

    /// One syndicate with its offerings priced.
    pub fn syndicate(&self, tag: &str) -> Result<Value> {
        standing::detail(&self.accounts.view(), tag, self.accounts.active_rank())
            .ok_or_else(|| UserError::new(format!("No syndicate is called {tag}.")).into())
    }

    /// Everything about one item, part or relic, with which relic farms are open right now.
    pub async fn detail(self: &Arc<Self>, key: String) -> Result<Value> {
        let open = self.world.open_eras().await;
        let me = Arc::clone(self);
        blocking(move || me.detail_with(&key, &open)).await?
    }

    fn detail_with(&self, key: &str, open: &HashSet<String>) -> Result<Value> {
        let view = self.accounts.view();
        let mut v = view.detail(key, open).ok_or_else(|| UserError::new(format!("Nothing is called {key}.")))?;
        let pinned = v["key"].as_str().is_some_and(|k| view.manual.pinned.iter().any(|p| p == k));
        v["pinned"] = json!(pinned);
        // What could go on the farming board from here: the item, or the mod, arcane or resource
        // a drop name stands for.
        let task = if v["kind"] == "item" {
            Some(key.to_string())
        } else if v["kind"] == "drop" {
            key.strip_prefix("lookup:")
                .filter(|uid| tasks::thing(&view, uid).is_some())
                .map(str::to_owned)
                .or_else(|| v["name"].as_str().and_then(|name| tasks::uid_by_name(&view, name)))
        } else {
            None
        };
        v["task"] = task.map_or(Value::Null, |uid| json!({"uid": uid, "onBoard": view.manual.has_task(&uid)}));
        Ok(v)
    }

    /// One task with everything its details show.
    pub async fn task(self: &Arc<Self>, uid: String) -> Result<Value> {
        let open = self.world.open_eras().await;
        let me = Arc::clone(self);
        blocking(move || {
            tasks::detail(&me.accounts.view(), &uid, &open)
                .ok_or_else(|| UserError::new("The farming board does not know that.").into())
        })
        .await?
    }

    pub fn history(&self) -> Value {
        let key = self.accounts.active_key();
        let (screens, best_value) = self.db.reward_totals(self.started, key.as_deref()).unwrap_or_default();
        json!({"entries": self.db.recent_rewards(40, key.as_deref()).unwrap_or_default(),
               "session": {"since": self.started, "screens": screens, "bestValue": best_value}})
    }

    pub fn search(&self, q: &str) -> Value {
        let q = q.trim().to_lowercase();
        if q.chars().count() < 2 {
            return json!([]);
        }
        Value::Array(
            self.market
                .search(&q)
                .into_iter()
                .map(|(slug, name, ducats)| {
                    self.market.request(&slug, true);
                    let p = self.market.cached(&slug).unwrap_or_default();
                    let image = self.market.image(Some(&slug));
                    json!({"slug": slug, "name": name, "ducats": ducats, "sell": p.sell, "buy": p.buy, "image": image})
                })
                .collect(),
        )
    }

    /// Every weapon that takes a riven, with its traded riven prices.
    pub async fn rivens(self: &Arc<Self>) -> Result<Value> {
        let (prices, err) = rivens::market(self.settings.get().platform).await;
        let cat = self.catalog.get();
        let rows = blocking(move || rivens::rows(&cat, &prices)).await?;
        Ok(json!({"rows": rows, "error": err}))
    }

    pub fn riven_table(&self, weapon: &str, positives: u8, negatives: u8) -> Result<Value> {
        let cat = self.catalog.get();
        let it = cat
            .item_by_name(weapon)
            .filter(|i| i.disposition.is_some_and(|d| d != 0.0))
            .ok_or_else(|| UserError::new("No riven data for that weapon."))?;
        if rivens::multipliers(positives, negatives).is_none() {
            return Err(UserError::new("A riven has 2-3 positives and 0-1 negatives.").into());
        }
        Ok(json!({"weapon": it.name, "class": it.riven_class, "disposition": it.disposition.map(|d| py_round(d, 3)),
                  "dots": it.dispo_dots, "positives": positives, "negatives": negatives,
                  "stats": rivens::stat_table(it, positives, negatives)}))
    }

    pub fn grade_riven(&self, body: &Value) -> Result<Value> {
        let cat = self.catalog.get();
        let it = cat
            .item_by_name(body["weapon"].as_str().unwrap_or(""))
            .filter(|i| i.disposition.is_some_and(|d| d != 0.0))
            .ok_or_else(|| UserError::new("No riven data for that weapon."))?;
        // A missing count means the common 3+1, but a legitimate 0 negatives must stay 0, or a
        // 3+0 riven is graded against the wrong multiplier.
        let pos = body["positives"].as_u64().map_or(3, |n| n.min(u8::MAX as u64) as u8);
        let neg = body["negatives"].as_u64().map_or(1, |n| n.min(u8::MAX as u64) as u8);
        let graded: Vec<Graded> = body["stats"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|st| {
                rivens::grade_roll(
                    it,
                    st["stat"].as_str().unwrap_or(""),
                    st["value"].as_f64().unwrap_or(0.0),
                    pos,
                    neg,
                    st["negative"].as_bool().unwrap_or(false),
                )
            })
            .collect();
        Ok(json!({"weapon": it.name, "disposition": it.disposition.map(|d| py_round(d, 3)), "dots": it.dispo_dots,
                  "stats": graded}))
    }

    pub fn rewards(&self) -> Option<Arc<RewardResult>> {
        self.scans.rewards()
    }

    pub fn riven(&self) -> Option<Arc<RivenResult>> {
        self.scans.riven()
    }

    // ------------------------------------------------------------ the window and the tray

    /// The desktop shell is up: from now on it gets the engine's requests. False when the app is
    /// quitting already; the shell then closes at once.
    #[must_use]
    pub fn set_ui(&self, tx: async_channel::Sender<UiEvent>) -> bool {
        let attached = self.shell.attach(tx, &self.stop);
        self.bus.state_changed();
        attached
    }

    pub fn show_window(&self) {
        self.shell.send(UiEvent::ShowWindow);
    }

    pub fn hide_window(&self) {
        self.shell.send(UiEvent::HideWindow);
    }

    pub fn toggle_window(&self) {
        self.shell.send(UiEvent::ToggleWindow);
    }

    /// Ends the app: the window closes (when there is one) and `main` shuts the engine down,
    /// finishing what it was writing. The request that asked for it is still answered.
    pub fn quit(&self) {
        self.shell.quit(&self.stop);
    }

    pub fn test_overlay(&self) -> Result<()> {
        if !self.shell.present() {
            return Err(UserError::new("The overlay only exists in the desktop app.").into());
        }
        self.shell.send(UiEvent::TestOverlay);
        Ok(())
    }

    /// Scans the reward screen without waiting for it (the tray's menu, from any thread).
    pub fn scan_rewards_later(self: &Arc<Self>) {
        let scans = Arc::clone(&self.scans);
        self.runtime.spawn(async move { scans.scan_rewards().await });
    }

    /// Grades the riven on screen without waiting for it (the tray's menu, from any thread).
    pub fn scan_riven_later(self: &Arc<Self>) {
        let scans = Arc::clone(&self.scans);
        self.runtime.spawn(async move { scans.scan_riven().await });
    }

    // ------------------------------------------------------------ hotkeys

    /// The port hotkeys call and the keys. The compositor's binds are kept in its config and
    /// outlive this run, so they call the port of the settings (which the next start binds); the
    /// keys Windows lends the app last only this run, so they call the port bound now.
    fn hotkey_keys(&self) -> (u16, [(String, &'static str); 3]) {
        let s = self.settings.get();
        let port = if cfg!(windows) { self.port } else { s.port };
        (port, [(s.hotkey_rewards, "rewards/scan"), (s.hotkey_riven, "riven/scan"), (s.hotkey_window, "window/toggle")])
    }

    pub fn hotkeys(&self) -> Value {
        let flavour = compositor::detect_flavour();
        let (port, keys) = self.hotkey_keys();
        let pairs: Vec<(&str, &str)> = keys.iter().map(|(k, p)| (k.as_str(), *p)).collect();
        json!({"compositor": flavour.map(|f| f.label()), "lines": compositor::hotkey_lines(flavour, port, &pairs),
               "note": compositor::hotkey_note(flavour)})
    }

    /// Windows: the app holds the keys itself (`compositor::register_hotkeys`), from startup on.
    #[cfg(windows)]
    pub fn install_hotkeys(&self) -> Result<Value> {
        let (port, keys) = self.hotkey_keys();
        let pairs: Vec<(&str, &str)> = keys.iter().map(|(k, p)| (k.as_str(), *p)).collect();
        let lines = compositor::register_hotkeys(port, &pairs)?;
        let names: Vec<&str> = pairs.iter().map(|(k, _)| *k).filter(|k| !k.trim().is_empty()).collect();
        self.accounts.event(&format!("Hotkeys registered: {}", names.join(", ")), "info");
        Ok(json!({"file": null, "sourced": false, "lines": lines}))
    }

    #[cfg(unix)]
    pub fn install_hotkeys(&self) -> Result<Value> {
        let flavour = compositor::detect_flavour();
        let (port, keys) = self.hotkey_keys();
        let pairs: Vec<(&str, &str)> = keys.iter().map(|(k, p)| (k.as_str(), *p)).collect();
        let lines = compositor::hotkey_lines(flavour, port, &pairs);
        let done = compositor::install_hotkeys(flavour, &lines)?;
        let name = done.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        self.accounts.event(&format!("Hotkeys written to {name} and the compositor reloaded"), "info");
        Ok(json!({"file": done.file, "sourced": done.sourced, "lines": lines}))
    }

    /// The event bus for the API's event stream.
    pub fn events(&self) -> &Bus {
        &self.bus
    }
}
