//! The accounts as the app sees them: every source merged, the scans, and the background work.
//! What the UI shows is computed in `views.rs` from a snapshot (`State::view`).
//!
//! Two accounts matter at any time: the one the window shows (`active`), and the one the game is
//! logged in to (`game`, from EE.log). They are usually the same - a login moves the window to
//! its account - but the user may look at the other one meanwhile. What the game produces (memory
//! captures, reward screens, mastery badges, ids in the log) belongs to `game`; what the user does
//! in the window (counts, pins, imports, a profile refresh) to `active`.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::Result;
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Value, json};

use crate::accounts::{self, Account, Evidence};
use crate::config::Settings;
use crate::difflib::py_round;
use crate::eelog::{self, LogEvent, LogStatus, LogWatcher};
use crate::gamedata::{self, Catalog};
use crate::market::Market;
use crate::memscan::{self, ScanError};
use crate::notifier::{self, Notifier};
use crate::ocr::image::Rgb;
use crate::ocr::matcher::{Names, RewardMatch};
use crate::ocr::{self, reward, screenshot};
use crate::rivens::{self, Graded};
use crate::sources::{self, Inventory, Profile};
use crate::views::View;
use crate::{UserError, compositor, history, now, standing, tasks, world};

/// DE temporarily IP-blocks game login for clients that query profiles too often.
pub const PROFILE_COOLDOWN: f64 = 300.0;
const DUCAT_UID: &str = "/Lotus/Types/Items/MiscItems/PrimeBucks";
/// A reward scan started this recently already covers the screen: the F9 hotkey and the
/// automatic trigger routinely fire for the same reward screen within a second of each other.
const COALESCE: f64 = 5.0;
/// How long a scan looks for the reward cards: from the log's open line they turn over after a
/// 5 s pause showing the relics, or at once; the hotkey is pressed with them in view.
const REWARD_WAIT: Duration = Duration::from_secs(12);
const HOTKEY_WAIT: Duration = Duration::from_secs(1);
/// Between two looks at the screen while waiting for the cards. A capture with grim takes 20-40 ms;
/// the other screenshot tools start a whole program each time.
const REWARD_POLL: Duration = Duration::from_millis(120);
const SLOW_POLL: Duration = Duration::from_secs(1);
/// Readings of one screen that found names, at most, while a card is still unread or unsure: one
/// that stays so is not worth more.
const REWARD_READS: usize = 3;
/// A card read at this score or better is not read again. On real screens the wrong names (a
/// sibling part on a hovered or tinted card) scored 0.77-0.85, and the same card read 1.00 in
/// the frames before and after.
const SURE: f64 = 0.9;
/// Readings of one screen at most, with or without names: what looked like cards may be none.
const REWARD_ATTEMPTS: usize = 6;
/// While the prices are up, the screen is checked this often for the cards still being there.
const CARDS_CHECK: Duration = Duration::from_millis(400);
/// Minimum time between the starts of memory scans.
const MEMSCAN_GAP: f64 = 10.0;

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub t: f64,
    pub text: String,
    pub kind: &'static str,
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
    /// The card it is on, 0 = leftmost; None when the names were searched for without cards.
    pub slot: Option<usize>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RewardResult {
    pub t: f64,
    pub items: Vec<RewardItem>,
    pub best: Option<String>,
    pub ocr: String,
    pub error: Option<String>,
    /// How many cards the screen showed, when it was recognised as the reward screen.
    pub cards: Option<usize>,
    /// The interface scale the cards were found at, which places the price panels under them.
    pub scale: Option<f64>,
}

impl RewardResult {
    /// What Settings' "show a test overlay" shows: four cards at interface scale `scale`.
    pub fn sample(scale: f64) -> RewardResult {
        let item = |slot, name: &str, price: Option<i64>, ducats: Option<i64>, have, needed, vaulted| RewardItem {
            name: name.into(),
            uid: String::new(),
            score: 1.0,
            slug: None,
            have,
            needed,
            vaulted: Some(vaulted),
            ducats,
            price,
            slot: Some(slot),
        };
        RewardResult {
            t: crate::now(),
            items: vec![
                item(0, "Ash Prime Systems Blueprint", Some(25), Some(65), 0, 1, false),
                item(1, "Braton Prime Barrel", Some(4), Some(15), 2, 0, false),
                item(2, "Forma Blueprint", None, None, 3, 0, false),
                item(3, "Fragor Prime Head", Some(6), Some(45), 0, 0, true),
            ],
            best: Some("Ash Prime Systems Blueprint".into()),
            cards: Some(4),
            scale: Some(scale),
            ..Default::default()
        }
    }
}

/// The pick worth the most platinum, the leftmost card on a tie. With no price known there is no
/// best to claim.
fn best_pick(items: &[RewardItem]) -> Option<String> {
    items
        .iter()
        .filter(|i| i.price.is_some_and(|p| p > 0))
        .fold(None::<&RewardItem>, |best, i| if best.is_none_or(|b| i.price > b.price) { Some(i) } else { best })
        .map(|i| i.name.clone())
}

/// What a look for the reward cards found.
enum Sighting {
    /// The screen closed first.
    Cancelled,
    /// The cards, the best name read on each, the frame and the words they came from.
    Cards { frame: Rgb, layout: reward::Layout, names: Vec<Option<RewardMatch>>, text: String },
    /// No reward cards before the time ran out; the last frame.
    NoCards(Rgb),
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct RivenResult {
    pub t: f64,
    pub weapon: Option<String>,
    /// The weapon's picture.
    pub image: Option<String>,
    pub positives: Option<u8>,
    pub negatives: Option<u8>,
    /// Stats the card has but OCR did not read (their count is inferred from the values).
    pub unread: u8,
    pub stats: Vec<Graded>,
    /// The grade of the average roll over all stats.
    pub overall: Option<&'static str>,
    /// What this weapon's rivens sold for lately (api.warframestat.us: `unrolled` and `rerolled`,
    /// each with `median`, `min`, `max`, `pop`).
    pub traded: Option<Value>,
    /// Rivens with the same positives listed on warframe.market; filled in after the grades are
    /// shown, since it is one more request.
    pub market: Option<crate::market::RivenListings>,
    #[serde(rename = "marketError")]
    pub market_error: Option<String>,
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
    /// The last successful read: {t, stacks, seconds, account}.
    pub last: Option<Value>,
    #[serde(skip)]
    last_attempt: f64,
    /// The game process that refused access; it is not asked again, a restarted game is.
    #[serde(skip)]
    denied_pid: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
enum ScheduledScan {
    Superseded,
    Cancelled,
    Wait,
    Start,
}

impl MemscanStatus {
    fn start_scheduled(&mut self, due: f64, enabled: bool, pid: Option<u32>, time: f64) -> ScheduledScan {
        if self.next != Some(due) {
            return ScheduledScan::Superseded;
        }
        if !enabled || (pid.is_some() && pid == self.denied_pid) {
            self.next = None;
            return ScheduledScan::Cancelled;
        }
        if self.state == "scanning" || time - self.last_attempt < MEMSCAN_GAP {
            return ScheduledScan::Wait;
        }
        self.next = None;
        self.state = "scanning";
        self.progress = 0.0;
        self.last_attempt = time;
        ScheduledScan::Start
    }
}

/// What the engine asks of the desktop shell, if one is running.
pub enum UiEvent {
    ShowWindow,
    /// New rewards; `fresh` is false when only the prices changed.
    Rewards {
        result: Arc<RewardResult>,
        fresh: bool,
    },
    HideRewards,
    /// A riven read; `fresh` is false when only its market listings arrived.
    Riven {
        result: Arc<RivenResult>,
        fresh: bool,
    },
    TestOverlay,
    /// Hide the overlays before a screenshot, so a scan never reads our own text; acknowledged
    /// on the sender once they are gone.
    HideForCapture(std::sync::mpsc::Sender<()>),
    Quit,
}

#[derive(Default)]
struct Data {
    accounts: Vec<Account>,
    /// The account the window shows.
    active: Option<String>,
    /// When `active` last changed (saved): a log read back at startup moves the window to its
    /// account only if that session is newer than the user's own choice.
    switched: f64,
    /// The account the game is logged in to, from the log's latest "Logged in <name>".
    game: Option<String>,
    events: VecDeque<Event>,
    last_profile_try: f64,
    /// Accounts whose profile is due, waiting for DE's rate limit.
    profile_queue: VecDeque<String>,
    profile_worker: bool,
    inventory_mtime: Option<std::time::SystemTime>,
}

impl Data {
    fn get(&self, key: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.key == key)
    }

    fn get_mut(&mut self, key: &str) -> Option<&mut Account> {
        self.accounts.iter_mut().find(|a| a.key == key)
    }

    fn by_name(&self, name: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| same_name(&a.name, name))
    }

    fn active(&self) -> Option<&Account> {
        self.active.as_deref().and_then(|k| self.get(k))
    }

    fn game(&self) -> Option<&Account> {
        self.game.as_deref().and_then(|k| self.get(k))
    }

    fn name_of(&self, key: &str) -> String {
        self.get(key).map_or_else(|| key.to_string(), |a| a.name.clone())
    }

    /// The key of the account called `name`, added when it is new.
    fn ensure(&mut self, name: &str) -> String {
        if let Some(a) = self.by_name(name) {
            return a.key.clone();
        }
        let taken: Vec<&str> = self.accounts.iter().map(|a| a.key.as_str()).collect();
        let key = accounts::new_key(name, &taken);
        self.accounts.push(Account::new(key.clone(), eelog::strip_glyphs(name)));
        key
    }

    /// The account the window shows; with none shown, the one playing or the first one, and with
    /// no account at all a new one.
    fn active_or_new(&mut self) -> String {
        if let Some(key) = self.active.clone().filter(|k| self.get(k).is_some()) {
            return key;
        }
        let known = self.game.clone().or_else(|| self.accounts.first().map(|a| a.key.clone()));
        let key = match known {
            Some(key) => key,
            None => self.ensure("Account 1"),
        };
        self.active = Some(key.clone());
        key
    }

    fn save(&self) {
        if let Err(e) = accounts::save(&self.active, self.switched, &self.accounts) {
            eprintln!("could not save the account list: {e:#}");
        }
    }

    fn push_event(&mut self, text: String, kind: &'static str) {
        self.events.push_back(Event { t: now(), text, kind });
        while self.events.len() > 60 {
            self.events.pop_front();
        }
    }
}

/// Whose inventory a capture is, judged by its mastery XP (`accounts::same_owner`).
enum Owner {
    /// The logged-in account's.
    Game,
    /// Nothing to compare it with: taken as the logged-in account's.
    Unknown,
    /// Another account the app knows.
    Other(String),
    /// Not the logged-in account's, and no other known account's either.
    Stranger,
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
    /// The interface scale the reward cards were last found at when it was not EE.cfg's.
    found_scale: Mutex<Option<f64>>,
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

/// Whose mastery XP `xp` is, as far as the accounts' saved data can tell. The profile, read by
/// account id from DE, is asked before an inventory, which an older build may have filed wrongly.
fn owner(d: &Data, game: &str, xp: &IndexMap<String, i64>) -> Owner {
    let verdict = |acc: &Account| acc.known_xp().into_iter().find_map(|known| accounts::same_owner(xp, known));
    let mine = d.get(game).and_then(verdict);
    if mine == Some(true) {
        return Owner::Game;
    }
    let other = d.accounts.iter().filter(|a| a.key != game).find(|a| verdict(a) == Some(true));
    match (mine, other) {
        (_, Some(o)) => Owner::Other(o.name.clone()),
        (Some(false), None) => Owner::Stranger,
        _ => Owner::Unknown,
    }
}

fn choose_capture(
    d: &Data,
    game: &str,
    sections: &memscan::Sections,
    captures: Vec<Value>,
) -> (Option<Value>, Option<String>, Option<Vec<String>>) {
    let mut held_by = None;
    let mut missing = None;
    for capture in captures {
        match owner(d, game, &sources::xp_map(capture.get("XPInfo"))) {
            Owner::Game | Owner::Unknown => {
                let absent = sections.missing(&capture);
                if absent.is_empty() {
                    return (Some(capture), held_by, missing);
                }
                if missing.is_none() {
                    missing = Some(absent);
                }
            }
            Owner::Other(name) => {
                held_by.get_or_insert(name);
            }
            Owner::Stranger => {
                held_by.get_or_insert_with(|| "another account".to_string());
            }
        }
    }
    (None, held_by, missing)
}

impl State {
    pub fn new(settings: Settings) -> Arc<State> {
        let settings = Arc::new(RwLock::new(settings));
        let (registry, notes) = accounts::load();
        let mut data = Data {
            accounts: registry.accounts,
            active: registry.active,
            switched: registry.switched,
            last_profile_try: f64::NEG_INFINITY,
            ..Data::default()
        };
        // A shown account that is gone (its folder deleted by hand): show the first one instead.
        if data.active().is_none() {
            data.active = data.accounts.first().map(|a| a.key.clone());
        }
        for note in notes {
            data.push_event(note, "info");
        }
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
            reward_scan: Mutex::new(RewardScan {
                started: f64::NEG_INFINITY,
                done: true,
                generation: 0,
                screen_generation: 0,
                result: None,
            }),
            reward_done: std::sync::Condvar::new(),
            found_scale: Mutex::new(None),
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
        lock(&self.data).push_event(text.to_string(), kind);
    }

    /// Whether an account's profile should be loaded again: never loaded, older than the refresh
    /// interval, loaded for another id, or saved by a build that did not keep standing yet (DE's
    /// profile always has it: every daily pool is sent even at rank 0).
    fn profile_stale(&self, acc: &Account) -> bool {
        let interval = self.settings().profile_refresh_minutes.max(10) as f64 * 60.0;
        acc.profile.as_ref().is_none_or(|p| {
            now() - p.fetched > interval
                || p.id.as_deref().is_some_and(|pid| Some(pid) != acc.id.as_deref())
                || (p.standing.is_none() && p.source.as_deref() == Some("DE public profile"))
        })
    }

    fn background(self: Arc<Self>) {
        // Give the log watcher time to find the log and read it back: the account it ends on is
        // the one shown, and the one whose profile is worth loading.
        std::thread::sleep(Duration::from_secs(10));
        let mut first = true;
        let mut tick = 0u64;
        loop {
            self.watch_inventory_file();
            if tick % 3 == 0 {
                self.notify_tick();
            }
            tick += 1;
            // At startup the account shown, when its profile is stale. While playing, with
            // automatic refresh switched on, the account logged in.
            let auto = self.settings().profile_auto_refresh && self.game_active();
            let due = {
                let d = lock(&self.data);
                let key = if first {
                    d.active.clone()
                } else if auto {
                    d.game.clone()
                } else {
                    None
                };
                key.filter(|k| d.get(k).is_some_and(|a| a.id.is_some() && self.profile_stale(a)))
            };
            first = false;
            if let Some(key) = due {
                self.request_profile(&key);
            }
            std::thread::sleep(Duration::from_secs(20));
        }
    }

    // ------------------------------------------------------------ the log

    fn on_log(self: &Arc<Self>, ev: LogEvent) {
        match ev {
            LogEvent::Name(name, live) => self.on_login(&name, live),
            LogEvent::Replayed => {
                self.follow_game(false);
                if memscan::find_game_pid().is_some() {
                    self.schedule_memscan(Duration::from_secs(3), 2);
                }
            }
            LogEvent::Account(id, name) => {
                let key = {
                    let mut d = lock(&self.data);
                    match name {
                        Some(n) => Some(d.ensure(&n)),
                        None => d.game.clone(),
                    }
                };
                if let Some(key) = key {
                    self.confirm(&key, &id, "EE.log");
                }
            }
            LogEvent::Candidate(id, sure, line) => self.on_candidate(id, sure, line),
            LogEvent::Mastery(lo, hi) => {
                let mut d = lock(&self.data);
                let Some(game) = d.game.clone() else { return };
                let changed = match d.get_mut(&game) {
                    Some(acc) if acc.badge != Some((lo, hi)) => {
                        acc.badge = Some((lo, hi));
                        true
                    }
                    _ => false,
                };
                if changed {
                    d.save();
                }
            }
            LogEvent::RewardScreen => {
                if self.settings().ocr_enabled {
                    let screen = {
                        let mut scan = lock(&self.reward_scan);
                        scan.invalidate();
                        scan.screen_generation
                    };
                    self.reward_done.notify_all();
                    let me = Arc::clone(self);
                    std::thread::spawn(move || {
                        me.scan_rewards_for(REWARD_WAIT, Some(screen));
                    });
                }
            }
            LogEvent::RewardScreenClosed => {
                let mut scan = lock(&self.reward_scan);
                scan.invalidate();
                self.ui(UiEvent::HideRewards);
                self.reward_done.notify_all();
            }
            LogEvent::Sync => self.schedule_memscan(Duration::from_secs(3), 2),
        }
    }

    /// "Logged in <name>": that account is the game's from now on. A new name is a new account.
    fn on_login(self: &Arc<Self>, name: &str, live: bool) {
        let name = eelog::strip_glyphs(name);
        if name.is_empty() {
            return;
        }
        let (key, switched) = {
            let mut d = lock(&self.data);
            let key = d.ensure(&name);
            let switched = d.game.as_deref() != Some(key.as_str());
            d.game = Some(key.clone());
            if live {
                if let Some(acc) = d.get_mut(&key) {
                    acc.seen = Some(now());
                }
            }
            d.save();
            (key, switched)
        };
        if live {
            self.follow_game(true);
            let due = lock(&self.data).get(&key).is_some_and(|a| a.id.is_some() && self.profile_stale(a));
            if switched && due {
                self.request_profile(&key);
            }
        }
        if live {
            // The inventory can be overwritten shortly after it arrives.
            self.schedule_memscan(Duration::from_secs(3), 2);
        }
    }

    /// Shows the account the game is logged in to. A login seen live always moves the window to
    /// it; one read back from an existing log only when that session is newer than the user's
    /// last choice of account.
    fn follow_game(&self, live: bool) {
        let log_time = self
            .log_path()
            .and_then(|p| fs::metadata(p).ok())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|t| t.as_secs_f64());
        let mut d = lock(&self.data);
        let Some(game) = d.game.clone() else { return };
        if d.active.as_deref() == Some(game.as_str()) || (!live && log_time.is_none_or(|t| t <= d.switched)) {
            return;
        }
        d.active = Some(game.clone());
        d.switched = now();
        let name = d.name_of(&game);
        d.push_event(format!("Showing {name}, the account the game is logged in to"), "info");
        d.save();
    }

    /// Never queries DE: a sure line links the logged-in account, anything else is shown to the
    /// user. An id another account already has is left alone.
    fn on_candidate(self: &Arc<Self>, id: String, sure: bool, line: String) {
        let link = {
            let mut d = lock(&self.data);
            let Some(game) = d.game.clone() else { return };
            if d.accounts.iter().any(|a| a.id.as_deref() == Some(id.as_str())) {
                return;
            }
            let Some(acc) = d.get_mut(&game) else { return };
            if sure && acc.id.is_none() {
                Some(game)
            } else {
                if acc.evidence.iter().all(|e| e.id != id) {
                    acc.evidence.push(Evidence { id: id.clone(), line });
                    let excess = acc.evidence.len().saturating_sub(12);
                    acc.evidence.drain(..excess);
                }
                None
            }
        };
        if let Some(key) = link {
            self.confirm(&key, &id, "EE.log");
        }
    }

    /// Links `id` to an account. An id belongs to one account: a link made now wins over an
    /// older one elsewhere.
    fn confirm(self: &Arc<Self>, key: &str, id: &str, source: &str) {
        let changed = {
            let mut d = lock(&self.data);
            let mut moved_from = None;
            for other in d.accounts.iter_mut().filter(|a| a.key != key && a.id.as_deref() == Some(id)) {
                other.id = None;
                other.source = None;
                moved_from = Some(other.name.clone());
            }
            let Some(acc) = d.get_mut(key) else { return };
            let changed = acc.id.as_deref() != Some(id);
            acc.id = Some(id.to_string());
            acc.source = Some(source.to_string());
            acc.profile_error = None;
            let name = acc.name.clone();
            if let Some(from) = moved_from {
                d.push_event(format!("The id {id} was linked to {from}; it is {name}'s now"), "warn");
            }
            if changed {
                d.push_event(format!("Account linked: {name}"), "info");
            }
            d.save();
            changed
        };
        if changed {
            self.request_profile(key);
        }
    }

    // ------------------------------------------------------------ the memory scan

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
        if let Err(e) = std::thread::Builder::new().name("memscan".into()).spawn(move || {
            std::thread::sleep(delay);
            let pid = loop {
                let enabled = me.settings().memory_scan_enabled;
                let pid = memscan::find_game_pid();
                let mut st = lock(&me.memscan);
                match st.start_scheduled(due, enabled, pid, now()) {
                    ScheduledScan::Superseded | ScheduledScan::Cancelled => return,
                    ScheduledScan::Start => break pid,
                    ScheduledScan::Wait => {}
                }
                drop(st);
                std::thread::sleep(Duration::from_secs(1));
            };
            me.memscan_now(pid, retries);
        }) {
            let mut st = lock(&self.memscan);
            if st.next == Some(due) {
                st.next = None;
            }
            if st.state != "scanning" {
                st.state = "error";
            }
            st.detail = Some(format!("Could not schedule the memory scan: {e}"));
            eprintln!("could not schedule the memory scan: {e}");
        }
    }

    fn memscan_now(self: &Arc<Self>, pid: Option<u32>, retries: u8) {
        let result = match pid {
            Some(pid) => memscan::scan_inventory(pid, |p| lock(&self.memscan).progress = py_round(p, 3)),
            None => Err(ScanError::NotRunning),
        };
        let retry = match result {
            Ok((captures, info)) => self.apply_capture(captures, info.seconds),
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
            self.schedule_memscan(Duration::from_secs(if retries > 1 { 10 } else { 30 }), retries - 1);
        }
    }

    /// Ends a read that applied nothing: the last good inventory still stands. True asks for a
    /// retry.
    fn capture_held_back(&self, detail: String) -> bool {
        let mut st = lock(&self.memscan);
        st.state = if st.last.is_some() { "ok" } else { "idle" };
        st.detail = Some(detail);
        true
    }

    /// Files the logged-in account's capture, unless it is cut short or already applied. The
    /// memory can hold another account's inventory too (logged out and in without restarting the
    /// game): mastery XP tells them apart. True asks for a retry.
    fn apply_capture(&self, captures: Vec<Value>, seconds: f64) -> bool {
        let game = lock(&self.data).game.clone();
        let Some(game) = game else {
            return self.capture_held_back(
                "Waiting for the game's \"Logged in\" line, which says whose inventory this is.".into(),
            );
        };
        let file = accounts::dir(&game).join("memscan_sections.json");
        let mut sections: memscan::Sections = crate::config::read_json(&file).unwrap_or_default();
        let (chosen, held_by, missing, game_name) = {
            let d = lock(&self.data);
            let (chosen, held_by, missing) = choose_capture(&d, &game, &sections, captures);
            (chosen, held_by, missing, d.name_of(&game))
        };
        let obj = match (chosen, missing) {
            (Some(obj), _) => obj,
            (None, Some(missing)) => {
                return self.capture_held_back(format!(
                    "The game held only part of {game_name}'s inventory just now ({} missing), so the last complete one is kept.",
                    missing.join(", ")
                ));
            }
            (None, None) => {
                let whose = held_by.map_or_else(|| "another account's".to_string(), |n| format!("{n}'s"));
                return self.capture_held_back(format!(
                    "The game still holds {whose} inventory; {game_name}'s is read after the next mission."
                ));
            }
        };
        // The same sync, read again with nothing new in it: leave the inventory and the log alone.
        let keys = obj.as_object().map_or(0, |o| o.len());
        let sync = obj["LastInventorySync"].as_f64();
        let current = lock(&self.data).get(&game).and_then(|a| a.inventory.as_ref().map(|i| (i.synced_at, i.sections)));
        if sync.is_some() && current.is_some_and(|(at, n)| at == sync && keys <= n) {
            let mut st = lock(&self.memscan);
            st.state = "ok";
            st.detail = None;
            return false;
        }
        let stored = sources::parse_inventory(&obj).map(|mut inv| {
            inv.source = Some("memory scan".into());
            self.store_inventory(&game, inv)
        });
        let mut st = lock(&self.memscan);
        match stored {
            Ok(inv) => {
                sections.learn(&obj);
                let saved = fs::create_dir_all(accounts::dir(&game))
                    .map_err(anyhow::Error::from)
                    .and_then(|_| crate::config::write_json(&file, &sections));
                if let Err(e) = saved {
                    eprintln!("could not save memory scan expectations: {e:#}");
                }
                st.state = "ok";
                st.detail = None;
                st.last =
                    Some(json!({"t": now(), "stacks": inv.counts.len(), "seconds": seconds, "account": game_name}));
            }
            Err(e) => {
                st.state = "error";
                st.detail = Some(e.to_string());
            }
        }
        false
    }

    // ------------------------------------------------------------ profiles

    /// Loads an account's profile as soon as DE's rate limit allows, if it is still due then. One
    /// worker serves every account, one request per cooldown.
    fn request_profile(self: &Arc<Self>, key: &str) {
        {
            let mut d = lock(&self.data);
            if !d.profile_queue.iter().any(|k| k == key) {
                d.profile_queue.push_back(key.to_string());
            }
            if d.profile_worker {
                return;
            }
            d.profile_worker = true;
        }
        let me = Arc::clone(self);
        std::thread::Builder::new().name("profile".into()).spawn(move || me.profile_worker()).ok();
    }

    fn profile_worker(self: Arc<Self>) {
        loop {
            let (key, wait) = {
                let mut d = lock(&self.data);
                let Some(key) = d.profile_queue.front().cloned() else {
                    d.profile_worker = false;
                    return;
                };
                (key, PROFILE_COOLDOWN - (now() - d.last_profile_try))
            };
            if wait > 0.0 {
                let name = lock(&self.data).name_of(&key);
                self.event(
                    &format!(
                        "Loading {name}'s profile in {} min (pausing for DE's rate limit)",
                        (wait / 60.0) as i64 + 1
                    ),
                    "info",
                );
                std::thread::sleep(Duration::from_secs_f64(wait + 1.0));
                continue;
            }
            let due = {
                let mut d = lock(&self.data);
                d.profile_queue.pop_front();
                d.get(&key).is_some_and(|a| a.id.is_some() && self.profile_stale(a))
            };
            if due {
                let _ = self.refresh_profile(&key, true);
            }
        }
    }

    /// Takes the one profile request DE's rate limit allows per cooldown.
    fn take_profile_slot(&self) -> Result<()> {
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
        Ok(())
    }

    /// Loads the profile of the account `key` (the one shown, from the window).
    pub fn refresh_profile(&self, key: &str, quiet: bool) -> Result<Option<Arc<Profile>>> {
        let id = lock(&self.data).get(key).and_then(|a| a.id.clone());
        let Some(id) = id else {
            return Err(UserError::new(
                "This account has no id yet. It is read from EE.log after a relic mission in a squad, or enter it in Settings.",
            )
            .into());
        };
        self.take_profile_slot()?;
        let prof = match sources::fetch_profile(&id, self.settings().platform) {
            Ok(p) => p,
            Err(e) => {
                let msg = e.to_string();
                if let Some(acc) = lock(&self.data).get_mut(key) {
                    acc.profile_error = Some(msg.clone());
                }
                if !quiet {
                    return Err(e);
                }
                self.event(&msg, "warn");
                return Ok(None);
            }
        };
        Ok(Some(self.attach_profile(key, Some(&id), prof, "DE public profile")))
    }

    /// Files a profile under the account it names. Its display name decides: an id linked to the
    /// wrong account moves to the account that owns it, which is added when it is new. With no
    /// account of that name and `key` not logged in right now, `key` was renamed.
    fn attach_profile(&self, key: &str, id: Option<&str>, mut prof: Profile, label: &str) -> Arc<Profile> {
        prof.source = Some(label.to_string());
        let id = prof.id.clone().or_else(|| id.map(str::to_owned));
        prof.id = id.clone();
        let prof = Arc::new(prof);
        let shown = prof.name.as_deref().map(eelog::strip_glyphs).filter(|n| !n.is_empty());
        let mut d = lock(&self.data);
        let mut target = key.to_string();
        if let Some(name) = &shown {
            if !d.get(key).is_some_and(|a| same_name(&a.name, name)) {
                match d.by_name(name).map(|a| a.key.clone()) {
                    Some(other) => target = other,
                    None if d.game.as_deref() != Some(key) && d.get(key).is_some() => {
                        if let Some(acc) = d.get_mut(key) {
                            acc.name = name.clone();
                        }
                    }
                    None => target = d.ensure(name),
                }
            }
        }
        if target != key {
            let owner = d.name_of(&target);
            let was = d.name_of(key);
            if let Some(acc) = d.get_mut(key) {
                if acc.id.is_some() && acc.id == id {
                    acc.id = None;
                    acc.source = None;
                    acc.profile_error =
                        Some(format!("The linked id belonged to {owner}. Pick or paste this account's own id."));
                }
            }
            d.push_event(format!("That id belongs to {owner}, not {was}: moved it there"), "warn");
        }
        if let Some(id) = &id {
            for other in d.accounts.iter_mut().filter(|a| a.key != target && a.id.as_ref() == Some(id)) {
                other.id = None;
                other.source = None;
            }
        }
        if let Some(acc) = d.get_mut(&target) {
            if id.is_some() && acc.id != id {
                acc.id = id.clone();
                acc.source = Some("DE profile".into());
            }
            acc.profile = Some(Arc::clone(&prof));
            acc.profile_error = None;
            if let Err(e) = acc.write("profile.json", &*prof) {
                eprintln!("could not save the profile: {e:#}");
            }
        }
        let name = d.name_of(&target);
        d.push_event(format!("Profile updated: {name}, {} items with mastery XP", prof.xp.len()), "info");
        d.save();
        prof
    }

    /// A profile file the user picked: filed under the account it names, else the one shown.
    pub fn import_profile(&self, data: &Value) -> Result<()> {
        let prof = sources::parse_profile(data)?;
        let key = {
            let mut d = lock(&self.data);
            let named = prof.name.as_deref().and_then(|n| d.by_name(n)).map(|a| a.key.clone());
            named.unwrap_or_else(|| d.active_or_new())
        };
        self.attach_profile(&key, None, prof, "Imported profile file");
        Ok(())
    }

    // ------------------------------------------------------------ inventories

    fn store_inventory(&self, key: &str, inv: Inventory) -> Arc<Inventory> {
        let inv = Arc::new(inv);
        let mut d = lock(&self.data);
        let name = d.name_of(key);
        if let Some(acc) = d.get_mut(key) {
            if let Err(e) = acc.write("inventory.json", &*inv) {
                eprintln!("could not save the inventory: {e:#}");
            }
            acc.inventory = Some(Arc::clone(&inv));
        }
        let equipment: i64 = inv.equipment.values().map(|e| e.count).sum();
        let source = inv.source.clone().unwrap_or_default();
        d.push_event(
            format!("{name}'s inventory loaded from {source}: {} item stacks, {equipment} equipment", inv.counts.len()),
            "info",
        );
        inv
    }

    /// An inventory file for the account shown. Refused when its mastery XP says it is another
    /// account's, so a file of the wrong account never mixes the two up.
    pub fn import_inventory(&self, data: &Value, label: &str) -> Result<Arc<Inventory>> {
        let mut inv = sources::parse_inventory(data)?;
        inv.source = Some(label.to_string());
        let key = {
            let mut d = lock(&self.data);
            let key = d.active_or_new();
            let name = d.name_of(&key);
            match owner(&d, &key, &inv.xp) {
                Owner::Other(other) => {
                    return Err(UserError::new(format!(
                        "This inventory is {other}'s, not {name}'s: its mastery XP matches {other}'s. Switch to {other} first."
                    ))
                    .into());
                }
                Owner::Stranger => {
                    return Err(UserError::new(format!(
                        "This inventory's mastery XP does not match {name}'s: it looks like another account's."
                    ))
                    .into());
                }
                Owner::Game | Owner::Unknown => key,
            }
        };
        Ok(self.store_inventory(&key, inv))
    }

    pub fn clear_inventory(&self) {
        let mut d = lock(&self.data);
        let Some(key) = d.active.clone() else { return };
        if let Some(acc) = d.get_mut(&key) {
            acc.inventory = None;
        }
        let _ = fs::remove_file(accounts::dir(&key).join("inventory.json"));
        let sections = accounts::dir(&key).join("memscan_sections.json");
        if let Err(e) = fs::remove_file(&sections) {
            if e.kind() != std::io::ErrorKind::NotFound {
                d.push_event(format!("Could not reset memory scan expectations: {e}"), "warn");
            }
        }
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
        match crate::config::read_json::<Value>(Path::new(&path)) {
            None => self.event(&format!("Could not read {path} as JSON"), "warn"),
            Some(v) => {
                if let Err(e) = self.import_inventory(&v, &label) {
                    self.event(&e.to_string(), "warn");
                }
            }
        }
    }

    // ------------------------------------------------------------ what the user sets by hand

    /// Changes the shown account's hand-set data and saves it.
    fn edit_manual(&self, change: impl FnOnce(&mut accounts::Manual)) -> Result<()> {
        let mut d = lock(&self.data);
        let key = d.active_or_new();
        let acc = d.get_mut(&key).ok_or_else(|| UserError::new("No account to save this for."))?;
        change(&mut acc.manual);
        acc.write("manual.json", &acc.manual)?;
        d.save();
        Ok(())
    }

    pub fn set_manual_count(&self, uid: &str, count: Option<i64>) -> Result<()> {
        self.edit_manual(|m| match count {
            None => {
                m.counts.shift_remove(uid);
            }
            Some(c) => {
                m.counts.insert(uid.to_string(), (c.max(0), now()));
            }
        })
    }

    pub fn set_manual_mastered(&self, uid: &str, mastered: bool) -> Result<()> {
        self.edit_manual(|m| {
            if mastered {
                m.mastered.insert(uid.to_string(), now());
            } else {
                m.mastered.shift_remove(uid);
            }
        })
    }

    pub fn set_pinned(&self, key: &str, pinned: bool) -> Result<()> {
        self.edit_manual(|m| {
            m.pinned.retain(|k| k != key);
            if pinned {
                m.pinned.push(key.to_string());
            }
        })
    }

    // ------------------------------------------------------------ the farming board

    /// Puts a task on the board of the account shown, or takes it off.
    pub fn set_task(&self, uid: &str, on_board: bool) -> Result<()> {
        if on_board && tasks::thing(&self.view(), uid).is_none() {
            return Err(
                UserError::new("Only equipment, mods, arcanes and resources can go on the farming board.").into()
            );
        }
        self.edit_manual(|m| {
            if !on_board {
                m.tasks.retain(|t| t.uid != uid);
            } else if !m.has_task(uid) {
                m.tasks.push(accounts::Task { uid: uid.to_string(), target: None, added: now() });
            }
        })
    }

    /// How many of a mod, an arcane or a resource a task asks for; `None` for its default.
    pub fn set_task_target(&self, uid: &str, target: Option<i64>) -> Result<()> {
        if !self.view().manual.has_task(uid) {
            return Err(UserError::new("That is not on the farming board.").into());
        }
        let target = target.map(|t| t.clamp(1, 1_000_000_000));
        self.edit_manual(|m| {
            for t in m.tasks.iter_mut().filter(|t| t.uid == uid) {
                t.target = target;
            }
        })
    }

    /// Takes the finished tasks off the board; how many there were.
    pub fn clear_done_tasks(&self) -> Result<usize> {
        let done = tasks::done(&self.view());
        if !done.is_empty() {
            self.edit_manual(|m| m.tasks.retain(|t| !done.contains(&t.uid)))?;
        }
        Ok(done.len())
    }

    pub fn tasks(&self) -> Value {
        tasks::board(&self.view())
    }

    /// One task with everything its details show.
    pub fn task(&self, uid: &str) -> Result<Value> {
        let open = self.open_eras();
        tasks::detail(&self.view(), uid, &open)
            .ok_or_else(|| UserError::new("The farming board does not know that.").into())
    }

    // ------------------------------------------------------------ accounts

    /// Links an id typed or picked in Settings to an account (the one shown by default).
    pub fn set_account_id(self: &Arc<Self>, raw: &str, key: Option<&str>) -> Result<()> {
        let id = raw.trim().trim_matches('"').to_lowercase();
        if id.len() != 24 || !id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
            return Err(UserError::new(
                "That isn't an account id: it should be exactly 24 characters of 0-9 and a-f, like 505875fb1a4d80894a000000.",
            )
            .into());
        }
        let key = {
            let mut d = lock(&self.data);
            match key.filter(|k| d.get(k).is_some()) {
                Some(k) => k.to_string(),
                None => d.active_or_new(),
            }
        };
        self.confirm(&key, &id, "entered by you");
        Ok(())
    }

    /// Shows another account, and loads its profile when that is out of date (one request, within
    /// DE's rate limit). The next login in the game moves the window to that account again.
    pub fn switch_account(self: &Arc<Self>, key: &str) -> Result<()> {
        let due = {
            let mut d = lock(&self.data);
            let Some(acc) = d.get(key) else {
                return Err(UserError::new("There is no such account.").into());
            };
            let due = acc.id.is_some() && self.profile_stale(acc);
            if d.active.as_deref() != Some(key) {
                d.active = Some(key.to_string());
                d.switched = now();
                d.save();
            }
            due
        };
        if due {
            self.request_profile(key);
        }
        Ok(())
    }

    /// Forgets an account and deletes its data. If the game logs in to it again, it comes back
    /// empty.
    pub fn forget_account(&self, key: &str) -> Result<()> {
        let mut d = lock(&self.data);
        let Some(pos) = d.accounts.iter().position(|a| a.key == key) else {
            return Err(UserError::new("There is no such account.").into());
        };
        let gone = d.accounts.remove(pos);
        if d.game.as_deref() == Some(key) {
            d.game = None;
        }
        if d.active.as_deref() == Some(key) {
            d.active = d.game.clone().or_else(|| d.accounts.first().map(|a| a.key.clone()));
            d.switched = now();
        }
        d.profile_queue.retain(|k| k != key);
        accounts::delete(key);
        d.push_event(format!("Forgot {} and deleted its data", gone.name), "info");
        d.save();
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
        if scan_now && memscan::find_game_pid().is_some() {
            self.schedule_memscan(Duration::from_secs(3), 1);
        }
        Ok(())
    }

    // ------------------------------------------------------------ views

    /// An immutable picture of one account and everything the views read, so no lock is held
    /// while computing.
    fn view_of(&self, key: Option<&str>) -> View {
        let d = lock(&self.data);
        let acc = key.and_then(|k| d.get(k));
        let inv = acc.and_then(|a| a.inventory.clone());
        let foundry =
            inv.as_ref().map_or_else(HashSet::new, |i| i.foundry.iter().filter_map(|f| f.recipe.clone()).collect());
        View {
            cat: self.catalog(),
            profile: acc.and_then(|a| a.profile.clone()),
            inv,
            manual: Arc::new(acc.map(|a| a.manual.clone()).unwrap_or_default()),
            market: Arc::clone(&self.market),
            foundry,
        }
    }

    /// The account shown.
    pub fn view(&self) -> View {
        let key = lock(&self.data).active.clone();
        self.view_of(key.as_deref())
    }

    /// The key of the account shown.
    pub fn active_key(&self) -> Option<String> {
        lock(&self.data).active.clone()
    }

    /// The account playing, else the one shown: what a reward screen is judged against.
    fn game_view(&self) -> (Option<String>, View) {
        let key = {
            let d = lock(&self.data);
            d.game.clone().or_else(|| d.active.clone())
        };
        let view = self.view_of(key.as_deref());
        (key, view)
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
        let (recent, active) = match self.log_status() {
            Some(s) => {
                let s = lock(&s);
                (s.recent.iter().rev().take(8).rev().cloned().collect::<Vec<_>>(), s.game_active())
            }
            None => (Vec::new(), false),
        };
        let d = lock(&self.data);
        let acc = d.active();
        let (screens, best_value) = history::totals(self.started, d.active.as_deref());
        let playing = |key: &str| d.game.as_deref() == Some(key);
        let account = match acc {
            Some(a) => json!({"key": a.key, "id": a.id, "name": a.name, "source": a.source, "playing": playing(&a.key),
                              "logName": d.game().map(|g| g.name.clone()), "evidence": a.evidence,
                              "profileError": a.profile_error}),
            None => {
                json!({"key": null, "id": null, "name": null, "logName": d.game().map(|g| g.name.clone()), "evidence": []})
            }
        };
        json!({
            "ready": v.cat.loaded_at.is_some(), "dataError": self.catalog_error(),
            "version": env!("CARGO_PKG_VERSION"), "os": std::env::consts::OS, "desktop": self.has_ui(),
            "account": account,
            "accounts": d.accounts.iter().map(|a| json!({
                "key": a.key, "name": a.name, "id": a.id, "source": a.source,
                "active": d.active.as_deref() == Some(a.key.as_str()), "playing": playing(&a.key),
                "masteryRank": a.mastery_rank(), "masteryRankMax": a.mastery_rank_max(),
                "profile": a.profile.as_ref().map(|p| p.fetched), "profileError": a.profile_error,
                "inventory": a.inventory.as_ref().map(|i| i.as_of()), "seen": a.seen,
                "evidence": a.evidence,
            })).collect::<Vec<_>>(),
            "masteryRank": acc.and_then(Account::mastery_rank),
            // The top of the badge's band (the badge for MR 16-18 is one icon), when that is all we know.
            "masteryRankMax": acc.and_then(Account::mastery_rank_max),
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
                            "error": acc.and_then(|a| a.profile_error.clone()), "items": v.profile.as_ref().map_or(0, |p| p.xp.len())},
                "inventory": {"source": inv.and_then(|i| i.source.clone()), "imported": inv.map(|i| i.imported),
                              "synced": inv.and_then(|i| i.synced_at), "stacks": inv.map_or(0, |i| i.counts.len())},
                "manual": {"counts": v.manual.counts.len(), "mastered": v.manual.mastered.len()},
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
        let v = self.view();
        let source = if v.inv.is_some() { "inventory" } else { "manual" };
        json!({"rows": v.trade(), "source": source})
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

    /// The rank of the account shown, as far as it is known.
    fn active_rank(&self) -> Option<i64> {
        lock(&self.data).active().and_then(Account::mastery_rank)
    }

    pub fn world(&self) -> Value {
        let platform = self.settings().platform;
        let mut w = world::summary(platform);
        let view = self.view();
        world::add_art(&mut w, &view.cat);
        // Where the account shown stands in the running Nightwave season.
        if let Some(tag) = world::nightwave_tag(platform) {
            let s = standing::summary(&view, self.active_rank(), Some(&tag));
            let season = s["syndicates"].as_array().and_then(|rows| rows.iter().find(|r| r["kind"] == "nightwave"));
            if let (Some(season), true) = (season, w["nightwave"].is_object()) {
                w["nightwave"]["mine"] = season.clone();
            }
        }
        w
    }

    /// Syndicate standing of the account shown.
    pub fn standing(&self) -> Value {
        let tag = world::nightwave_tag(self.settings().platform);
        let mut s = standing::summary(&self.view(), self.active_rank(), tag.as_deref());
        s["account"] = json!(lock(&self.data).active().map(|a| a.name.clone()));
        s
    }

    /// One syndicate with its offerings priced.
    pub fn syndicate(&self, tag: &str) -> Result<Value> {
        standing::detail(&self.view(), tag, self.active_rank())
            .ok_or_else(|| UserError::new(format!("No syndicate is called {tag}.")).into())
    }

    /// Relic eras with a fissure open right now ("Omnia" opens them all); storms are Railjack's.
    fn open_eras(&self) -> HashSet<String> {
        let (fissures, _) = world::fissures(self.settings().platform);
        fissures.iter().filter(|f| f["storm"] != true).filter_map(|f| f["tier"].as_str().map(str::to_owned)).collect()
    }

    /// Everything about one item, part or relic, with which relic farms are open right now.
    pub fn detail(&self, key: &str) -> Result<Value> {
        let open = self.open_eras();
        let view = self.view();
        let mut v = view.detail(key, &open).ok_or_else(|| UserError::new(format!("Nothing is called {key}.")))?;
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

    pub fn history(&self) -> Value {
        let key = lock(&self.data).active.clone();
        let (screens, best_value) = history::totals(self.started, key.as_deref());
        json!({"entries": history::recent(40, key.as_deref()),
               "session": {"since": self.started, "screens": screens, "bestValue": best_value}})
    }

    /// Desktop notifications, when switched on: watched fissures, Baro, finished builds.
    fn notify_tick(&self) {
        let s = self.settings();
        // Builds of every account; with more than one account in play, the name says whose.
        let builds: Vec<notifier::Build> = if s.notify_foundry {
            let cat = self.catalog();
            let d = lock(&self.data);
            let several = d.accounts.iter().filter(|a| a.inventory.is_some()).count() > 1;
            d.accounts
                .iter()
                .flat_map(|a| {
                    let cat = &cat;
                    a.inventory.iter().flat_map(move |i| {
                        i.foundry.iter().filter_map(move |f| {
                            let uid = f.recipe.clone()?;
                            let name = cat.name_of(&uid);
                            let shown = if several { format!("{name} ({})", a.name) } else { name };
                            Some((format!("{}:{uid}", a.key), shown, f.done?))
                        })
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let baro = if s.notify_baro { Some(self.world()["baro"].clone()) } else { None };
        let mut n = lock(&self.notifier);
        if s.notify_fissures {
            n.fissures(s.platform, &s.fissure_watch);
        }
        if let Some(baro) = baro {
            n.baro(&baro);
        }
        if s.notify_foundry {
            n.foundry(&builds);
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
                    let image = self.market.image(Some(&slug));
                    json!({"slug": slug, "name": name, "ducats": ducats, "sell": p.sell, "buy": p.buy, "image": image})
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
        // A missing count means the common 3+1, but a legitimate 0 negatives must stay 0, or a
        // 3+0 riven is graded against the wrong multiplier.
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

    fn publish_rewards(&self, result: RewardResult, fresh: bool, generation: u64) -> Arc<RewardResult> {
        let result = Arc::new(result);
        let mut scan = lock(&self.reward_scan);
        if scan.is_current(generation) {
            scan.result = Some(Arc::clone(&result));
            *lock(&self.rewards) = Some(Arc::clone(&result));
            self.ui(UiEvent::Rewards { result: Arc::clone(&result), fresh });
        }
        result
    }

    /// The interface scales to look for the reward cards at: EE.cfg's, then the one they were
    /// last found at if that differs (the game may write a changed scale only when it exits).
    /// Empty when neither is known: then every scale is tried.
    pub fn ui_scales(&self) -> Vec<f64> {
        let cfg = self.log_path().and_then(|log| eelog::ui_scale(&log));
        let found = *lock(&self.found_scale);
        cfg.into_iter().chain(found.filter(|f| cfg.is_none_or(|c| (c - f).abs() > 1e-6))).collect()
    }

    /// Screenshots the reward screen, reads the names, and prices them (the hotkey and the API).
    pub fn scan_rewards(self: &Arc<Self>) -> Arc<RewardResult> {
        self.scan_rewards_for(HOTKEY_WAIT, None).expect("a manual reward scan is not cancelled before starting")
    }

    /// Reads the reward screen: `screen` is the log's reward screen this scan belongs to (None for
    /// the hotkey), and the scan gives up quietly once that screen has closed.
    fn scan_rewards_for(self: &Arc<Self>, wait: Duration, screen: Option<u64>) -> Option<Arc<RewardResult>> {
        let mut slot = lock(&self.reward_scan);
        if screen.is_some_and(|screen| screen != slot.screen_generation) {
            return None;
        }
        if !slot.done || now() - slot.started < COALESCE {
            // The same screen as a scan still running or just finished: share its result, prices
            // included.
            let (done, _) = self
                .reward_done
                .wait_timeout_while(slot, Duration::from_secs(15), |s| !s.done)
                .unwrap_or_else(|e| e.into_inner());
            if let Some(r) = &done.result {
                return Some(Arc::clone(r));
            }
            slot = done;
            if screen.is_some_and(|screen| screen != slot.screen_generation) {
                return None;
            }
        }
        slot.started = now();
        slot.done = false;
        slot.generation = slot.generation.wrapping_add(1);
        slot.result = None;
        let generation = slot.generation;
        drop(slot);
        let _finished = ScanFinished(self, generation);
        let capture = lock(&self.scan_lock);
        let settings = self.settings();
        self.hide_overlays_for_capture();

        let cancelled = || screen.is_some() && !lock(&self.reward_scan).is_current(generation);
        // Names are shown as soon as a card is read; the rest of them, and the prices, follow in
        // place. The choice timer is 15 s and warframe.market can take seconds.
        let mut shown = false;
        let mut partial = |layout: &reward::Layout, names: &[Option<RewardMatch>]| {
            let (_, result) = self.reward_result(names, Some(layout), String::new());
            self.publish_rewards(result, !shown, generation);
            shown = true;
        };
        let (frame, layout, names, text) = match self.find_cards(wait, &cancelled, &mut partial) {
            Ok(Sighting::Cancelled) => return None,
            Ok(Sighting::Cards { frame, layout, names, text }) if names.iter().any(Option::is_some) => {
                (frame, Some(layout), names, text)
            }
            Ok(Sighting::Cards { frame, .. } | Sighting::NoCards(frame)) if screen.is_some() => {
                // The log announced the rewards, but no card with a readable name showed up:
                // there is nothing to put over the game. The frame is kept to find out why.
                self.event("Reward screen: no reward card could be read on screen", "warn");
                drop(capture);
                ocr::archive("reward", frame, String::new(), json!({"cards": null, "items": []}));
                return None;
            }
            Ok(Sighting::Cards { frame, .. } | Sighting::NoCards(frame)) => {
                // The hotkey on a screen without readable cards (another theme, a scale they
                // were not found at): look for names anywhere in the middle of the screen.
                match ocr::read_rewards(&frame, &self.names()) {
                    Ok((found, text)) => (frame, None, found.into_iter().map(Some).collect(), text),
                    Err(e) => return Some(self.reward_scan_failed(&e, generation)),
                }
            }
            Err(e) => return Some(self.reward_scan_failed(&e, generation)),
        };
        if cancelled() {
            return None;
        }
        if let Some(layout) = &layout {
            if !self.ui_scales().iter().any(|s| (s - layout.ui_scale).abs() < 1e-6) {
                *lock(&self.found_scale) = Some(layout.ui_scale);
            }
        }
        let (account, base) = self.reward_result(&names, layout.as_ref(), tail(&text, 800));
        self.publish_rewards(base.clone(), !shown, generation);
        // The screen is read; a riven scan may run while the prices load.
        drop(capture);
        ocr::archive("reward", frame, text, matches_json(&base));
        if let Some(layout) = layout {
            self.watch_cards(layout, generation);
        }

        let mut items = base.items.clone();
        let slugs: Vec<Option<String>> = items.iter().map(|i| i.slug.clone()).collect();
        let prices = self.market.fetch_now(&slugs, Duration::from_secs(6));
        for it in &mut items {
            if let Some(sell) = it.slug.as_ref().and_then(|s| prices.get(s)).and_then(|p| p.sell) {
                it.price = Some(sell);
            }
        }
        let best = best_pick(&items);
        // The history keeps what a reward screen offered, even when the screen has closed in the
        // meantime; names found without cards may be another screen's, so they are not kept.
        if layout.is_some() && !items.is_empty() {
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
                account,
            });
        }
        if cancelled() {
            return None;
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
        Some(self.publish_rewards(RewardResult { items, best, ..base }, false, generation))
    }

    fn reward_scan_failed(&self, e: &anyhow::Error, generation: u64) -> Arc<RewardResult> {
        self.event(&format!("Reward scan failed: {e}"), "warn");
        self.publish_rewards(
            RewardResult { t: now(), error: Some(e.to_string()), ..Default::default() },
            true,
            generation,
        )
    }

    /// Looks at the screen until the reward cards are up and read, or `wait` runs out.
    ///
    /// The scan starts at the log's open line, seconds before the cards turn over, so the screen is
    /// captured every `REWARD_POLL` until the cards' rarity bars show. A card may still be fading
    /// in then: what is read is passed to `partial` and the screen read again, up to
    /// `REWARD_READS` times, each card keeping the best name any reading gave it.
    fn find_cards(
        &self,
        wait: Duration,
        cancelled: &dyn Fn() -> bool,
        partial: &mut dyn FnMut(&reward::Layout, &[Option<RewardMatch>]),
    ) -> Result<Sighting> {
        let tool = self.settings().screenshot_tool;
        let names = self.names();
        let scales = self.ui_scales();
        let deadline = Instant::now() + wait;
        let poll = if screenshot::cheap(tool) { REWARD_POLL } else { SLOW_POLL };
        let (mut reads, mut attempts) = (0, 0);
        let mut seen: Option<(Rgb, reward::Layout, Vec<Option<RewardMatch>>, String)> = None;
        loop {
            if cancelled() {
                return Ok(Sighting::Cancelled);
            }
            let frame = screenshot::capture(tool)?;
            let late = Instant::now() >= deadline;
            // A scale that EE.cfg does not know, or knows wrong: every scale, at the end.
            let layout = reward::find_layout(&frame, &scales)
                .or_else(|| if late && !scales.is_empty() { reward::find_layout(&frame, &[]) } else { None });
            let Some(layout) = layout else {
                if late {
                    return Ok(match seen {
                        Some((frame, layout, names, text)) => Sighting::Cards { frame, layout, names, text },
                        None => Sighting::NoCards(frame),
                    });
                }
                std::thread::sleep(poll);
                continue;
            };
            let read = reward::read(&frame, &layout, &names)?;
            attempts += 1;
            if read.names.iter().any(Option::is_some) {
                reads += 1;
            }
            let (mut merged, before) = match seen.take() {
                Some((_, l, names, _)) if l.cards == layout.cards => (names.clone(), names),
                _ => (vec![None; layout.cards], vec![None; layout.cards]),
            };
            for (slot, new) in merged.iter_mut().zip(read.names) {
                if let Some(new) = new {
                    if slot.as_ref().is_none_or(|old| reward::better(&new, old)) {
                        *slot = Some(new);
                    }
                }
            }
            let sure = merged.iter().all(|m| m.as_ref().is_some_and(|m| m.score >= SURE));
            if late || sure || reads >= REWARD_READS || attempts >= REWARD_ATTEMPTS {
                return Ok(Sighting::Cards { frame, layout, names: merged, text: read.text });
            }
            if merged != before {
                partial(&layout, &merged);
            }
            seen = Some((frame, layout, merged, read.text));
            std::thread::sleep(poll);
        }
    }

    /// The reward items for what the cards say, with everything known without asking the
    /// market: counts, needs, vaulted, ducats and the cached price. Owned and needed are the
    /// playing account's, whichever account the window shows; that account is returned too.
    fn reward_result(
        &self,
        names: &[Option<RewardMatch>],
        layout: Option<&reward::Layout>,
        ocr: String,
    ) -> (Option<String>, RewardResult) {
        let (account, v) = self.game_view();
        let needed = v.needed_parts();
        let items: Vec<RewardItem> = names
            .iter()
            .enumerate()
            .filter_map(|(k, m)| {
                let m = m.as_ref()?;
                let part = v.cat.parts.get(v.cat.part_alias.get(&m.uid).unwrap_or(&m.uid));
                let slug = self.market.slug_for(Some(&m.uid), part.and_then(|p| p.slug.as_deref()), Some(&m.name));
                Some(RewardItem {
                    name: m.name.clone(),
                    uid: m.uid.clone(),
                    score: m.score,
                    have: v.count(&m.uid),
                    needed: needed.get(&m.uid).copied().unwrap_or(0),
                    vaulted: part.map(|p| p.vaulted),
                    ducats: self.market.ducats(slug.as_deref()),
                    price: slug.as_deref().and_then(|s| self.market.cached(s)).and_then(|p| p.sell),
                    slug,
                    slot: layout.map(|_| k),
                })
            })
            .collect();
        let result = RewardResult {
            t: now(),
            best: best_pick(&items),
            ocr,
            error: items.is_empty().then(|| "No reward names recognized.".to_string()),
            cards: layout.map(|l| l.cards),
            scale: layout.map(|l| l.ui_scale),
            items,
        };
        (account, result)
    }

    /// Hides the prices as soon as the cards leave the screen. The log says so too ("Relic reward
    /// screen shut down"), but a little later, and a line can be missed; this looks for the cards'
    /// bars every `CARDS_CHECK` and hides after two looks without them. Only with a screenshot
    /// tool that is quick and silent.
    fn watch_cards(self: &Arc<Self>, layout: reward::Layout, generation: u64) {
        let tool = self.settings().screenshot_tool;
        if !screenshot::cheap(tool) {
            return;
        }
        let me = Arc::clone(self);
        let watch = move || {
            let until = Instant::now() + Duration::from_secs(40);
            let mut misses = 0;
            while Instant::now() < until {
                std::thread::sleep(CARDS_CHECK);
                if !lock(&me.reward_scan).is_current(generation) {
                    return;
                }
                let Ok(frame) = screenshot::capture(tool) else { return };
                misses = if reward::still_up(&frame, &layout) { 0 } else { misses + 1 };
                if misses >= 2 {
                    let mut scan = lock(&me.reward_scan);
                    if scan.is_current(generation) {
                        scan.invalidate();
                        drop(scan);
                        me.reward_done.notify_all();
                        me.ui(UiEvent::HideRewards);
                    }
                    return;
                }
            }
        };
        std::thread::Builder::new().name("reward-watch".into()).spawn(watch).ok();
    }

    fn riven_failed(&self, weapon: Option<String>, text: &str, error: String) -> Arc<RivenResult> {
        let r =
            Arc::new(RivenResult { t: now(), weapon, ocr: tail(text, 600), error: Some(error), ..Default::default() });
        *lock(&self.riven) = Some(Arc::clone(&r));
        if self.settings().riven_overlay_enabled {
            self.ui(UiEvent::Riven { result: Arc::clone(&r), fresh: true });
        }
        r
    }

    /// Reads the riven card currently on screen and grades it.
    pub fn scan_riven(self: &Arc<Self>) -> Arc<RivenResult> {
        let _one_scan_at_a_time = lock(&self.scan_lock);
        let settings = self.settings();
        self.hide_overlays_for_capture();
        let cat = self.catalog();
        let scales = self.ui_scales();
        let read = screenshot::capture(settings.screenshot_tool).and_then(|frame| {
            let (r, text) = ocr::read_riven(&frame, &cat.riven_weapons(), &scales)?;
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
        let overall = (!stats.is_empty())
            .then(|| rivens::grade_for(stats.iter().map(|g| g.deviation).sum::<f64>() / stats.len() as f64));
        let r = Arc::new(RivenResult {
            t: now(),
            weapon: Some(it.name.clone()),
            image: crate::gamedata::image_url(it.image.as_deref()),
            positives: Some(pos),
            negatives: Some(neg),
            unread,
            stats,
            overall,
            traded: rivens::traded(&it.name, settings.platform),
            ocr: tail(&text, 600),
            ..Default::default()
        });
        *lock(&self.riven) = Some(Arc::clone(&r));
        if settings.riven_overlay_enabled {
            self.ui(UiEvent::Riven { result: Arc::clone(&r), fresh: true });
        }
        self.find_riven_listings(&r, it.uid.clone());
        r
    }

    /// Looks up rivens like `r` on warframe.market in the background and updates the result (and
    /// the panel, if it is still up) when the answer comes.
    fn find_riven_listings(self: &Arc<Self>, r: &Arc<RivenResult>, uid: String) {
        let positives: Vec<&'static str> =
            r.stats.iter().filter(|g| !g.negative).filter_map(|g| rivens::market_slug(g.stat)).collect();
        let (Some(weapon), false) = (r.weapon.clone(), positives.is_empty()) else { return };
        let (state, r) = (Arc::clone(self), Arc::clone(r));
        let look = move || {
            // The panel shows the weapon's picture only from the art cache.
            if let Some(url) = &r.image {
                let _ = crate::art::get(url, 128);
            }
            let mut updated = (*r).clone();
            match state.market.riven_listings(&uid, &weapon, &positives) {
                Ok(found) => updated.market = Some(found),
                Err(e) => updated.market_error = Some(format!("{e:#}")),
            }
            let updated = Arc::new(updated);
            {
                let mut current = lock(&state.riven);
                // A newer riven was read meanwhile: this answer belongs to the old one.
                if current.as_ref().is_none_or(|c| c.t != r.t) {
                    return;
                }
                *current = Some(Arc::clone(&updated));
            }
            if state.settings().riven_overlay_enabled {
                state.ui(UiEvent::Riven { result: updated, fresh: false });
            }
        };
        if let Err(e) = std::thread::Builder::new().name("riven-market".into()).spawn(look) {
            eprintln!("could not look up riven listings: {e}");
        }
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

    /// Windows: the app holds the keys itself (`compositor::register_hotkeys`), from startup on.
    #[cfg(windows)]
    pub fn install_hotkeys(&self) -> Result<Value> {
        let (port, keys) = self.hotkey_keys();
        let pairs: Vec<(&str, &str)> = keys.iter().map(|(k, p)| (k.as_str(), *p)).collect();
        let lines = compositor::register_hotkeys(port, &pairs)?;
        let names: Vec<&str> = pairs.iter().map(|(k, _)| *k).filter(|k| !k.trim().is_empty()).collect();
        self.event(&format!("Hotkeys registered: {}", names.join(", ")), "info");
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
        self.event(&format!("Hotkeys written to {name} and the compositor reloaded"), "info");
        Ok(json!({"file": done.file, "sourced": done.sourced, "lines": lines}))
    }
}

struct RewardScan {
    started: f64,
    done: bool,
    generation: u64,
    screen_generation: u64,
    result: Option<Arc<RewardResult>>,
}

impl RewardScan {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.screen_generation = self.screen_generation.wrapping_add(1);
        self.started = f64::NEG_INFINITY;
        self.done = true;
        self.result = None;
    }
    fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }

    fn finish(&mut self, generation: u64) {
        if self.is_current(generation) {
            self.done = true;
        }
    }
}

/// Marks the running reward scan finished, however it ends, and wakes callers sharing it.
struct ScanFinished<'a>(&'a State, u64);

impl Drop for ScanFinished<'_> {
    fn drop(&mut self) {
        lock(&self.0.reward_scan).finish(self.1);
        self.0.reward_done.notify_all();
    }
}

fn matches_json(r: &RewardResult) -> Value {
    let items: Vec<Value> =
        r.items.iter().map(|i| json!({"name": i.name, "uid": i.uid, "score": i.score, "slot": i.slot})).collect();
    json!({"cards": r.cards, "scale": r.scale, "items": items})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn older_reward_scan_cannot_finish_the_newer_scan() {
        let mut scan = RewardScan { started: 10.0, done: false, generation: 2, screen_generation: 0, result: None };
        assert!(!scan.is_current(1));
        scan.finish(1);
        assert!(!scan.done);
        assert!(scan.is_current(2));
        scan.finish(2);
        assert!(scan.done);
    }

    #[test]
    fn closing_reward_screen_invalidates_pending_and_running_scans() {
        let mut scan = RewardScan { started: 10.0, done: false, generation: 2, screen_generation: 5, result: None };
        scan.invalidate();
        assert_ne!(scan.screen_generation, 5);
        assert!(!scan.is_current(2));
        assert!(scan.done);
        assert_eq!(scan.started, f64::NEG_INFINITY);
        scan.finish(2);
        assert!(scan.done);
    }

    #[test]
    fn scheduled_scans_wait_for_the_active_scan_and_cooldown() {
        let mut status =
            MemscanStatus { state: "scanning", last_attempt: 100.0, next: Some(103.0), ..Default::default() };
        assert_eq!(status.start_scheduled(103.0, true, Some(1), 103.0), ScheduledScan::Wait);
        assert_eq!(status.next, Some(103.0));
        status.state = "idle";
        assert_eq!(status.start_scheduled(103.0, true, Some(1), 109.0), ScheduledScan::Wait);
        assert_eq!(status.start_scheduled(103.0, true, Some(1), 110.0), ScheduledScan::Start);
        assert_eq!(status.state, "scanning");
        assert_eq!(status.next, None);
        assert_eq!(status.start_scheduled(103.0, true, Some(1), 111.0), ScheduledScan::Superseded);
        status.next = Some(120.0);
        assert_eq!(status.start_scheduled(120.0, false, Some(1), 120.0), ScheduledScan::Cancelled);
        assert_eq!(status.next, None);
    }

    #[test]
    fn a_complete_capture_is_preferred_over_a_newer_fragment() {
        let data = Data { accounts: vec![Account::new("alpha".into(), "Alpha".into())], ..Data::default() };
        let whole = json!({"MiscItems": [], "Suits": [], "LastInventorySync": 1});
        let fragment = json!({"MiscItems": [], "LastInventorySync": 2});
        let mut sections = memscan::Sections::default();
        for _ in 0..3 {
            sections.learn(&whole);
        }
        let (chosen, _, _) = choose_capture(&data, "alpha", &sections, vec![fragment.clone(), whole.clone()]);
        assert_eq!(chosen, Some(whole));
        let (chosen, _, missing) = choose_capture(&data, "alpha", &sections, vec![fragment]);
        assert!(chosen.is_none());
        assert_eq!(missing, Some(vec!["Suits".to_string()]));
    }

    fn account(key: &str, xp: &[(&str, i64)]) -> Account {
        let mut acc = Account::new(key.into(), key.to_uppercase());
        acc.profile =
            Some(Arc::new(Profile { xp: xp.iter().map(|(k, v)| (k.to_string(), *v)).collect(), ..Profile::default() }));
        acc
    }

    #[test]
    fn a_capture_goes_to_the_account_its_mastery_xp_belongs_to() {
        let items: Vec<(String, i64)> = (0..12).map(|i| (format!("/I{i}"), 100 + i)).collect();
        let mine: Vec<(&str, i64)> = items.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        let theirs: Vec<(&str, i64)> = items.iter().map(|(k, v)| (k.as_str(), *v + 5000)).collect();
        let d = Data {
            accounts: vec![
                account("alpha", &mine),
                account("beta", &theirs),
                Account::new("gamma".into(), "Gamma".into()),
            ],
            ..Data::default()
        };
        let xp = |pairs: &[(&str, i64)]| pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect::<IndexMap<_, _>>();
        assert!(matches!(owner(&d, "alpha", &xp(&mine)), Owner::Game));
        assert!(matches!(owner(&d, "alpha", &xp(&theirs)), Owner::Other(n) if n == "BETA"));
        // Gamma has nothing to compare with: a capture nobody else claims is taken as Gamma's...
        let fresh: Vec<(&str, i64)> = items.iter().map(|(k, v)| (k.as_str(), *v + 9000)).collect();
        assert!(matches!(owner(&d, "gamma", &xp(&fresh)), Owner::Unknown));
        // ...but Alpha's own inventory, still in the game's memory, is not.
        assert!(matches!(owner(&d, "gamma", &xp(&mine)), Owner::Other(n) if n == "ALPHA"));
        // Alpha's data says no and nobody claims it: some account the app has never seen.
        assert!(matches!(owner(&d, "alpha", &xp(&fresh)), Owner::Stranger));
    }

    #[test]
    fn a_new_name_is_a_new_account_with_a_key_of_its_own() {
        let mut d = Data::default();
        let a = d.ensure("Tenno\u{e000}");
        assert_eq!(d.ensure("tenno"), a, "names compare without the glyph and case");
        let b = d.ensure("Tenno2");
        assert_ne!(a, b);
        assert_eq!(d.get(&a).map(|x| x.name.as_str()), Some("Tenno"));
        assert_eq!(d.active_or_new(), d.accounts[0].key, "with nothing shown yet, the first account");
    }
}
