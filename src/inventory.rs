//! Names, pictures and kinds for everything an inventory can hold, for the Inventory page.
//!
//! WFCD names the equipment, the parts and the common resources. The game's own export
//! (warframe-public-export-plus) names the rest - fish, gems, arcanes, ship decorations, glyphs,
//! blueprints of every kind - and gives each the path of its icon, which browse.wf serves. On the
//! user's inventory (1,346 stacks, 2026-11-09) the export named all but the relics, which the
//! catalog knows.

use std::collections::HashMap;

use serde_json::Value;

/// The export files read, beside `ExportRecipes.json` (already loaded for the foundry) and
/// `dict.en.json` (the names).
pub const EXPORTS: [&str; 12] = [
    "ExportResources.json",
    "ExportUpgrades.json",
    "ExportArcanes.json",
    "ExportFlavour.json",
    "ExportGear.json",
    "ExportKeys.json",
    "ExportWeapons.json",
    "ExportWarframes.json",
    "ExportSentinels.json",
    "ExportDrones.json",
    "ExportAvionics.json",
    "ExportRailjackWeapons.json",
];

/// Where browse.wf serves a game texture: this plus its path ("/Lotus/Interface/...").
const TEXTURES: &str = "https://browse.wf";

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// None for a blueprint whose result is named elsewhere (`result`).
    pub name: Option<String>,
    /// A picture URL.
    pub icon: Option<String>,
    pub kind: &'static str,
    /// What a blueprint builds.
    pub result: Option<String>,
    /// The top rank of a mod or arcane (`fusionLimit`).
    pub rank: Option<i64>,
    /// "Common" ... "Legendary", for mods and arcanes.
    pub rarity: Option<String>,
    /// Left out of the codex (`excludeFromCodex`): internal entries such as the "Unfused
    /// Artifact" placeholders and the helper copies of Arcane Steadfast. Nothing to farm.
    pub hidden: bool,
}

fn icon_url(path: Option<&str>) -> Option<String> {
    path.filter(|p| p.starts_with("/Lotus/Interface/")).map(|p| format!("{TEXTURES}{p}"))
}

/// The kind an export entry is filed under on the page, from its file and its path.
/// The arcanes are all in their own file. The `/CosmeticEnhancers/` entries in the mod file are
/// the Peculiar mods (checked 2026-11-09), which rank like any mod.
fn kind_of(file: &str, uid: &str, entry: &Value) -> &'static str {
    match file {
        "ExportUpgrades.json" => "Mods",
        "ExportArcanes.json" => "Arcanes",
        "ExportWeapons.json" if uid.contains("/OperatorAmplifiers/") => "Amps",
        "ExportWeapons.json" => "Weapons",
        "ExportWarframes.json" => "Warframes",
        "ExportSentinels.json" => "Companions",
        "ExportDrones.json" => "Gear",
        "ExportAvionics.json" | "ExportRailjackWeapons.json" => "Railjack",
        "ExportFlavour.json" if uid.contains("/AvatarImages/") => "Glyphs",
        "ExportFlavour.json" => "Cosmetics",
        "ExportGear.json" => "Gear",
        "ExportKeys.json" => "Quests and keys",
        _ if uid.contains("/Fish/") => "Fish",
        _ if uid.contains("/Gems/") || uid.contains("/Ores/") => "Gems and ores",
        _ if entry["productCategory"].as_str() == Some("ShipDecorations") => "Decorations",
        _ => "Resources",
    }
}

/// The kind of something no file names, from its path.
pub fn kind_by_path(uid: &str) -> &'static str {
    if uid.contains("/Upgrades/Mods/") {
        "Mods"
    } else if uid.contains("/Powersuits/") || uid.contains("/Weapons/") {
        "Weapons"
    } else if ["/Resources/", "/MiscItems/", "/Research/"].iter().any(|p| uid.contains(p)) {
        "Resources"
    } else {
        "Other"
    }
}

/// A name as the page shows it: the game's text markup ("<SHARD_BLUE_SIMPLE> Azure Archon
/// Shard") goes, and a name written all in capitals ("ADVANCES DEBT-BOND") gets ordinary case.
pub(crate) fn clean(name: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in name.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if out.chars().any(char::is_lowercase) {
        return out;
    }
    out.split(' ')
        .map(|w| {
            let mut cs = w.chars();
            cs.next().map_or(String::new(), |f| f.to_string() + &cs.as_str().to_lowercase())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// uid -> what the page shows for it. `exports` pairs each file name with its contents.
pub fn index(exports: &[(&str, &Value)], recipes: Option<&Value>, dict: &Value) -> HashMap<String, Entry> {
    let name_of = |key: &str| dict.get(key).and_then(Value::as_str).map(clean).filter(|n| !n.is_empty());
    let mut out = HashMap::new();
    for (file, data) in exports {
        for (uid, entry) in data.as_object().into_iter().flatten() {
            let Some(key) = entry["name"].as_str() else { continue };
            let kind = kind_of(file, uid, entry);
            let ranked = matches!(kind, "Mods" | "Arcanes");
            out.insert(
                uid.clone(),
                Entry {
                    name: name_of(key),
                    icon: icon_url(entry["icon"].as_str()),
                    kind,
                    result: None,
                    rank: entry["fusionLimit"].as_i64().filter(|_| ranked),
                    rarity: entry["rarity"].as_str().filter(|_| ranked).map(clean),
                    hidden: entry["excludeFromCodex"] == true,
                },
            );
        }
    }
    for (uid, recipe) in recipes.and_then(Value::as_object).into_iter().flatten() {
        let Some(result) = recipe["resultType"].as_str() else { continue };
        let built = out.get(result).cloned();
        out.entry(uid.clone()).or_insert(Entry {
            name: built.as_ref().and_then(|b| b.name.as_ref()).map(|n| format!("{n} Blueprint")),
            icon: built.and_then(|b| b.icon),
            kind: "Blueprints",
            result: Some(result.to_string()),
            rank: None,
            rarity: None,
            hidden: false,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn exports_give_names_icons_and_kinds() {
        let resources = json!({
            "/Lotus/Types/Items/MiscItems/Ferrite": {"name": "/Lang/Ferrite", "icon": "/Lotus/Interface/Icons/Ferrite.png"},
            "/Lotus/Types/Items/Fish/Eidolon/DayFishItem": {"name": "/Lang/Fish"},
            "/Lotus/Types/Items/ShipDecos/Lamp": {"name": "/Lang/Lamp", "productCategory": "ShipDecorations"},
        });
        let upgrades = json!({
            "/Lotus/Upgrades/Mods/Warframe/AvatarArmourMod": {"name": "/Lang/Fiber", "fusionLimit": 10, "rarity": "COMMON"},
            "/Lotus/Upgrades/CosmeticEnhancers/Peculiars/InflationMod": {"name": "/Lang/Growth", "fusionLimit": 3},
            "/Lotus/Weapons/CrewShip/CrewShipInnateDamageRandomMod": {"name": "/Lang/Unfused", "excludeFromCodex": true},
        });
        let arcanes = json!({"/Lotus/Upgrades/CosmeticEnhancers/Offensive/Arc": {"name": "/Lang/Arc", "fusionLimit": 5, "rarity": "LEGENDARY"}});
        let recipes =
            json!({"/Lotus/Types/Recipes/LampBlueprint": {"resultType": "/Lotus/Types/Items/ShipDecos/Lamp"}});
        let dict = json!({"/Lang/Ferrite": "Ferrite", "/Lang/Fish": "Mawfish", "/Lang/Lamp": "Lamp", "/Lang/Arc": "Arcane Energize",
                          "/Lang/Fiber": "Steel Fiber", "/Lang/Growth": "Peculiar Growth", "/Lang/Unfused": "Unfused Artifact"});
        let idx = index(
            &[
                ("ExportResources.json", &resources),
                ("ExportUpgrades.json", &upgrades),
                ("ExportArcanes.json", &arcanes),
            ],
            Some(&recipes),
            &dict,
        );
        let ferrite = &idx["/Lotus/Types/Items/MiscItems/Ferrite"];
        assert_eq!(ferrite.name.as_deref(), Some("Ferrite"));
        assert_eq!(ferrite.icon.as_deref(), Some("https://browse.wf/Lotus/Interface/Icons/Ferrite.png"));
        assert_eq!(ferrite.kind, "Resources");
        assert_eq!((ferrite.rank, ferrite.hidden), (None, false));
        assert_eq!(idx["/Lotus/Types/Items/Fish/Eidolon/DayFishItem"].kind, "Fish");
        assert_eq!(idx["/Lotus/Types/Items/ShipDecos/Lamp"].kind, "Decorations");
        let arcane = &idx["/Lotus/Upgrades/CosmeticEnhancers/Offensive/Arc"];
        assert_eq!((arcane.kind, arcane.rank, arcane.rarity.as_deref()), ("Arcanes", Some(5), Some("Legendary")));
        let fiber = &idx["/Lotus/Upgrades/Mods/Warframe/AvatarArmourMod"];
        assert_eq!((fiber.kind, fiber.rank, fiber.rarity.as_deref()), ("Mods", Some(10), Some("Common")));
        assert_eq!(
            idx["/Lotus/Upgrades/CosmeticEnhancers/Peculiars/InflationMod"].kind, "Mods",
            "a Peculiar mod is a mod"
        );
        assert!(idx["/Lotus/Weapons/CrewShip/CrewShipInnateDamageRandomMod"].hidden);
        let bp = &idx["/Lotus/Types/Recipes/LampBlueprint"];
        assert_eq!((bp.name.as_deref(), bp.kind), (Some("Lamp Blueprint"), "Blueprints"));
    }

    #[test]
    fn markup_and_capitals_are_cleaned() {
        assert_eq!(clean("<SHARD_BLUE_SIMPLE> Azure Archon Shard"), "Azure Archon Shard");
        assert_eq!(clean("ADVANCES DEBT-BOND"), "Advances Debt-bond");
        assert_eq!(clean("Amar's Anguish"), "Amar's Anguish");
    }
}
