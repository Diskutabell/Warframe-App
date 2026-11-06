//! The accounts played on this machine, kept apart.
//!
//! Every account has a folder of its own, `<data>/accounts/<key>/`, for its profile, inventory,
//! hand-set counts and memory-scan statistics, and `accounts.json` lists them with their names
//! and ids. Which account the game is logged in to comes from EE.log ("Logged in <name>").
//!
//! Up to 0.2.0 the app kept a single account in the data folder itself. With two accounts on one
//! machine that mixed them up: the profile of one sat next to the inventory the memory scan read
//! from the other. `load` moves that layout into a folder once, and leaves an inventory that does
//! not belong to the profile's account aside instead of adopting it.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config;
use crate::eelog;
use crate::sources::{Inventory, Profile};

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

/// A log line that may hold the account's id, for the user to confirm.
#[derive(Clone, Debug, Serialize)]
pub struct Evidence {
    pub id: String,
    pub line: String,
}

/// One account: what `accounts.json` keeps about it, and what is loaded from its folder.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Account {
    /// The folder name, made from the name the account was first seen with. It stays when the
    /// name changes.
    pub key: String,
    /// The display name, without the platform glyph.
    pub name: String,
    pub id: Option<String>,
    /// Where the id came from: "EE.log", "entered by you", "DE profile", "saved".
    #[serde(default)]
    pub source: Option<String>,
    /// The mastery badge the log last showed while this account was logged in: (low, high), high
    /// `None` when the badge is exact. Kept, because the log only covers the latest session.
    #[serde(default)]
    pub badge: Option<(u32, Option<u32>)>,
    /// When the game last logged in to this account while the app watched.
    #[serde(default)]
    pub seen: Option<f64>,
    #[serde(skip)]
    pub profile: Option<Arc<Profile>>,
    #[serde(skip)]
    pub inventory: Option<Arc<Inventory>>,
    #[serde(skip)]
    pub manual: Manual,
    /// Ids seen in EE.log while this account was logged in.
    #[serde(skip)]
    pub evidence: Vec<Evidence>,
    #[serde(skip)]
    pub profile_error: Option<String>,
}

impl Account {
    pub fn new(key: String, name: String) -> Account {
        Account { key, name, ..Account::default() }
    }

    /// The rank held: DE's profile has it from rank 1 up, an inventory now and then, and the
    /// log's badge always (as a band of three ranks above 0).
    pub fn mastery_rank(&self) -> Option<i64> {
        self.profile
            .as_ref()
            .and_then(|p| p.mastery_rank)
            .or_else(|| self.inventory.as_ref().and_then(|i| i.mastery_rank))
            .or(self.badge.map(|b| b.0 as i64))
    }

    /// The top of the badge's band, when the badge is all that is known.
    pub fn mastery_rank_max(&self) -> Option<i64> {
        let exact = self.profile.as_ref().and_then(|p| p.mastery_rank).is_some()
            || self.inventory.as_ref().and_then(|i| i.mastery_rank).is_some();
        if exact { None } else { self.badge.and_then(|b| b.1).map(i64::from) }
    }

    /// Mastery XP of every item, from the newest of profile and inventory: what `same_owner`
    /// compares a capture with.
    pub fn known_xp(&self) -> Vec<&IndexMap<String, i64>> {
        let mut out = Vec::new();
        if let Some(p) = &self.profile {
            out.push(&p.xp);
        }
        if let Some(i) = &self.inventory {
            out.push(&i.xp);
        }
        out
    }

    /// Writes one of the account's files.
    pub fn write<T: Serialize + ?Sized>(&self, file: &str, value: &T) -> anyhow::Result<()> {
        let dir = dir(&self.key);
        fs::create_dir_all(&dir)?;
        config::write_json(&dir.join(file), value)
    }
}

/// `accounts.json` as read.
#[derive(Default, Deserialize)]
pub struct Registry {
    /// The account the window shows.
    pub active: Option<String>,
    /// When `active` last changed, by a login or by the user's choice.
    #[serde(default)]
    pub switched: f64,
    pub accounts: Vec<Account>,
}

fn registry_file() -> PathBuf {
    config::dirs().data.join("accounts.json")
}

pub fn dir(key: &str) -> PathBuf {
    config::dirs().data.join("accounts").join(key)
}

/// `accounts.json` as written: the same fields as `Registry`, borrowed.
#[derive(Serialize)]
struct RegistryRef<'a> {
    active: &'a Option<String>,
    switched: f64,
    accounts: &'a [Account],
}

pub fn save(active: &Option<String>, switched: f64, accounts: &[Account]) -> anyhow::Result<()> {
    config::write_json_pretty(&registry_file(), &RegistryRef { active, switched, accounts })
}

/// The accounts with their data, and what happened while loading them (for the activity list).
pub fn load() -> (Registry, Vec<String>) {
    let mut notes = Vec::new();
    let mut registry = match config::read_json::<Registry>(&registry_file()) {
        Some(r) => r,
        None => {
            let r = migrate(&mut notes);
            if let Err(e) = save(&r.active, r.switched, &r.accounts) {
                notes.push(format!("Could not save the account list: {e:#}"));
            }
            r
        }
    };
    for acc in &mut registry.accounts {
        let folder = dir(&acc.key);
        acc.profile = config::read_json::<Profile>(&folder.join("profile.json")).map(Arc::new);
        acc.inventory = config::read_json::<Inventory>(&folder.join("inventory.json")).map(Arc::new);
        acc.manual = config::read_json(&folder.join("manual.json")).unwrap_or_default();
    }
    (registry, notes)
}

/// Removes an account's folder for good.
pub fn delete(key: &str) {
    let _ = fs::remove_dir_all(dir(key));
}

/// A folder name for a new account: the name in lower case, letters and digits only, and a
/// number added when another account already has it.
pub fn new_key(name: &str, taken: &[&str]) -> String {
    let base: String = eelog::strip_glyphs(name)
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let base = if base.is_empty() { "account".to_string() } else { base };
    let mut key = base.clone();
    let mut n = 2;
    while taken.contains(&key.as_str()) {
        key = format!("{base}-{n}");
        n += 1;
    }
    key
}

/// Whether two mastery-XP lists (item -> XP, as `XPInfo` has them) come from the same account:
/// `None` when they share too few items to tell.
///
/// An item's mastery XP only ever grows, and between two readings of one account only the items
/// used in between change, so most shared items match exactly. Two accounts practically never
/// do: one account's profile and memory capture typically match on almost every shared item,
/// two different accounts on none.
pub fn same_owner(a: &IndexMap<String, i64>, b: &IndexMap<String, i64>) -> Option<bool> {
    const MIN_SHARED: usize = 10;
    let (mut shared, mut equal) = (0usize, 0usize);
    for (item, xa) in a {
        let Some(xb) = b.get(item) else { continue };
        // Items never used say nothing: every account has them at 0.
        if *xa <= 0 || *xb <= 0 {
            continue;
        }
        shared += 1;
        if xa == xb {
            equal += 1;
        }
    }
    if shared < MIN_SHARED {
        return None;
    }
    // A quarter matching is far above chance, and far below what even weeks of play change.
    Some(equal * 4 >= shared)
}

/// Moves the single-account layout of 0.2.0 into an account folder.
fn migrate(notes: &mut Vec<String>) -> Registry {
    let data = config::dirs().data.clone();
    let legacy: Value = config::read_json(&data.join("account.json")).unwrap_or(Value::Null);
    // The account id could also be set in settings.json (since removed from the settings).
    let settings: Value = config::read_json(&config::settings_file()).unwrap_or(Value::Null);
    let set_id = settings["account_id"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
    let profile: Option<Profile> = config::read_json(&data.join("profile.json"));
    let inventory: Option<Inventory> = config::read_json(&data.join("inventory.json"));
    let manual: Option<Manual> = config::read_json(&data.join("manual.json"));

    // The profile names the account it belongs to; the old account file may name another one
    // (it followed whoever logged in last).
    let name = profile
        .as_ref()
        .and_then(|p| p.name.as_deref())
        .or(legacy["name"].as_str())
        .map(eelog::strip_glyphs)
        .filter(|n| !n.is_empty());
    let id = set_id.clone().or_else(|| legacy["id"].as_str().map(str::to_owned));
    if name.is_none() && id.is_none() && profile.is_none() && inventory.is_none() && manual.is_none() {
        return Registry::default();
    }
    let name = name.unwrap_or_else(|| "Account 1".to_string());
    let mut acc = Account::new(new_key(&name, &[]), name);
    acc.id = id;
    acc.source = Some(if set_id.is_some() { "Settings" } else { "saved" }.to_string());
    let folder = dir(&acc.key);
    let _ = fs::create_dir_all(&folder);
    let moved = |file: &str| {
        let _ = fs::rename(data.join(file), folder.join(file));
    };
    moved("profile.json");
    moved("manual.json");
    if let Some(inv) = &inventory {
        let theirs = profile.as_ref().and_then(|p| same_owner(&inv.xp, &p.xp));
        if theirs == Some(false) {
            // Read from the other account's game session: keep it out of this account. The next
            // memory scan reads the logged-in account's own.
            let _ = fs::rename(data.join("inventory.json"), data.join("inventory.unassigned.json"));
            let _ = fs::remove_file(data.join("memscan_sections.json"));
            notes.push(format!(
                "The saved inventory was another account's, not {}'s; it was set aside (inventory.unassigned.json).",
                acc.name
            ));
        } else {
            moved("inventory.json");
            moved("memscan_sections.json");
        }
    }
    let _ = fs::remove_file(data.join("account.json"));
    notes.push(format!("Your data now lives in an account of its own: {}.", acc.name));
    Registry { active: Some(acc.key.clone()), switched: 0.0, accounts: vec![acc] }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn xp(pairs: &[(&str, i64)]) -> IndexMap<String, i64> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn owners_are_told_apart_by_exact_mastery_xp() {
        let items: Vec<String> = (0..20).map(|i| format!("/Item{i}")).collect();
        let base: IndexMap<String, i64> = items.iter().enumerate().map(|(i, k)| (k.clone(), 1000 + i as i64)).collect();
        // The same account a few missions later: three items levelled, one new.
        let mut later = base.clone();
        for k in &items[..3] {
            *later.get_mut(k).unwrap() += 777;
        }
        later.insert("/New".into(), 5);
        assert_eq!(same_owner(&base, &later), Some(true));
        // Another account with the same gear: no value matches.
        let other: IndexMap<String, i64> =
            items.iter().enumerate().map(|(i, k)| (k.clone(), 5000 + i as i64)).collect();
        assert_eq!(same_owner(&base, &other), Some(false));
        // Too little in common to say; unused items (0) do not count.
        assert_eq!(same_owner(&xp(&[("/A", 1), ("/B", 2)]), &xp(&[("/A", 1), ("/B", 2)])), None);
        let unused: IndexMap<String, i64> = items.iter().map(|k| (k.clone(), 0)).collect();
        assert_eq!(same_owner(&unused, &unused), None);
    }

    #[test]
    fn keys_are_plain_and_unique() {
        assert_eq!(new_key("Tenno\u{e000}", &[]), "tenno");
        assert_eq!(new_key("Tenno", &["tenno"]), "tenno-2");
        assert_eq!(new_key("Tenno", &["tenno", "tenno-2"]), "tenno-3");
        assert_eq!(new_key("Ünïcode Name", &[]), "n-code-name");
        assert_eq!(new_key("", &[]), "account");
    }

    #[test]
    fn the_rank_comes_from_the_profile_then_the_badge() {
        let mut acc = Account::new("a".into(), "A".into());
        assert_eq!(acc.mastery_rank(), None);
        acc.badge = Some((16, Some(18)));
        assert_eq!((acc.mastery_rank(), acc.mastery_rank_max()), (Some(16), Some(18)));
        acc.profile = Some(Arc::new(Profile { mastery_rank: Some(17), ..Profile::default() }));
        assert_eq!((acc.mastery_rank(), acc.mastery_rank_max()), (Some(17), None));
    }
}
