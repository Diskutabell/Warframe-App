//! The one-time import of the JSON files the app kept up to 0.9: the accounts with their
//! profiles, inventories, hand-set data and memory-scan expectations, the reward history and the
//! cached prices. The files are read, never changed (the accounts' loader may set a damaged file
//! aside and rebuild the list, as it always has) and stay where they are: a downgrade still finds
//! them.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Transaction, params};

use super::{Legacy, save_account_row, save_manual, set_meta};
use crate::accounts;
use crate::config::Platform;
use crate::history::Entry;
use crate::market::Price;
use crate::memscan::Sections;

pub(super) fn legacy(tx: &Transaction, legacy: &Legacy, notes: &mut Vec<String>) -> anyhow::Result<()> {
    let had_accounts = legacy.data.join("accounts.json").exists() || legacy.data.join("account.json").exists();
    if had_accounts || legacy.data.join("profile.json").exists() || legacy.data.join("inventory.json").exists() {
        import_accounts(tx, legacy.data, notes)?;
    }
    import_history(tx, &legacy.data.join("rewards.jsonl"), notes)?;
    import_prices(tx, legacy.cache, notes)?;
    set_meta(tx, "imported", Some(&crate::now().to_string()))?;
    Ok(())
}

fn import_accounts(tx: &Transaction, data: &Path, notes: &mut Vec<String>) -> anyhow::Result<()> {
    let (registry, loaded) = accounts::load_from(data);
    notes.extend(loaded);
    set_meta(tx, "active", registry.active.as_deref())?;
    set_meta(tx, "switched", Some(&registry.switched.to_string()))?;
    set_meta(tx, "profile_requested", Some(&registry.profile_requests.requested.to_string()))?;
    set_meta(tx, "profile_paused_until", Some(&registry.profile_requests.paused_until.to_string()))?;
    for (position, acc) in registry.accounts.iter().enumerate() {
        save_account_row(tx, position, acc)?;
        if let Some(p) = &acc.profile {
            tx.execute(
                "INSERT INTO profiles (account, fetched, data) VALUES (?1, ?2, ?3)",
                params![acc.key, p.fetched, serde_json::to_string(&**p)?],
            )?;
        }
        if let Some(i) = &acc.inventory {
            tx.execute(
                "INSERT INTO inventories (account, synced_at, imported, source, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![acc.key, i.synced_at, i.imported, i.source, serde_json::to_string(&**i)?],
            )?;
        }
        save_manual(tx, &acc.key, &acc.manual)?;
        let sections: Option<Sections> =
            crate::config::read_json(&accounts::dir_in(data, &acc.key).join("memscan_sections.json"));
        if let Some(s) = sections {
            tx.execute(
                "INSERT INTO memscan_sections (account, data) VALUES (?1, ?2)",
                params![acc.key, serde_json::to_string(&s)?],
            )?;
        }
    }
    if !registry.accounts.is_empty() {
        notes.push(format!("{} account(s) moved into the app's database.", registry.accounts.len()));
    }
    Ok(())
}

/// `rewards.jsonl`, one screen per line; a line that does not parse is skipped.
fn import_history(tx: &Transaction, file: &Path, notes: &mut Vec<String>) -> anyhow::Result<()> {
    let Ok(text) = std::fs::read_to_string(file) else { return Ok(()) };
    let (mut kept, mut skipped) = (0, 0);
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match serde_json::from_str::<Entry>(line) {
            Ok(e) => {
                tx.execute(
                    "INSERT INTO reward_history (t, account, best, drops) VALUES (?1, ?2, ?3, ?4)",
                    params![e.t, e.account, e.best, serde_json::to_string(&e.drops)?],
                )?;
                kept += 1;
            }
            Err(_) => skipped += 1,
        }
    }
    if skipped > 0 {
        notes.push(format!(
            "{kept} reward screens moved into the database; {skipped} unreadable line(s) were left out."
        ));
    }
    Ok(())
}

/// The price caches: `prices.json` (PC) and `prices_<platform>.json`.
fn import_prices(tx: &Transaction, cache: &Path, _notes: &mut [String]) -> anyhow::Result<()> {
    for platform in [Platform::Pc, Platform::Ps4, Platform::Xb1, Platform::Swi] {
        let name = match platform {
            Platform::Pc => "prices.json".to_string(),
            other => format!("prices_{}.json", other.as_str()),
        };
        let Some(prices) = crate::config::read_json::<HashMap<String, Price>>(&cache.join(name)) else { continue };
        for (slug, p) in prices {
            tx.execute(
                "INSERT OR REPLACE INTO prices (platform, slug, sell, sell_avg, buy, t, err) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![platform.as_str(), slug, p.sell, p.sell_avg, p.buy, p.t, p.err],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::super::Db;
    use super::*;
    use std::sync::Mutex;

    /// A folder laid out like an older version's data, cache and config.
    struct OldData(std::path::PathBuf);

    impl OldData {
        fn new() -> OldData {
            use std::sync::atomic::{AtomicU64, Ordering};
            static N: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "wfp-import-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("data/accounts/tenno")).unwrap();
            std::fs::create_dir_all(dir.join("cache")).unwrap();
            OldData(dir)
        }
        fn data(&self) -> std::path::PathBuf {
            self.0.join("data")
        }
        fn cache(&self) -> std::path::PathBuf {
            self.0.join("cache")
        }
        fn write(&self, rel: &str, text: &str) {
            std::fs::write(self.0.join(rel), text).unwrap();
        }
    }

    impl Drop for OldData {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn import_into(old: &OldData) -> (Db, Vec<String>) {
        let db = Db { conn: Mutex::new(rusqlite::Connection::open_in_memory().unwrap()) };
        let (data, cache) = (old.data(), old.cache());
        let notes = db.migrate(Some(Legacy { data: &data, cache: &cache })).unwrap();
        (db, notes)
    }

    #[test]
    fn an_older_versions_files_are_imported_and_left_alone() {
        let old = OldData::new();
        old.write(
            "data/accounts.json",
            r#"{"active": "tenno", "switched": 5.0, "profile_requests": {"requested": 1000.0, "paused_until": 0.0},
                "accounts": [{"key": "tenno", "name": "Tennoplayer", "id": "5f1e2d3c4b5a69788796a5b4", "badge": [16, 18]}]}"#,
        );
        old.write(
            "data/accounts/tenno/profile.json",
            r#"{"name": "Tennoplayer", "mastery_rank": 17, "xp": {"/Ash": 9}, "fetched": 50.0}"#,
        );
        old.write(
            "data/accounts/tenno/manual.json",
            r#"{"counts": {"/Part": [2, 11.0]}, "pinned": ["Lith A1"], "tasks": [{"uid": "/Volt", "added": 3.0}]}"#,
        );
        old.write("data/accounts/tenno/memscan_sections.json", r#"{"captures": 4, "seen": {"Suits": 4}}"#);
        old.write(
            "data/rewards.jsonl",
            "{\"t\": 1.0, \"drops\": [], \"best\": null}\nnot json\n{\"t\": 2.0, \"drops\": [], \"best\": null, \"account\": \"tenno\"}\n",
        );
        old.write("cache/prices.json", r#"{"ash_prime_set": {"sell": 100, "buy": 90, "t": 7.0}}"#);
        old.write("cache/prices_ps4.json", r#"{"ash_prime_set": {"sell": 70, "buy": 60, "t": 7.0}}"#);
        let before = std::fs::read_to_string(old.data().join("accounts.json")).unwrap();

        let (db, notes) = import_into(&old);
        let reg = db.load_accounts().unwrap();
        assert_eq!(reg.active.as_deref(), Some("tenno"));
        assert_eq!(reg.profile_requests.requested, 1000.0, "the rate limit survives the move");
        let acc = &reg.accounts[0];
        assert_eq!((acc.name.as_str(), acc.badge), ("Tennoplayer", Some((16, Some(18)))));
        assert_eq!(acc.profile.as_ref().and_then(|p| p.mastery_rank), Some(17));
        assert_eq!(acc.manual.counts["/Part"], (2, 11.0));
        assert_eq!(acc.manual.tasks[0].uid, "/Volt");
        assert_eq!(db.memscan_sections("tenno").unwrap().captures, 4);
        assert_eq!(db.recent_rewards(10, None).unwrap().len(), 2);
        assert!(notes.iter().any(|n| n.contains("1 unreadable line")));
        assert_eq!(db.load_prices(Platform::Ps4).unwrap()["ash_prime_set"].sell, Some(70));
        assert_eq!(std::fs::read_to_string(old.data().join("accounts.json")).unwrap(), before, "untouched");
        // Opened again: nothing is imported twice.
        assert_eq!(db.migrate(None).unwrap(), Vec::<String>::new());
        assert_eq!(db.recent_rewards(10, None).unwrap().len(), 2);
    }

    #[test]
    fn a_new_install_imports_nothing_and_writes_no_old_files() {
        let old = OldData::new();
        let (db, _) = import_into(&old);
        assert!(db.load_accounts().unwrap().accounts.is_empty());
        assert!(!old.data().join("accounts.json").exists());
    }
}
