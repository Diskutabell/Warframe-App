//! The accounts as the app sees them: every source merged, and DE's profile rate limit.
//!
//! Two accounts matter at any time: the one the window shows (`active`), and the one the game is
//! logged in to (`game`, from EE.log). They are usually the same - a login moves the window to
//! its account - but the user may look at the other one meanwhile. What the game produces (memory
//! captures, reward screens, mastery badges, ids in the log) belongs to `game`; what the user does
//! in the window (counts, pins, imports, a profile refresh) to `active`.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::catalog::CatalogStore;
use super::settings::SettingsStore;
use super::watched::Watched;
use crate::accounts::{self, Account, Evidence};
use crate::bus::{Bus, Topic};
use crate::eelog::{self, DuviriOffers};
use crate::market::Market;
use crate::memscan;
use crate::notifier;
use crate::sources::{self, Inventory, Profile};
use crate::views::View;
use crate::{UserError, net, now, tasks};

/// DE temporarily IP-blocks game login for clients that query profiles too often.
pub const PROFILE_COOLDOWN: f64 = 300.0;
/// After DE's profile service refuses a request (HTTP 403, which is how it rate-limits), it is
/// left alone this long: asking again at every cooldown would keep the address blocked.
pub const PROFILE_PAUSE: f64 = 3600.0;
/// Activity lines kept.
const EVENTS_KEPT: usize = 60;

/// One line of the activity list.
#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub t: f64,
    pub text: String,
    pub kind: &'static str,
}

#[derive(Default)]
pub(crate) struct Data {
    pub(crate) accounts: Vec<Account>,
    /// The account the window shows.
    pub(crate) active: Option<String>,
    /// When `active` last changed (saved): a log read back at startup moves the window to its
    /// account only if that session is newer than the user's own choice.
    switched: f64,
    /// The account the game is logged in to, from the log's latest "Logged in <name>".
    pub(crate) game: Option<String>,
    events: VecDeque<Event>,
    /// The profile service's rate limit: when it was last asked, and until when it is left alone
    /// after a refusal. Saved with the accounts, so a restart cannot shorten the gap.
    profile_requests: accounts::ProfileTimes,
    /// Accounts whose profile is due, waiting for DE's rate limit.
    profile_queue: VecDeque<String>,
    profile_worker: bool,
}

impl Data {
    pub(crate) fn get(&self, key: &str) -> Option<&Account> {
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
        if let Err(e) = accounts::save(&self.active, self.switched, self.profile_requests, &self.accounts) {
            eprintln!("could not save the account list: {e:#}");
        }
    }

    /// Seconds until DE's profile service may be asked again: the cooldown since the last
    /// request, or longer while the service is left alone after a refusal. Zero or less: now.
    fn profile_wait(&self) -> f64 {
        (self.profile_requests.requested + PROFILE_COOLDOWN).max(self.profile_requests.paused_until) - now()
    }

    /// Whether the service is being left alone after a refusal.
    fn profile_paused(&self) -> bool {
        now() < self.profile_requests.paused_until
    }

    fn push_event(&mut self, text: String, kind: &'static str) {
        self.events.push_back(Event { t: now(), text, kind });
        while self.events.len() > EVENTS_KEPT {
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

fn same_name(a: &str, b: &str) -> bool {
    let clean = |s: &str| eelog::strip_glyphs(s).to_lowercase();
    !a.is_empty() && !b.is_empty() && clean(a) == clean(b)
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

/// Which of a memory scan's captures to file for the logged-in account: the first that is its own
/// and complete. Otherwise whose the captures were, or what the best of its own lacked.
pub struct CaptureChoice {
    pub chosen: Option<Value>,
    pub held_by: Option<String>,
    pub missing: Option<Vec<String>>,
}

fn choose_capture(d: &Data, game: &str, sections: &memscan::Sections, captures: Vec<Value>) -> CaptureChoice {
    let mut held_by = None;
    let mut missing = None;
    for capture in captures {
        match owner(d, game, &sources::xp_map(capture.get("XPInfo"))) {
            Owner::Game | Owner::Unknown => {
                let absent = sections.missing(&capture);
                if absent.is_empty() {
                    return CaptureChoice { chosen: Some(capture), held_by, missing };
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
    CaptureChoice { chosen: None, held_by, missing }
}

/// Whose a fetched or imported profile is, by the name it carries.
#[derive(Debug, PartialEq)]
enum Target {
    /// The account it was asked for (same name, or no name to go by).
    Key,
    /// The account it was asked for, renamed in the game since: it takes the new name.
    Renamed(String),
    /// Another account the app knows by that name.
    Other(String),
    /// An account the app has not seen yet.
    New(String),
}

/// Files a profile asked for account `key` (with account id `id`, when one was asked for). Its
/// display name decides. With no account of that name, `key` may have been renamed in the game -
/// but only while it is not the one logged in (the game would say its name), and only when
/// nothing says the profile is another account's: another id, or mastery XP that is not `key`'s.
fn profile_target(d: &Data, key: &str, prof: &Profile, id: Option<&str>) -> Target {
    let Some(name) = prof.name.as_deref().map(eelog::strip_glyphs).filter(|n| !n.is_empty()) else {
        return Target::Key;
    };
    if d.get(key).is_some_and(|a| same_name(&a.name, &name)) {
        return Target::Key;
    }
    if let Some(other) = d.by_name(&name) {
        return Target::Other(other.key.clone());
    }
    let may_be_renamed = |acc: &Account| {
        let same_id = match (acc.id.as_deref(), id.or(prof.id.as_deref())) {
            (Some(mine), Some(theirs)) => mine == theirs,
            _ => true,
        };
        same_id && acc.known_xp().into_iter().all(|known| accounts::same_owner(&prof.xp, known) != Some(false))
    };
    if d.game.as_deref() != Some(key) && d.get(key).is_some_and(may_be_renamed) {
        Target::Renamed(name)
    } else {
        Target::New(name)
    }
}

/// What the summary shows about the accounts.
pub struct AccountsSummary {
    pub active: Option<String>,
    pub account: Value,
    pub accounts: Vec<Value>,
    pub mastery_rank: Option<i64>,
    pub mastery_rank_max: Option<i64>,
    pub profile_error: Option<String>,
    pub events: Vec<Event>,
}

pub struct AccountService {
    data: Watched<Data>,
    settings: Arc<SettingsStore>,
    catalog: Arc<CatalogStore>,
    market: Arc<Market>,
    bus: Bus,
    stop: CancellationToken,
}

/// Runs file work (the accounts' JSON files) on the blocking pool from async code.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|e| anyhow!("an account task failed: {e}"))
}

impl AccountService {
    pub fn new(
        settings: Arc<SettingsStore>,
        catalog: Arc<CatalogStore>,
        market: Arc<Market>,
        bus: &Bus,
        stop: CancellationToken,
    ) -> Arc<AccountService> {
        let (registry, notes) = accounts::load();
        let mut data = Data {
            accounts: registry.accounts,
            active: registry.active,
            switched: registry.switched,
            // Never in the future: a clock set back must not hold requests up for longer than
            // the cooldown or the pause themselves.
            profile_requests: accounts::ProfileTimes {
                requested: registry.profile_requests.requested.min(now()),
                paused_until: registry.profile_requests.paused_until.min(now() + PROFILE_PAUSE),
            },
            ..Data::default()
        };
        // A shown account that is gone (its folder deleted by hand): show the first one instead.
        if data.active().is_none() {
            data.active = data.accounts.first().map(|a| a.key.clone());
        }
        for note in notes {
            data.push_event(note, "info");
        }
        Arc::new(AccountService { data: Watched::new(data, bus), settings, catalog, market, bus: bus.clone(), stop })
    }

    /// Adds a line to the activity list.
    pub fn event(&self, text: &str, kind: &'static str) {
        self.data.write().push_event(text.to_string(), kind);
    }

    // ------------------------------------------------------------ views

    /// An immutable picture of one account and everything the views read, so no lock is held
    /// while computing.
    pub fn view_of(&self, key: Option<&str>) -> View {
        let d = self.data.read();
        let acc = key.and_then(|k| d.get(k));
        let inv = acc.and_then(|a| a.inventory.clone());
        let foundry =
            inv.as_ref().map_or_else(HashSet::new, |i| i.foundry.iter().filter_map(|f| f.recipe.clone()).collect());
        View {
            cat: self.catalog.get(),
            profile: acc.and_then(|a| a.profile.clone()),
            inv,
            manual: Arc::new(acc.map(|a| a.manual.clone()).unwrap_or_default()),
            market: Arc::clone(&self.market),
            foundry,
        }
    }

    /// The account shown.
    pub fn view(&self) -> View {
        let key = self.data.read().active.clone();
        self.view_of(key.as_deref())
    }

    /// The key of the account shown.
    pub fn active_key(&self) -> Option<String> {
        self.data.read().active.clone()
    }

    /// The account playing, else the one shown: what a reward screen is judged against.
    pub fn game_view(&self) -> (Option<String>, View) {
        let key = {
            let d = self.data.read();
            d.game.clone().or_else(|| d.active.clone())
        };
        let view = self.view_of(key.as_deref());
        (key, view)
    }

    /// The rank of the account shown, as far as it is known.
    pub fn active_rank(&self) -> Option<i64> {
        self.data.read().active().and_then(Account::mastery_rank)
    }

    pub fn active_name(&self) -> Option<String> {
        self.data.read().active().map(|a| a.name.clone())
    }

    /// What Duviri offers the account shown, as the game last logged it.
    pub fn duviri_offers(&self) -> Option<DuviriOffers> {
        self.data.read().active().and_then(|a| a.duviri.clone())
    }

    /// The key and name of the account the game is logged in to.
    pub fn game_account(&self) -> Option<(String, String)> {
        let d = self.data.read();
        d.game.clone().map(|key| {
            let name = d.name_of(&key);
            (key, name)
        })
    }

    pub fn summary(&self) -> AccountsSummary {
        let d = self.data.read();
        let acc = d.active();
        let playing = |key: &str| d.game.as_deref() == Some(key);
        let account = match acc {
            Some(a) => {
                json!({"key": a.key, "id": a.id, "name": a.name, "source": a.source, "playing": playing(&a.key),
                       "logName": d.game().map(|g| g.name.clone()), "evidence": a.evidence,
                       "profileError": a.profile_error})
            }
            None => {
                json!({"key": null, "id": null, "name": null, "logName": d.game().map(|g| g.name.clone()), "evidence": []})
            }
        };
        let accounts = d
            .accounts
            .iter()
            .map(|a| {
                json!({
                    "key": a.key, "name": a.name, "id": a.id, "source": a.source,
                    "active": d.active.as_deref() == Some(a.key.as_str()), "playing": playing(&a.key),
                    "masteryRank": a.mastery_rank(), "masteryRankMax": a.mastery_rank_max(),
                    "profile": a.profile.as_ref().map(|p| p.fetched), "profileError": a.profile_error,
                    "inventory": a.inventory.as_ref().map(|i| i.as_of()), "seen": a.seen,
                    "evidence": a.evidence,
                })
            })
            .collect();
        AccountsSummary {
            active: d.active.clone(),
            account,
            accounts,
            mastery_rank: acc.and_then(Account::mastery_rank),
            mastery_rank_max: acc.and_then(Account::mastery_rank_max),
            profile_error: acc.and_then(|a| a.profile_error.clone()),
            events: d.events.iter().rev().take(15).cloned().collect(),
        }
    }

    /// Finished and running foundry builds of every account, for the notifications. With more
    /// than one account in play, the name says whose.
    pub fn builds(&self) -> Vec<notifier::Build> {
        let cat = self.catalog.get();
        let d = self.data.read();
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
    }

    // ------------------------------------------------------------ the log

    /// "Logged in <name>": that account is the game's from now on. A new name is a new account.
    /// `log_time` is the log file's modification time.
    pub fn on_login(self: &Arc<Self>, name: &str, live: bool, log_time: Option<f64>) {
        let name = eelog::strip_glyphs(name);
        if name.is_empty() {
            return;
        }
        let (key, switched) = {
            let mut d = self.data.write();
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
            self.follow_game(true, log_time);
            let due = self.data.read().get(&key).is_some_and(|a| a.id.is_some() && self.profile_stale(a));
            if switched && due {
                self.request_profile(&key);
            }
        }
    }

    /// Shows the account the game is logged in to. A login seen live always moves the window to
    /// it; one read back from an existing log only when that session is newer than the user's
    /// last choice of account.
    pub fn follow_game(&self, live: bool, log_time: Option<f64>) {
        {
            let d = self.data.read();
            let Some(game) = d.game.as_deref() else { return };
            if d.active.as_deref() == Some(game) || (!live && log_time.is_none_or(|t| t <= d.switched)) {
                return;
            }
        }
        let mut d = self.data.write();
        let Some(game) = d.game.clone() else { return };
        if d.active.as_deref() == Some(game.as_str()) {
            return;
        }
        d.active = Some(game.clone());
        d.switched = now();
        let name = d.name_of(&game);
        d.push_event(format!("Showing {name}, the account the game is logged in to"), "info");
        d.save();
        drop(d);
        self.bus.signal(Topic::Account);
    }

    /// An explicit account id line, with the player name known at the time.
    pub fn on_account_line(self: &Arc<Self>, id: &str, name: Option<&str>) {
        let key = {
            let mut d = self.data.write();
            match name {
                Some(n) => Some(d.ensure(n)),
                None => d.game.clone(),
            }
        };
        if let Some(key) = key {
            self.confirm(&key, id, "EE.log");
        }
    }

    /// Never queries DE: a sure line links the logged-in account, anything else is shown to the
    /// user. An id another account already has is left alone.
    pub fn on_candidate(self: &Arc<Self>, id: String, sure: bool, line: String) {
        let link = {
            let mut d = self.data.write();
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

    /// The local player's mastery badge, kept for the logged-in account.
    pub fn on_mastery(&self, lo: u32, hi: Option<u32>) {
        let unchanged = {
            let d = self.data.read();
            let Some(game) = d.game.as_deref() else { return };
            d.get(game).is_none_or(|acc| acc.badge == Some((lo, hi)))
        };
        if unchanged {
            return;
        }
        let mut d = self.data.write();
        let Some(game) = d.game.clone() else { return };
        if let Some(acc) = d.get_mut(&game) {
            acc.badge = Some((lo, hi));
        }
        d.save();
    }

    /// What Duviri offers the logged-in account, rolled anew.
    pub fn on_duviri(&self, offers: DuviriOffers) {
        let mut d = self.data.write();
        let Some(game) = d.game.clone() else { return };
        let Some(acc) = d.get_mut(&game) else { return };
        // A log read back at startup may hold an older roll than the one kept.
        let older = match (&acc.duviri, offers.rolled) {
            (Some(kept), Some(rolled)) => kept.rolled.is_some_and(|t| t > rolled),
            _ => false,
        };
        if older || acc.duviri.as_ref() == Some(&offers) {
            return;
        }
        acc.duviri = Some(offers);
        d.save();
    }

    /// Links `id` to an account. An id belongs to one account: a link made now wins over an
    /// older one elsewhere.
    fn confirm(self: &Arc<Self>, key: &str, id: &str, source: &str) {
        let changed = {
            let mut d = self.data.write();
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

    // ------------------------------------------------------------ profiles

    /// Whether an account's profile should be loaded again: never loaded, older than the refresh
    /// interval, or loaded for another id (the account was linked to a new one since; the fetch
    /// settles it). A copy that lacks something, such as the standing a build before 0.3.0 did
    /// not keep, is refreshed at the ordinary interval like any other: a reason a fetch cannot
    /// cure must never ask DE again at every cooldown.
    fn profile_stale(&self, acc: &Account) -> bool {
        let interval = self.settings.get().profile_refresh_minutes.max(10) as f64 * 60.0;
        acc.profile.as_ref().is_none_or(|p| {
            now() - p.fetched > interval || p.id.as_deref().is_some_and(|pid| Some(pid) != acc.id.as_deref())
        })
    }

    /// The account whose profile is due on its own: at startup (`first`) the one shown, when its
    /// copy is stale; while playing, with automatic refresh switched on, the one logged in.
    /// Offline, none: nothing would be sent anyway.
    fn profile_due(&self, first: bool, game_active: bool) -> Option<String> {
        if net::offline() {
            return None;
        }
        let auto = self.settings.get().profile_auto_refresh && game_active;
        let d = self.data.read();
        let key = if first {
            d.active.clone()
        } else if auto {
            d.game.clone()
        } else {
            None
        };
        key.filter(|k| d.get(k).is_some_and(|a| a.id.is_some() && self.profile_stale(a)))
    }

    /// Asks for due profiles: once shortly after startup (after the log has been read back, so
    /// the account it ends on is the one shown), then once a minute.
    pub async fn run_profile_checks(self: Arc<Self>, game_active: impl Fn() -> bool + Send + 'static) {
        let mut first = true;
        let mut wait = Duration::from_secs(10);
        loop {
            tokio::select! {
                _ = self.stop.cancelled() => return,
                _ = tokio::time::sleep(wait) => {}
            }
            wait = Duration::from_secs(60);
            if let Some(key) = self.profile_due(first, game_active()) {
                self.request_profile(&key);
            }
            first = false;
        }
    }

    /// Loads an account's profile as soon as DE's rate limit allows, if it is still due then. One
    /// worker serves every account, one request per cooldown.
    pub fn request_profile(self: &Arc<Self>, key: &str) {
        if net::offline() {
            return;
        }
        {
            let mut d = self.data.write();
            if !d.profile_queue.iter().any(|k| k == key) {
                d.profile_queue.push_back(key.to_string());
            }
            if d.profile_worker {
                return;
            }
            d.profile_worker = true;
        }
        tokio::spawn(Arc::clone(self).profile_worker());
    }

    async fn profile_worker(self: Arc<Self>) {
        // Clears `profile_worker` if the worker ends on a panic; disarmed on normal exit under
        // the data lock so it never races with a newly spawned worker.
        struct WorkerGuard(Option<Arc<AccountService>>);
        impl Drop for WorkerGuard {
            fn drop(&mut self) {
                if let Some(me) = self.0.take() {
                    me.data.write().profile_worker = false;
                }
            }
        }
        let mut guard = WorkerGuard(Some(Arc::clone(&self)));
        loop {
            let (key, wait, paused) = {
                let mut d = self.data.write();
                let Some(key) = d.profile_queue.front().cloned() else {
                    d.profile_worker = false;
                    guard.0 = None;
                    return;
                };
                (key, d.profile_wait(), d.profile_paused())
            };
            if wait > 0.0 {
                let name = self.data.read().name_of(&key);
                let why = if paused { "DE refused the last request" } else { "pausing for DE's rate limit" };
                self.event(&format!("Loading {name}'s profile in {} min ({why})", (wait / 60.0) as i64 + 1), "info");
                tokio::select! {
                    _ = self.stop.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs_f64(wait + 1.0)) => {}
                }
                continue;
            }
            let due = {
                let mut d = self.data.write();
                // Take out exactly this account: while the worker waited, it may have been
                // forgotten, and then the front of the queue is another account still to do.
                if let Some(at) = d.profile_queue.iter().position(|k| *k == key) {
                    d.profile_queue.remove(at);
                }
                d.get(&key).is_some_and(|a| a.id.is_some() && self.profile_stale(a))
            };
            if due {
                let _ = self.refresh_profile(&key, true).await;
            }
        }
    }

    /// Takes the one profile request DE's rate limit allows per cooldown, and none while the
    /// service is left alone after a refusal. The time is saved at once: a restart must not
    /// forget it.
    fn take_profile_slot(&self) -> Result<()> {
        let mut d = self.data.write();
        let wait = d.profile_wait();
        if wait > 0.0 {
            let why = if d.profile_paused() {
                "DE's profile service refused the last request, which is how it rate-limits. To keep your IP from \
                 being blocked from logging in, no profile is asked for during the next hour."
            } else {
                "To stay clear of DE's rate limit, which can block your IP from logging in, profile requests are \
                 limited to one every 5 minutes."
            };
            let (minutes, seconds) = ((wait / 60.0) as i64, (wait % 60.0) as i64);
            return Err(UserError::new(format!("{why} Try again in {minutes}:{seconds:02}.")).into());
        }
        d.profile_requests.requested = now();
        d.save();
        Ok(())
    }

    /// Loads the profile of the account `key` from DE (within the rate limit).
    pub async fn refresh_profile(self: &Arc<Self>, key: &str, quiet: bool) -> Result<Option<Arc<Profile>>> {
        let id = self.data.read().get(key).and_then(|a| a.id.clone());
        let Some(id) = id else {
            return Err(UserError::new(
                "This account has no id yet. It is read from EE.log after a relic mission in a squad, or enter it in Settings.",
            )
            .into());
        };
        if net::offline() {
            // Nothing would be sent; the rate limit's slot is not used up for it either.
            return Err(UserError::new("The app runs offline: no profile is loaded.").into());
        }
        let me = Arc::clone(self);
        blocking(move || me.take_profile_slot()).await??;
        let fetched = sources::fetch_profile(&id, self.settings.get().platform).await;
        let me = Arc::clone(self);
        let key = key.to_string();
        let prof = match fetched {
            Ok(p) => p,
            Err(e) => {
                let msg = e.to_string();
                let refused = e.downcast_ref::<sources::ProfileRefused>().is_some_and(|r| r.status == 403);
                let record = msg.clone();
                blocking(move || me.profile_failed(&key, record, refused)).await?;
                if !quiet {
                    return Err(e);
                }
                self.event(&msg, "warn");
                return Ok(None);
            }
        };
        let attached = blocking(move || me.attach_profile(&key, Some(&id), prof, "DE public profile")).await?;
        Ok(Some(attached))
    }

    /// Records a failed profile request. A 403 is the service rate-limiting this address: leave
    /// it alone for a while, and forget what else was waiting to ask it.
    fn profile_failed(&self, key: &str, msg: String, refused: bool) {
        let mut d = self.data.write();
        if let Some(acc) = d.get_mut(key) {
            acc.profile_error = Some(msg);
        }
        if refused {
            d.profile_requests.paused_until = now() + PROFILE_PAUSE;
            d.profile_queue.clear();
            d.push_event(
                "DE's profile service refused a request; no profile is asked for during the next hour".into(),
                "warn",
            );
            d.save();
        }
    }

    /// Files a profile under the account it names. Its display name decides: an id linked to the
    /// wrong account moves to the account that owns it, which is added when it is new. With no
    /// account of that name and `key` not logged in right now, `key` was renamed.
    fn attach_profile(&self, key: &str, id: Option<&str>, mut prof: Profile, label: &str) -> Arc<Profile> {
        prof.source = Some(label.to_string());
        let id = prof.id.clone().or_else(|| id.map(str::to_owned));
        prof.id = id.clone();
        let prof = Arc::new(prof);
        let mut d = self.data.write();
        let target = match profile_target(&d, key, &prof, id.as_deref()) {
            Target::Key => key.to_string(),
            Target::Renamed(name) => {
                if let Some(acc) = d.get_mut(key) {
                    acc.name = name;
                }
                key.to_string()
            }
            Target::Other(other) => other,
            Target::New(name) => d.ensure(&name),
        };
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
        drop(d);
        self.bus.signal(Topic::Inventory);
        prof
    }

    /// A profile file the user picked: filed under the account it names, else the one shown.
    pub fn import_profile(&self, data: &Value) -> Result<()> {
        let prof = sources::parse_profile(data)?;
        let key = {
            let mut d = self.data.write();
            let named = prof.name.as_deref().and_then(|n| d.by_name(n)).map(|a| a.key.clone());
            named.unwrap_or_else(|| d.active_or_new())
        };
        self.attach_profile(&key, None, prof, "Imported profile file");
        Ok(())
    }

    // ------------------------------------------------------------ inventories

    pub fn store_inventory(&self, key: &str, inv: Inventory) -> Arc<Inventory> {
        let inv = Arc::new(inv);
        let mut d = self.data.write();
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
        drop(d);
        self.bus.signal(Topic::Inventory);
        inv
    }

    /// An inventory file for the account shown. Refused when its mastery XP says it is another
    /// account's, so a file of the wrong account never mixes the two up.
    pub fn import_inventory(&self, data: &Value, label: &str) -> Result<Arc<Inventory>> {
        let mut inv = sources::parse_inventory(data)?;
        inv.source = Some(label.to_string());
        let key = {
            let mut d = self.data.write();
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

    /// Reads a watched inventory file (Settings: inventory file) and imports it.
    pub fn import_inventory_file(&self, path: &Path) {
        let label = path.file_name().map_or_else(|| path.display().to_string(), |f| f.to_string_lossy().into_owned());
        match crate::config::read_json::<Value>(path) {
            None => self.event(&format!("Could not read {} as JSON", path.display()), "warn"),
            Some(v) => {
                if let Err(e) = self.import_inventory(&v, &label) {
                    self.event(&e.to_string(), "warn");
                }
            }
        }
    }

    pub fn clear_inventory(&self) {
        let mut d = self.data.write();
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
        drop(d);
        self.bus.signal(Topic::Inventory);
    }

    /// Which capture of a memory scan belongs to the account logged in (`game`).
    pub fn choose_capture(&self, game: &str, sections: &memscan::Sections, captures: Vec<Value>) -> CaptureChoice {
        choose_capture(&self.data.read(), game, sections, captures)
    }

    /// The sync time and section count of an account's inventory, to tell a capture of the same
    /// sync from a new one.
    pub fn inventory_sync(&self, key: &str) -> Option<(Option<f64>, usize)> {
        self.data.read().get(key).and_then(|a| a.inventory.as_ref().map(|i| (i.synced_at, i.sections)))
    }

    // ------------------------------------------------------------ what the user sets by hand

    /// Changes the shown account's hand-set data and saves it.
    fn edit_manual(&self, change: impl FnOnce(&mut accounts::Manual)) -> Result<()> {
        let mut d = self.data.write();
        let key = d.active_or_new();
        let acc = d.get_mut(&key).ok_or_else(|| UserError::new("No account to save this for."))?;
        change(&mut acc.manual);
        acc.write("manual.json", &acc.manual)?;
        d.save();
        drop(d);
        // Hand-set counts, pins and tasks all show on the pages and the farming board.
        self.bus.signal(Topic::Inventory);
        self.bus.signal(Topic::Tasks);
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

    // ------------------------------------------------------------ accounts

    /// Links an id typed or picked in Settings to an account (the one shown by default).
    pub fn set_account_id(self: &Arc<Self>, raw: &str, key: Option<&str>) -> Result<()> {
        let id = raw.trim().trim_matches('"').to_lowercase();
        if !sources::valid_account_id(&id) {
            return Err(UserError::new(
                "That isn't an account id: it should be exactly 24 characters of 0-9 and a-f, like 505875fb1a4d80894a000000.",
            )
            .into());
        }
        let key = {
            let mut d = self.data.write();
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
            let mut d = self.data.write();
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
        self.bus.signal(Topic::Account);
        if due {
            self.request_profile(key);
        }
        Ok(())
    }

    /// Forgets an account and deletes its data. If the game logs in to it again, it comes back
    /// empty.
    pub fn forget_account(&self, key: &str) -> Result<()> {
        let mut d = self.data.write();
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
        drop(d);
        self.bus.signal(Topic::Account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_capture_is_preferred_over_a_newer_fragment() {
        let data = Data { accounts: vec![Account::new("alpha".into(), "Alpha".into())], ..Data::default() };
        let whole = json!({"MiscItems": [], "Suits": [], "LastInventorySync": 1});
        let fragment = json!({"MiscItems": [], "LastInventorySync": 2});
        let mut sections = memscan::Sections::default();
        for _ in 0..3 {
            sections.learn(&whole);
        }
        let choice = choose_capture(&data, "alpha", &sections, vec![fragment.clone(), whole.clone()]);
        assert_eq!(choice.chosen, Some(whole));
        let choice = choose_capture(&data, "alpha", &sections, vec![fragment]);
        assert!(choice.chosen.is_none());
        assert_eq!(choice.missing, Some(vec!["Suits".to_string()]));
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
    fn a_profile_renames_an_account_only_when_nothing_says_it_is_another_ones() {
        let items: Vec<(String, i64)> = (0..12).map(|i| (format!("/I{i}"), 100 + i)).collect();
        let mine: Vec<(&str, i64)> = items.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        let theirs: Vec<(&str, i64)> = items.iter().map(|(k, v)| (k.as_str(), *v + 5000)).collect();
        let named = |name: &str, xp: &[(&str, i64)], id: Option<&str>| Profile {
            name: Some(name.into()),
            id: id.map(str::to_owned),
            xp: xp.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            ..Profile::default()
        };
        let mut alice = account("alice", &mine);
        alice.id = Some("a1".into());
        let d = Data { accounts: vec![alice], ..Data::default() };
        // Alice's own XP under a new name: renamed in the game.
        assert_eq!(
            profile_target(&d, "alice", &named("Alicia", &mine, None), Some("a1")),
            Target::Renamed("Alicia".into())
        );
        // Someone else's XP, or another account id: a new account, Alice stays as she is.
        assert_eq!(profile_target(&d, "alice", &named("Bob", &theirs, None), None), Target::New("Bob".into()));
        assert_eq!(profile_target(&d, "alice", &named("Bob", &[], Some("b2")), None), Target::New("Bob".into()));
        // Her own name: hers.
        assert_eq!(profile_target(&d, "alice", &named("ALICE", &mine, None), None), Target::Key);
        // Logged in to the game under her old name: the game is right, the profile is another's.
        let playing = Data { game: Some("alice".into()), ..d };
        assert_eq!(
            profile_target(&playing, "alice", &named("Alicia", &mine, None), None),
            Target::New("Alicia".into())
        );
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

    #[test]
    fn profile_requests_wait_for_the_cooldown_and_the_pause() {
        let mut d = Data::default();
        assert!(d.profile_wait() <= 0.0, "never asked: no wait");
        d.profile_requests.requested = now();
        let wait = d.profile_wait();
        assert!(wait > PROFILE_COOLDOWN - 2.0 && wait <= PROFILE_COOLDOWN);
        assert!(!d.profile_paused());
        d.profile_requests.paused_until = now() + PROFILE_PAUSE;
        assert!(d.profile_paused());
        assert!(d.profile_wait() > PROFILE_COOLDOWN, "a refusal holds requests up for longer than the cooldown");
        d.profile_requests.requested = 0.0;
        assert!(d.profile_wait() > PROFILE_PAUSE - 2.0, "the pause counts on its own");
    }
}
