//! Relic drop tables when WFCD's are missing.
//!
//! Since late October 2026, WFCD's `Relics.json` lists every relic without its rewards and
//! without `vaulted`. That emptied relic values and farm hints, made every relic look vaulted,
//! and left the reward-screen reader with no names to look for. This module fills those gaps in the WFCD file, before the catalog is built:
//!
//! * which items a relic holds, with their exact ids and rarities: the game's own export
//!   (warframe-public-export-plus `ExportRelics.json`, whose `rewardManifest` points into
//!   `ExportRewards.json`);
//! * the names the game prints for them: WFCD's copy of DE's drop tables
//!   (drops.warframestat.us `relics.json`), paired with the export's items within each rarity;
//! * whether a relic still drops: whether DE's mission and bounty tables name it today. A relic
//!   that appears in none of them is vaulted. The export's `vaultedAt` is only the fallback: 21
//!   old relics (Axi A2, Neo O1...) have none and drop nowhere.
//!
//! The chance of a drop follows from its rarity and the relic's refinement alone; the table below
//! matched DE's drop tables on all 18,528 relic drops.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

/// Drop chance in percent: (common, uncommon, rare) per refinement.
fn chance(refinement: &str, rarity: &str) -> Option<f64> {
    let row = match refinement {
        "Intact" => [25.33, 11.0, 2.0],
        "Exceptional" => [23.33, 13.0, 4.0],
        "Flawless" => [20.0, 17.0, 6.0],
        "Radiant" => [16.67, 20.0, 10.0],
        _ => return None,
    };
    match rarity {
        "COMMON" => Some(row[0]),
        "UNCOMMON" => Some(row[1]),
        "RARE" => Some(row[2]),
        _ => None,
    }
}

/// The export's quality names for the four refinements.
fn refinement(quality: &str) -> Option<&'static str> {
    match quality {
        "VPQ_BRONZE" => Some("Intact"),
        "VPQ_SILVER" => Some("Exceptional"),
        "VPQ_GOLD" => Some("Flawless"),
        "VPQ_PLATINUM" => Some("Radiant"),
        _ => None,
    }
}

/// "COMMON" -> "Common", the spelling WFCD's files use.
fn rarity_word(rarity: &str) -> String {
    let lower = rarity.to_lowercase();
    let mut chars = lower.chars();
    chars.next().map(|f| f.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
}

/// The rarity a drop chance in an intact relic stands for. WFCD's drop data labels every common
/// drop "Uncommon" (its own issue tracker, 2026-06), so the chance is what counts.
fn rarity_of_intact_chance(chance: f64) -> Option<&'static str> {
    if chance > 20.0 {
        Some("COMMON")
    } else if chance > 5.0 {
        Some("UNCOMMON")
    } else if chance > 0.0 {
        Some("RARE")
    } else {
        None
    }
}

/// Lower-case words of an id's last segment or of a name: "PrimeDaikyuLowerLimb" and "Daikyu
/// Prime Lower Limb" give the same set.
fn words(s: &str) -> HashSet<String> {
    let leaf = s.rsplit('/').next().unwrap_or(s).replace('&', " and ");
    let mut out = HashSet::new();
    let mut word = String::new();
    let mut prev: Option<char> = None;
    for c in leaf.chars() {
        let boundary = !c.is_alphanumeric()
            || (c.is_uppercase() && prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit()))
            || (c.is_ascii_digit() && prev.is_some_and(|p| p.is_alphabetic()));
        if boundary && !word.is_empty() {
            out.insert(std::mem::take(&mut word).to_lowercase());
        }
        if c.is_alphanumeric() {
            word.push(c);
        }
        prev = Some(c);
    }
    if !word.is_empty() {
        out.insert(word.to_lowercase());
    }
    out
}

fn likeness(uid: &str, name: &str) -> f64 {
    let (a, b) = (words(uid), words(name));
    let union = a.union(&b).count();
    if union == 0 { 0.0 } else { a.intersection(&b).count() as f64 / union as f64 }
}

/// Names the items of one relic. The export gives ids and rarities, the drop tables names and
/// chances; within a rarity there are one to three of each, so the pairs with the most words in
/// common are taken first and the last ones fall into place (`RivenFragment` is the "Riven
/// Sliver", `PrimeCronusLongSwordBlueprint` the "Dakra Prime Blueprint").
fn pair_names(items: &[(String, String)], names: &[(String, f64)]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for rarity in ["COMMON", "UNCOMMON", "RARE"] {
        let ids: Vec<&String> = items.iter().filter(|(_, r)| r == rarity).map(|(u, _)| u).collect();
        let labels: Vec<&String> =
            names.iter().filter(|(_, c)| rarity_of_intact_chance(*c) == Some(rarity)).map(|(n, _)| n).collect();
        let mut pairs: Vec<(f64, &String, &String)> =
            ids.iter().flat_map(|u| labels.iter().map(move |n| (likeness(u, n), *u, *n))).collect();
        pairs.sort_by(|a, b| b.0.total_cmp(&a.0));
        let mut taken: HashSet<&String> = HashSet::new();
        for (_, uid, name) in pairs {
            if out.contains_key(uid) || taken.contains(name) {
                continue;
            }
            taken.insert(name);
            out.insert(uid.clone(), name.clone());
        }
    }
    out
}

/// Every relic DE's current drop tables name ("Lith A12"), from any of their files.
pub fn dropping(tables: &[Value]) -> HashSet<String> {
    fn walk(v: &Value, out: &mut HashSet<String>) {
        match v {
            Value::String(s) => {
                if let Some(base) = s.strip_suffix(" Relic") {
                    let era = base.split(' ').next().unwrap_or("");
                    if ["Lith", "Meso", "Neo", "Axi", "Requiem"].contains(&era) && base.contains(' ') {
                        out.insert(base.to_string());
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            Value::Object(map) => map.values().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = HashSet::new();
    for t in tables {
        walk(t, &mut out);
    }
    out
}

/// Whether WFCD's relic file lacks its drop tables (and needs `patch`).
pub fn lacks_rewards(wfcd_relics: &Value) -> bool {
    let list = wfcd_relics.as_array().map(Vec::as_slice).unwrap_or(&[]);
    !list.is_empty() && list.iter().all(|r| r["rewards"].as_array().is_none_or(Vec::is_empty))
}

/// Fills the rewards and the vaulted flag of every WFCD relic that lacks them, and adds the
/// relics the export has and WFCD does not. `drop_names` is drops.warframestat.us `relics.json`,
/// `dropping` the relics today's drop tables name; either may be missing.
pub fn patch(
    wfcd_relics: &mut Value,
    export_relics: &Value,
    export_rewards: &Value,
    drop_names: Option<&Value>,
    dropping: Option<&HashSet<String>>,
) -> usize {
    // (era, name) -> the intact relic's drop names and chances.
    let mut names: HashMap<(String, String), Vec<(String, f64)>> = HashMap::new();
    for r in drop_names.and_then(|d| d["relics"].as_array()).into_iter().flatten() {
        if r["state"] != "Intact" {
            continue;
        }
        let (Some(era), Some(name)) = (r["tier"].as_str(), r["relicName"].as_str()) else { continue };
        let list = r["rewards"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|x| Some((x["itemName"].as_str()?.to_string(), x["chance"].as_f64()?)))
            .collect();
        names.insert((era.to_string(), name.to_string()), list);
    }
    let Some(list) = wfcd_relics.as_array_mut() else { return 0 };
    // A relic's picture is the same for its whole era.
    let mut era_image: HashMap<String, Value> = HashMap::new();
    for r in list.iter() {
        if let Some(era) = r["name"].as_str().and_then(|n| n.split(' ').next()) {
            era_image.entry(era.to_string()).or_insert_with(|| r["imageName"].clone());
        }
    }
    let mut index: HashMap<String, usize> = HashMap::new();
    for (i, r) in list.iter().enumerate() {
        if let Some(uid) = r["uniqueName"].as_str() {
            index.insert(uid.to_string(), i);
        }
    }
    let mut filled = 0;
    for (uid, r) in export_relics.as_object().into_iter().flatten() {
        let (Some(era), Some(name), Some(refine)) =
            (r["era"].as_str(), r["category"].as_str(), r["quality"].as_str().and_then(refinement))
        else {
            continue;
        };
        let base = format!("{era} {name}");
        let items: Vec<(String, String)> = export_rewards[r["rewardManifest"].as_str().unwrap_or("")]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|tier| tier.as_array().into_iter().flatten())
            .filter_map(|i| {
                // The export lists what a relic gives as store items: the item's own id follows.
                let id = i["type"].as_str()?.replacen("/StoreItems", "", 1);
                Some((id, i["rarity"].as_str()?.to_string()))
            })
            .collect();
        if items.is_empty() {
            continue;
        }
        let printed =
            names.get(&(era.to_string(), name.to_string())).map(|n| pair_names(&items, n)).unwrap_or_default();
        let rewards: Vec<Value> = items
            .iter()
            .map(|(id, rarity)| {
                let shown = printed.get(id).cloned().unwrap_or_else(|| crate::gamedata::camel_name(id));
                json!({"item": {"uniqueName": id, "name": shown}, "rarity": rarity_word(rarity),
                       "chance": chance(refine, rarity)})
            })
            .collect();
        let vaulted = match dropping {
            Some(set) if !set.is_empty() => !set.contains(&base),
            _ => r.get("vaultedAt").is_some(),
        };
        match index.get(uid) {
            Some(&i) => {
                if list[i]["rewards"].as_array().is_none_or(Vec::is_empty) {
                    list[i]["rewards"] = json!(rewards);
                    list[i]["vaulted"] = json!(vaulted);
                    filled += 1;
                }
            }
            None => {
                list.push(json!({"uniqueName": uid, "name": format!("{base} {refine}"), "rewards": rewards,
                                 "vaulted": vaulted, "imageName": era_image.get(era).cloned().unwrap_or(Value::Null)}));
                filled += 1;
            }
        }
    }
    filled
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One relic as the three sources describe it (shortened from the Lith A12 entries).
    fn sources() -> (Value, Value, Value, Value) {
        let wfcd = json!([{"uniqueName": "/R/LithA12Bronze", "name": "Lith A12 Intact", "imageName": "RelicLithA.png", "rewards": []}]);
        let export_relics = json!({
            "/R/LithA12Bronze": {"era": "Lith", "category": "A12", "quality": "VPQ_BRONZE", "rewardManifest": "/M/A12"},
            "/R/LithA12Platinum": {"era": "Lith", "category": "A12", "quality": "VPQ_PLATINUM", "rewardManifest": "/M/A12"},
            "/R/AxiA2Bronze": {"era": "Axi", "category": "A2", "quality": "VPQ_BRONZE", "rewardManifest": "/M/A2"}});
        let export_rewards = json!({
            "/M/A12": [[
                {"type": "/Lotus/StoreItems/Types/Recipes/Weapons/AlternoxPrimeBlueprint", "rarity": "RARE"},
                {"type": "/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/PrimeDaikyuLowerLimb", "rarity": "UNCOMMON"},
                {"type": "/Lotus/StoreItems/Types/Recipes/Weapons/PrimeCronusLongSwordBlueprint", "rarity": "UNCOMMON"},
                {"type": "/Lotus/StoreItems/Types/Recipes/Components/FormaBlueprint", "rarity": "COMMON"},
                {"type": "/Lotus/StoreItems/Types/Items/MiscItems/RivenFragment", "rarity": "COMMON"}]],
            "/M/A2": [[{"type": "/Lotus/StoreItems/Types/Items/MiscItems/Kuva", "rarity": "COMMON"}]]});
        // WFCD's drop data: every common labelled "Uncommon", as it really is.
        let drop_names = json!({"relics": [{"tier": "Lith", "relicName": "A12", "state": "Intact", "rewards": [
            {"itemName": "Riven Sliver", "rarity": "Uncommon", "chance": 25.33},
            {"itemName": "Dakra Prime Blueprint", "rarity": "Uncommon", "chance": 11},
            {"itemName": "Alternox Prime Blueprint", "rarity": "Rare", "chance": 2},
            {"itemName": "Forma Blueprint", "rarity": "Uncommon", "chance": 25.33},
            {"itemName": "Daikyu Prime Lower Limb", "rarity": "Uncommon", "chance": 11}]}]});
        (wfcd, export_relics, export_rewards, drop_names)
    }

    #[test]
    fn relics_get_their_drops_names_chances_and_vaulted_flag() {
        let (mut wfcd, relics, rewards, names) = sources();
        assert!(lacks_rewards(&wfcd));
        let dropping = HashSet::from(["Lith A12".to_string()]);
        let filled = patch(&mut wfcd, &relics, &rewards, Some(&names), Some(&dropping));
        assert_eq!(filled, 3, "the intact relic filled, the radiant one and Axi A2 added");
        assert!(!lacks_rewards(&wfcd));
        let intact = &wfcd[0];
        assert_eq!(intact["vaulted"], false);
        let by_name = |r: &Value, id: &str| {
            r["rewards"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["item"]["uniqueName"].as_str().unwrap().ends_with(id))
                .cloned()
                .unwrap()
        };
        assert_eq!(by_name(intact, "PrimeCronusLongSwordBlueprint")["item"]["name"], "Dakra Prime Blueprint");
        assert_eq!(by_name(intact, "PrimeDaikyuLowerLimb")["item"]["name"], "Daikyu Prime Lower Limb");
        assert_eq!(by_name(intact, "RivenFragment")["item"]["name"], "Riven Sliver");
        assert_eq!(by_name(intact, "FormaBlueprint")["item"]["name"], "Forma Blueprint");
        assert_eq!(by_name(intact, "AlternoxPrimeBlueprint")["rarity"], "Rare");
        assert_eq!(by_name(intact, "AlternoxPrimeBlueprint")["chance"], 2.0);
        assert_eq!(
            by_name(intact, "PrimeCronusLongSwordBlueprint")["item"]["uniqueName"],
            "/Lotus/Types/Recipes/Weapons/PrimeCronusLongSwordBlueprint",
            "the store item becomes the item's own id"
        );
        let radiant = wfcd.as_array().unwrap().iter().find(|r| r["name"] == "Lith A12 Radiant").unwrap().clone();
        assert_eq!(by_name(&radiant, "FormaBlueprint")["chance"], 16.67);
        assert_eq!(radiant["imageName"], "RelicLithA.png", "a new relic takes its era's picture");
        let axi = wfcd.as_array().unwrap().iter().find(|r| r["name"] == "Axi A2 Intact").unwrap().clone();
        assert_eq!(axi["vaulted"], true, "named by no drop table today");
        assert_eq!(axi["rewards"][0]["item"]["name"], "Kuva", "no printed name: the id's words");
    }

    #[test]
    fn drop_tables_name_the_relics_that_still_drop() {
        let tables = [
            json!({"missionRewards": {"Earth": {"Mantle": {"rewards": {"A": [{"itemName": "Lith A12 Relic"}]}}}}}),
            json!([{"bountyLevel": "x", "rewards": [{"itemName": "Requiem II Relic"}, {"itemName": "Relic Pack"}]}]),
        ];
        let set = dropping(&tables);
        assert_eq!(set, HashSet::from(["Lith A12".to_string(), "Requiem II".to_string()]));
    }

    #[test]
    fn words_split_ids_and_names_alike() {
        assert_eq!(
            words("/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeDaikyuLowerLimb"),
            words("Daikyu Prime Lower Limb")
        );
        assert_eq!(words("Ack & Brunt"), words("AckAndBrunt"));
        assert!(likeness("/X/XakuPrimeSystemsBlueprint", "Xaku Prime Systems Blueprint") > 0.99);
    }
}
