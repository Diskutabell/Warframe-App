//! The arbitration schedule: which node hosts the arbitration in each hour.
//!
//! The world state no longer says which arbitration runs (warframestat answers "SolNode000",
//! 2026-11-10), but the rotation is fixed long in advance. browse.wf publishes it one hour per
//! line for years ahead (`arbys.txt`: "1794268800,SolNode211") and grades the nodes with the
//! Arbitration Goons' tiers (arbi.guide) in `arbyTiers.js`; its own schedule page reads the same
//! two files. Names, planets, mission types and factions come from the star chart export.

use std::collections::HashMap;

use anyhow::anyhow;
use serde::Serialize;
use ts_rs::TS;

use crate::gamedata::{self, Catalog};
use crate::{config, now};

const SCHEDULE_URL: &str = "https://browse.wf/arbys.txt";
const TIERS_URL: &str = "https://browse.wf/supplemental-data/arbyTiers.js";
/// Each arbitration runs for one hour.
const HOUR: i64 = 3600;
/// How far ahead the World page lists them.
const AHEAD: usize = 24;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Schedule {
    /// When each hour starts (epoch seconds) and its node tag, in time order.
    pub slots: Vec<(i64, String)>,
    /// Node tag -> "S", "A", "B", "C" or "D"; a node the list leaves out is F tier, as on
    /// browse.wf. Empty when the list could not be loaded: then no tier is shown at all.
    pub tiers: HashMap<String, String>,
}

impl Schedule {
    /// Only the hours that have not ended yet: the file runs from 2024.
    pub fn current(mut self) -> Schedule {
        let t = now() as i64;
        self.slots.retain(|(start, _)| start + HOUR > t);
        self
    }
}

/// The "<start>,<node>" lines of `arbys.txt`, in time order. Other lines are skipped.
pub fn parse_schedule(text: &str) -> Vec<(i64, String)> {
    let mut slots: Vec<(i64, String)> = text
        .lines()
        .filter_map(|line| {
            let (start, node) = line.trim().split_once(',')?;
            let start: i64 = start.trim().parse().ok()?;
            let node = node.trim();
            let tag_like = !node.is_empty() && node.chars().all(|c| c.is_ascii_alphanumeric());
            tag_like.then(|| (start, node.to_string()))
        })
        .collect();
    slots.sort_by_key(|(start, _)| *start);
    slots
}

/// The tier list in `arbyTiers.js`: one `SolNode450: "S",` line per node inside an object
/// literal. Everything after the object (a comment with more notes) is left alone.
pub fn parse_tiers(js: &str) -> HashMap<String, String> {
    let object = js.split("};").next().unwrap_or("");
    object
        .lines()
        .filter_map(|line| {
            let (node, tier) = line.trim().trim_end_matches(',').split_once(':')?;
            let (node, tier) = (node.trim(), tier.trim().trim_matches('"'));
            let ok = !node.is_empty()
                && node.chars().all(|c| c.is_ascii_alphanumeric())
                && matches!(tier, "S" | "A" | "B" | "C" | "D" | "F");
            ok.then(|| (node.to_string(), tier.to_string()))
        })
        .collect()
}

/// Both files, revalidated once a day like the game data (`gamedata::download_file`). Without
/// them the World page simply has no arbitration card.
pub async fn download(force: bool) -> Schedule {
    let cache = config::dirs().cache.clone();
    let read_schedule: gamedata::Reader<Vec<(i64, String)>> = std::sync::Arc::new(|bytes: &[u8]| {
        let slots = parse_schedule(&String::from_utf8_lossy(bytes));
        if slots.is_empty() { Err(anyhow!("no arbitration in the schedule")) } else { Ok(slots) }
    });
    let read_tiers: gamedata::Reader<HashMap<String, String>> = std::sync::Arc::new(|bytes: &[u8]| {
        let tiers = parse_tiers(&String::from_utf8_lossy(bytes));
        if tiers.is_empty() { Err(anyhow!("no tier in the tier list")) } else { Ok(tiers) }
    });
    let (schedule_path, tiers_path) = (cache.join("arbys.txt"), cache.join("arbyTiers.js"));
    let (slots, tiers) = tokio::join!(
        gamedata::download_file(SCHEDULE_URL, &schedule_path, force, read_schedule),
        gamedata::download_file(TIERS_URL, &tiers_path, force, read_tiers),
    );
    let mut slots = slots.unwrap_or_else(|e| {
        eprintln!("arbitration schedule unavailable: {e:#}");
        Vec::new()
    });
    // The file starts in 2024; only what has not ended yet is of use.
    let t = now() as i64;
    slots.retain(|(start, _)| start + HOUR > t);
    let tiers = tiers.unwrap_or_else(|e| {
        eprintln!("arbitration tiers unavailable: {e:#}");
        HashMap::new()
    });
    Schedule { slots, tiers }
}

impl Schedule {
    /// The slot running at `t`, by index.
    fn running(&self, t: i64) -> Option<usize> {
        // The first slot that starts after `t`; the one before it runs now, if it has not ended.
        let after = self.slots.partition_point(|(start, _)| *start <= t);
        let i = after.checked_sub(1)?;
        (t < self.slots[i].0 + HOUR).then_some(i)
    }

    fn tier(&self, tag: &str) -> Option<&str> {
        if self.tiers.is_empty() {
            return None;
        }
        Some(self.tiers.get(tag).map_or("F", String::as_str))
    }
}

/// One hour of the schedule as the page shows it.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct Slot {
    pub start: i64,
    pub end: i64,
    pub tag: String,
    pub node: String,
    pub planet: String,
    pub mission: String,
    pub faction: String,
    /// A Dark Sector's extra resources, in percent.
    pub bonus: Option<i64>,
    /// "S" to "F"; None when the tier list could not be loaded.
    pub tier: Option<String>,
}

/// The World page's arbitration panel.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct Arbitrations {
    pub current: Option<Slot>,
    pub upcoming: Vec<Slot>,
    /// When each of the S, A and B tiers comes up next.
    pub next: Vec<Slot>,
    /// Whether tiers are known.
    pub tiers: bool,
}

fn slot(cat: &Catalog, start: i64, tag: &str) -> Slot {
    let node = cat.nodes.get(tag).cloned().unwrap_or_default();
    // A node the star chart export does not know (yet) keeps its tag as its name.
    let name = if node.name.is_empty() { tag.to_string() } else { node.name };
    Slot {
        start,
        end: start + HOUR,
        tag: tag.to_string(),
        node: name,
        planet: node.planet,
        mission: node.mission,
        faction: node.faction,
        bonus: node.resource_bonus,
        tier: cat.arbitrations.tier(tag).map(str::to_owned),
    }
}

/// What the World page shows at time `t`: the arbitration running now, the next `AHEAD` hours,
/// and when each of the S, A and B tiers comes up next (after the one running).
pub fn summary(cat: &Catalog, t: f64) -> Arbitrations {
    let s = &cat.arbitrations;
    let t = t as i64;
    let Some(now) = s.running(t) else {
        return Arbitrations { current: None, upcoming: Vec::new(), next: Vec::new(), tiers: !s.tiers.is_empty() };
    };
    let row = |i: usize| slot(cat, s.slots[i].0, &s.slots[i].1);
    let later = now + 1..s.slots.len();
    let upcoming: Vec<Slot> = later.clone().take(AHEAD).map(row).collect();
    let mut next = Vec::new();
    if !s.tiers.is_empty() {
        for tier in ["S", "A", "B"] {
            if let Some(i) = later.clone().find(|i| s.tier(&s.slots[*i].1) == Some(tier)) {
                next.push(row(i));
            }
        }
    }
    Arbitrations { current: Some(row(now)), upcoming, next, tiers: !s.tiers.is_empty() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// The answer as the page receives it.
    fn summary(cat: &Catalog, t: f64) -> Value {
        serde_json::to_value(super::summary(cat, t)).unwrap()
    }
    use crate::gamedata::Node;

    #[test]
    fn the_schedule_file_reads_line_by_line() {
        let text =
            "1727888400,ClanNode10\n1727884800,SettlementNode11\r\n\nnot a line\n17,../etc\n1727892000,SolNode64";
        let slots = parse_schedule(text);
        let tags: Vec<&str> = slots.iter().map(|(_, tag)| tag.as_str()).collect();
        assert_eq!(tags, ["SettlementNode11", "ClanNode10", "SolNode64"], "sorted by time, junk skipped");
        assert_eq!(slots[0].0, 1727884800);
    }

    #[test]
    fn the_tier_list_reads_from_its_script() {
        let js = "// Credit to the Arbitration Goons.\n\nwindow.arbyTiers = {\n\tSolNode450: \"S\",\n\tSolNode149: \"B\",\n\
                  \tBad Node: \"S\",\n\tSolNode1: \"Z\",\n};\n\n/* Recommended number of Cyte-09s:\n{\n\t\"Casta\": 0,\n\tSolNode9: \"A\"\n}\n*/\n";
        let tiers = parse_tiers(js);
        assert_eq!(tiers.len(), 2, "{tiers:?}");
        assert_eq!(tiers["SolNode450"], "S");
        assert_eq!(tiers["SolNode149"], "B");
    }

    fn catalog(slots: &[(i64, &str)], tiers: &[(&str, &str)]) -> Catalog {
        let node = |name: &str, mission: &str, faction: &str| Node {
            name: name.into(),
            planet: "Ceres".into(),
            mission: mission.into(),
            faction: faction.into(),
            resource_bonus: None,
        };
        Catalog {
            nodes: HashMap::from([
                ("SolNode149".to_string(), node("Casta", "Defense", "Grineer")),
                ("SolNode147".to_string(), node("Cinxia", "Interception", "Grineer")),
            ]),
            arbitrations: Schedule {
                slots: slots.iter().map(|(t, tag)| (*t, tag.to_string())).collect(),
                tiers: tiers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            },
            ..Catalog::default()
        }
    }

    #[test]
    fn the_hour_running_and_the_next_good_ones() {
        let h = 1_800_000_000;
        let cat = catalog(
            &[(h, "SolNode1"), (h + HOUR, "SolNode149"), (h + 2 * HOUR, "SolNode147"), (h + 3 * HOUR, "SolNode149")],
            &[("SolNode149", "S"), ("SolNode147", "A")],
        );
        let s = summary(&cat, (h + 10) as f64);
        assert_eq!(s["current"]["tag"], "SolNode1");
        assert_eq!(s["current"]["node"], "SolNode1", "an unknown node keeps its tag");
        assert_eq!(s["current"]["tier"], "F", "a node the list leaves out is F");
        assert_eq!((s["current"]["start"].as_i64(), s["current"]["end"].as_i64()), (Some(h), Some(h + HOUR)));
        let upcoming: Vec<&str> =
            s["upcoming"].as_array().unwrap().iter().map(|r| r["node"].as_str().unwrap()).collect();
        assert_eq!(upcoming, ["Casta", "Cinxia", "Casta"]);
        assert_eq!(s["upcoming"][0]["mission"], "Defense");
        // The next S is Casta at h + 1h, the next A Cinxia; no B comes up.
        let next: Vec<(&str, i64)> = s["next"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["tier"].as_str().unwrap(), r["start"].as_i64().unwrap()))
            .collect();
        assert_eq!(next, [("S", h + HOUR), ("A", h + 2 * HOUR)]);
        // At the last hour nothing is left to come; after it nothing runs.
        assert_eq!(summary(&cat, (h + 3 * HOUR + 59) as f64)["upcoming"], json!([]));
        assert_eq!(summary(&cat, (h + 4 * HOUR) as f64)["current"], Value::Null);
        assert_eq!(summary(&cat, (h - 1) as f64)["current"], Value::Null, "before the first hour");
    }

    #[test]
    fn without_a_tier_list_no_tier_is_claimed() {
        let h = 1_800_000_000;
        let cat = catalog(&[(h, "SolNode149"), (h + HOUR, "SolNode147")], &[]);
        let s = summary(&cat, h as f64);
        assert_eq!(s["current"]["tier"], Value::Null);
        assert_eq!(s["next"], json!([]));
        assert_eq!(s["tiers"], false);
    }
}
