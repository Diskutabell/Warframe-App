//! When the inventory is read from the game's memory, and what is done with what was read.
//!
//! Off unless the user switches it on. Then it is automatic and only automatic: after a login,
//! after a mission (the log's sync hints), when the log has been read back while the game runs,
//! and shortly after it is switched on - never on a timer. Scans never overlap and start at least
//! `MEMSCAN_GAP` apart; a capture that is cut short or another account's is held back and the
//! read retried (`retries`).

use std::sync::Arc;
use std::time::Duration;

use anyhow::anyhow;
use serde::Serialize;
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use super::accounts::AccountService;
use super::settings::SettingsStore;
use super::watched::Watched;
use crate::bus::Bus;
use crate::difflib::py_round;
use crate::memscan::{self, ScanError};
use crate::{now, sources};

/// Minimum time between the starts of memory scans.
const MEMSCAN_GAP: f64 = 10.0;
/// The progress shown moves in steps of this much.
const PROGRESS_STEP: f64 = 0.02;

#[derive(Clone, Debug, Default, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct MemscanStatus {
    /// "idle" (nothing read yet), "scanning", "ok" or "error"; the summary adds "off" and
    /// "unavailable".
    pub state: &'static str,
    /// Why the last attempt read nothing, or what went wrong.
    pub detail: Option<String>,
    pub progress: f64,
    /// When the next read is due (epoch seconds), if one is scheduled.
    pub next: Option<f64>,
    /// The last successful read.
    pub last: Option<LastScan>,
    #[serde(skip)]
    last_attempt: f64,
    /// The game process that refused access; it is not asked again, a restarted game is.
    #[serde(skip)]
    denied_pid: Option<u32>,
}

/// A memory scan that read an inventory.
#[derive(Clone, Debug, Serialize, ts_rs::TS)]
#[ts(export)]
pub struct LastScan {
    pub t: f64,
    pub stacks: usize,
    pub seconds: f64,
    /// The account it was filed under.
    pub account: String,
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

pub struct MemscanService {
    /// The app's background tasks: shutdown waits for a capture being applied.
    tasks: TaskTracker,
    status: Watched<MemscanStatus>,
    settings: Arc<SettingsStore>,
    accounts: Arc<AccountService>,
    stop: CancellationToken,
}

/// Runs blocking work (reading another process, parsing a multi-megabyte capture, the account
/// files) on the blocking pool.
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> anyhow::Result<T> {
    tokio::task::spawn_blocking(work).await.map_err(|e| anyhow!("{e}"))
}

impl MemscanService {
    pub fn new(
        settings: Arc<SettingsStore>,
        accounts: Arc<AccountService>,
        bus: &Bus,
        stop: CancellationToken,
        tasks: TaskTracker,
    ) -> MemscanService {
        MemscanService {
            tasks,
            status: Watched::new(
                MemscanStatus { state: "idle", last_attempt: f64::NEG_INFINITY, ..Default::default() },
                bus,
            ),
            settings,
            accounts,
            stop,
        }
    }

    /// The status as the summary shows it.
    pub fn status(&self) -> MemscanStatus {
        let mut st = self.status.read().clone();
        if !self.settings.get().memory_scan_enabled {
            st.state = "off";
        } else if let Err(why) = memscan::availability() {
            st.state = "unavailable";
            st.detail = Some(why);
        }
        st
    }

    /// Whether the game is running, by its process; only asked with the scan switched on.
    fn game_pid() -> Option<u32> {
        memscan::find_game_pid()
    }

    /// Reads the inventory after `delay`, if the scan is switched on and allowed. The game only
    /// holds the whole inventory text for a while after a sync, so this runs on events (login,
    /// mission end), never on a timer; `retries` more attempts follow when it is not there yet.
    pub fn schedule(self: &Arc<Self>, delay: Duration, retries: u8) {
        if !self.settings.get().memory_scan_enabled || memscan::availability().is_err() {
            return;
        }
        let me = Arc::clone(self);
        self.tasks.spawn(async move {
            // Looking at the process list reads /proc: off the async threads.
            let denied = me.status.read().denied_pid;
            let pid = blocking(Self::game_pid).await.ok().flatten();
            if denied.is_some() && pid == denied {
                return;
            }
            let due = now() + delay.as_secs_f64();
            {
                let mut st = me.status.write();
                if st.next.is_some_and(|p| p <= due) {
                    return; // an earlier scan is already on its way
                }
                st.next = Some(due);
            }
            tokio::select! {
                _ = me.stop.cancelled() => return,
                _ = tokio::time::sleep(delay) => {}
            }
            let pid = loop {
                let enabled = me.settings.get().memory_scan_enabled;
                let pid = blocking(Self::game_pid).await.ok().flatten();
                let started = me.status.write().start_scheduled(due, enabled, pid, now());
                match started {
                    ScheduledScan::Superseded | ScheduledScan::Cancelled => return,
                    ScheduledScan::Start => break pid,
                    ScheduledScan::Wait => {}
                }
                tokio::select! {
                    _ = me.stop.cancelled() => return,
                    _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                }
            };
            me.scan_now(pid, retries).await;
        });
    }

    async fn scan_now(self: &Arc<Self>, pid: Option<u32>, retries: u8) {
        let me = Arc::clone(self);
        // A panic in the read (a bug) is caught here: the status would otherwise stay "scanning"
        // for good, and every scan scheduled after it would wait for that one forever.
        let read = blocking(move || match pid {
            Some(pid) => memscan::scan_inventory(pid, |p| {
                // Every step would push a summary to the UI: only whole steps of 2% are worth one.
                let shown = me.status.read().progress;
                if p >= 1.0 || p - shown >= PROGRESS_STEP {
                    me.status.write().progress = py_round(p, 3);
                }
            }),
            None => Err(ScanError::NotRunning),
        })
        .await;
        let result = match read {
            Ok(result) => result,
            Err(_) => Err(ScanError::Io("the read hit a bug (see the app's log); nothing was changed".into())),
        };
        let retry = match result {
            Ok((captures, info)) => {
                let me = Arc::clone(self);
                blocking(move || me.apply_capture(captures, info.seconds)).await.unwrap_or_else(|_| {
                    let mut st = self.status.write();
                    st.state = "error";
                    st.detail = Some("filing the inventory hit a bug (see the app's log)".into());
                    false
                })
            }
            Err(e) => {
                let mut st = self.status.write();
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
            self.schedule(Duration::from_secs(if retries > 1 { 10 } else { 30 }), retries - 1);
        }
    }

    /// Ends a read that applied nothing: the last good inventory still stands. True asks for a
    /// retry.
    fn capture_held_back(&self, detail: String) -> bool {
        let mut st = self.status.write();
        st.state = if st.last.is_some() { "ok" } else { "idle" };
        st.detail = Some(detail);
        true
    }

    /// Files the logged-in account's capture, unless it is cut short or already applied. The
    /// memory can hold another account's inventory too (logged out and in without restarting the
    /// game): mastery XP tells them apart. True asks for a retry.
    fn apply_capture(&self, captures: Vec<Value>, seconds: f64) -> bool {
        let Some((game, game_name)) = self.accounts.game_account() else {
            return self.capture_held_back(
                "Waiting for the game's \"Logged in\" line, which says whose inventory this is.".into(),
            );
        };
        let mut sections = self.accounts.memscan_sections(&game);
        let choice = self.accounts.choose_capture(&game, &sections, captures);
        let obj = match (choice.chosen, choice.missing) {
            (Some(obj), _) => obj,
            (None, Some(missing)) => {
                return self.capture_held_back(format!(
                    "The game held only part of {game_name}'s inventory just now ({} missing), so the last complete one is kept.",
                    missing.join(", ")
                ));
            }
            (None, None) => {
                let whose = choice.held_by.map_or_else(|| "another account's".to_string(), |n| format!("{n}'s"));
                return self.capture_held_back(format!(
                    "The game still holds {whose} inventory; {game_name}'s is read after the next mission."
                ));
            }
        };
        // The same sync, read again with nothing new in it: leave the inventory and the log alone.
        let keys = obj.as_object().map_or(0, |o| o.len());
        let sync = obj["LastInventorySync"].as_f64();
        if sync.is_some() && self.accounts.inventory_sync(&game).is_some_and(|(at, n)| at == sync && keys <= n) {
            let mut st = self.status.write();
            st.state = "ok";
            st.detail = None;
            return false;
        }
        let stored = sources::parse_inventory(&obj).map(|mut inv| {
            inv.source = Some("memory scan".into());
            self.accounts.store_inventory(&game, inv)
        });
        match stored {
            Ok(inv) => {
                sections.learn(&obj);
                self.accounts.save_memscan_sections(&game, &sections);
                let mut st = self.status.write();
                st.state = "ok";
                st.detail = None;
                st.last = Some(LastScan { t: now(), stacks: inv.counts.len(), seconds, account: game_name.clone() });
            }
            Err(e) => {
                let mut st = self.status.write();
                st.state = "error";
                st.detail = Some(e.to_string());
            }
        }
        false
    }

    /// The scan was switched on or off. On: read right away, even after a refusal (the system
    /// may allow it now).
    pub fn switched(self: &Arc<Self>, on: bool) {
        if !on {
            return;
        }
        {
            let mut st = self.status.write();
            st.denied_pid = None;
            st.last_attempt = f64::NEG_INFINITY;
        }
        let me = Arc::clone(self);
        self.tasks.spawn(async move {
            if blocking(Self::game_pid).await.ok().flatten().is_some() {
                me.schedule(Duration::from_secs(3), 1);
            }
        });
    }

    /// At startup: the game may already be running and synced.
    pub fn at_startup(self: &Arc<Self>) {
        if !self.settings.get().memory_scan_enabled {
            return;
        }
        let me = Arc::clone(self);
        self.tasks.spawn(async move {
            if blocking(Self::game_pid).await.ok().flatten().is_some() {
                me.schedule(Duration::from_secs(20), 1);
            }
        });
    }

    /// The log has been read back: if the game runs, its inventory may be in memory.
    pub fn after_replay(self: &Arc<Self>) {
        if !self.settings.get().memory_scan_enabled {
            return;
        }
        let me = Arc::clone(self);
        self.tasks.spawn(async move {
            if blocking(Self::game_pid).await.ok().flatten().is_some() {
                me.schedule(Duration::from_secs(3), 2);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
