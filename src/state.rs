//! The account as the app sees it: every source merged, the scans, and the background work.
//! What the UI shows is computed in `views.rs` from a snapshot (`State::view`).

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::Duration;

use anyhow::Result;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{self, Settings};
use crate::difflib::py_round;
use crate::eelog::{self, LogEvent, LogStatus, LogWatcher};
use crate::gamedata::{self, Catalog};
use crate::market::Market;
use crate::memscan::{self, ScanError};
use crate::notifier::{self, Notifier};
use crate::ocr::{self, matcher::Names, screenshot};
use crate::rivens::{self, Graded};
use crate::sources::{self, Inventory, Profile};
use crate::views::View;
use crate::{UserError, compositor, history, now, world};

/// DE temporarily IP-blocks game login for clients that query profiles too often.
pub const PROFILE_COOLDOWN: f64 = 300.0;
const DUCAT_UID: &str = "/Lotus/Types/Items/MiscItems/PrimeBucks";
/// A reward scan started this recently already covers the screen: the F9 hotkey and the
/// automatic trigger routinely fire for the same reward screen within a second of each other.
const COALESCE: f64 = 3.0;
/// Memory scans run at most this often, retries excepted.
const MEMSCAN_GAP: f64 = 90.0;

fn data_file(name: &str) -> PathBuf {
    config::dirs().data.join(name)
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Manual {
    /// uid -> (count, when it was set)
    #[serde(default)]
    pub counts: IndexMap<String, (i64, f64)>,
    /// uid -> when it was marked
    #[serde(default)]
    pub mastered: IndexMap<String, f64>,
    /// Pinned to the Overview: item, part or build uids, and relic names.
    #[serde(default)]
    pub pinned: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Account {
    pub id: Option<String>,
    pub name: Option<String>,
    pub source: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub t: f64,
    pub text: String,
    pub kind: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct Evidence {
    pub id: String,
    pub line: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RewardItem {
    pub name: String,
    pub uid: String,
    pub score: f64,
    pub slug: Option<String>,
    pub have: i64,
    pub needed: i64,
    pub vaulted: Option<bool>,
    pub ducats: Option<i64>,
    pub price: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RewardResult {
    pub t: f64,
    pub items: Vec<RewardItem>,
    pub best: Option<String>,
    pub ocr: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RivenResult {
    pub t: f64,
    pub weapon: Option<String>,
    pub disposition: Option<f64>,
    pub dots: Option<i64>,
    pub positives: Option<u8>,
    pub negatives: Option<u8>,
    /// Stats the card has but OCR did not read (their count is inferred from the values).
    pub unread: u8,
    pub stats: Vec<Graded>,
    pub ocr: String,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct MemscanStatus {
    /// "idle" (nothing read yet), "scanning", "ok" or "error"; the summary adds "off" and
    /// "unavailable".
    pub state: &'static str,
    /// Why the last attempt read nothing, or what went wrong.
    pub detail: Option<String>,
    pub progress: f64,
    /// When the next read is due (epoch seconds), if one is scheduled.
    pub next: Option<f64>,
    /// The last successful read: {t, stacks, seconds}.
    pub last: Option<Value>,
    #[serde(skip)]
    last_attempt: f64,
    /// How many sections the capture behind the current inventory had.
    #[serde(skip)]
    last_keys: usize,
    /// The game process that refused access; it is not asked again, a restarted game is.
    #[serde(skip)]
    denied_pid: Option<u32>,
}

/// What the engine asks of the desktop shell, if one is running.
pub enum UiEvent {
    ShowWindow,
    /// New rewards; `fresh` is false when only the prices changed.
    Rewards {
        result: Arc<RewardResult>,
        fresh: bool,
    },
    Riven(Arc<RivenResult>),
    TestOverlay,
    /// Hide the overlays before a screenshot, so a scan never reads our own text; acknowledged
    /// on the sender once they are gone.
    HideForCapture(std::sync::mpsc::Sender<()>),
    Quit,
}

#[derive(Default)]
struct Data {
    profile: Option<Arc<Profile>>,
    inventory: Option<Arc<Inventory>>,
    manual: Manual,
    account: Account,
    /// The name from this session's "Logged in <name>".
    log_name: Option<String>,
    /// (low, high) from the local player's mastery badge.
    log_mastery: Option<(u32, Option<u32>)>,
    /// Log lines that may hold the account id, for the user to confirm.
    evidence: Vec<Evidence>,
    events: VecDeque<Event>,
    profile_error: Option<String>,
    last_profile_try: f64,
    profile_timer: bool,
    inventory_mtime: Option<std::time::SystemTime>,
}

pub struct State {
    pub settings: Arc<RwLock<Settings>>,
    catalog: RwLock<Arc<Catalog>>,
    catalog_error: Mutex<Option<String>>,
    names: Mutex<Option<(Arc<Catalog>, Arc<Names>)>>,
    pub market: Arc<Market>,
    data: Mutex<Data>,
    rewards: Mutex<Option<Arc<RewardResult>>>,
    riven: Mutex<Option<Arc<RivenResult>>>,
    memscan: Mutex<MemscanStatus>,
    /// One capture and OCR at a time, rewards and rivens alike.
    scan_lock: Mutex<()>,
    reward_scan: Mutex<RewardScan>,
    reward_done: std::sync::Condvar,
    log: Mutex<Option<Arc<Mutex<LogStatus>>>>,
    ui: Mutex<Option<async_channel::Sender<UiEvent>>>,
    tools: Mutex<(f64, Value)>,
    notifier: Mutex<Notifier>,
    /// When this run of the app started: the reward session counts from here.
    started: f64,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn same_name(a: &str, b: &str) -> bool {
    let clean = |s: &str| eelog::strip_glyphs(s).to_lowercase();
    !a.is_empty() && !b.is_empty() && clean(a) == clean(b)
}

fn tail(s: &str, n: usize) -> String {
    let count = s.chars().count();
    s.chars().skip(count.saturating_sub(n)).collect()
}

impl State {
    pub fn new(settings: Settings) -> Arc<State> {
        let settings = Arc::new(RwLock::new(settings));
        let saved: Value = config::read_json(&data_file("account.json")).unwrap_or(Value::Null);
        let override_id = settings.read().unwrap_or_else(|e| e.into_inner()).account_id.clone();
        let account = if !override_id.is_empty() {
            Account {
                id: Some(override_id),
                name: saved["name"].as_str().map(str::to_owned),
                source: Some("Settings".into()),
            }
        } else {
            let id = saved["id"].as_str().map(str::to_owned);
            Account {
                source: id.as_ref().map(|_| "saved".to_string()),
                id,
                name: saved["name"].as_str().map(str::to_owned),
            }
        };
        let data = Data {
            profile: config::read_json::<Profile>(&data_file("profile.json")).map(Arc::new),
            inventory: config::read_json::<Inventory>(&data_file("inventory.json")).map(Arc::new),
            manual: config::read_json(&data_file("manual.json")).unwrap_or_default(),
            account,
            last_profile_try: f64::NEG_INFINITY,
            ..Data::default()
        };
        Arc::new(State {
            market: Market::new(Arc::clone(&settings)),
            settings,
            catalog: RwLock::new(Arc::new(Catalog::default())),
            catalog_error: Mutex::new(None),
            names: Mutex::new(None),
            data: Mutex::new(data),
            rewards: Mutex::new(None),
            riven: Mutex::new(None),
            memscan: Mutex::new(MemscanStatus { state: "idle", last_attempt: f64::NEG_INFINITY, ..Default::default() }),
            scan_lock: Mutex::new(()),
            reward_scan: Mutex::new(RewardScan { started: f64::NEG_INFINITY, done: true }),
            reward_done: std::sync::Condvar::new(),
            log: Mutex::new(None),
            ui: Mutex::new(None),
            tools: Mutex::new((f64::NEG_INFINITY, Value::Null)),
            notifier: Mutex::new(Notifier::default()),
            started: now(),
        })
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        Arc::clone(&self.catalog.read().unwrap_or_else(|e| e.into_inner()))
    }

    pub fn catalog_error(&self) -> Option<String> {
        lock(&self.catalog_error).clone()
    }

    pub fn set_ui(&self, tx: async_channel::Sender<UiEvent>) {
        *lock(&self.ui) = Some(tx);
    }

    pub fn has_ui(&self) -> bool {
        lock(&self.ui).is_some()
    }

    fn ui(&self, ev: UiEvent) {
        if let Some(tx) = lock(&self.ui).as_ref() {
            let _ = tx.send_blocking(ev);
        }
    }

    // ------------------------------------------------------------ lifecycle

    /// Starts the background work: game data, the log watcher, and the slow periodic checks.
    pub fn start(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::thread::Builder::new().name("gamedata".into()).spawn(move || me.load_static(false)).ok();

        let paths = Arc::clone(self);
        let events = Arc::clone(self);
        let status = LogWatcher::spawn(Box::new(move || paths.log_path()), Box::new(move |ev| events.on_log(ev)));
        *lock(&self.log) = Some(status);

        let me = Arc::clone(self);
        std::thread::Builder::new().name("background".into()).spawn(move || me.background()).ok();
        // The game may already be running and synced.
        if memscan::find_game_pid().is_some() {
            self.schedule_memscan(Duration::from_secs(20), 1);
        }
    }

    pub fn load_static(&self, force: bool) {
        match gamedata::load(force) {
            Ok(c) => {
                *self.catalog.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(c);
                *lock(&self.catalog_error) = None;
            }
            Err(e) => *lock(&self.catalog_error) = Some(format!("{e:#}")),
        }
        self.market.load_items();
        match self.catalog_error() {
            None => self.event("Game data loaded", "info"),
            Some(e) => self.event(&e, "warn"),
        }
    }

    pub fn log_path(&self) -> Option<PathBuf> {
        let configured = self.settings().ee_log_path;
        if configured.is_empty() { eelog::find_ee_log() } else { Some(PathBuf::from(configured)) }
    }

    fn log_status(&self) -> Option<Arc<Mutex<LogStatus>>> {
        lock(&self.log).clone()
    }

    fn game_active(&self) -> bool {
        self.log_status().is_some_and(|s| lock(&s).game_active())
    }

    pub fn event(&self, text: &str, kind: &'static str) {
        let mut d = lock(&self.data);
        d.events.push_back(Event { t: now(), text: text.to_string(), kind });
        while d.events.len() > 60 {
            d.events.pop_front();
        }
    }

    fn background(self: Arc<Self>) {
        let mut first = true;
        let mut tick = 0u64;
        loop {
            self.watch_inventory_file();
            if tick % 3 == 0 {
                self.notify_tick();
            }
            tick += 1;
            let s = self.settings();
            let interval = s.profile_refresh_minutes.max(10) as f64 * 60.0;
            let (has_id, stale) = {
                let d = lock(&self.data);
                (d.account.id.is_some(), d.profile.as_ref().is_none_or(|p| now() - p.fetched > interval))
            };
            if has_id && stale && (first || (s.profile_auto_refresh && self.game_active())) {
                first = false;
                let _ = self.refresh_profile(true);
            }
            std::thread::sleep(Duration::from_secs(20));
        }
    }

    // ------------------------------------------------------------ the log

    fn on_log(self: &Arc<Self>, ev: LogEvent) {
        match ev {
            LogEvent::Name(n) => {
                self.on_name(n);
                // Logged in: the inventory arrives a little later.
                self.schedule_memscan(Duration::from_secs(45), 2);
            }
            LogEvent::Account(acc, name) => {
                if self.settings().account_id.is_empty() {
                    let name = name.or_else(|| lock(&self.data).log_name.clone());
                    self.confirm(&acc, name, "EE.log");
                }
            }
            LogEvent::Candidate(id, sure, line) => self.on_candidate(id, sure, line),
            LogEvent::Mastery(lo, hi) => lock(&self.data).log_mastery = Some((lo, hi)),
            LogEvent::RewardScreen => {
                if self.settings().ocr_enabled {
                    let me = Arc::clone(self);
                    std::thread::spawn(move || {
                        me.scan_rewards(Duration::from_millis(600));
                    });
                }
            }
            LogEvent::Sync => self.schedule_memscan(Duration::from_secs(8), 2),
        }
    }

    fn on_name(&self, name: String) {
        let mut d = lock(&self.data);
        d.log_name = Some(name.clone());
        if !self.settings().account_id.is_empty() {
            return;
        }
        let saved = d.account.name.clone();
        match saved {
            Some(saved) if d.account.id.is_some() && !same_name(&saved, &name) => {
                d.account = Account { id: None, name: Some(name.clone()), source: None };
                drop(d);
                self.event(&format!("Logged in as {name}, not {saved}; looking for this account's id"), "warn");
            }
            None => d.account.name = Some(name),
            _ => {}
        }
    }

    /// Never queries DE: a sure line links the account, anything else is shown to the user.
    fn on_candidate(self: &Arc<Self>, id: String, sure: bool, line: String) {
        if !self.settings().account_id.is_empty() {
            return;
        }
        let mut d = lock(&self.data);
        if d.account.id.as_deref() == Some(id.as_str()) {
            return;
        }
        if sure && d.account.id.is_none() {
            let name = d.log_name.clone();
            drop(d);
            self.confirm(&id, name, "EE.log");
            return;
        }
        if d.evidence.iter().all(|e| e.id != id) {
            d.evidence.push(Evidence { id, line });
            let excess = d.evidence.len().saturating_sub(12);
            d.evidence.drain(..excess);
        }
    }

    /// Reads the inventory from game memory after `delay`, if enabled and allowed. The game only
    /// holds the whole inventory text for a while after a sync, so this runs on events (login,
    /// mission end), never on a timer; `retries` more attempts follow when it is not there yet.
    fn schedule_memscan(self: &Arc<Self>, delay: Duration, retries: u8) {
        if !self.settings().memory_scan_enabled || memscan::availability().is_err() {
            return;
        }
        let denied = lock(&self.memscan).denied_pid;
        if denied.is_some() && memscan::find_game_pid() == denied {
            return;
        }
        let due = now() + delay.as_secs_f64();
        {
            let mut st = lock(&self.memscan);
            if st.next.is_some_and(|p| p <= due) {
                return; // an earlier scan is already on its way
            }
            st.next = Some(due);
        }
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("memscan".into())
            .spawn(move || {
                std::thread::sleep(delay);
                {
                    let mut st = lock(&me.memscan);
                    if st.next != Some(due) {
                        return; // superseded by an earlier one
                    }
                    st.next = None;
                }
                me.memscan_now(retries);
            })
            .ok();
    }

    fn memscan_now(self: &Arc<Self>, retries: u8) {
        if !self.settings().memory_scan_enabled {
            return;
        }
        let pid = memscan::find_game_pid();
        {
            let mut st = lock(&self.memscan);
            // One scan at a time, and the game's memory is not combed again within the gap.
            if st.state == "scanning"
                || now() - st.last_attempt < MEMSCAN_GAP
                || (pid.is_some() && pid == st.denied_pid)
            {
                return;
            }
            st.state = "scanning";
            st.progress = 0.0;
            st.last_attempt = now();
        }
        let result = match pid {
            Some(pid) => memscan::scan_inventory(pid, |p| lock(&self.memscan).progress = py_round(p, 3)),
            None => Err(ScanError::NotRunning),
        };
        let retry = match result {
            Ok((obj, info)) => self.apply_capture(&obj, info.seconds),
            Err(e) => {
                let mut st = lock(&self.memscan);
                st.detail = Some(e.to_string());
                if matches!(e, ScanError::Denied(_)) {
                    st.denied_pid = pid;
                }
                // Nothing to read is not a failure: the last good read still stands.
                st.state = match e {
                    ScanError::NotRunning | ScanError::NotFound { .. } if st.last.is_some() => "ok",
                    ScanError::NotRunning | ScanError::NotFound { .. } => "idle",
                    ScanError::Denied(_) | ScanError::Io(_) => "error",
                };
                matches!(e, ScanError::NotFound { .. })
            }
        };
        if retry && retries > 0 {
            // A retry is not held to the gap: the sync that asked for it is still fresh.
            lock(&self.memscan).last_attempt = f64::NEG_INFINITY;
            self.schedule_memscan(Duration::from_secs(if retries > 1 { 30 } else { 90 }), retries - 1);
        }
    }

    /// Applies a memory capture unless it is cut short or already applied. True asks for a retry.
    fn apply_capture(&self, obj: &Value, seconds: f64) -> bool {
        let file = data_file("memscan_sections.json");
        let mut sections: memscan::Sections = config::read_json(&file).unwrap_or_default();
        let missing = sections.missing(obj);
        if !missing.is_empty() && !sections.refuse() {
            let _ = config::write_json(&file, &sections);
            let mut st = lock(&self.memscan);
            st.state = if st.last.is_some() { "ok" } else { "idle" };
            st.detail = Some(format!(
                "The game held only part of the inventory just now ({} missing), so the last complete one is kept.",
                missing.join(", ")
            ));
            return true;
        }
        // The same sync, read again with nothing new in it: leave the inventory and the log alone.
        let keys = obj.as_object().map_or(0, |o| o.len());
        let sync = obj["LastInventorySync"].as_f64();
        let current = lock(&self.data).inventory.as_ref().and_then(|i| i.synced_at);
        {
            let mut st = lock(&self.memscan);
            if sync.is_some() && sync == current && keys <= st.last_keys {
                st.state = "ok";
                st.detail = None;
                return false;
            }
        }
        let imported = self.import_inventory(obj, "memory scan");
        let mut st = lock(&self.memscan);
        match imported {
            Ok(inv) => {
                sections.learn(obj);
                let _ = config::write_json(&file, &sections);
                st.state = "ok";
                st.detail = None;
                st.last_keys = keys;
                st.last = Some(json!({"t": now(), "stacks": inv.counts.len(), "seconds": seconds}));
            }
            Err(e) => {
                st.state = "error";
                st.detail = Some(e.to_string());
            }
        }
        false
    }

    fn confirm(self: &Arc<Self>, id: &str, name: Option<String>, source: &str) {
        let name = name.map(|n| eelog::strip_glyphs(&n)).filter(|n| !n.is_empty());
        let changed = {
            let mut d = lock(&self.data);
            let changed = d.account.id.as_deref() != Some(id);
            d.account.id = Some(id.to_string());
            if name.is_some() {
                d.account.name = name;
            }
            d.account.source = Some(source.to_string());
            let _ = config::write_json(&data_file("account.json"), &json!({"id": id, "name": d.account.name}));
            changed
        };
        if changed {
            let shown = lock(&self.data).account.name.clone().unwrap_or_else(|| id.to_string());
            self.event(&format!("Account linked: {shown}"), "info");
            self.request_profile();
        }
    }

    /// Loads the profile as soon as DE's rate limit allows; never more than one pending load.
    fn request_profile(self: &Arc<Self>) {
        let wait = {
            let d = lock(&self.data);
            PROFILE_COOLDOWN - (now() - d.last_profile_try)
        };
        let me = Arc::clone(self);
        if wait <= 0.0 {
            std::thread::spawn(move || {
                let _ = me.refresh_profile(true);
            });
            return;
        }
        {
            let mut d = lock(&self.data);
            if d.profile_timer {
                return;
            }
            d.profile_timer = true;
        }
        self.event(
            &format!("Loading your profile in {} min (pausing for DE's rate limit)", (wait / 60.0) as i64 + 1),
            "info",
        );
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(wait + 1.0));
            lock(&me.data).profile_timer = false;
            let _ = me.refresh_profile(true);
        });
    }

    // ------------------------------------------------------------ sources

    pub fn refresh_profile(&self, quiet: bool) -> Result<Option<Arc<Profile>>> {
        let s = self.settings();
        let acc =
            if s.account_id.is_empty() { lock(&self.data).account.id.clone() } else { Some(s.account_id.clone()) };
        let Some(acc) = acc else {
            return Err(UserError::new(
                "No account id yet. Start Warframe once (it is read from EE.log) or enter it in Settings.",
            )
            .into());
        };
        {
            let mut d = lock(&self.data);
            let wait = PROFILE_COOLDOWN - (now() - d.last_profile_try);
            if wait > 0.0 {
                return Err(UserError::new(format!(
                    "To stay clear of DE's rate limit, which can block your IP from logging in, profile requests are \
                     limited to one every 5 minutes. Try again in {}:{:02}.",
                    (wait / 60.0) as i64,
                    (wait % 60.0) as i64
                ))
                .into());
            }
            d.last_profile_try = now();
        }
        let prof = match sources::fetch_profile(&acc, s.platform) {
            Ok(p) => p,
            Err(e) => {
                let msg = e.to_string();
                lock(&self.data).profile_error = Some(msg.clone());
                if !quiet {
                    return Err(e);
                }
                self.event(&msg, "warn");
                return Ok(None);
            }
        };
        let mut d = lock(&self.data);
        d.profile_error = None;
        if let (Some(log_name), Some(prof_name)) = (d.log_name.clone(), prof.name.clone()) {
            if !same_name(&prof_name, &log_name) && s.account_id.is_empty() {
                let shown = eelog::strip_glyphs(&prof_name);
                d.account.id = None;
                d.account.source = None;
                d.profile_error = Some(format!("The linked id belonged to {shown}. Pick or paste your own id."));
                let _ = config::write_json(&data_file("account.json"), &json!({"id": null, "name": log_name}));
                drop(d);
                self.event(&format!("That id belongs to {shown}, not {log_name}; unlinked it"), "warn");
                return Ok(None);
            }
        }
        drop(d);
        Ok(Some(self.set_profile(prof, "DE public profile")))
    }

    fn set_profile(&self, mut prof: Profile, label: &str) -> Arc<Profile> {
        prof.source = Some(label.to_string());
        let prof = Arc::new(prof);
        {
            let mut d = lock(&self.data);
            d.profile = Some(Arc::clone(&prof));
            let _ = config::write_json(&data_file("profile.json"), &*prof);
            if let Some(n) = prof.name.as_deref().map(eelog::strip_glyphs).filter(|n| !n.is_empty()) {
                d.account.name = Some(n);
            }
        }
        self.event(&format!("Profile updated: {} items with mastery XP", prof.xp.len()), "info");
        prof
    }

    pub fn import_profile(&self, data: &Value) -> Result<()> {
        self.set_profile(sources::parse_profile(data)?, "Imported profile file");
        Ok(())
    }

    pub fn import_inventory(&self, data: &Value, label: &str) -> Result<Arc<Inventory>> {
        let mut inv = sources::parse_inventory(data)?;
        inv.source = Some(label.to_string());
        let inv = Arc::new(inv);
        config::write_json(&data_file("inventory.json"), &*inv)?;
        lock(&self.data).inventory = Some(Arc::clone(&inv));
        let equipment: i64 = inv.equipment.values().map(|e| e.count).sum();
        self.event(
            &format!("Inventory loaded from {label}: {} item stacks, {equipment} equipment", inv.counts.len()),
            "info",
        );
        Ok(inv)
    }

    pub fn clear_inventory(&self) {
        lock(&self.data).inventory = None;
        let _ = fs::remove_file(data_file("inventory.json"));
    }

    fn watch_inventory_file(&self) {
        let path = self.settings().inventory_json_path;
        if path.is_empty() {
            return;
        }
        let Ok(modified) = fs::metadata(&path).and_then(|m| m.modified()) else { return };
        {
            let mut d = lock(&self.data);
            if d.inventory_mtime.is_some_and(|m| modified <= m) {
                return;
            }
            d.inventory_mtime = Some(modified);
        }
        let label = Path::new(&path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or(path.clone());
        match config::read_json::<Value>(Path::new(&path)) {
            None => self.event(&format!("Could not read {path} as JSON"), "warn"),
            Some(v) => {
                if let Err(e) = self.import_inventory(&v, &label) {
                    self.event(&e.to_string(), "warn");
                }
            }
        }
    }

    pub fn set_manual_count(&self, uid: &str, count: Option<i64>) -> Result<()> {
        let mut d = lock(&self.data);
        match count {
            None => {
                d.manual.counts.shift_remove(uid);
            }
            Some(c) => {
                d.manual.counts.insert(uid.to_string(), (c.max(0), now()));
            }
        }
        config::write_json(&data_file("manual.json"), &d.manual)
    }

    pub fn set_manual_mastered(&self, uid: &str, mastered: bool) -> Result<()> {
        let mut d = lock(&self.data);
        if mastered {
            d.manual.mastered.insert(uid.to_string(), now());
        } else {
            d.manual.mastered.shift_remove(uid);
        }
        config::write_json(&data_file("manual.json"), &d.manual)
    }

    pub fn set_pinned(&self, key: &str, pinned: bool) -> Result<()> {
        let mut d = lock(&self.data);
        d.manual.pinned.retain(|k| k != key);
        if pinned {
            d.manual.pinned.push(key.to_string());
        }
        config::write_json(&data_file("manual.json"), &d.manual)
    }

    pub fn set_account_id(self: &Arc<Self>, raw: &str) -> Result<()> {
        let acc = raw.trim().trim_matches('"').to_lowercase();
        if acc.len() != 24 || !acc.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(UserError::new(
                "That isn't an account id: it should be exactly 24 characters of 0-9 and a-f, like 505875fb1a4d80894a000000.",
            )
            .into());
        }
        let name = {
            let d = lock(&self.data);
            d.log_name.clone().or_else(|| d.account.name.clone())
        };
        self.confirm(&acc, name, "entered by you");
        Ok(())
    }

    pub fn update_settings(self: &Arc<Self>, patch: &Value) -> Result<()> {
        let updated = self.settings().patched(patch)?;
        // A --port from the command line is for this run only: keep the file's own port unless the
        // patch sets one, or any saved setting would make the override permanent.
        let mut to_save = updated.clone();
        if patch.get("port").is_none() {
            to_save.port = Settings::load().port;
        }
        to_save.save()?;
        let scan_now = updated.memory_scan_enabled && !self.settings().memory_scan_enabled;
        if scan_now {
            // Switching it on reads right away, even after a refusal (the system may allow it now).
            let mut st = lock(&self.memscan);
            st.denied_pid = None;
            st.last_attempt = f64::NEG_INFINITY;
        }
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = updated;
        if let Some(id) = patch["account_id"].as_str().filter(|s| !s.is_empty()) {
            let mut d = lock(&self.data);
            d.account.id = Some(id.to_string());
            d.account.source = Some("Settings".into());
        }
        if scan_now && memscan::find_game_pid().is_some() {
            self.schedule_memscan(Duration::from_secs(3), 1);
        }
        Ok(())
    }

    // ------------------------------------------------------------ views

    /// An immutable picture of everything the views read, so no lock is held while computing.
    pub fn view(&self) -> View {
        let d = lock(&self.data);
        let inv = d.inventory.clone();
        let foundry =
            inv.as_ref().map_or_else(HashSet::new, |i| i.foundry.iter().filter_map(|f| f.recipe.clone()).collect());
        View {
            cat: self.catalog(),
            profile: d.profile.clone(),
            inv,
            manual: Arc::new(d.manual.clone()),
            market: Arc::clone(&self.market),
            foundry,
        }
    }

    fn tools(&self) -> Value {
        let mut cache = lock(&self.tools);
        if now() - cache.0 > 30.0 {
            let t = ocr::detect_tools(self.settings().screenshot_tool);
            cache.1 = json!({"tesseract": t.tesseract, "screenshot": t.screenshot, "notify-send": t.notify_send});
            cache.0 = now();
        }
        cache.1.clone()
    }

    fn memscan_status(&self) -> Value {
        let mut st = serde_json::to_value(&*lock(&self.memscan)).unwrap_or_default();
        if !self.settings().memory_scan_enabled {
            st["state"] = json!("off");
        } else if let Err(why) = memscan::availability() {
            st["state"] = json!("unavailable");
            st["detail"] = json!(why);
        }
        st
    }

    pub fn summary(&self) -> Value {
        let v = self.view();
        let (mastered, total) = v.mastered_count();
        let inv = v.inv.as_deref();
        let mut currencies = inv.map_or(json!({}), |i| serde_json::to_value(&i.currencies).unwrap_or_default());
        if let Some(i) = inv {
            currencies["ducats"] = json!(i.counts.get(DUCAT_UID));
        }
        let log = self.log_path();
        let (screens, best_value) = history::totals(self.started);
        let d = lock(&self.data);
        // DE's profile never has it and a scanned inventory rarely does; the log's badge always.
        let mastery_rank = d
            .profile
            .as_ref()
            .and_then(|p| p.mastery_rank)
            .or_else(|| inv.and_then(|i| i.mastery_rank))
            .or(d.log_mastery.map(|m| m.0 as i64));
        let (recent, active) = match self.log_status() {
            Some(s) => {
                let s = lock(&s);
                (s.recent.iter().rev().take(8).rev().cloned().collect::<Vec<_>>(), s.game_active())
            }
            None => (Vec::new(), false),
        };
        json!({
            "ready": v.cat.loaded_at.is_some(), "dataError": self.catalog_error(),
            "version": env!("CARGO_PKG_VERSION"), "desktop": self.has_ui(),
            "account": {"id": d.account.id, "name": d.account.name, "source": d.account.source,
                        "logName": d.log_name, "evidence": d.evidence},
            "masteryRank": mastery_rank,
            // The top of the badge's band (the badge for MR 16-18 is one icon), when that is all we know.
            "masteryRankMax": if d.profile.as_ref().and_then(|p| p.mastery_rank).is_none()
                && inv.and_then(|i| i.mastery_rank).is_none()
            {
                d.log_mastery.and_then(|m| m.1)
            } else {
                None
            },
            "mastered": mastered, "total": total,
            "mastery": v.mastery_progress(),
            "currencies": currencies, "foundry": v.pending_foundry(),
            "collection": v.collection(),
            "pinned": v.pinned(),
            "session": {"since": self.started, "screens": screens, "bestValue": best_value},
            "sources": {
                "eeLog": {"path": log, "active": active, "searching": eelog::searching() && log.is_none()},
                "profile": {"fetched": d.profile.as_ref().map(|p| p.fetched),
                            "source": d.profile.as_ref().and_then(|p| p.source.clone()),
                            "error": d.profile_error, "items": d.profile.as_ref().map_or(0, |p| p.xp.len())},
                "inventory": {"source": inv.and_then(|i| i.source.clone()), "imported": inv.map(|i| i.imported),
                              "synced": inv.and_then(|i| i.synced_at), "stacks": inv.map_or(0, |i| i.counts.len())},
                "manual": {"counts": d.manual.counts.len(), "mastered": d.manual.mastered.len()},
                "memscan": self.memscan_status(),
                "tools": self.tools(),
                "market": {"error": lock(&self.market.error).clone(), "queued": self.market.pending()},
            },
            "events": d.events.iter().rev().take(15).collect::<Vec<_>>(),
            "logLines": recent,
        })
    }

    /// Spare prime parts, valued; "source" says whether the counts come from an inventory.
    pub fn trade(&self) -> Value {
        let source = if lock(&self.data).inventory.is_some() { "inventory" } else { "manual" };
        json!({"rows": self.view().trade(), "source": source})
    }

    /// The open fissures, each with the relics owned for its tier and the best one to bring.
    pub fn fissures(&self) -> Value {
        let (mut data, err) = world::fissures(self.settings().platform);
        let picks = self.view().relic_picks();
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
            f["relics"] = if tier == "Omnia" { all.clone() } else { picks.get(&tier).cloned().unwrap_or(Value::Null) };
            f["watch"] = json!(world::watch_key(f));
        }
        json!({"fissures": data, "error": err, "watch": self.settings().fissure_watch})
    }

    pub fn world(&self) -> Value {
        world::summary(self.settings().platform)
    }

    /// Everything about one item, part or relic, with which relic farms are open right now.
    pub fn detail(&self, key: &str) -> Result<Value> {
        let (fissures, _) = world::fissures(self.settings().platform);
        let open: HashSet<String> = fissures
            .iter()
            .filter(|f| f["storm"] != true)
            .filter_map(|f| f["tier"].as_str().map(str::to_owned))
            .collect();
        let view = self.view();
        let mut v = view.detail(key, &open).ok_or_else(|| UserError::new(format!("Nothing is called {key}.")))?;
        let pinned = v["key"].as_str().is_some_and(|k| view.manual.pinned.iter().any(|p| p == k));
        v["pinned"] = json!(pinned);
        Ok(v)
    }

    pub fn history(&self) -> Value {
        let (screens, best_value) = history::totals(self.started);
        json!({"entries": history::recent(40),
               "session": {"since": self.started, "screens": screens, "bestValue": best_value}})
    }

    /// Desktop notifications, when switched on: watched fissures, Baro, finished builds.
    fn notify_tick(&self) {
        let s = self.settings();
        let mut n = lock(&self.notifier);
        if s.notify_fissures {
            n.fissures(s.platform, &s.fissure_watch);
        }
        if s.notify_baro {
            n.baro(&self.world()["baro"]);
        }
        if s.notify_foundry {
            let v = self.view();
            let done: Vec<notifier::Build> = v
                .inv
                .as_deref()
                .map(|i| {
                    i.foundry
                        .iter()
                        .filter_map(|f| {
                            let uid = f.recipe.clone()?;
                            Some((uid.clone(), v.cat.name_of(&uid), f.done?))
                        })
                        .collect()
                })
                .unwrap_or_default();
            n.foundry(&done);
        }
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
                    json!({"slug": slug, "name": name, "ducats": ducats, "sell": p.sell, "buy": p.buy})
                })
                .collect(),
        )
    }

    pub fn rivens(&self) -> Value {
        let (rows, err) = rivens::rows(&self.catalog(), self.settings().platform);
        json!({"rows": rows, "error": err})
    }

    pub fn riven_table(&self, weapon: &str, positives: u8, negatives: u8) -> Result<Value> {
        let cat = self.catalog();
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
        let cat = self.catalog();
        let it = cat
            .item_by_name(body["weapon"].as_str().unwrap_or(""))
            .filter(|i| i.disposition.is_some_and(|d| d != 0.0))
            .ok_or_else(|| UserError::new("No riven data for that weapon."))?;
        // A missing count means the common 3+1, but a legitimate 0 negatives must stay 0: the
        // Python app once turned it into 1 and graded every 3+0 riven against the wrong multiplier.
        let pos = body["positives"].as_u64().map_or(3, |n| n as u8);
        let neg = body["negatives"].as_u64().map_or(1, |n| n as u8);
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

    // ------------------------------------------------------------ scans

    pub fn rewards(&self) -> Option<Arc<RewardResult>> {
        lock(&self.rewards).clone()
    }

    pub fn riven(&self) -> Option<Arc<RivenResult>> {
        lock(&self.riven).clone()
    }

    fn names(&self) -> Arc<Names> {
        let cat = self.catalog();
        let mut cache = lock(&self.names);
        if let Some((c, n)) = cache.as_ref() {
            if Arc::ptr_eq(c, &cat) {
                return Arc::clone(n);
            }
        }
        let names = Arc::new(Names::new(&cat.reward_names()));
        *cache = Some((cat, Arc::clone(&names)));
        names
    }

    /// Hides our overlays and waits until they are off screen, so a capture never reads them.
    fn hide_overlays_for_capture(&self) {
        if !self.has_ui() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        self.ui(UiEvent::HideForCapture(tx));
        if rx.recv_timeout(Duration::from_millis(300)).is_ok() {
            // The compositor needs a frame to take the surface down.
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn publish_rewards(&self, result: RewardResult, fresh: bool) -> Arc<RewardResult> {
        let result = Arc::new(result);
        *lock(&self.rewards) = Some(Arc::clone(&result));
        self.ui(UiEvent::Rewards { result: Arc::clone(&result), fresh });
        result
    }

    /// Screenshots the reward screen, reads the names, and prices them.
    pub fn scan_rewards(&self, delay: Duration) -> Arc<RewardResult> {
        std::thread::sleep(delay);
        let mut slot = lock(&self.reward_scan);
        if now() - slot.started < COALESCE {
            // The same screen as a scan that just started: share its result, prices included.
            let (done, _) = self
                .reward_done
                .wait_timeout_while(slot, Duration::from_secs(15), |s| !s.done)
                .unwrap_or_else(|e| e.into_inner());
            drop(done);
            if let Some(r) = self.rewards() {
                return r;
            }
            slot = lock(&self.reward_scan);
        }
        *slot = RewardScan { started: now(), done: false };
        drop(slot);
        let _finished = ScanFinished(self);
        let capture = lock(&self.scan_lock);
        let settings = self.settings();
        self.hide_overlays_for_capture();
        let read = screenshot::capture(settings.screenshot_tool).and_then(|frame| {
            let (matches, text) = ocr::read_rewards(&frame, &self.names())?;
            Ok((frame, matches, text))
        });
        let (frame, matches, text) = match read {
            Ok(r) => r,
            Err(e) => {
                self.event(&format!("Reward scan failed: {e}"), "warn");
                return self.publish_rewards(
                    RewardResult { t: now(), error: Some(e.to_string()), ..Default::default() },
                    true,
                );
            }
        };
        let v = self.view();
        let needed = v.needed_parts();
        let mut items: Vec<RewardItem> = matches
            .iter()
            .map(|m| {
                let part = v.cat.parts.get(v.cat.part_alias.get(&m.uid).unwrap_or(&m.uid));
                let slug = self.market.slug_for(Some(&m.uid), part.and_then(|p| p.slug.as_deref()), Some(&m.name));
                RewardItem {
                    name: m.name.clone(),
                    uid: m.uid.clone(),
                    score: m.score,
                    have: v.count(&m.uid),
                    needed: needed.get(&m.uid).copied().unwrap_or(0),
                    vaulted: part.map(|p| p.vaulted),
                    ducats: self.market.ducats(slug.as_deref()),
                    price: slug.as_deref().and_then(|s| self.market.cached(s)).and_then(|p| p.sell),
                    slug,
                }
            })
            .collect();
        // Publish the names at once: the choice timer is ~10 s and warframe.market can take
        // several seconds. Prices fill in below and the overlay updates in place.
        let base = RewardResult {
            t: now(),
            items: items.clone(),
            best: None,
            ocr: tail(&text, 800),
            error: items.is_empty().then(|| "No reward names recognized.".to_string()),
        };
        self.publish_rewards(base.clone(), true);
        // The screen is read; a riven scan may run while the prices load.
        drop(capture);
        ocr::archive("reward", frame, text, matches_json(&items));

        let slugs: Vec<Option<String>> = items.iter().map(|i| i.slug.clone()).collect();
        let prices = self.market.fetch_now(&slugs, Duration::from_secs(6));
        for it in &mut items {
            if let Some(sell) = it.slug.as_ref().and_then(|s| prices.get(s)).and_then(|p| p.sell) {
                it.price = Some(sell);
            }
        }
        // "Best" means the most platinum, the leftmost card on a tie; with no price known there is
        // no best to claim.
        let best = items
            .iter()
            .filter(|i| i.price.is_some_and(|p| p > 0))
            .fold(None::<&RewardItem>, |best, i| if best.is_none_or(|b| i.price > b.price) { Some(i) } else { best })
            .map(|i| i.name.clone());
        if !items.is_empty() {
            history::record(history::Entry {
                t: now(),
                drops: items
                    .iter()
                    .map(|i| history::Reward {
                        name: i.name.clone(),
                        uid: i.uid.clone(),
                        price: i.price,
                        ducats: i.ducats,
                        needed: i.needed > i.have,
                    })
                    .collect(),
                best: best.clone(),
            });
        }
        if !items.is_empty() && settings.notify {
            let lines: Vec<String> = items
                .iter()
                .map(|i| {
                    let star = if best.as_deref() == Some(i.name.as_str()) { "★ " } else { "" };
                    let price = i.price.map_or("?".to_string(), |p| p.to_string());
                    let need = if i.needed > i.have { " · need" } else { "" };
                    format!("{star}{}: {price}p{need}", i.name)
                })
                .collect();
            notifier::send("Relic rewards", &lines.join("\n"));
        }
        self.publish_rewards(RewardResult { items, best, ..base }, false)
    }

    fn riven_failed(&self, weapon: Option<String>, text: &str, error: String) -> Arc<RivenResult> {
        let r =
            Arc::new(RivenResult { t: now(), weapon, ocr: tail(text, 600), error: Some(error), ..Default::default() });
        *lock(&self.riven) = Some(Arc::clone(&r));
        if self.settings().riven_overlay_enabled {
            self.ui(UiEvent::Riven(Arc::clone(&r)));
        }
        r
    }

    /// Reads the riven card currently on screen and grades it.
    pub fn scan_riven(&self) -> Arc<RivenResult> {
        let _one_scan_at_a_time = lock(&self.scan_lock);
        let settings = self.settings();
        self.hide_overlays_for_capture();
        let cat = self.catalog();
        let read = screenshot::capture(settings.screenshot_tool).and_then(|frame| {
            let (r, text) = ocr::read_riven(&frame, &cat.riven_weapons())?;
            Ok((frame, r, text))
        });
        let (frame, read, text) = match read {
            Ok(r) => r,
            Err(e) => return self.riven_failed(None, "", e.to_string()),
        };
        ocr::archive("riven", frame, text.clone(), json!({"weapon": read.weapon, "stats": read.stats}));
        let Some(weapon) = read.weapon.clone() else {
            return self.riven_failed(
                None,
                &text,
                "No weapon name recognised. Open the riven so its card is visible.".into(),
            );
        };
        if read.stats.is_empty() {
            return self.riven_failed(Some(weapon), &text, "No stat lines recognised on screen.".into());
        }
        let Some(it) = cat.item_by_name(&weapon).filter(|i| i.disposition.is_some_and(|d| d != 0.0)) else {
            return self.riven_failed(Some(weapon.clone()), &text, format!("No riven data for {weapon}."));
        };
        // 2-3 positives and 0-1 negative; a stat line OCR lost is recovered from the values.
        let readings: Vec<(&str, f64, bool)> = read.stats.iter().map(|s| (s.stat, s.value, s.negative)).collect();
        let (pos, neg, unread) = rivens::infer_counts(it, &readings);
        let stats: Vec<Graded> =
            read.stats.iter().filter_map(|s| rivens::grade_roll(it, s.stat, s.value, pos, neg, s.negative)).collect();
        let r = Arc::new(RivenResult {
            t: now(),
            weapon: Some(it.name.clone()),
            disposition: it.disposition.map(|d| py_round(d, 3)),
            dots: it.dispo_dots,
            positives: Some(pos),
            negatives: Some(neg),
            unread,
            stats,
            ocr: tail(&text, 600),
            error: None,
        });
        *lock(&self.riven) = Some(Arc::clone(&r));
        if settings.riven_overlay_enabled {
            self.ui(UiEvent::Riven(Arc::clone(&r)));
        }
        r
    }

    pub fn show_window(&self) {
        self.ui(UiEvent::ShowWindow);
    }

    /// Ends the app shortly after the request that asked for it has been answered.
    pub fn quit(&self) {
        if self.has_ui() {
            self.ui(UiEvent::Quit);
        } else {
            std::thread::spawn(|| {
                std::thread::sleep(Duration::from_millis(200));
                std::process::exit(0);
            });
        }
    }

    pub fn test_overlay(&self) -> Result<()> {
        if !self.has_ui() {
            return Err(UserError::new("The overlay only exists in the desktop app.").into());
        }
        self.ui(UiEvent::TestOverlay);
        Ok(())
    }

    // ------------------------------------------------------------ hotkeys

    fn hotkey_keys(&self) -> (u16, [(String, &'static str); 2]) {
        let s = self.settings();
        (s.port, [(s.hotkey_rewards, "rewards/scan"), (s.hotkey_riven, "riven/scan")])
    }

    pub fn hotkeys(&self) -> Value {
        let flavour = compositor::detect_flavour();
        let (port, keys) = self.hotkey_keys();
        let pairs: Vec<(&str, &str)> = keys.iter().map(|(k, p)| (k.as_str(), *p)).collect();
        json!({"compositor": flavour.map(|f| f.label()), "lines": compositor::hotkey_lines(flavour, port, &pairs),
               "note": compositor::hotkey_note(flavour)})
    }

    pub fn install_hotkeys(&self) -> Result<Value> {
        let flavour = compositor::detect_flavour();
        let (port, keys) = self.hotkey_keys();
        let pairs: Vec<(&str, &str)> = keys.iter().map(|(k, p)| (k.as_str(), *p)).collect();
        let lines = compositor::hotkey_lines(flavour, port, &pairs);
        let done = compositor::install_hotkeys(flavour, &lines)?;
        let name = done.file.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
        self.event(&format!("Hotkeys written to {name} and the compositor reloaded"), "info");
        Ok(json!({"file": done.file, "sourced": done.sourced, "lines": lines}))
    }
}

struct RewardScan {
    started: f64,
    done: bool,
}

/// Marks the running reward scan finished, however it ends, and wakes callers sharing it.
struct ScanFinished<'a>(&'a State);

impl Drop for ScanFinished<'_> {
    fn drop(&mut self) {
        lock(&self.0.reward_scan).done = true;
        self.0.reward_done.notify_all();
    }
}

fn matches_json(items: &[RewardItem]) -> Value {
    json!(items.iter().map(|i| json!({"name": i.name, "uid": i.uid, "score": i.score})).collect::<Vec<_>>())
}
