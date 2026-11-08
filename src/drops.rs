//! Searchable mission, bounty and enemy blueprint drops from WFCD's copy of DE's tables.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Drop {
    pub item: String,
    pub location: String,
    pub mode: Option<String>,
    pub rotation: Option<String>,
    pub stage: Option<String>,
    /// Percent in the named reward pool, or conditional on an enemy dropping a blueprint.
    pub chance: f64,
    pub rarity: Option<String>,
    pub enemy_drop_chance: Option<f64>,
    pub enemy_drop_kind: Option<&'static str>,
}

#[derive(Default)]
pub struct Drops {
    by_name: HashMap<String, Vec<Drop>>,
    pub available: usize,
    pub total: usize,
}

impl Drops {
    pub fn from_tables(tables: &[(&str, &Value)], total: usize) -> Self {
        let mut out = Drops { available: tables.len(), total, ..Drops::default() };
        for (name, table) in tables {
            if matches!(*name, "blueprintLocations" | "modLocations") {
                let mods = *name == "modLocations";
                for entry in table[*name].as_array().into_iter().flatten() {
                    let Some(item) = entry[if mods { "modName" } else { "itemName" }].as_str() else { continue };
                    for enemy in entry["enemies"].as_array().into_iter().flatten() {
                        let (Some(location), Some(chance), Some(drop)) = (
                            enemy["enemyName"].as_str(),
                            enemy["chance"].as_f64(),
                            enemy[if mods { "enemyModDropChance" } else { "enemyBlueprintDropChance" }].as_f64(),
                        ) else {
                            continue;
                        };
                        out.add(Drop {
                            item: item.to_string(),
                            location: location.to_string(),
                            mode: Some("Enemy".to_string()),
                            rotation: None,
                            stage: None,
                            chance,
                            rarity: enemy["rarity"].as_str().map(str::to_owned),
                            enemy_drop_chance: Some(drop),
                            enemy_drop_kind: Some(if mods { "mod" } else { "blueprint" }),
                        });
                    }
                }
            } else if let Some(data) = table.get(*name) {
                let path = if *name == "sortieRewards" { vec!["Sortie".to_string()] } else { Vec::new() };
                out.walk(data, &path, None, None);
            }
        }
        out
    }

    fn add(&mut self, drop: Drop) {
        let list = self.by_name.entry(drop.item.to_lowercase()).or_default();
        if !list.contains(&drop) {
            list.push(drop);
        }
    }

    fn walk(&mut self, value: &Value, path: &[String], mode: Option<&str>, rotation: Option<&str>) {
        match value {
            Value::Array(values) => {
                for value in values {
                    self.walk(value, path, mode, rotation);
                }
            }
            Value::Object(map) => {
                if let (Some(item), Some(chance)) = (value["itemName"].as_str(), value["chance"].as_f64()) {
                    if !path.is_empty() {
                        self.add(Drop {
                            item: item.to_string(),
                            location: path.join(" · "),
                            mode: mode.map(str::to_owned),
                            rotation: value["rotation"].as_str().or(rotation).map(str::to_owned),
                            stage: value["stage"].as_str().map(str::to_owned),
                            chance,
                            rarity: value["rarity"].as_str().map(str::to_owned),
                            enemy_drop_chance: None,
                            enemy_drop_kind: None,
                        });
                    }
                    return;
                }
                let mut next = path.to_vec();
                if let Some(label) = value["objectiveName"].as_str().or_else(|| value["bountyLevel"].as_str()) {
                    next.push(label.to_string());
                }
                let mode = value["gameMode"].as_str().or(mode);
                for (key, child) in map {
                    if !child.is_object() && !child.is_array() {
                        continue;
                    }
                    let is_rotation = matches!(key.as_str(), "A" | "B" | "C");
                    if is_rotation || key == "rewards" {
                        self.walk(child, &next, mode, if is_rotation { Some(key) } else { rotation });
                    } else {
                        let mut nested = next.clone();
                        nested.push(key.clone());
                        self.walk(child, &nested, mode, rotation);
                    }
                }
            }
            _ => {}
        }
    }

    pub fn for_name(&self, name: &str, include_blueprint: bool) -> Vec<&Drop> {
        let mut out: Vec<&Drop> = self.by_name.get(&name.to_lowercase()).into_iter().flatten().collect();
        if include_blueprint && !name.to_lowercase().ends_with(" blueprint") {
            out.extend(self.by_name.get(&format!("{} blueprint", name.to_lowercase())).into_iter().flatten());
        }
        out
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.by_name.values().filter_map(|drops| drops.first().map(|drop| drop.item.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn indexes_missions_bounties_and_enemy_blueprints_with_context() {
        let missions = json!({"missionRewards": {"Venus": {"Fossa": {"gameMode": "Assassination",
            "rewards": {"A": [{"itemName": "Rhino Chassis Blueprint", "chance": 38.72, "rarity": "Common"}]}}}}});
        let bounties = json!({"cetusBountyRewards": [{"bountyLevel": "Level 5 - 15 Cetus Bounty",
            "rewards": {"B": [{"itemName": "Gara Blueprint", "chance": 7.5, "stage": "Stage 2"}]}}]});
        let enemies = json!({"blueprintLocations": [{"itemName": "Stahlta Blueprint", "enemies": [
            {"enemyName": "Jackal", "enemyBlueprintDropChance": 30, "chance": 50}]}]});
        let mods = json!({"modLocations": [{"modName": "Vitality", "enemies": [
            {"enemyName": "Lancer", "enemyModDropChance": 3, "chance": 22}]}]});
        let drops = Drops::from_tables(
            &[
                ("missionRewards", &missions),
                ("cetusBountyRewards", &bounties),
                ("blueprintLocations", &enemies),
                ("modLocations", &mods),
            ],
            5,
        );
        assert_eq!((drops.available, drops.total), (4, 5));
        let rhino = drops.for_name("Rhino Chassis", true);
        assert_eq!(rhino[0].location, "Venus · Fossa");
        assert_eq!(rhino[0].mode.as_deref(), Some("Assassination"));
        assert_eq!(rhino[0].rotation.as_deref(), Some("A"));
        assert_eq!(rhino[0].chance, 38.72);
        assert!(drops.for_name("Rhino Chassis", false).is_empty());
        let gara = drops.for_name("Gara Blueprint", true);
        assert_eq!(gara[0].stage.as_deref(), Some("Stage 2"));
        assert_eq!(gara[0].rotation.as_deref(), Some("B"));
        let stahlta = drops.for_name("Stahlta Blueprint", false);
        assert_eq!(stahlta[0].enemy_drop_chance, Some(30.0));
        assert_eq!(stahlta[0].chance, 50.0);
        let vitality = drops.for_name("vitality", false);
        assert_eq!(vitality[0].enemy_drop_chance, Some(3.0));
        assert_eq!(vitality[0].enemy_drop_kind, Some("mod"));
        assert!(drops.names().any(|name| name == "Vitality"));
    }
}
