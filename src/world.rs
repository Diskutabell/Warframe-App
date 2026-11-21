//! The live world from the community worldstate API (api.warframestat.us): void fissures,
//! open-world cycles, Baro Ki'Teer, sortie, archon hunt, Steel Path, Nightwave, Darvo, the
//! Circuit and Archimedea; with what Duviri offers the account, from its log (`eelog.rs`).

use serde::Serialize;
use serde_json::{Number, Value};
use ts_rs::TS;

use crate::clock;
use crate::eelog::DuviriOffers;
use crate::gamedata::{self, Catalog};
use crate::now;
use crate::views::View;

/// The worldstate of a platform: `{platform}` is `Platform::as_str`.
pub const API: &str = "https://api.warframestat.us/{platform}?language=en";

/// The syndicate tag of the running Nightwave season, as the profile names it: the API spaces it
/// out ("Radio Legion Intermission16 Syndicate" for "RadioLegionIntermission16Syndicate").
pub fn nightwave_tag(raw: &Value) -> Option<String> {
    let tag: String = raw["nightwave"]["tag"].as_str()?.chars().filter(|c| !c.is_whitespace()).collect();
    (!tag.is_empty()).then_some(tag)
}

/// When the running Duviri spiral began. Duviri's loadout picks are rolled anew with each one.
pub fn spiral_began(raw: &Value) -> Option<f64> {
    raw["duviriCycle"]["activation"].as_str().and_then(clock::parse_iso)
}

/// The game's equipment slots in the order the page lists them.
const SLOTS: [(u8, &str); 4] = [(3, "Warframes"), (1, "Primary"), (0, "Secondary"), (5, "Melee")];

/// What Duviri offers the account (from its log), named and pictured, by slot.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct DuviriPicks {
    pub groups: Vec<DuviriGroup>,
    pub rolled: Option<f64>,
    /// A spiral has begun since they were rolled: the game has picked new ones.
    pub stale: bool,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct DuviriGroup {
    pub slot: &'static str,
    pub picks: Vec<DuviriChoice>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct DuviriChoice {
    pub uid: String,
    pub name: String,
    pub owned: bool,
    pub mastered: bool,
    pub image: Option<String>,
}

pub fn duviri_picks(view: &View, offers: &DuviriOffers, spiral_began: Option<f64>) -> DuviriPicks {
    let pick = |p: &crate::eelog::DuviriPick| {
        let item = view.cat.items.get(&p.uid);
        DuviriChoice {
            uid: p.uid.clone(),
            name: view.cat.name_of(&p.uid),
            owned: p.owned,
            mastered: item.is_some_and(|it| view.item_status(it).status == "mastered"),
            image: item.and_then(|it| gamedata::image_url(it.image.as_deref())),
        }
    };
    let mut groups: Vec<DuviriGroup> = SLOTS
        .iter()
        .map(|(category, slot)| DuviriGroup {
            slot,
            picks: offers.picks.iter().filter(|p| p.category == *category).map(pick).collect(),
        })
        .collect();
    let other = offers.picks.iter().filter(|p| SLOTS.iter().all(|(c, _)| *c != p.category)).map(pick).collect();
    groups.push(DuviriGroup { slot: "Other", picks: other });
    groups.retain(|g| !g.picks.is_empty());
    let stale = offers.rolled.zip(spiral_began).is_some_and(|(rolled, began)| rolled < began);
    DuviriPicks { groups, rolled: offers.rolled, stale }
}

/// A string the worldstate gives, as it gives it.
fn text(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

/// A number the worldstate gives, exactly as it gives it (an integer stays one).
fn number(v: &Value) -> Option<Number> {
    v.as_number().cloned()
}

/// A void fissure open now.
#[derive(Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Fissure {
    pub id: Option<String>,
    pub tier: Option<String>,
    pub mission: Option<String>,
    pub node: Option<String>,
    pub enemy: Option<String>,
    pub activation: Option<String>,
    pub expiry: Option<String>,
    /// A Railjack void storm.
    pub storm: bool,
    /// On the Steel Path.
    pub hard: bool,
    #[ts(type = "number | null")]
    pub tier_num: Option<Number>,
}

/// The fissures open right now, normal ones first, then by tier and expiry.
pub fn fissures(raw: Option<&Value>) -> Vec<Fissure> {
    let t = now();
    let mut data: Vec<Fissure> = raw
        .and_then(|r| r["fissures"].as_array())
        .into_iter()
        .flatten()
        // The API's own flag, and the time: a copy kept while the API is unreachable (or answered
        // while a fresh one is fetched) still lists fissures that have closed since.
        .filter(|f| {
            f["expired"] != Value::Bool(true) && f["expiry"].as_str().and_then(clock::parse_iso).is_none_or(|e| e > t)
        })
        .map(|f| Fissure {
            id: text(&f["id"]),
            tier: text(&f["tier"]),
            mission: text(&f["missionType"]),
            node: text(&f["node"]),
            enemy: text(&f["enemy"]),
            activation: text(&f["activation"]),
            expiry: text(&f["expiry"]),
            storm: f["isStorm"] == Value::Bool(true),
            hard: f["isHard"] == Value::Bool(true),
            tier_num: number(&f["tierNum"]),
        })
        .collect();
    data.sort_by(|a, b| {
        let key = |f: &Fissure| {
            (
                f.storm,
                f.hard,
                f.tier_num.as_ref().and_then(Number::as_i64).unwrap_or(9),
                f.expiry.clone().unwrap_or_default(),
            )
        };
        key(a).cmp(&key(b))
    });
    data
}

/// The key a fissure is watched by: tier, mission and kind ("normal", "steel" or "storm").
pub fn watch_key(f: &Fissure) -> String {
    let kind = if f.storm {
        "storm"
    } else if f.hard {
        "steel"
    } else {
        "normal"
    };
    format!("{}|{}|{kind}", f.tier.as_deref().unwrap_or(""), f.mission.as_deref().unwrap_or(""))
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Baro {
    /// Here now (activation <= now < expiry).
    pub active: bool,
    pub location: Option<String>,
    pub activation: Option<String>,
    pub expiry: Option<String>,
    pub inventory: Vec<BaroItem>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct BaroItem {
    pub item: Option<String>,
    #[ts(type = "number | null")]
    pub ducats: Option<Number>,
    #[ts(type = "number | null")]
    pub credits: Option<Number>,
    pub image: Option<String>,
}

/// Baro Ki'Teer: here now, or when he comes next.
pub fn baro(raw: &Value, cat: &Catalog) -> Baro {
    let vt = &raw["voidTrader"];
    let at = |k: &str| vt[k].as_str().and_then(clock::parse_iso);
    let active = matches!((at("activation"), at("expiry")), (Some(a), Some(e)) if a <= now() && now() < e);
    Baro {
        active,
        location: text(&vt["location"]),
        activation: text(&vt["activation"]),
        expiry: text(&vt["expiry"]),
        inventory: vt["inventory"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|i| BaroItem {
                image: i["item"].as_str().and_then(|n| cat.art_by_name(n)),
                item: text(&i["item"]),
                ducats: number(&i["ducats"]),
                credits: number(&i["credits"]),
            })
            .collect(),
    }
}

/// "3x Forma, 10,000 credits" from a worldstate reward.
fn reward_text(r: &Value) -> String {
    let mut out: Vec<String> =
        r["items"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect();
    for c in r["countedItems"].as_array().into_iter().flatten() {
        let name = c["type"].as_str().unwrap_or("");
        match c["count"].as_i64() {
            Some(n) if n > 1 => out.push(format!("{n}x {name}")),
            _ => out.push(name.to_string()),
        }
    }
    if let Some(cr) = r["credits"].as_i64().filter(|&c| c > 0) {
        out.push(format!("{cr} credits"));
    }
    out.join(", ")
}

/// "AckAndBrunt" -> "Ack And Brunt": the Circuit lists its choices by internal name.
fn spaced(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if i > 0 && c.is_uppercase() {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// A name and what it does, from the worldstate. A placeholder the game itself has not filled
/// in ("[PH] Infected Wounds Desc", 2026-11-10) is no description.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Described {
    pub name: Option<String>,
    pub description: Option<String>,
}

fn described(v: &Value) -> Described {
    let description = v["description"].as_str().filter(|d| !d.trim().is_empty() && !d.starts_with("[PH]"));
    Described { name: text(&v["name"]), description: description.map(str::to_owned) }
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Archimedea {
    pub name: &'static str,
    pub expiry: Option<String>,
    pub missions: Vec<ArchimedeaMission>,
    pub modifiers: Vec<Described>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct ArchimedeaMission {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub faction: Option<String>,
    pub deviation: Described,
    pub risks: Vec<Risk>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Risk {
    #[serde(flatten)]
    pub described: Described,
    /// Only in Elite Archimedea.
    pub elite: bool,
}

/// A week of Deep or Temporal Archimedea: the three missions with their deviation and risk
/// variables (`elite`: only in Elite Archimedea), and the personal modifiers on offer. These are
/// the same for everyone. The loadout picks are not here: the game draws them for each player
/// from the week's seed and that player's own gear, and only tells DE how many were worn
/// (`ConquestEquipmentSuggestionsFulfilled`). Checked 2026-11-10: not in DE's world state or the
/// inventory, no other tool shows them, and a session's EE.log in which the Archimedea screens
/// were not opened has none.
fn archimedea(a: &Value) -> Archimedea {
    // The type arrives as spaced-out internal text: "C T_ L A B" is the labs, "C T_ H E X" Hollvania.
    let key: String = a["typeKey"].as_str().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
    let name = match key.as_str() {
        "CT_LAB" => "Deep Archimedea",
        "CT_HEX" => "Temporal Archimedea",
        _ => "Archimedea",
    };
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let missions = list(&a["missions"])
        .iter()
        .map(|m| ArchimedeaMission {
            kind: text(&m["missionType"]),
            faction: text(&m["faction"]),
            deviation: described(&m["deviation"]),
            risks: list(&m["risks"])
                .iter()
                .map(|r| Risk { described: described(r), elite: r["isHard"] == true })
                .collect(),
        })
        .collect();
    let modifiers = list(&a["personalModifiers"]).iter().map(described).collect();
    Archimedea { name, expiry: text(&a["expiry"]), missions, modifiers }
}

/// The World page: the worldstate, or why there is none, and what does not come with the
/// worldstate (filled in by the caller).
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct WorldView {
    #[serde(flatten)]
    pub state: WorldStateOrError,
    /// The schedule is known for years ahead (`arbitration.rs`).
    pub arbitration: crate::arbitration::Arbitrations,
    /// What Duviri offers the account shown, as the game last logged it.
    pub duviri: Option<DuviriPicks>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(untagged)]
pub enum WorldStateOrError {
    Live(Box<WorldState>),
    Unavailable { error: Option<String> },
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct WorldState {
    /// When the worldstate was made.
    pub fetched: Option<String>,
    pub cycles: Vec<Cycle>,
    pub baro: Baro,
    pub sortie: Sortie,
    pub archon: Archon,
    pub steel_path: SteelPath,
    pub nightwave: Nightwave,
    pub darvo: Option<Darvo>,
    pub resets: Resets,
    pub alerts: Vec<Alert>,
    pub invasions: Vec<Invasion>,
    pub bounties: Vec<Bounty>,
    pub circuit: Vec<CircuitGroup>,
    pub archimedea: Vec<Archimedea>,
    pub events: Vec<Event>,
    pub varzia: Varzia,
    pub error: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Cycle {
    pub name: &'static str,
    pub place: &'static str,
    pub state: Option<String>,
    pub expiry: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Sortie {
    pub boss: Option<String>,
    pub faction: Option<String>,
    pub expiry: Option<String>,
    pub missions: Vec<SortieMission>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct SortieMission {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub node: Option<String>,
    pub modifier: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Archon {
    pub boss: Option<String>,
    pub expiry: Option<String>,
    pub missions: Vec<ArchonMission>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct ArchonMission {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub node: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct SteelPath {
    pub reward: Option<String>,
    #[ts(type = "number | null")]
    pub cost: Option<Number>,
    pub expiry: Option<String>,
    pub image: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Nightwave {
    #[ts(type = "number | null")]
    pub season: Option<Number>,
    pub expiry: Option<String>,
    pub challenges: Vec<Challenge>,
    /// Where the account shown stands in the season.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub mine: Option<crate::standing::SyndicateRow>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Challenge {
    pub title: Option<String>,
    pub desc: Option<String>,
    #[ts(type = "number | null")]
    pub reputation: Option<Number>,
    pub daily: Option<bool>,
    pub elite: Option<bool>,
    pub expiry: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Darvo {
    pub item: Option<String>,
    #[ts(type = "number | null")]
    pub price: Option<Number>,
    #[ts(type = "number | null")]
    pub original_price: Option<Number>,
    #[ts(type = "number | null")]
    pub discount: Option<Number>,
    #[ts(type = "number | null")]
    pub stock: Option<Number>,
    #[ts(type = "number | null")]
    pub sold: Option<Number>,
    pub expiry: Option<String>,
    pub image: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Resets {
    pub daily: String,
    pub weekly: String,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Alert {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub node: Option<String>,
    pub faction: Option<String>,
    pub what: Option<String>,
    pub reward: String,
    /// The reward text counted up ("Fieldron, Fieldron" is Fieldron x2), with pictures.
    pub rewards: Vec<AlertReward>,
    pub expiry: Option<String>,
}

#[derive(Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct AlertReward {
    pub name: String,
    pub count: i64,
    pub image: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Invasion {
    pub node: Option<String>,
    pub what: Option<String>,
    #[ts(type = "number | null")]
    pub completion: Option<Number>,
    pub attacker: Option<String>,
    pub attacker_reward: String,
    pub attacker_image: Option<String>,
    pub defender: Option<String>,
    pub defender_reward: String,
    pub defender_image: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Bounty {
    pub syndicate: Option<String>,
    pub jobs: usize,
    pub expiry: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct CircuitGroup {
    /// The Steel Path's choices.
    pub steel: bool,
    pub choices: Vec<Named>,
}

#[derive(Debug, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Named {
    pub name: String,
    pub image: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Event {
    pub name: Option<String>,
    pub node: Option<String>,
    pub expiry: Option<String>,
    #[ts(type = "number | null")]
    pub score: Option<Number>,
    #[ts(type = "number | null")]
    pub goal: Option<Number>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct Varzia {
    pub location: Option<String>,
    pub expiry: Option<String>,
}

/// "Fieldron, Fieldron, 10000 credits" -> Fieldron x2, 10000 credits, with pictures.
fn counted_rewards(text: &str, cat: &Catalog) -> Vec<AlertReward> {
    let mut rewards: Vec<(String, i64)> = Vec::new();
    for name in text.split(", ").filter(|n| !n.is_empty()) {
        match rewards.iter_mut().find(|(n, _)| n == name) {
            Some((_, count)) => *count += 1,
            None => rewards.push((name.to_string(), 1)),
        }
    }
    rewards.into_iter().map(|(name, count)| AlertReward { image: cat.art_by_name(&name), name, count }).collect()
}

/// Everything the world panel shows, from the worldstate `raw` (None when there is none, with
/// `err` saying why), with pictures for what the world state only names (Circuit picks,
/// rewards, Darvo's deal, Baro's stock). Arbitration and Duviri are filled in by the caller.
pub fn summary(raw: Option<&Value>, err: Option<String>, cat: &Catalog) -> WorldView {
    WorldView { state: state(raw, err, cat), arbitration: crate::arbitration::summary(cat, now()), duviri: None }
}

fn state(raw: Option<&Value>, err: Option<String>, cat: &Catalog) -> WorldStateOrError {
    let Some(raw) = raw else {
        return WorldStateOrError::Unavailable { error: err };
    };
    let art = |name: &Option<String>| name.as_deref().and_then(|n| cat.art_by_name(n));
    let cycle = |key: &str, name: &'static str, place: &'static str| {
        let c = &raw[key];
        c["expiry"].as_str().map(|_| Cycle { name, place, state: text(&c["state"]), expiry: text(&c["expiry"]) })
    };
    let cycles: Vec<Cycle> = [
        cycle("cetusCycle", "Cetus", "Plains of Eidolon"),
        cycle("vallisCycle", "Orb Vallis", "Venus"),
        cycle("cambionCycle", "Cambion Drift", "Deimos"),
        cycle("zarimanCycle", "Zariman", "Faction"),
        cycle("duviriCycle", "Duviri", "Spiral"),
        cycle("earthCycle", "Earth", "Day and night"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let list = |v: &Value| -> Vec<Value> { v.as_array().cloned().unwrap_or_default() };
    let sortie = &raw["sortie"];
    let archon = &raw["archonHunt"];
    let sp = &raw["steelPath"];
    let nw = &raw["nightwave"];
    let t = now();
    let steel_reward = text(&sp["currentReward"]["name"]);
    WorldStateOrError::Live(Box::new(WorldState {
        fetched: text(&raw["timestamp"]),
        cycles,
        baro: baro(raw, cat),
        sortie: Sortie {
            boss: text(&sortie["boss"]),
            faction: text(&sortie["faction"]),
            expiry: text(&sortie["expiry"]),
            missions: list(&sortie["variants"])
                .iter()
                .map(|m| SortieMission {
                    kind: text(&m["missionType"]),
                    node: text(&m["node"]),
                    modifier: text(&m["modifier"]),
                })
                .collect(),
        },
        archon: Archon {
            boss: text(&archon["boss"]),
            expiry: text(&archon["expiry"]),
            missions: list(&archon["missions"])
                .iter()
                .map(|m| ArchonMission { kind: text(&m["type"]), node: text(&m["node"]) })
                .collect(),
        },
        steel_path: SteelPath {
            image: art(&steel_reward),
            reward: steel_reward,
            cost: number(&sp["currentReward"]["cost"]),
            expiry: text(&sp["expiry"]),
        },
        nightwave: Nightwave {
            season: number(&nw["season"]),
            expiry: text(&nw["expiry"]),
            challenges: list(&nw["activeChallenges"])
                .iter()
                .map(|c| Challenge {
                    title: text(&c["title"]),
                    desc: text(&c["desc"]),
                    reputation: number(&c["reputation"]),
                    daily: c["isDaily"].as_bool(),
                    elite: c["isElite"].as_bool(),
                    expiry: text(&c["expiry"]),
                })
                .collect(),
            mine: None,
        },
        darvo: raw["dailyDeals"].as_array().and_then(|d| d.first()).map(|d| {
            let item = text(&d["item"]);
            Darvo {
                image: art(&item),
                item,
                price: number(&d["salePrice"]),
                original_price: number(&d["originalPrice"]),
                discount: number(&d["discount"]),
                stock: number(&d["total"]),
                sold: number(&d["sold"]),
                expiry: text(&d["expiry"]),
            }
        }),
        resets: Resets {
            daily: clock::iso(clock::next_daily_reset(t)),
            weekly: clock::iso(clock::next_weekly_reset(t)),
        },
        alerts: list(&raw["alerts"])
            .iter()
            .map(|a| {
                let m = &a["mission"];
                let reward = reward_text(&m["reward"]);
                Alert {
                    kind: text(&m["type"]),
                    node: text(&m["node"]),
                    faction: text(&m["faction"]),
                    what: text(&m["description"]),
                    rewards: counted_rewards(&reward, cat),
                    reward,
                    expiry: text(&a["expiry"]),
                }
            })
            .collect(),
        invasions: list(&raw["invasions"])
            .iter()
            .filter(|i| i["completed"] != true)
            .map(|i| {
                let attacker_reward = reward_text(&i["attacker"]["reward"]);
                let defender_reward = reward_text(&i["defender"]["reward"]);
                Invasion {
                    node: text(&i["node"]),
                    what: text(&i["desc"]),
                    completion: number(&i["completion"]),
                    attacker: text(&i["attacker"]["faction"]),
                    attacker_image: cat.art_by_name(&attacker_reward),
                    attacker_reward,
                    defender: text(&i["defender"]["faction"]),
                    defender_image: cat.art_by_name(&defender_reward),
                    defender_reward,
                }
            })
            .collect(),
        // The open worlds' bounty boards rotate together; one timer per board.
        bounties: list(&raw["syndicateMissions"])
            .iter()
            .filter(|s| s["jobs"].as_array().is_some_and(|j| !j.is_empty()))
            .map(|s| Bounty {
                syndicate: text(&s["syndicate"]),
                jobs: s["jobs"].as_array().map_or(0, Vec::len),
                expiry: text(&s["expiry"]),
            })
            .collect(),
        circuit: list(&raw["duviriCycle"]["choices"])
            .iter()
            .map(|c| CircuitGroup {
                steel: c["category"] == "hard",
                choices: c["choices"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|n| {
                        let name = spaced(n);
                        Named { image: cat.art_by_name(&name), name }
                    })
                    .collect(),
            })
            .collect(),
        archimedea: list(&raw["archimedeas"]).iter().map(archimedea).collect(),
        events: list(&raw["events"])
            .iter()
            .map(|e| Event {
                name: text(&e["description"]),
                node: text(&e["node"]),
                expiry: text(&e["expiry"]),
                score: number(&e["currentScore"]),
                goal: number(&e["maximumScore"]),
            })
            .collect(),
        varzia: Varzia { location: text(&raw["vaultTrader"]["location"]), expiry: text(&raw["vaultTrader"]["expiry"]) },
        error: err,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// An answer as the page receives it.
    fn json<T: serde::Serialize>(v: T) -> Value {
        serde_json::to_value(v).unwrap()
    }

    fn archimedea(a: &Value) -> Value {
        json(super::archimedea(a))
    }

    fn duviri_picks(view: &View, offers: &DuviriOffers, spiral_began: Option<f64>) -> Value {
        json(super::duviri_picks(view, offers, spiral_began))
    }

    fn baro(raw: &Value) -> Value {
        json(super::baro(raw, &Catalog::default()))
    }

    #[test]
    fn pictures_are_added_by_name() {
        let cat = Catalog {
            art: [("soma", "soma.png"), ("fieldron", "f.png")].map(|(k, v)| (k.to_string(), v.to_string())).into(),
            ..Catalog::default()
        };
        let raw = json!({
            "duviriCycle": {"choices": [{"category": "hard", "choices": ["Soma", "UnknownThing"]}]},
            "dailyDeals": [{"item": "Soma"}], "steelPath": {"currentReward": {"name": "3x Fieldron"}},
            "alerts": [{"mission": {"reward": {"items": ["Fieldron", "Fieldron"], "credits": 10000}}}],
            "invasions": [{"attacker": {"reward": {}}, "defender": {"reward": {"items": ["Soma Blueprint"]}}}]});
        let w = json(summary(Some(&raw), None, &cat));
        let url = |v: &Value| v.as_str().map(|u| u.rsplit('/').next().unwrap_or("").to_string());
        assert_eq!(w["circuit"][0]["choices"][0]["name"], "Soma");
        assert_eq!(url(&w["circuit"][0]["choices"][0]["image"]).as_deref(), Some("soma.png"));
        assert_eq!(w["circuit"][0]["choices"][1], json!({"name": "Unknown Thing", "image": null}));
        assert_eq!(url(&w["darvo"]["image"]).as_deref(), Some("soma.png"));
        assert_eq!(url(&w["steelPath"]["image"]).as_deref(), Some("f.png"));
        assert_eq!(
            w["alerts"][0]["rewards"][0],
            json!({"name": "Fieldron", "count": 2, "image": w["steelPath"]["image"]})
        );
        assert_eq!(w["alerts"][0]["rewards"][1]["name"], "10000 credits");
        assert_eq!(w["invasions"][0]["attackerImage"], Value::Null);
        assert_eq!(url(&w["invasions"][0]["defenderImage"]).as_deref(), Some("soma.png"));
        // Without a worldstate, only the reason.
        let offline = json(summary(None, Some("offline".into()), &cat));
        assert_eq!(offline["error"], "offline");
        assert!(offline["arbitration"].is_object(), "the schedule needs no worldstate");
        assert!(offline.get("cycles").is_none());
    }

    #[test]
    fn rewards_and_circuit_names_read_naturally() {
        let r =
            json!({"items": ["Orokin Catalyst"], "countedItems": [{"type": "Fieldron", "count": 3}], "credits": 10000});
        assert_eq!(reward_text(&r), "Orokin Catalyst, 3x Fieldron, 10000 credits");
        assert_eq!(spaced("AckAndBrunt"), "Ack And Brunt");
    }

    #[test]
    fn duviri_picks_are_named_and_grouped_by_slot() {
        use crate::eelog::{DuviriOffers, DuviriPick};
        use std::collections::HashSet;
        use std::sync::Arc;
        let cat = gamedata::build(&indexmap::IndexMap::from([
            (
                "Warframes".to_string(),
                json!([{"uniqueName": "/Lotus/Powersuits/Paladin/Paladin", "name": "Oberon", "masterable": true, "imageName": "oberon.png"}]),
            ),
            ("Primary".to_string(), json!([{"uniqueName": "/W/GrnBurstRifle", "name": "Hind", "masterable": true}])),
        ]));
        let view = View {
            cat: Arc::new(cat),
            profile: None,
            inv: None,
            manual: Arc::new(crate::accounts::Manual::default()),
            market: crate::market::Market::for_tests(),
            foundry: HashSet::new(),
        };
        let pick = |uid: &str, category, owned| DuviriPick { uid: uid.into(), category, owned };
        let offers = DuviriOffers {
            picks: vec![
                pick("/W/GrnBurstRifle", 1, true),
                pick("/Lotus/Powersuits/Paladin/Paladin", 3, false),
                pick("/Lotus/Weapons/Tenno/Bows/DuelistBow/DuelistBow", 1, true),
            ],
            rolled: Some(100.0),
        };
        let d = duviri_picks(&view, &offers, Some(50.0));
        let slots: Vec<&str> = d["groups"].as_array().unwrap().iter().map(|g| g["slot"].as_str().unwrap()).collect();
        assert_eq!(slots, ["Warframes", "Primary"], "Warframes first, empty slots left out");
        let oberon = &d["groups"][0]["picks"][0];
        assert_eq!((oberon["name"].as_str(), oberon["owned"].as_bool()), (Some("Oberon"), Some(false)));
        assert!(oberon["image"].as_str().is_some_and(|u| u.ends_with("oberon.png")));
        assert_eq!(d["groups"][1]["picks"][0]["name"], "Hind");
        assert!(
            d["groups"][1]["picks"][1]["name"].as_str().is_some_and(|n| !n.is_empty()),
            "an unknown uid still gets a name"
        );
        assert_eq!(d["stale"], false);
        assert_eq!(duviri_picks(&view, &offers, Some(200.0))["stale"], true, "a spiral began after the roll");
        assert_eq!(duviri_picks(&view, &offers, None)["stale"], false);
    }

    #[test]
    fn archimedea_lists_missions_risks_and_modifiers() {
        // The shape api.warframestat.us served on 2026-11-10, shortened.
        let a = json!({"typeKey": "C T_ L A B", "expiry": "2026-11-16T00:00:00.000Z",
            "missions": [{"faction": "Man in the Wall", "missionType": "Extermination",
                "deviation": {"key": "GrowingIncursion", "name": "Fissure Cascade", "description": "Fissures rip into the mission."},
                "risks": [{"key": "ShieldedFoes", "name": "Bolstered Belligerents", "description": "Overguard.", "isHard": false},
                          {"key": "Quicksand", "name": "Entanglement", "description": "Slowed.", "isHard": true}]}],
            "personalModifiers": [{"key": "ShieldDelay", "name": "Lethargic Shields", "description": "Shield recharge delay increased 500%."}]});
        let w = archimedea(&a);
        assert_eq!(w["name"], "Deep Archimedea");
        assert_eq!(
            w["missions"][0]["deviation"],
            json!({"name": "Fissure Cascade", "description": "Fissures rip into the mission."})
        );
        let risks: Vec<(&str, bool)> = w["missions"][0]["risks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| (r["name"].as_str().unwrap(), r["elite"].as_bool().unwrap()))
            .collect();
        assert_eq!(risks, [("Bolstered Belligerents", false), ("Entanglement", true)]);
        assert_eq!(w["modifiers"][0]["name"], "Lethargic Shields");
        let placeholder =
            json!({"personalModifiers": [{"name": "Infected Wounds", "description": "[PH] Infected Wounds Desc"}]});
        assert_eq!(archimedea(&placeholder)["modifiers"][0], json!({"name": "Infected Wounds", "description": null}));
        assert_eq!(archimedea(&json!({"typeKey": "C T_ H E X"}))["name"], "Temporal Archimedea");
        assert_eq!(archimedea(&json!({}))["missions"], json!([]));
    }

    #[test]
    fn watch_keys_tell_the_kinds_apart() {
        let f = |storm, hard| Fissure {
            id: None,
            tier: Some("Axi".into()),
            mission: Some("Capture".into()),
            node: None,
            enemy: None,
            activation: None,
            expiry: None,
            storm,
            hard,
            tier_num: None,
        };
        assert_eq!(watch_key(&f(false, false)), "Axi|Capture|normal");
        assert_eq!(watch_key(&f(false, true)), "Axi|Capture|steel");
        assert_eq!(watch_key(&f(true, false)), "Axi|Capture|storm");
    }

    #[test]
    fn closed_fissures_are_gone_even_from_an_old_copy() {
        let raw = json!({"fissures": [
            {"id": "a", "tier": "Lith", "missionType": "Capture", "expiry": clock::iso(now() - 5.0)},
            {"id": "b", "tier": "Meso", "missionType": "Survival", "expiry": clock::iso(now() + 600.0)},
            {"id": "c", "tier": "Neo", "missionType": "Defense", "expiry": clock::iso(now() + 600.0), "expired": true},
        ]});
        let open = fissures(Some(&raw));
        let ids: Vec<&str> = open.iter().filter_map(|f| f.id.as_deref()).collect();
        assert_eq!(ids, ["b"]);
    }

    #[test]
    fn baro_is_here_between_activation_and_expiry() {
        let past = clock::iso(now() - 3600.0);
        let future = clock::iso(now() + 3600.0);
        let here = json!({"voidTrader": {"activation": past, "expiry": future, "location": "Strata Relay (Earth)"}});
        assert_eq!(baro(&here)["active"], true);
        let later = json!({"voidTrader": {"activation": future, "expiry": future}});
        assert_eq!(baro(&later)["active"], false);
    }
}
