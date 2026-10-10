//! The live world from the community worldstate API (api.warframestat.us): void fissures,
//! open-world cycles, Baro Ki'Teer, sortie, archon hunt, Steel Path, Nightwave and Darvo.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::clock;
use crate::config::Platform;
use crate::gamedata::Catalog;
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
        // The type arrives as spaced-out internal text: "C T_ L A B" is the labs, "C T_ H E X" Hollvania.
        "archimedea": raw["archimedeas"].as_array().into_iter().flatten()
            .map(|a| {
                let key: String = a["typeKey"].as_str().unwrap_or("").chars().filter(|c| !c.is_whitespace()).collect();
                let name = match key.as_str() { "CT_LAB" => "Deep Archimedea", "CT_HEX" => "Temporal Archimedea", _ => "Archimedea" };
                json!({"name": name, "expiry": a["expiry"],
                       "missions": a["missions"].as_array().into_iter().flatten()
                           .map(|m| json!({"type": m["missionType"], "faction": m["faction"], "deviation": m["deviation"]["name"]}))
                           .collect::<Vec<_>>()})
            })
            .collect::<Vec<_>>(),
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
        assert_eq!(w["alerts"][0]["rewards"][0], json!({"name": "Fieldron", "count": 2, "image": w["steelPath"]["image"]}));
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
