//! The live world from the community worldstate API (api.warframestat.us): void fissures,
//! open-world cycles, Baro Ki'Teer, sortie, archon hunt, Steel Path, Nightwave and Darvo.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use crate::clock;
use crate::config::Platform;
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
        "error": err,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
