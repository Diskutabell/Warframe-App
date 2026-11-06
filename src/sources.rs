//! Account data that does not come from game memory.
//!
//! * The public profile (getProfileViewingData): what the in-game profile screen shows other
//!   players - mastery XP per item and the display name. It needs only the account id.
//! * An inventory.json: the raw inventory payload, from a file the user imports, a watched file,
//!   or a memory scan. Parsed locally, never uploaded.

use std::time::Duration;

use anyhow::Result;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Platform;
use crate::{UserError, net, now};

fn profile_host(platform: Platform) -> &'static str {
    match platform {
        Platform::Pc => "https://api.warframe.com",
        Platform::Ps4 => "https://api-ps4.warframe.com",
        Platform::Xb1 => "https://api-xb1.warframe.com",
        Platform::Swi => "https://api-swi.warframe.com",
    }
}

/// Equipment arrays in the inventory; each entry is one owned copy.
const EQUIPMENT_KEYS: [&str; 18] = [
    "Suits",
    "LongGuns",
    "Pistols",
    "Melee",
    "SpaceSuits",
    "SpaceGuns",
    "SpaceMelee",
    "Sentinels",
    "SentinelWeapons",
    "KubrowPets",
    "MoaPets",
    "Hoverboards",
    "MechSuits",
    "OperatorAmps",
    "SpecialItems",
    "CrewShipWeapons",
    "CrewShipHarnesses",
    "KubrowPetEggs",
];
/// Stackable arrays: ItemType + ItemCount.
const COUNTED_KEYS: [&str; 7] =
    ["MiscItems", "Recipes", "RawUpgrades", "Consumables", "FlavourItems", "LevelKeys", "ShipDecorations"];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Profile {
    pub name: Option<String>,
    /// The account id the profile belongs to (`AccountId`), when it says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// `PlayerLevel`. DE leaves it out at rank 0 (a default value is not sent).
    pub mastery_rank: Option<i64>,
    pub xp: IndexMap<String, i64>,
    /// Star chart and intrinsics. `None` in a profile saved before the app read them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    /// Syndicate standing and today's limits. `None` in a profile saved before the app read them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<Standing>,
    pub fetched: f64,
    #[serde(default)]
    pub source: Option<String>,
}

/// One syndicate's standing as the game stores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Affiliation {
    /// Counted from zero over all ranks, not from the start of the current rank: 289,873 is
    /// rank 5, whose range is 240,000-372,000 in the game's export.
    pub standing: i64,
    /// The rank ("title"), -2 to 5 for most syndicates; a Nightwave season counts its tiers.
    pub title: i64,
}

/// Syndicate standing and the daily limits, as DE's profile and a raw inventory carry them:
/// `Affiliations: [{"Tag", "Standing", "Title"}]`, one `DailyAffiliation*` number per limit pool,
/// and `DailyFocus`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Standing {
    /// Syndicate tag ("SteelMeridianSyndicate") -> its standing and rank.
    pub syndicates: IndexMap<String, Affiliation>,
    /// Standing still allowed today per limit pool ("DailyAffiliationCetus" -> 16000) when the
    /// source was read. The pools refill at the daily reset; a capped pool reads 0.
    pub daily: IndexMap<String, i64>,
    /// Focus still allowed today.
    pub daily_focus: Option<i64>,
}

/// `None` when the payload has no standing at all, as in a memory capture that lost its head.
fn standing_from(data: &Value) -> Option<Standing> {
    let list = data.get("Affiliations").and_then(Value::as_array);
    let mut out = Standing { daily_focus: data.get("DailyFocus").and_then(Value::as_i64), ..Standing::default() };
    for a in list.into_iter().flatten() {
        let Some(tag) = a["Tag"].as_str().filter(|t| !t.is_empty()) else { continue };
        out.syndicates.insert(tag.to_string(), Affiliation { standing: int(&a["Standing"]), title: int(&a["Title"]) });
    }
    for (key, value) in data.as_object().into_iter().flatten() {
        if key.starts_with("DailyAffiliation") {
            if let Some(left) = value.as_i64() {
                out.daily.insert(key.clone(), left);
            }
        }
    }
    (list.is_some() || !out.daily.is_empty() || out.daily_focus.is_some()).then_some(out)
}

/// Item -> mastery XP from an `XPInfo` list: the record of what each item has earned, which only
/// ever grows. Comparing two of these tells whether they belong to one account
/// (`accounts::same_owner`).
pub fn xp_map(list: Option<&Value>) -> IndexMap<String, i64> {
    list.and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|e| e["ItemType"].as_str().filter(|t| !t.is_empty()).map(|t| (t.to_string(), int(&e["XP"]))))
        .collect()
}

/// What gives mastery besides gear: the star chart nodes completed and the intrinsic ranks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    /// Node tag ("SolNode27", "EarthToVenusJunction") -> 0 once completed, 1 once completed on
    /// the Steel Path as well.
    pub nodes: IndexMap<String, i64>,
    /// Intrinsic ("LPS_PILOTING", "LPS_DRIFT_COMBAT") -> its rank.
    pub intrinsics: IndexMap<String, i64>,
}

impl Progress {
    /// Both sources only ever grow, so the higher value of each entry is the current one.
    pub fn merge(&mut self, other: &Progress) {
        for (tag, tier) in &other.nodes {
            let mine = self.nodes.entry(tag.clone()).or_insert(*tier);
            *mine = (*mine).max(*tier);
        }
        for (skill, rank) in &other.intrinsics {
            let mine = self.intrinsics.entry(skill.clone()).or_insert(*rank);
            *mine = (*mine).max(*rank);
        }
    }
}

/// Reads `Missions` and `PlayerSkills`, which DE's profile and a raw inventory carry in the same
/// form: `{"Tag": "SolNode27", "Completes": 2}`, with `"Tier": 1` on a node done on the Steel
/// Path, and `{"LPS_PILOTING": 7, "LPP_SPACE": 29267}`.
/// `None` when neither key is there, as in a memory scan, which keeps neither.
fn progress_from(data: &Value) -> Option<Progress> {
    let missions = data.get("Missions").and_then(Value::as_array);
    let skills = data.get("PlayerSkills").and_then(Value::as_object);
    if missions.is_none() && skills.is_none() {
        return None;
    }
    let mut progress = Progress::default();
    for m in missions.into_iter().flatten() {
        let Some(tag) = m["Tag"].as_str().filter(|t| !t.is_empty()) else { continue };
        let tier = int(&m["Tier"]);
        if int(&m["Completes"]) > 0 || tier > 0 {
            let best = progress.nodes.entry(tag.to_string()).or_insert(tier);
            *best = (*best).max(tier);
        }
    }
    for (skill, rank) in skills.into_iter().flatten() {
        // LPS_* are the ranks; LPP_* are the points not spent yet.
        if skill.starts_with("LPS_") {
            progress.intrinsics.insert(skill.clone(), int(rank));
        }
    }
    Some(progress)
}

/// Fetches the public profile. The caller enforces the cooldown: DE temporarily IP-blocks game
/// login for clients that query profiles too often.
pub fn fetch_profile(account_id: &str, platform: Platform) -> Result<Profile> {
    let url = format!("{}/cdn/getProfileViewingData.php?playerId={account_id}", profile_host(platform));
    let resp = net::get(&url, &[("Accept", "application/json")], Duration::from_secs(20))
        .map_err(|e| UserError::new(format!("Could not reach DE's profile service: {e}")))?;
    match resp.status {
        200..=299 => {}
        403 => {
            return Err(UserError::new(
                "DE's server refused the request (403). This happens when it is called too often or from \
                 some VPN/VPS addresses. Wait a while, or use 'Import profile file'.",
            )
            .into());
        }
        400 | 409 => return Err(UserError::new("The account id was not accepted. Check it in Settings.").into()),
        code => return Err(UserError::new(format!("Profile request failed with HTTP {code}.")).into()),
    }
    let data: Value = serde_json::from_slice(&resp.body)
        .map_err(|_| UserError::new("The profile service returned something that is not JSON."))?;
    parse_profile(&data)
}

fn find_key<'a>(obj: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 6 {
        return None;
    }
    match obj {
        Value::Object(map) => {
            if let Some(v) = map.get(key) {
                return Some(v);
            }
            map.values().find_map(|v| find_key(v, key, depth + 1).filter(|v| !v.is_null()))
        }
        Value::Array(items) => items.iter().take(5).find_map(|v| find_key(v, key, depth + 1).filter(|v| !v.is_null())),
        _ => None,
    }
}

pub fn parse_profile(data: &Value) -> Result<Profile> {
    if !data.is_object() {
        return Err(UserError::new("Profile data is not a JSON object.").into());
    }
    let res = match data.get("Results").and_then(Value::as_array) {
        Some(list) => list.first().unwrap_or(data),
        None => data,
    };
    let xp = xp_map(find_key(res, "XPInfo", 0).or_else(|| find_key(data, "XPInfo", 0)));
    if xp.is_empty() {
        return Err(UserError::new("No mastery data (XPInfo) found in the profile JSON.").into());
    }
    let name = res["DisplayName"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .or_else(|| find_key(res, "DisplayName", 0).and_then(Value::as_str).map(str::to_owned));
    let mastery_rank = [res.get("PlayerLevel"), find_key(res, "PlayerLevel", 0), find_key(data, "PlayerLevel", 0)]
        .into_iter()
        .flatten()
        .find(|v| !v.is_null())
        .and_then(Value::as_i64);
    // {"$oid": "..."} in DE's answer; a plain string in files other tools wrote.
    let id = res.get("AccountId").map(|v| v.get("$oid").unwrap_or(v)).and_then(Value::as_str).map(str::to_owned);
    Ok(Profile {
        name,
        id,
        mastery_rank,
        xp,
        progress: progress_from(res),
        standing: standing_from(res),
        fetched: now(),
        source: None,
    })
}

/// An integer from a JSON number or numeric string; anything else counts as 0.
fn int(v: &Value) -> i64 {
    match v {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(0),
        Value::String(s) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Equipment {
    pub count: i64,
    pub xp: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FoundryEntry {
    pub recipe: Option<String>,
    /// When it finishes, epoch seconds.
    pub done: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Currencies {
    pub platinum: Option<i64>,
    pub credits: Option<i64>,
    pub endo: Option<i64>,
    pub ducats: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub counts: IndexMap<String, i64>,
    pub equipment: IndexMap<String, Equipment>,
    pub xp: IndexMap<String, i64>,
    pub foundry: Vec<FoundryEntry>,
    pub currencies: Currencies,
    pub mods: i64,
    pub mastery_rank: Option<i64>,
    /// Star chart and intrinsics, when the payload has them (an imported file does; a memory
    /// capture does when its head survived).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    /// Syndicate standing and today's limits, when the payload has them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standing: Option<Standing>,
    pub synced_at: Option<f64>,
    pub imported: f64,
    /// How many top-level sections the payload had: a later capture of the same sync with no more
    /// of them has nothing new.
    #[serde(default)]
    pub sections: usize,
    #[serde(default)]
    pub source: Option<String>,
}

impl Inventory {
    /// When the game last synced it, or when it was read if the payload does not say.
    pub fn as_of(&self) -> f64 {
        self.synced_at.filter(|t| *t > 0.0).unwrap_or(self.imported)
    }
}

/// Epoch seconds from a Mongo ObjectId (its first four bytes are a unix timestamp), or the
/// number a memory scan already resolved it to.
fn oid_time(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return (n > 0.0).then_some(n);
    }
    let oid = v.get("$oid").unwrap_or(v).as_str()?;
    i64::from_str_radix(oid.get(..8)?, 16).ok().map(|t| t as f64)
}

/// Epoch seconds from DE's date field: Mongo extended JSON, or a bare number in ms or seconds.
fn date(v: &Value) -> Option<f64> {
    if let Some(n) = v.as_f64() {
        return Some(if n > 1e11 { n / 1000.0 } else { n });
    }
    let d = v.get("$date")?;
    let n = d.get("$numberLong").unwrap_or(d);
    let ms = match n {
        Value::String(s) => s.parse::<i64>().ok()? as f64,
        Value::Number(x) => x.as_f64()?,
        _ => return None,
    };
    Some(ms / 1000.0)
}

/// Turns an inventory payload into counts, equipment, foundry and currencies.
pub fn parse_inventory(data: &Value) -> Result<Inventory> {
    // Some tools wrap the payload, sometimes as a JSON string.
    let unwrapped;
    let data = match data.get("InventoryJson") {
        Some(Value::String(s)) => {
            unwrapped = serde_json::from_str::<Value>(s)
                .map_err(|_| UserError::new("InventoryJson holds a string that is not JSON."))?;
            &unwrapped
        }
        Some(inner) => inner,
        None => data,
    };
    if !data.is_object() || !["MiscItems", "Suits", "Recipes"].iter().any(|k| data.get(k).is_some()) {
        return Err(UserError::new("This does not look like a Warframe inventory (no MiscItems/Suits/Recipes).").into());
    }
    let array = |key: &str| data[key].as_array().map(Vec::as_slice).unwrap_or(&[]).iter();
    let mut counts = IndexMap::new();
    for key in COUNTED_KEYS {
        for e in array(key) {
            if let Some(t) = e["ItemType"].as_str().filter(|t| !t.is_empty()) {
                *counts.entry(t.to_string()).or_insert(0) += int(&e["ItemCount"]);
            }
        }
    }
    let mut equipment: IndexMap<String, Equipment> = IndexMap::new();
    for key in EQUIPMENT_KEYS {
        for e in array(key) {
            if let Some(t) = e["ItemType"].as_str().filter(|t| !t.is_empty()) {
                let eq = equipment.entry(t.to_string()).or_default();
                eq.count += 1;
                eq.xp = eq.xp.max(int(&e["XP"]));
            }
        }
    }
    let xp = xp_map(data.get("XPInfo"));
    let foundry = array("PendingRecipes")
        .map(|e| FoundryEntry { recipe: e["ItemType"].as_str().map(str::to_owned), done: date(&e["CompletionDate"]) })
        .collect();
    let mods = array("Upgrades").count() as i64 + array("RawUpgrades").map(|e| int(&e["ItemCount"])).sum::<i64>();
    Ok(Inventory {
        counts,
        equipment,
        xp,
        foundry,
        currencies: Currencies {
            platinum: data["PremiumCredits"].as_i64(),
            credits: data["RegularCredits"].as_i64(),
            endo: data["FusionPoints"].as_i64(),
            ducats: data["PrimeTokens"].as_i64(),
        },
        mods,
        mastery_rank: data["PlayerLevel"].as_i64(),
        progress: progress_from(data),
        standing: standing_from(data),
        synced_at: oid_time(&data["LastInventorySync"]),
        imported: now(),
        sections: data.as_object().map_or(0, |o| o.len()),
        source: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn profile_keeps_xp_and_tolerates_nesting() {
        let p = parse_profile(&json!({"Results": [{"DisplayName": "Tenno\u{e000}",
            "LoadOutInventory": {"XPInfo": [{"ItemType": "/A", "XP": 900000}, {"ItemType": "/B", "XP": "12"}]}}]}))
        .unwrap();
        assert_eq!(p.xp["/A"], 900_000);
        assert_eq!(p.xp["/B"], 12);
        assert_eq!(p.name.as_deref(), Some("Tenno\u{e000}"));
        assert_eq!(p.mastery_rank, None);
        assert!(parse_profile(&json!({"Results": [{}]})).is_err());
    }

    #[test]
    fn profile_reads_the_star_chart_and_intrinsics() {
        let p = parse_profile(&json!({"Results": [{
            "LoadOutInventory": {"XPInfo": [{"ItemType": "/A", "XP": 1}]},
            "Missions": [{"Tag": "SolNode27", "Completes": 2}, {"Tag": "SolNode1", "Completes": 5, "Tier": 1},
                         {"Tag": "SolNode9", "Completes": 0}, {"Completes": 3}],
            "PlayerSkills": {"LPP_SPACE": 29267, "LPS_PILOTING": 7, "LPS_DRIFT_COMBAT": 2}}]}))
        .unwrap();
        let progress = p.progress.expect("the profile has both sections");
        assert_eq!(progress.nodes, IndexMap::from([("SolNode27".to_string(), 0), ("SolNode1".to_string(), 1)]));
        assert_eq!(
            progress.intrinsics,
            IndexMap::from([("LPS_PILOTING".to_string(), 7), ("LPS_DRIFT_COMBAT".to_string(), 2)]),
            "unspent points (LPP_) are not ranks"
        );
        // A profile saved before these were read has neither, and says so rather than "nothing done".
        let old = parse_profile(&json!({"Results": [{"LoadOutInventory": {"XPInfo": [{"ItemType": "/A", "XP": 1}]}}]}));
        assert_eq!(old.unwrap().progress, None);
    }

    #[test]
    fn progress_merges_to_the_higher_value() {
        let mut a = Progress {
            nodes: IndexMap::from([("N1".to_string(), 1), ("N2".to_string(), 0)]),
            intrinsics: IndexMap::from([("LPS_GUNNERY".to_string(), 3)]),
        };
        a.merge(&Progress {
            nodes: IndexMap::from([("N1".to_string(), 0), ("N2".to_string(), 1), ("N3".to_string(), 0)]),
            intrinsics: IndexMap::from([("LPS_GUNNERY".to_string(), 5), ("LPS_COMMAND".to_string(), 1)]),
        });
        assert_eq!(a.nodes.values().copied().collect::<Vec<_>>(), [1, 1, 0]);
        assert_eq!(a.intrinsics["LPS_GUNNERY"], 5);
        assert_eq!(a.intrinsics["LPS_COMMAND"], 1);
    }

    #[test]
    fn inventory_counts_dates_and_currencies() {
        let inv = parse_inventory(&json!({
            "MiscItems": [{"ItemType": "/P", "ItemCount": 3}, {"ItemType": "/P", "ItemCount": 2}],
            "Suits": [{"ItemType": "/Ash", "XP": 10}, {"ItemType": "/Ash", "XP": 900000}],
            "PendingRecipes": [{"ItemType": "/R", "CompletionDate": {"$date": {"$numberLong": "1791032114000"}}}],
            "PremiumCredits": 0, "LastInventorySync": {"$oid": "5f1e2d3c4b5a69788796a5b4"}
        }))
        .unwrap();
        assert_eq!(inv.counts["/P"], 5);
        assert_eq!(inv.equipment["/Ash"].count, 2);
        assert_eq!(inv.equipment["/Ash"].xp, 900_000);
        assert_eq!(inv.foundry[0].done, Some(1_791_032_114.0));
        assert_eq!(inv.currencies.platinum, Some(0), "a real 0 must survive");
        assert_eq!(inv.synced_at, Some(0x5f1e2d3c as f64));
        assert!(parse_inventory(&json!({"Foo": 1})).is_err());
    }

    #[test]
    fn inventory_wrapped_as_a_string() {
        let inv =
            parse_inventory(&json!({"InventoryJson": "{\"Recipes\": [{\"ItemType\": \"/X\", \"ItemCount\": 1}]}"}))
                .unwrap();
        assert_eq!(inv.counts["/X"], 1);
    }
}
