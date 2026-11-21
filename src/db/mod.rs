//! The app's own data in one SQLite file, `<data>/warframeproject.db`: the accounts, their
//! profiles, inventories and hand-set data, what the memory scan expects of a capture, every
//! reward screen read, and warframe.market's prices.
//!
//! Up to 0.9 this lived in a dozen JSON files (`accounts.json`, `accounts/<key>/*.json`,
//! `rewards.jsonl` capped at 300 lines, `prices*.json`). The first start of a version with the
//! database imports them once (`import.rs`) and leaves them where they are.
//!
//! Writes are rare and small, so one connection serves the whole app behind a mutex; every call
//! is blocking and is made from blocking code (the blocking pool, or startup). WAL with full sync:
//! a write is on disk when the call returns - the profile service's rate limit is kept here, and
//! a crash must not lose when DE was last asked.

mod import;

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use crate::accounts::{Account, Manual, ProfileTimes, Registry, Task};
use crate::config::Platform;
use crate::history::{Entry, Reward};
use crate::market::Price;
use crate::memscan::Sections;
use crate::services::lock;
use crate::sources::{Delta, Inventory, Profile};

/// The schema, one step per version: `user_version` says how many have been applied.
const MIGRATIONS: &[&str] = &[
    // 1: everything 0.9 kept in JSON files.
    r#"
    CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    CREATE TABLE accounts (
        key TEXT PRIMARY KEY,
        position INTEGER NOT NULL,
        name TEXT NOT NULL,
        id TEXT,
        source TEXT,
        badge_lo INTEGER,
        badge_hi INTEGER,
        seen REAL,
        duviri TEXT
    );
    CREATE TABLE profiles (account TEXT PRIMARY KEY, fetched REAL NOT NULL, data TEXT NOT NULL);
    CREATE TABLE inventories (
        account TEXT PRIMARY KEY,
        synced_at REAL,
        imported REAL NOT NULL,
        source TEXT,
        data TEXT NOT NULL
    );
    CREATE TABLE manual_counts (
        account TEXT NOT NULL, uid TEXT NOT NULL, count INTEGER NOT NULL, set_at REAL NOT NULL,
        PRIMARY KEY (account, uid)
    );
    CREATE TABLE manual_mastered (account TEXT NOT NULL, uid TEXT NOT NULL, at REAL NOT NULL, PRIMARY KEY (account, uid));
    CREATE TABLE pins (account TEXT NOT NULL, position INTEGER NOT NULL, key TEXT NOT NULL, PRIMARY KEY (account, key));
    CREATE TABLE tasks (
        account TEXT NOT NULL, position INTEGER NOT NULL, uid TEXT NOT NULL, target INTEGER, added REAL NOT NULL,
        PRIMARY KEY (account, uid)
    );
    CREATE TABLE memscan_sections (account TEXT PRIMARY KEY, data TEXT NOT NULL);
    CREATE TABLE reward_history (
        id INTEGER PRIMARY KEY,
        t REAL NOT NULL,
        account TEXT,
        best TEXT,
        drops TEXT NOT NULL
    );
    CREATE INDEX reward_history_by_time ON reward_history (t);
    CREATE INDEX reward_history_by_account ON reward_history (account, t);
    CREATE TABLE prices (
        platform TEXT NOT NULL, slug TEXT NOT NULL,
        sell INTEGER, sell_avg REAL, buy INTEGER, t REAL NOT NULL, err INTEGER,
        PRIMARY KEY (platform, slug)
    );
    CREATE TABLE price_history (
        platform TEXT NOT NULL, slug TEXT NOT NULL, t REAL NOT NULL, sell INTEGER, buy INTEGER,
        PRIMARY KEY (platform, slug, t)
    );
    "#,
    // 2: changes to an inventory made after it was read (a reward picked, a build marked done),
    // kept until a read includes them; and which reward of a screen the player picked.
    r#"
    CREATE TABLE inventory_deltas (
        id INTEGER PRIMARY KEY,
        account TEXT NOT NULL,
        t REAL NOT NULL,
        kind TEXT NOT NULL,
        uid TEXT NOT NULL,
        change INTEGER NOT NULL,
        equipment INTEGER NOT NULL DEFAULT 0,
        reward INTEGER
    );
    CREATE INDEX inventory_deltas_by_account ON inventory_deltas (account, t);
    ALTER TABLE reward_history ADD COLUMN picked TEXT;
    "#,
];

/// A price is kept in the history at most this often per item, and this long.
const PRICE_HISTORY_EVERY: f64 = 6.0 * 3600.0;
const PRICE_HISTORY_KEEP: f64 = 180.0 * 86400.0;
/// Reading the same reward screen again this soon (the hotkey pressed twice) replaces the first
/// record instead of counting the screen twice.
const SAME_SCREEN: f64 = 20.0;

pub struct Db {
    conn: Mutex<Connection>,
}

/// A point of an item's price history: when, the lowest sell order and the highest buy order.
#[derive(Clone, Debug, PartialEq, serde::Serialize, ts_rs::TS)]
#[ts(export)]
pub struct PricePoint {
    pub t: f64,
    pub sell: Option<i64>,
    pub buy: Option<i64>,
}

/// Where the JSON files of older versions are, for the import.
pub struct Legacy<'a> {
    pub data: &'a Path,
    pub cache: &'a Path,
}

impl Db {
    /// Opens the database at `path`, creating it when it is new and bringing its schema up to
    /// date. A new database imports `legacy`'s files first. What the import has to say comes back
    /// for the activity list.
    pub fn open(path: &Path, legacy: Legacy) -> Result<(Db, Vec<String>)> {
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let db = Db { conn: Mutex::new(conn) };
        let notes = db.migrate(Some(legacy))?;
        Ok((db, notes))
    }

    /// An empty database in memory, for tests.
    #[cfg(test)]
    pub fn memory() -> Db {
        let db = Db { conn: Mutex::new(Connection::open_in_memory().expect("an in-memory database")) };
        db.migrate(None).expect("the schema applies");
        db
    }

    /// Applies the migrations not applied yet, all in one transaction: a failure leaves the
    /// database as it was (and a failed import is tried again at the next start).
    fn migrate(&self, legacy: Option<Legacy>) -> Result<Vec<String>> {
        let mut conn = lock(&self.conn);
        let version: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        let version = usize::try_from(version).unwrap_or(0);
        let mut notes = Vec::new();
        if version >= MIGRATIONS.len() {
            return Ok(notes);
        }
        let tx = conn.transaction()?;
        for sql in &MIGRATIONS[version..] {
            tx.execute_batch(sql)?;
        }
        if version == 0 {
            if let Some(legacy) = legacy {
                import::legacy(&tx, &legacy, &mut notes).context("importing the data of the older version")?;
            }
        }
        tx.pragma_update(None, "user_version", MIGRATIONS.len() as i64)?;
        tx.commit()?;
        Ok(notes)
    }

    /// Folds the write-ahead log back into the database file, at shutdown.
    pub fn checkpoint(&self) -> Result<()> {
        self.with(|c| c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);"))
    }

    fn with<T>(&self, work: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        Ok(work(&lock(&self.conn))?)
    }

    fn transaction<T>(&self, work: impl FnOnce(&Transaction) -> rusqlite::Result<T>) -> Result<T> {
        let mut conn = lock(&self.conn);
        let tx = conn.transaction()?;
        let value = work(&tx)?;
        tx.commit()?;
        Ok(value)
    }

    // ------------------------------------------------------------ accounts

    /// The accounts with everything kept about them.
    pub fn load_accounts(&self) -> Result<Registry> {
        self.with(|c| {
            let meta = |key: &str| -> rusqlite::Result<Option<String>> {
                c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()
            };
            let number =
                |key: &str| -> rusqlite::Result<f64> { Ok(meta(key)?.and_then(|v| v.parse().ok()).unwrap_or(0.0)) };
            let mut accounts = Vec::new();
            let mut stmt = c.prepare(
                "SELECT key, name, id, source, badge_lo, badge_hi, seen, duviri FROM accounts ORDER BY position, key",
            )?;
            let rows = stmt.query_map([], |r| {
                let mut acc = Account::new(r.get(0)?, r.get(1)?);
                acc.id = r.get(2)?;
                acc.source = r.get(3)?;
                let (lo, hi): (Option<u32>, Option<u32>) = (r.get(4)?, r.get(5)?);
                acc.badge = lo.map(|lo| (lo, hi));
                acc.seen = r.get(6)?;
                acc.duviri = r.get::<_, Option<String>>(7)?.and_then(|d| serde_json::from_str(&d).ok());
                Ok(acc)
            })?;
            for acc in rows {
                accounts.push(acc?);
            }
            for acc in &mut accounts {
                acc.profile =
                    json_row::<Profile>(c, "SELECT data FROM profiles WHERE account = ?1", &acc.key)?.map(Arc::new);
                acc.inventory = json_row::<Inventory>(c, "SELECT data FROM inventories WHERE account = ?1", &acc.key)?
                    .map(Arc::new);
                acc.manual = load_manual(c, &acc.key)?;
                acc.deltas = load_deltas(c, &acc.key)?;
            }
            Ok(Registry {
                active: meta("active")?,
                switched: number("switched")?,
                profile_requests: ProfileTimes {
                    requested: number("profile_requested")?,
                    paused_until: number("profile_paused_until")?,
                    unanswered: meta("profile_unanswered")?.is_some(),
                },
                accounts,
            })
        })
    }

    /// The account list: which account is shown, the profile service's rate limit, and each
    /// account's own fields (not its profile, inventory or hand-set data, which are saved on
    /// their own). An account missing from `accounts` is not deleted: only `delete_account`
    /// deletes.
    pub fn save_registry(
        &self,
        active: &Option<String>,
        switched: f64,
        times: ProfileTimes,
        accounts: &[Account],
    ) -> Result<()> {
        self.transaction(|tx| {
            set_meta(tx, "active", active.as_deref())?;
            set_meta(tx, "switched", Some(&switched.to_string()))?;
            set_meta(tx, "profile_requested", Some(&times.requested.to_string()))?;
            set_meta(tx, "profile_paused_until", Some(&times.paused_until.to_string()))?;
            set_meta(tx, "profile_unanswered", times.unanswered.then_some("1"))?;
            for (position, acc) in accounts.iter().enumerate() {
                save_account_row(tx, position, acc)?;
            }
            Ok(())
        })
    }

    pub fn save_profile(&self, key: &str, profile: &Profile) -> Result<()> {
        let data = serde_json::to_string(profile)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO profiles (account, fetched, data) VALUES (?1, ?2, ?3)
                 ON CONFLICT (account) DO UPDATE SET fetched = excluded.fetched, data = excluded.data",
                params![key, profile.fetched, data],
            )
            .map(|_| ())
        })
    }

    pub fn save_inventory(&self, key: &str, inv: &Inventory) -> Result<()> {
        let data = serde_json::to_string(inv)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO inventories (account, synced_at, imported, source, data) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (account) DO UPDATE SET synced_at = excluded.synced_at, imported = excluded.imported,
                     source = excluded.source, data = excluded.data",
                params![key, inv.synced_at, inv.imported, inv.source, data],
            )
            .map(|_| ())
        })
    }

    /// Forgets an account's inventory and what the memory scan learnt about its captures.
    pub fn delete_inventory(&self, key: &str) -> Result<()> {
        self.transaction(|tx| {
            tx.execute("DELETE FROM inventories WHERE account = ?1", [key])?;
            tx.execute("DELETE FROM memscan_sections WHERE account = ?1", [key])?;
            Ok(())
        })
    }

    /// Replaces an account's hand-set data: counts, mastered marks, pins and the farming board.
    pub fn save_manual(&self, key: &str, manual: &Manual) -> Result<()> {
        self.transaction(|tx| save_manual(tx, key, manual))
    }

    /// Deletes an account and everything kept about it. Its reward history stays (it is the
    /// record of what dropped), no longer shown under any account's name.
    pub fn delete_account(&self, key: &str) -> Result<()> {
        self.transaction(|tx| {
            for table in [
                "profiles",
                "inventories",
                "manual_counts",
                "manual_mastered",
                "pins",
                "tasks",
                "memscan_sections",
                "inventory_deltas",
            ] {
                tx.execute(&format!("DELETE FROM {table} WHERE account = ?1"), [key])?;
            }
            tx.execute("DELETE FROM accounts WHERE key = ?1", [key])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------ the memory scan

    pub fn memscan_sections(&self, key: &str) -> Result<Sections> {
        self.with(|c| Ok(json_row(c, "SELECT data FROM memscan_sections WHERE account = ?1", key)?.unwrap_or_default()))
    }

    pub fn save_memscan_sections(&self, key: &str, sections: &Sections) -> Result<()> {
        let data = serde_json::to_string(sections)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO memscan_sections (account, data) VALUES (?1, ?2)
                 ON CONFLICT (account) DO UPDATE SET data = excluded.data",
                params![key, data],
            )
            .map(|_| ())
        })
    }

    // ------------------------------------------------------------ reward history

    /// Records a reward screen and says which row it is. Reading the same screen again (F9
    /// pressed twice) replaces the first record instead of counting the screen twice.
    pub fn record_reward(&self, entry: &Entry) -> Result<i64> {
        let drops = serde_json::to_string(&entry.drops)?;
        self.transaction(|tx| {
            let last: Option<(i64, f64, Option<String>, String)> = tx
                .query_row(
                    "SELECT id, t, account, drops FROM reward_history ORDER BY t DESC, id DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let same_screen = last.as_ref().is_some_and(|(_, t, account, drops)| {
                let names = |d: &[Reward]| d.iter().map(|r| r.name.clone()).collect::<Vec<_>>();
                let before: Vec<Reward> = serde_json::from_str(drops).unwrap_or_default();
                entry.t - t < SAME_SCREEN && *account == entry.account && names(&before) == names(&entry.drops)
            });
            match last.filter(|_| same_screen) {
                Some((id, ..)) => {
                    tx.execute(
                        "UPDATE reward_history SET t = ?2, best = ?3, drops = ?4 WHERE id = ?1",
                        params![id, entry.t, entry.best, drops],
                    )?;
                    Ok(id)
                }
                None => {
                    tx.execute(
                        "INSERT INTO reward_history (t, account, best, drops) VALUES (?1, ?2, ?3, ?4)",
                        params![entry.t, entry.account, entry.best, drops],
                    )?;
                    Ok(tx.last_insert_rowid())
                }
            }
        })
    }

    /// One reward screen by its row.
    pub fn reward(&self, id: i64) -> Result<Option<Entry>> {
        self.with(|c| {
            c.query_row("SELECT t, account, best, drops, id, picked FROM reward_history WHERE id = ?1", [id], entry_row)
                .optional()
        })
    }

    /// Records which reward of a screen the player picked (None: undone), with the changes that
    /// makes to the account's inventory, replacing those of an earlier pick on the same screen.
    pub fn set_pick(&self, id: i64, account: &str, picked: Option<&str>, deltas: &[Delta]) -> Result<()> {
        self.transaction(|tx| {
            tx.execute("DELETE FROM inventory_deltas WHERE reward = ?1", [id])?;
            tx.execute("UPDATE reward_history SET picked = ?2 WHERE id = ?1", params![id, picked])?;
            for d in deltas {
                insert_delta(tx, account, d)?;
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------ inventory changes

    /// Changes made to an account's inventory that no read has included yet, oldest first.
    pub fn deltas(&self, account: &str) -> Result<Vec<Delta>> {
        self.with(|c| load_deltas(c, account))
    }

    pub fn add_deltas(&self, account: &str, deltas: &[Delta]) -> Result<()> {
        self.transaction(|tx| {
            for d in deltas {
                insert_delta(tx, account, d)?;
            }
            Ok(())
        })
    }

    /// Drops the changes an inventory read at `as_of` includes: everything made before it.
    pub fn settle_deltas(&self, account: &str, as_of: f64) -> Result<()> {
        self.with(|c| {
            c.execute("DELETE FROM inventory_deltas WHERE account = ?1 AND t <= ?2", params![account, as_of])
                .map(|_| ())
        })
    }

    /// The newest `limit` reward screens of an account, newest first. Screens recorded before
    /// accounts existed (no account) show for every account, as do all with no account given.
    pub fn recent_rewards(&self, limit: usize, account: Option<&str>) -> Result<Vec<Entry>> {
        self.with(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT t, account, best, drops, id, picked FROM reward_history
                 WHERE ?1 IS NULL OR account IS NULL OR account = ?1
                 ORDER BY t DESC, id DESC LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![account, limit as i64], entry_row)?;
            rows.collect()
        })
    }

    /// An account's reward screens since `since`, and the platinum of the most valuable pick on
    /// each, summed.
    pub fn reward_totals(&self, since: f64, account: Option<&str>) -> Result<(usize, i64)> {
        self.with(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT t, account, best, drops, id, picked FROM reward_history
                 WHERE t >= ?1 AND (?2 IS NULL OR account IS NULL OR account = ?2)",
            )?;
            let entries: Vec<Entry> = stmt.query_map(params![since, account], entry_row)?.collect::<Result<_, _>>()?;
            let best: i64 = entries.iter().map(|e| e.drops.iter().filter_map(|d| d.price).max().unwrap_or(0)).sum();
            Ok((entries.len(), best))
        })
    }

    // ------------------------------------------------------------ prices

    /// warframe.market's prices kept for a platform.
    pub fn load_prices(&self, platform: Platform) -> Result<HashMap<String, Price>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT slug, sell, sell_avg, buy, t, err FROM prices WHERE platform = ?1")?;
            let rows = stmt.query_map([platform.as_str()], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    Price { sell: r.get(1)?, sell_avg: r.get(2)?, buy: r.get(3)?, t: r.get(4)?, err: r.get(5)? },
                ))
            })?;
            rows.collect()
        })
    }

    /// Keeps prices fetched for a platform, and adds them to the price history (one point per
    /// item every few hours, for half a year).
    pub fn save_prices(&self, platform: Platform, prices: &[(String, Price)]) -> Result<()> {
        if prices.is_empty() {
            return Ok(());
        }
        self.transaction(|tx| {
            let p = platform.as_str();
            for (slug, price) in prices {
                tx.execute(
                    "INSERT INTO prices (platform, slug, sell, sell_avg, buy, t, err) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT (platform, slug) DO UPDATE SET sell = excluded.sell, sell_avg = excluded.sell_avg,
                         buy = excluded.buy, t = excluded.t, err = excluded.err",
                    params![p, slug, price.sell, price.sell_avg, price.buy, price.t, price.err],
                )?;
                if price.err.is_some() || (price.sell.is_none() && price.buy.is_none()) {
                    continue;
                }
                let last: Option<f64> = tx
                    .query_row(
                        "SELECT MAX(t) FROM price_history WHERE platform = ?1 AND slug = ?2",
                        params![p, slug],
                        |r| r.get(0),
                    )
                    .optional()?
                    .flatten();
                if last.is_none_or(|last| price.t - last >= PRICE_HISTORY_EVERY) {
                    tx.execute(
                        "INSERT OR REPLACE INTO price_history (platform, slug, t, sell, buy) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![p, slug, price.t, price.sell, price.buy],
                    )?;
                }
            }
            let oldest = prices.iter().map(|(_, p)| p.t).fold(f64::NEG_INFINITY, f64::max) - PRICE_HISTORY_KEEP;
            tx.execute("DELETE FROM price_history WHERE t < ?1", [oldest])?;
            Ok(())
        })
    }

    /// An item's price history on a platform, oldest first.
    pub fn price_history(&self, platform: Platform, slug: &str) -> Result<Vec<PricePoint>> {
        self.with(|c| {
            let mut stmt = c.prepare_cached(
                "SELECT t, sell, buy FROM price_history WHERE platform = ?1 AND slug = ?2 ORDER BY t",
            )?;
            let rows = stmt.query_map(params![platform.as_str(), slug], |r| {
                Ok(PricePoint { t: r.get(0)?, sell: r.get(1)?, buy: r.get(2)? })
            })?;
            rows.collect()
        })
    }
}

// ------------------------------------------------------------ rows

fn set_meta(tx: &Transaction, key: &str, value: Option<&str>) -> rusqlite::Result<()> {
    match value {
        Some(v) => tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, v],
        )?,
        None => tx.execute("DELETE FROM meta WHERE key = ?1", [key])?,
    };
    Ok(())
}

fn save_account_row(tx: &Transaction, position: usize, acc: &Account) -> rusqlite::Result<()> {
    let duviri = acc.duviri.as_ref().and_then(|d| serde_json::to_string(d).ok());
    tx.execute(
        "INSERT INTO accounts (key, position, name, id, source, badge_lo, badge_hi, seen, duviri)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT (key) DO UPDATE SET position = excluded.position, name = excluded.name, id = excluded.id,
             source = excluded.source, badge_lo = excluded.badge_lo, badge_hi = excluded.badge_hi,
             seen = excluded.seen, duviri = excluded.duviri",
        params![
            acc.key,
            position as i64,
            acc.name,
            acc.id,
            acc.source,
            acc.badge.map(|b| b.0),
            acc.badge.and_then(|b| b.1),
            acc.seen,
            duviri
        ],
    )?;
    Ok(())
}

fn save_manual(tx: &Transaction, key: &str, manual: &Manual) -> rusqlite::Result<()> {
    for table in ["manual_counts", "manual_mastered", "pins", "tasks"] {
        tx.execute(&format!("DELETE FROM {table} WHERE account = ?1"), [key])?;
    }
    for (uid, (count, set_at)) in &manual.counts {
        tx.execute(
            "INSERT INTO manual_counts (account, uid, count, set_at) VALUES (?1, ?2, ?3, ?4)",
            params![key, uid, count, set_at],
        )?;
    }
    for (uid, at) in &manual.mastered {
        tx.execute("INSERT INTO manual_mastered (account, uid, at) VALUES (?1, ?2, ?3)", params![key, uid, at])?;
    }
    for (position, pin) in manual.pinned.iter().enumerate() {
        tx.execute(
            "INSERT OR IGNORE INTO pins (account, position, key) VALUES (?1, ?2, ?3)",
            params![key, position as i64, pin],
        )?;
    }
    for (position, task) in manual.tasks.iter().enumerate() {
        tx.execute(
            "INSERT OR IGNORE INTO tasks (account, position, uid, target, added) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![key, position as i64, task.uid, task.target, task.added],
        )?;
    }
    Ok(())
}

fn load_manual(c: &Connection, key: &str) -> rusqlite::Result<Manual> {
    let mut manual = Manual::default();
    let mut stmt =
        c.prepare_cached("SELECT uid, count, set_at FROM manual_counts WHERE account = ?1 ORDER BY set_at")?;
    for row in stmt.query_map([key], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))? {
        let (uid, v) = row?;
        manual.counts.insert(uid, v);
    }
    let mut stmt = c.prepare_cached("SELECT uid, at FROM manual_mastered WHERE account = ?1 ORDER BY at")?;
    for row in stmt.query_map([key], |r| Ok((r.get::<_, String>(0)?, r.get(1)?)))? {
        let (uid, at) = row?;
        manual.mastered.insert(uid, at);
    }
    let mut stmt = c.prepare_cached("SELECT key FROM pins WHERE account = ?1 ORDER BY position")?;
    manual.pinned = stmt.query_map([key], |r| r.get(0))?.collect::<Result<_, _>>()?;
    let mut stmt = c.prepare_cached("SELECT uid, target, added FROM tasks WHERE account = ?1 ORDER BY position")?;
    manual.tasks = stmt
        .query_map([key], |r| Ok(Task { uid: r.get(0)?, target: r.get(1)?, added: r.get(2)? }))?
        .collect::<Result<_, _>>()?;
    Ok(manual)
}

/// A JSON column of the one row `sql` selects for `key`; a row that does not parse (written by a
/// newer version, say) counts as none.
fn json_row<T: serde::de::DeserializeOwned>(c: &Connection, sql: &str, key: &str) -> rusqlite::Result<Option<T>> {
    let text: Option<String> = c.query_row(sql, [key], |r| r.get(0)).optional()?;
    Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
}

/// A reward screen from `SELECT t, account, best, drops, id, picked`.
fn entry_row(r: &rusqlite::Row) -> rusqlite::Result<Entry> {
    let drops: String = r.get(3)?;
    Ok(Entry {
        t: r.get(0)?,
        account: r.get(1)?,
        best: r.get(2)?,
        drops: serde_json::from_str(&drops).unwrap_or_default(),
        id: r.get(4)?,
        picked: r.get(5)?,
    })
}

fn load_deltas(c: &Connection, account: &str) -> rusqlite::Result<Vec<Delta>> {
    let mut stmt = c.prepare_cached(
        "SELECT t, kind, uid, change, equipment, reward FROM inventory_deltas WHERE account = ?1 ORDER BY t, id",
    )?;
    let rows = stmt.query_map([account], |r| {
        Ok(Delta {
            t: r.get(0)?,
            kind: r.get(1)?,
            uid: r.get(2)?,
            change: r.get(3)?,
            equipment: r.get::<_, i64>(4)? != 0,
            reward: r.get(5)?,
        })
    })?;
    rows.collect()
}

fn insert_delta(tx: &Transaction, account: &str, d: &Delta) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO inventory_deltas (account, t, kind, uid, change, equipment, reward) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![account, d.t, d.kind, d.uid, d.change, d.equipment as i64, d.reward],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reward(name: &str, price: Option<i64>) -> Reward {
        Reward { name: name.into(), uid: format!("/{name}"), price, ducats: None, needed: false }
    }

    #[test]
    fn accounts_and_everything_about_them_come_back() {
        let db = Db::memory();
        let mut acc = Account::new("tenno".into(), "Tennoplayer".into());
        acc.id = Some("5f1e2d3c4b5a69788796a5b4".into());
        acc.badge = Some((16, Some(18)));
        acc.seen = Some(5.0);
        let other = Account::new("second".into(), "Second".into());
        let times = ProfileTimes { requested: 100.0, paused_until: 3700.0, unanswered: true };
        db.save_registry(&Some("tenno".into()), 9.0, times, &[acc.clone(), other]).unwrap();
        let profile = Profile { name: Some("Tennoplayer".into()), fetched: 42.0, ..Profile::default() };
        db.save_profile("tenno", &profile).unwrap();
        let mut inv = Inventory { imported: 7.0, ..Inventory::default() };
        inv.counts.insert("/P".into(), 3);
        db.save_inventory("tenno", &inv).unwrap();
        let mut manual = Manual::default();
        manual.counts.insert("/Part".into(), (2, 11.0));
        manual.mastered.insert("/Item".into(), 12.0);
        manual.pinned = vec!["Lith A1".into(), "/Item".into()];
        manual.tasks = vec![Task { uid: "/Volt".into(), target: Some(3), added: 13.0 }];
        db.save_manual("tenno", &manual).unwrap();

        let back = db.load_accounts().unwrap();
        assert_eq!(back.active.as_deref(), Some("tenno"));
        assert_eq!(back.switched, 9.0);
        assert_eq!((back.profile_requests.requested, back.profile_requests.paused_until), (100.0, 3700.0));
        assert!(back.profile_requests.unanswered, "a request still waiting for its answer is remembered");
        let keys: Vec<&str> = back.accounts.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["tenno", "second"], "in their order");
        let a = &back.accounts[0];
        assert_eq!((a.id.as_deref(), a.badge, a.seen), (acc.id.as_deref(), Some((16, Some(18))), Some(5.0)));
        assert_eq!(a.profile.as_ref().map(|p| p.fetched), Some(42.0));
        assert_eq!(a.inventory.as_ref().map(|i| i.counts["/P"]), Some(3));
        assert_eq!(a.manual.counts["/Part"], (2, 11.0));
        assert_eq!(a.manual.pinned, ["Lith A1", "/Item"]);
        assert_eq!(a.manual.tasks, manual.tasks);

        db.delete_account("tenno").unwrap();
        let back = db.load_accounts().unwrap();
        assert_eq!(back.accounts.len(), 1);
        assert!(db.memscan_sections("tenno").unwrap().captures == 0);
    }

    #[test]
    fn a_pick_is_one_change_per_screen_and_a_newer_read_settles_it() {
        let db = Db::memory();
        let acc = Account::new("tenno".into(), "Tennoplayer".into());
        db.save_registry(&Some("tenno".into()), 0.0, ProfileTimes::default(), std::slice::from_ref(&acc)).unwrap();
        let entry = Entry {
            t: 100.0,
            drops: vec![reward("Ash Prime Systems Blueprint", Some(20)), reward("Forma Blueprint", None)],
            best: None,
            account: Some("tenno".into()),
            id: None,
            picked: None,
        };
        let id = db.record_reward(&entry).unwrap();
        assert_eq!(db.record_reward(&Entry { t: 103.0, ..entry.clone() }).unwrap(), id, "the same screen");
        let change = |uid: &str| Delta {
            t: 103.0,
            kind: "pick".into(),
            uid: uid.into(),
            change: 1,
            equipment: false,
            reward: Some(id),
        };
        db.set_pick(id, "tenno", Some("Forma Blueprint"), &[change("/Forma Blueprint")]).unwrap();
        db.set_pick(id, "tenno", Some("Ash Prime Systems Blueprint"), &[change("/Ash Prime Systems Blueprint")])
            .unwrap();
        assert_eq!(
            db.deltas("tenno").unwrap(),
            [change("/Ash Prime Systems Blueprint")],
            "changing the pick replaces it"
        );
        assert_eq!(db.reward(id).unwrap().unwrap().picked.as_deref(), Some("Ash Prime Systems Blueprint"));
        assert_eq!(db.load_accounts().unwrap().accounts[0].deltas.len(), 1, "kept across a restart");
        db.settle_deltas("tenno", 90.0).unwrap();
        assert_eq!(db.deltas("tenno").unwrap().len(), 1, "an older read does not include it");
        db.settle_deltas("tenno", 110.0).unwrap();
        assert!(db.deltas("tenno").unwrap().is_empty());
        db.set_pick(id, "tenno", None, &[]).unwrap();
        assert_eq!(db.reward(id).unwrap().unwrap().picked, None);
    }

    #[test]
    fn the_same_screen_read_twice_is_one_record_and_the_history_has_no_cap() {
        let db = Db::memory();
        let entry = |t, account: Option<&str>, price| Entry {
            t,
            drops: vec![reward("Ash Prime Systems Blueprint", price), reward("Forma Blueprint", None)],
            best: None,
            account: account.map(str::to_owned),
            id: None,
            picked: None,
        };
        db.record_reward(&entry(100.0, Some("a"), None)).unwrap();
        db.record_reward(&entry(105.0, Some("a"), Some(25))).unwrap();
        let recent = db.recent_rewards(10, Some("a")).unwrap();
        assert_eq!(recent.len(), 1, "F9 twice: one screen");
        assert_eq!(recent[0].drops[0].price, Some(25), "with what the second read knew");
        for k in 0..400 {
            db.record_reward(&entry(1000.0 + k as f64 * 60.0, Some(if k % 2 == 0 { "a" } else { "b" }), Some(k)))
                .unwrap();
        }
        assert_eq!(db.recent_rewards(1000, None).unwrap().len(), 401, "nothing is dropped");
        assert_eq!(db.recent_rewards(1000, Some("b")).unwrap().len(), 200);
        let (screens, best) = db.reward_totals(1000.0 + 390.0 * 60.0, Some("a")).unwrap();
        assert_eq!((screens, best), (5, 390 + 392 + 394 + 396 + 398));
        // A screen read before accounts existed shows for everyone.
        db.record_reward(&entry(99_999.0, None, Some(1))).unwrap();
        assert_eq!(db.recent_rewards(1, Some("b")).unwrap()[0].account, None);
    }

    #[test]
    fn prices_are_kept_per_platform_with_a_sparse_history() {
        let db = Db::memory();
        let price = |sell, t| Price { sell: Some(sell), sell_avg: None, buy: Some(sell - 2), t, err: None };
        db.save_prices(Platform::Pc, &[("ash_prime_set".into(), price(100, 1000.0))]).unwrap();
        db.save_prices(Platform::Pc, &[("ash_prime_set".into(), price(105, 2000.0))]).unwrap();
        db.save_prices(Platform::Pc, &[("ash_prime_set".into(), price(110, 1000.0 + PRICE_HISTORY_EVERY))]).unwrap();
        db.save_prices(Platform::Ps4, &[("ash_prime_set".into(), price(70, 1000.0))]).unwrap();
        assert_eq!(db.load_prices(Platform::Pc).unwrap()["ash_prime_set"].sell, Some(110));
        assert_eq!(db.load_prices(Platform::Ps4).unwrap()["ash_prime_set"].sell, Some(70));
        let history = db.price_history(Platform::Pc, "ash_prime_set").unwrap();
        assert_eq!(
            history.iter().map(|h| h.sell).collect::<Vec<_>>(),
            [Some(100), Some(110)],
            "one point per few hours"
        );
    }

    #[test]
    fn a_failed_migration_changes_nothing() {
        let conn = Connection::open_in_memory().unwrap();
        // A table in the way of the first migration: it fails, and the version stays 0.
        conn.execute_batch("CREATE TABLE meta (x INTEGER);").unwrap();
        let db = Db { conn: Mutex::new(conn) };
        assert!(db.migrate(None).is_err());
        let version: i64 = lock(&db.conn).pragma_query_value(None, "user_version", |r| r.get(0)).unwrap();
        assert_eq!(version, 0);
    }
}
