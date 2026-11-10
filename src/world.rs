//! The live world from the community worldstate API (api.warframestat.us): void fissures,
//! open-world cycles, Baro Ki'Teer, sortie, archon hunt, Steel Path, Nightwave, Darvo, the
//! Circuit and Archimedea; with what Duviri offers the account, from its log (`eelog.rs`).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::clock;
use crate::config::Platform;
use crate::eelog::DuviriOffers;
use crate::gamedata::{self, Catalog};
use crate::views::View;
use crate::{net, now};

const API: &str = "https://api.warframestat.us/{platform}?language=en";
const MAX_AGE: f64 = 60.0;

struct Cache {
    t: f64,
    platform: Option<Platform>,
    raw: Option<Arc<Value>>,
    error: Option<String>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { t: f64::NEG_INFINITY, platform: None, raw: None, error: None });

/// The whole worldstate, fetched at most once a minute. A failed fetch keeps the last good copy.
fn worldstate(platform: Platform) -> (Option<Arc<Value>>, Option<String>) {
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let fresh = now() - c.t < MAX_AGE && c.platform == Some(platform);
    if !fresh {
        match net::get_json(&API.replace("{platform}", platform.as_str()), &[], Duration::from_secs(20)) {
            Ok(raw) => {
                c.raw = Some(Arc::new(raw));
                c.error = None;
            }
            Err(e) => c.error = Some(format!("World state unavailable: {e}")),
        }
        c.t = now();
        c.platform = Some(platform);
    }
    (c.raw.clone(), c.error.clone())
}

/// The syndicate tag of the running Nightwave season, as the profile names it: the API spaces it
/// out ("Radio Legion Intermission16 Syndicate" for "RadioLegionIntermission16Syndicate").
pub fn nightwave_tag(platform: Platform) -> Option<String> {
    let (raw, _) = worldstate(platform);
    let tag: String = raw.as_deref()?["nightwave"]["tag"].as_str()?.chars().filter(|c| !c.is_whitespace()).collect();
    (!tag.is_empty()).then_some(tag)
}

/// When the running Duviri spiral began. Duviri's loadout picks are rolled anew with each one.
pub fn spiral_began(platform: Platform) -> Option<f64> {
    let (raw, _) = worldstate(platform);
    raw.as_deref()?["duviriCycle"]["activation"].as_str().and_then(clock::parse_iso)
}

/// The game's equipment slots in the order the page lists them.
const SLOTS: [(u8, &str); 4] = [(3, "Warframes"), (1, "Primary"), (0, "Secondary"), (5, "Melee")];

/// What Duviri offers the account (from its log), named and pictured, by slot. `stale` once a
/// spiral has begun since they were rolled: then the game has picked new ones.
pub fn duviri_picks(view: &View, offers: &DuviriOffers, spiral_began: Option<f64>) -> Value {
    let pick = |p: &crate::eelog::DuviriPick| {
        let item = view.cat.items.get(&p.uid);
        let mastered = item.is_some_and(|it| view.item_status(it).status == "mastered");
        json!({"uid": p.uid, "name": view.cat.name_of(&p.uid), "owned": p.owned, "mastered": mastered,
               "image": item.and_then(|it| gamedata::image_url(it.image.as_deref()))})
    };
    let mut groups: Vec<Value> = SLOTS
        .iter()
        .map(|(category, slot)| {
            let picks: Vec<Value> = offers.picks.iter().filter(|p| p.category == *category).map(pick).collect();
            json!({"slot": slot, "picks": picks})
        })
        .collect();
    let other: Vec<Value> =
        offers.picks.iter().filter(|p| SLOTS.iter().all(|(c, _)| *c != p.category)).map(pick).collect();
    groups.push(json!({"slot": "Other", "picks": other}));
    groups.retain(|g| g["picks"].as_array().is_some_and(|p| !p.is_empty()));
    let stale = offers.rolled.zip(spiral_began).is_some_and(|(rolled, began)| rolled < began);
    json!({"groups": groups, "rolled": offers.rolled, "stale": stale})
}

/// The fissures open right now, normal ones first, then by tier and expiry.
pub fn fissures(platform: Platform) -> (Vec<Value>, Option<String>) {
    let (raw, err) = worldstate(platform);
    let mut data: Vec<Value> = raw
        .as_deref()
        .and_then(|r| r["fissures"].as_array())
        .into_iter()
        .flatten()
        .filter(|f| f["expired"] != Value::Bool(true))
        .map(|f| {
            json!({"id": f["id"], "tier": f["tier"], "mission": f["missionType"], "node": f["node"],
                   "enemy": f["enemy"], "activation": f["activation"], "expiry": f["expiry"],
                   "storm": f["isStorm"] == Value::Bool(true), "hard": f["isHard"] == Value::Bool(true),
                   "tierNum": f["tierNum"]})
        })
        .collect();
    data.sort_by(|a, b| {
        let key = |f: &Value| {
            (
                f["storm"].as_bool(),
                f["hard"].as_bool(),
                f["tierNum"].as_i64().unwrap_or(9),
                f["expiry"].as_str().unwrap_or("").to_string(),
            )
        };
        key(a).cmp(&key(b))
    });
    (data, err)
}

/// The key a fissure is watched by: tier, mission and kind ("normal", "steel" or "storm").
pub fn watch_key(f: &Value) -> String {
    let kind = if f["storm"] == Value::Bool(true) {
        "storm"
    } else if f["hard"] == Value::Bool(true) {
        "steel"
    } else {
        "normal"
    };
    format!("{}|{}|{kind}", f["tier"].as_str().unwrap_or(""), f["mission"].as_str().unwrap_or(""))
}

/// Baro Ki'Teer: here now (activation <= now < expiry), or when he comes next.
pub fn baro(raw: &Value) -> Value {
    let vt = &raw["voidTrader"];
    let at = |k: &str| vt[k].as_str().and_then(clock::parse_iso);
    let active = matches!((at("activation"), at("expiry")), (Some(a), Some(e)) if a <= now() && now() < e);
    json!({
        "active": active, "location": vt["location"], "activation": vt["activation"], "expiry": vt["expiry"],
        "inventory": vt["inventory"].as_array().into_iter().flatten()
            .map(|i| json!({"item": i["item"], "ducats": i["ducats"], "credits": i["credits"]}))
            .collect::<Vec<_>>(),
    })
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

/// "Name: what it does" pairs from the worldstate as `{name, description}`. A placeholder the
/// game itself has not filled in ("[PH] Infected Wounds Desc", 2026-11-10) is no description.
fn described(v: &Value) -> Value {
    let description = v["description"].as_str().filter(|d| !d.trim().is_empty() && !d.starts_with("[PH]"));
    json!({"name": v["name"], "description": description})
}

/// A week of Deep or Temporal Archimedea: the three missions with their deviation and risk
/// variables (`elite`: only in Elite Archimedea), and the personal modifiers on offer. These are
/// the same for everyone. The loadout picks are not here: the game draws them for each player
/// from the week's seed and that player's own gear, and only tells DE how many were worn
/// (`ConquestEquipmentSuggestionsFulfilled`). Checked 2026-11-10: not in DE's world state or the
/// inventory, no other tool shows them, and a session's EE.log in which the Archimedea screens
/// were not opened has none.
fn archimedea(a: &Value) -> Value {
    // The type arrives as spaced-out internal text: "C T_ L A B" is the labs, "C T_ H E X" Hollvania.
    let key: String = a["typeKey"].as_str().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
    let name = match key.as_str() {
        "CT_LAB" => "Deep Archimedea",
        "CT_HEX" => "Temporal Archimedea",
        _ => "Archimedea",
    };
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    let missions: Vec<Value> = list(&a["missions"])
        .iter()
        .map(|m| {
            let risks: Vec<Value> = list(&m["risks"])
                .iter()
                .map(|r| {
                    let mut risk = described(r);
                    risk["elite"] = json!(r["isHard"] == true);
                    risk
                })
                .collect();
            json!({"type": m["missionType"], "faction": m["faction"], "deviation": described(&m["deviation"]), "risks": risks})
        })
        .collect();
    let modifiers: Vec<Value> = list(&a["personalModifiers"]).iter().map(described).collect();
    json!({"name": name, "expiry": a["expiry"], "missions": missions, "modifiers": modifiers})
}

/// Everything the world panel shows.
pub fn summary(platform: Platform) -> Value {
    let (raw, err) = worldstate(platform);
    let Some(raw) = raw else {
        return json!({"error": err});
    };
    let cycle = |key: &str, name: &str, place: &str| {
        let c = &raw[key];
        c["expiry"].as_str().map(|_| json!({"name": name, "place": place, "state": c["state"], "expiry": c["expiry"]}))
    };
    let cycles: Vec<Value> = [
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
    let sortie = &raw["sortie"];
    let archon = &raw["archonHunt"];
    let sp = &raw["steelPath"];
    let nw = &raw["nightwave"];
    let t = now();
    json!({
        "fetched": raw["timestamp"],
        "cycles": cycles,
        "baro": baro(&raw),
        "sortie": {"boss": sortie["boss"], "faction": sortie["faction"], "expiry": sortie["expiry"],
                   "missions": sortie["variants"].as_array().into_iter().flatten()
                       .map(|m| json!({"type": m["missionType"], "node": m["node"], "modifier": m["modifier"]}))
                       .collect::<Vec<_>>()},
        "archon": {"boss": archon["boss"], "expiry": archon["expiry"],
                   "missions": archon["missions"].as_array().into_iter().flatten()
                       .map(|m| json!({"type": m["type"], "node": m["node"]}))
                       .collect::<Vec<_>>()},
        "steelPath": {"reward": sp["currentReward"]["name"], "cost": sp["currentReward"]["cost"], "expiry": sp["expiry"]},
        "nightwave": {"season": nw["season"], "expiry": nw["expiry"],
                      "challenges": nw["activeChallenges"].as_array().into_iter().flatten()
                          .map(|c| json!({"title": c["title"], "desc": c["desc"], "reputation": c["reputation"],
                                          "daily": c["isDaily"], "elite": c["isElite"], "expiry": c["expiry"]}))
                          .collect::<Vec<_>>()},
        "darvo": raw["dailyDeals"].as_array().and_then(|d| d.first()).map(|d| json!({
            "item": d["item"], "price": d["salePrice"], "originalPrice": d["originalPrice"], "discount": d["discount"],
            "stock": d["total"], "sold": d["sold"], "expiry": d["expiry"]})),
        "resets": {"daily": clock::iso(clock::next_daily_reset(t)), "weekly": clock::iso(clock::next_weekly_reset(t))},
        "alerts": raw["alerts"].as_array().into_iter().flatten()
            .map(|a| json!({"type": a["mission"]["type"], "node": a["mission"]["node"], "faction": a["mission"]["faction"],
                            "what": a["mission"]["description"], "reward": reward_text(&a["mission"]["reward"]),
                            "expiry": a["expiry"]}))
            .collect::<Vec<_>>(),
        "invasions": raw["invasions"].as_array().into_iter().flatten().filter(|i| i["completed"] != true)
            .map(|i| json!({"node": i["node"], "what": i["desc"], "completion": i["completion"],
                            "attacker": i["attacker"]["faction"], "attackerReward": reward_text(&i["attacker"]["reward"]),
                            "defender": i["defender"]["faction"], "defenderReward": reward_text(&i["defender"]["reward"])}))
            .collect::<Vec<_>>(),
        // The open worlds' bounty boards rotate together; one timer per board.
        "bounties": raw["syndicateMissions"].as_array().into_iter().flatten()
            .filter(|s| s["jobs"].as_array().is_some_and(|j| !j.is_empty()))
            .map(|s| json!({"syndicate": s["syndicate"], "jobs": s["jobs"].as_array().map_or(0, Vec::len), "expiry": s["expiry"]}))
            .collect::<Vec<_>>(),
        "circuit": raw["duviriCycle"]["choices"].as_array().into_iter().flatten()
            .map(|c| json!({"steel": c["category"] == "hard",
                            "choices": c["choices"].as_array().into_iter().flatten().filter_map(Value::as_str).map(spaced).collect::<Vec<_>>()}))
            .collect::<Vec<_>>(),
        "archimedea": raw["archimedeas"].as_array().into_iter().flatten().map(archimedea).collect::<Vec<_>>(),
        "events": raw["events"].as_array().into_iter().flatten()
            .map(|e| json!({"name": e["description"], "node": e["node"], "expiry": e["expiry"],
                            "score": e["currentScore"], "goal": e["maximumScore"]}))
            .collect::<Vec<_>>(),
        "varzia": {"location": raw["vaultTrader"]["location"], "expiry": raw["vaultTrader"]["expiry"]},
        "error": err,
    })
}

/// Adds pictures to a summary for what the world state only names: Circuit picks, rewards,
/// Darvo's deal, Baro's stock. Changes `circuit[].choices` from names to `{name, image}` and
/// gives alerts a `rewards` list (`{name, count, image}`) next to the plain text.
pub fn add_art(w: &mut Value, cat: &Catalog) {
    // `w[key]` on a mutable value would create the key; these only look.
    fn list<'a>(w: &'a mut Value, key: &str) -> impl Iterator<Item = &'a mut Value> {
        w.get_mut(key).and_then(Value::as_array_mut).into_iter().flatten()
    }
    let art = |name: &Value| name.as_str().and_then(|n| cat.art_by_name(n));
    for group in list(w, "circuit") {
        let choices: Vec<Value> =
            group["choices"].as_array().into_iter().flatten().map(|n| json!({"name": n, "image": art(n)})).collect();
        group["choices"] = json!(choices);
    }
    for key in ["steelPath", "darvo"] {
        if w[key].is_object() {
            let name = if key == "darvo" { w[key]["item"].clone() } else { w[key]["reward"].clone() };
            w[key]["image"] = json!(art(&name));
        }
    }
    for item in w.get_mut("baro").map(|baro| list(baro, "inventory")).into_iter().flatten() {
        item["image"] = json!(art(&item["item"]));
    }
    for alert in list(w, "alerts") {
        // "Fieldron, Fieldron, 10000 credits" -> Fieldron x2, 10000 credits.
        let mut rewards: Vec<(String, i64)> = Vec::new();
        for name in alert["reward"].as_str().unwrap_or("").split(", ").filter(|n| !n.is_empty()) {
            match rewards.iter_mut().find(|(n, _)| n == name) {
                Some((_, count)) => *count += 1,
                None => rewards.push((name.to_string(), 1)),
            }
        }
        alert["rewards"] = rewards
            .iter()
            .map(|(name, count)| json!({"name": name, "count": count, "image": cat.art_by_name(name)}))
            .collect();
    }
    for inv in list(w, "invasions") {
        inv["attackerImage"] = json!(art(&inv["attackerReward"]));
        inv["defenderImage"] = json!(art(&inv["defenderReward"]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_are_added_by_name() {
        let cat = Catalog {
            art: [("soma", "soma.png"), ("fieldron", "f.png")].map(|(k, v)| (k.to_string(), v.to_string())).into(),
            ..Catalog::default()
        };
        let mut w = json!({
            "circuit": [{"steel": true, "choices": ["Soma", "Unknown Thing"]}],
            "darvo": {"item": "Soma"}, "steelPath": {"reward": "3x Fieldron"},
            "alerts": [{"reward": "Fieldron, Fieldron, 10000 credits"}],
            "invasions": [{"attackerReward": "", "defenderReward": "Soma Blueprint"}]});
        add_art(&mut w, &cat);
        let url = |v: &Value| v.as_str().map(|u| u.rsplit('/').next().unwrap_or("").to_string());
        assert_eq!(w["circuit"][0]["choices"][0]["name"], "Soma");
        assert_eq!(url(&w["circuit"][0]["choices"][0]["image"]).as_deref(), Some("soma.png"));
        assert_eq!(w["circuit"][0]["choices"][1]["image"], Value::Null);
        assert_eq!(url(&w["darvo"]["image"]).as_deref(), Some("soma.png"));
        assert_eq!(url(&w["steelPath"]["image"]).as_deref(), Some("f.png"));
        assert_eq!(
            w["alerts"][0]["rewards"][0],
            json!({"name": "Fieldron", "count": 2, "image": w["steelPath"]["image"]})
        );
        assert_eq!(w["alerts"][0]["rewards"][1]["name"], "10000 credits");
        assert_eq!(w["invasions"][0]["attackerImage"], Value::Null);
        assert_eq!(url(&w["invasions"][0]["defenderImage"]).as_deref(), Some("soma.png"));
        // A summary that is only an error stays as it is.
        let mut failed = json!({"error": "offline"});
        add_art(&mut failed, &cat);
        assert_eq!(failed, json!({"error": "offline"}));
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
        use std::sync::{Arc, RwLock};
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
            market: crate::market::Market::new(Arc::new(RwLock::new(crate::config::Settings::default()))),
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
        let f = |storm, hard| json!({"tier": "Axi", "mission": "Capture", "storm": storm, "hard": hard});
        assert_eq!(watch_key(&f(false, false)), "Axi|Capture|normal");
        assert_eq!(watch_key(&f(false, true)), "Axi|Capture|steel");
        assert_eq!(watch_key(&f(true, false)), "Axi|Capture|storm");
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
