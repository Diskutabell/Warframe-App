//! Every reward screen the app read, so the overview can show what dropped and what it was worth.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::config;

/// Kept on disk; older entries are dropped when the file is rewritten.
const KEEP: usize = 300;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Reward {
    pub name: String,
    pub uid: String,
    pub price: Option<i64>,
    pub ducats: Option<i64>,
    /// Still needed for an unfinished set when the screen was read.
    pub needed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub t: f64,
    pub drops: Vec<Reward>,
    pub best: Option<String>,
    /// The key of the account that was logged in (`accounts.rs`). Entries from before 0.3.0 have
    /// none and are shown for every account.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
}

impl Entry {
    fn belongs_to(&self, account: Option<&str>) -> bool {
        self.account.is_none() || account.is_none() || self.account.as_deref() == account
    }
}

static LOCK: Mutex<()> = Mutex::new(());

fn file() -> PathBuf {
    config::dirs().data.join("rewards.jsonl")
}

fn read_all() -> Vec<Entry> {
    fs::read_to_string(file()).unwrap_or_default().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// Records a reward screen. Reading the same screen again (F9 pressed twice) replaces the first
/// record instead of counting the screen twice.
pub fn record(entry: Entry) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = read_all();
    let same_screen = all.last().is_some_and(|last| {
        entry.t - last.t < 20.0
            && last.account == entry.account
            && last.drops.iter().map(|d| &d.name).eq(entry.drops.iter().map(|d| &d.name))
    });
    if same_screen {
        all.pop();
    }
    if same_screen || all.len() >= KEEP + 50 {
        all.push(entry);
        let start = all.len().saturating_sub(KEEP);
        let text: String =
            all[start..].iter().filter_map(|e| serde_json::to_string(e).ok()).map(|l| l + "\n").collect();
        let _ = config::write_atomic(&file(), text.as_bytes());
    } else if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(file()) {
        if let Ok(line) = serde_json::to_string(&entry) {
            let _ = writeln!(f, "{line}");
        }
    }
}

/// The newest `limit` entries of an account, newest first.
pub fn recent(limit: usize, account: Option<&str>) -> Vec<Entry> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all: Vec<Entry> = read_all().into_iter().filter(|e| e.belongs_to(account)).collect();
    all.reverse();
    all.truncate(limit);
    all
}

/// An account's totals since `since`: screens read and the platinum of the most valuable pick on
/// each.
pub fn totals(since: f64, account: Option<&str>) -> (usize, i64) {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let session: Vec<Entry> = read_all().into_iter().filter(|e| e.t >= since && e.belongs_to(account)).collect();
    let best_value = session.iter().map(|e| e.drops.iter().filter_map(|d| d.price).max().unwrap_or(0)).sum();
    (session.len(), best_value)
}
