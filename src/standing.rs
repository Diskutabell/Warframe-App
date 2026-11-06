//! Syndicate standing: each syndicate's ranks and what they cost, the daily limits, the
//! medallions held, and what standing buys on warframe.market.
//!
//! The syndicates come from the game's own export (warframe-public-export-plus
//! `ExportSyndicates.json`, English names from its `dict.en.json`); the account's standing from
//! DE's profile or the inventory (`sources::Standing`). Standing counts up over all ranks
//! (289,873 is rank 5, whose range the export gives as 240,000-372,000), and every
//! `DailyAffiliation*` pool reads 16,000 + 500 x MR when nothing was earned that day, 0 once
//! it is capped.

use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::gamedata;
use crate::sources::{Affiliation, Standing};
use crate::views::View;
use crate::{clock, now};

/// Where the game's interface pictures are served (see `art.rs`).
const GAME_FILES: &str = "https://browse.wf";
/// Counts for any syndicate: shown once, not under every syndicate.
const UNIVERSAL_MEDALLION: &str = "/Lotus/Types/Items/SyndicateDogTags/UniversalSyndicateDogTag";

/// The order syndicates are shown in: the six factions, then the open worlds and the rest roughly
/// in the order the game added them. A syndicate not listed here comes last.
const ORDER: [&str; 21] = [
    "SteelMeridianSyndicate",
    "ArbitersSyndicate",
    "CephalonSudaSyndicate",
    "PerrinSyndicate",
    "RedVeilSyndicate",
    "NewLokaSyndicate",
    "CetusSyndicate",
    "QuillsSyndicate",
    "SolarisSyndicate",
    "VoxSyndicate",
    "VentKidsSyndicate",
    "EntratiSyndicate",
    "NecraloidSyndicate",
    "ZarimanSyndicate",
    "EntratiLabSyndicate",
    "HexSyndicate",
    "KahlSyndicate",
    "NightcapJournalSyndicate",
    "ConclaveSyndicate",
    "LibrarySyndicate",
    "EventSyndicate",
];

/// Standing allowed per day in each pool: 16,000 + 500 per mastery rank (34,000 at MR 36).
pub fn daily_cap(mastery_rank: i64) -> i64 {
    16_000 + 500 * mastery_rank.max(0)
}

/// Focus allowed per day: 250,000 + 5,000 per mastery rank (430,000 at MR 36, 250,000 at MR 0).
pub fn daily_focus_cap(mastery_rank: i64) -> i64 {
    250_000 + 5_000 * mastery_rank.max(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Steel Meridian, Arbiters, Suda, Perrin, Red Veil, New Loka: one shared daily pool.
    Faction,
    /// A Nightwave season; only the current one matters.
    Nightwave,
    Other,
}

#[derive(Clone, Debug)]
pub struct Rank {
    pub level: i64,
    pub name: String,
    pub icon: Option<String>,
    /// The standing range of the rank, counted like the account's standing.
    pub min: i64,
    pub max: i64,
    /// What reaching this rank costs.
    pub credits: i64,
    pub items: Vec<(String, i64)>,
}

#[derive(Clone, Debug)]
pub struct Offer {
    /// The item's own uid (the export lists it as a store item).
    pub uid: String,
    pub standing: i64,
    /// The rank it needs.
    pub level: i64,
}

#[derive(Clone, Debug)]
pub struct Syndicate {
    pub tag: String,
    pub name: String,
    pub icon: Option<String>,
    /// "#rrggbb", the syndicate's own colour.
    pub colour: Option<String>,
    pub kind: Kind,
    /// The daily pool it counts against, by the name the profile gives it
    /// ("DailyAffiliationCetus"); `None` when its standing has no daily limit.
    pub pool: Option<String>,
    /// Lowest rank first. Rank 0 (neutral) is not listed; it lies between -1 and 1.
    pub ranks: Vec<Rank>,
    /// For factions: what standing gained here does to the others, as a share (-1 to 1).
    pub allies: Vec<(String, f64)>,
    pub offers: Vec<Offer>,
    /// Items handed in for standing, with the standing each gives.
    pub medallions: Vec<(String, i64)>,
    /// Whether those hand-ins count against the daily pool.
    pub medallions_capped: bool,
}

impl Syndicate {
    fn rank(&self, level: i64) -> Option<&Rank> {
        self.ranks.iter().find(|r| r.level == level)
    }

    /// Ranks counted in steps of one rather than standing (Kahl's Garrison, Nightcap).
    pub fn counts_steps(&self) -> bool {
        self.ranks.last().is_some_and(|r| r.max <= 10)
    }
}

#[derive(Default)]
pub struct Syndicates {
    pub list: IndexMap<String, Syndicate>,
}

/// "STANDING_LIMIT_BIN_CETUS" -> "DailyAffiliationCetus", the name of the same pool in the
/// profile; the shared faction pool is plain "DailyAffiliation".
fn pool_name(bin: &str) -> Option<String> {
    let rest = bin.strip_prefix("STANDING_LIMIT_BIN_")?;
    match rest {
        "NONE" => None,
        "NORMAL" => Some("DailyAffiliation".to_string()),
        _ => {
            let lower = rest.to_lowercase();
            let mut chars = lower.chars();
            let first = chars.next()?.to_ascii_uppercase();
            Some(format!("DailyAffiliation{first}{}", chars.as_str()))
        }
    }
}

/// The game writes some names in capitals ("OLD MATE"); the app shows them like the others.
fn tidy(name: &str) -> String {
    if name.chars().any(|c| c.is_lowercase()) {
        return name.to_string();
    }
    name.split(' ')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// "0x00F39579" -> "#f39579".
fn colour(v: &Value) -> Option<String> {
    let raw = v["value"].as_str()?.trim_start_matches("0x");
    let hex = raw.get(raw.len().checked_sub(6)?..)?;
    hex.chars().all(|c| c.is_ascii_hexdigit()).then(|| format!("#{}", hex.to_lowercase()))
}

/// The syndicates from the export, named from the dictionary.
pub fn build(export: &Value, dict: &Value) -> Syndicates {
    let text = |key: &Value| key.as_str().and_then(|k| dict[k].as_str()).map(tidy);
    let mut list = IndexMap::new();
    for (tag, s) in export.as_object().into_iter().flatten() {
        let kind = if tag.starts_with("RadioLegion") {
            Kind::Nightwave
        } else if s["dailyLimitBin"] == "STANDING_LIMIT_BIN_NORMAL" {
            Kind::Faction
        } else {
            Kind::Other
        };
        let mut ranks: Vec<Rank> = s["titles"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| Rank {
                level: t["level"].as_i64().unwrap_or(0),
                name: text(&t["name"]).unwrap_or_else(|| format!("Rank {}", t["level"])),
                icon: t["icon"].as_str().map(|p| format!("{GAME_FILES}{p}")),
                min: t["minStanding"].as_i64().unwrap_or(0),
                max: t["maxStanding"].as_i64().unwrap_or(0),
                credits: t["sacrifice"]["credits"].as_i64().unwrap_or(0),
                items: t["sacrifice"]["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|i| Some((i["ItemType"].as_str()?.to_string(), i["ItemCount"].as_i64().unwrap_or(1))))
                    .collect(),
            })
            .collect();
        ranks.sort_by_key(|r| r.level);
        let offers = s["favours"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                // "/Lotus/StoreItems/Upgrades/Mods/..." sells "/Lotus/Upgrades/Mods/...".
                let uid = f["storeItem"].as_str()?.replacen("/StoreItems", "", 1);
                Some(Offer {
                    uid,
                    standing: f["standingCost"].as_i64().unwrap_or(0),
                    level: f["requiredLevel"].as_i64().unwrap_or(0),
                })
            })
            .collect();
        let syndicate = Syndicate {
            tag: tag.clone(),
            name: text(&s["name"]).unwrap_or_else(|| gamedata::camel_name(tag).trim_end_matches(" Syndicate").to_string()),
            icon: s["icon"].as_str().map(|p| format!("{GAME_FILES}{p}")),
            colour: colour(&s["colour"]),
            kind,
            pool: s["dailyLimitBin"].as_str().and_then(pool_name),
            ranks,
            allies: s["alignments"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(other, share)| Some((other.clone(), share.as_f64()?)))
                .collect(),
            offers,
            medallions: s["medallions"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|m| Some((m["itemType"].as_str()?.to_string(), m["standing"].as_i64()?)))
                .collect(),
            medallions_capped: s["medallionsCappedByDailyLimit"] == true,
        };
        list.insert(tag.clone(), syndicate);
    }
    let place = |tag: &str| ORDER.iter().position(|t| *t == tag).unwrap_or(ORDER.len());
    list.sort_by(|a, _, b, _| place(a).cmp(&place(b)));
    Syndicates { list }
}

/// Where an account stands within its rank.
#[derive(Debug, PartialEq)]
pub struct Progress {
    /// Standing within the rank: from 0 up to `size`; below 0 for a negative rank, down to
    /// `-size`.
    pub value: i64,
    pub size: i64,
    /// The rank's standing is full and a higher rank exists: rank up to earn more.
    pub at_cap: bool,
    /// The top rank, full: anything earned now is lost unless standing is spent.
    pub full: bool,
}

/// Rank progress the way the game shows it: neutral counts from 0 to the first rank's start,
/// a positive rank from its start to its end, a negative one down from its upper end.
pub fn progress(s: &Syndicate, a: Affiliation) -> Progress {
    let top = s.ranks.iter().map(|r| r.level).max().unwrap_or(0);
    if s.counts_steps() {
        // Ranked by what the account has done: no standing to fill, spend or rank up with.
        return Progress { value: a.title.max(0), size: top, at_cap: false, full: false };
    }
    match s.rank(a.title) {
        Some(r) if a.title < 0 => Progress { value: a.standing - r.max, size: r.max - r.min, at_cap: false, full: false },
        Some(r) => {
            let full_rank = a.standing >= r.max;
            Progress {
                value: a.standing - r.min,
                size: r.max - r.min,
                at_cap: full_rank && a.title < top,
                full: full_rank && a.title >= top,
            }
        }
        None => {
            // Neutral (or a syndicate without ranks, like Simaris): up to the first rank.
            let first = s.rank(1).map(|r| r.min).filter(|f| *f > 0);
            Progress {
                value: a.standing,
                size: first.unwrap_or(0),
                at_cap: first.is_some_and(|f| a.standing >= f),
                full: false,
            }
        }
    }
}

/// The account's standing from its profile and inventory: each syndicate from the newer source
/// that has it, the daily pools from the newest source that has them.
pub struct Merged {
    pub syndicates: IndexMap<String, Affiliation>,
    /// Pools with when they were read.
    pub daily: Option<(IndexMap<String, i64>, f64)>,
    pub focus: Option<(i64, f64)>,
    /// When the newest source was read, and what it was.
    pub as_of: Option<(f64, &'static str)>,
}

pub fn merge(readings: &[(&Standing, f64, &'static str)]) -> Merged {
    let mut sorted: Vec<&(&Standing, f64, &str)> = readings.iter().collect();
    sorted.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut out = Merged { syndicates: IndexMap::new(), daily: None, focus: None, as_of: None };
    for (st, t, label) in sorted {
        for (tag, a) in &st.syndicates {
            out.syndicates.insert(tag.clone(), *a);
        }
        if !st.daily.is_empty() {
            out.daily = Some((st.daily.clone(), *t));
        }
        if let Some(f) = st.daily_focus {
            out.focus = Some((f, *t));
        }
        out.as_of = Some((*t, *label));
    }
    out
}

/// What is left of a daily allowance: as read, unless the read is from before the last reset,
/// when the whole allowance is back.
fn left_today(read: i64, read_at: f64, cap: Option<i64>) -> (Option<i64>, bool) {
    let last_reset = clock::next_daily_reset(now()) - 86_400.0;
    if read_at < last_reset {
        return (cap, false);
    }
    (Some(read), true)
}

/// The Standing page and the Overview card: every syndicate, the daily pools, and the medallions.
pub fn summary(view: &View, mastery_rank: Option<i64>, nightwave: Option<&str>) -> Value {
    let cat = &view.cat;
    let mut readings = Vec::new();
    if let Some(st) = view.profile.as_ref().and_then(|p| p.standing.as_ref().map(|s| (s, p.fetched))) {
        readings.push((st.0, st.1, "profile"));
    }
    if let Some(inv) = view.inv.as_deref() {
        if let Some(st) = &inv.standing {
            readings.push((st, inv.as_of(), "inventory"));
        }
    }
    let merged = merge(&readings);
    let counts = view.inv.as_ref().map(|i| &i.counts);
    let held = |uid: &str| counts.and_then(|c| c.get(uid)).copied().unwrap_or(0);
    let cap = mastery_rank.map(daily_cap);

    let mut rows = Vec::new();
    for s in cat.syndicates.list.values() {
        let mine = merged.syndicates.get(&s.tag).copied();
        let current_season = nightwave.is_some_and(|t| t == s.tag);
        match s.kind {
            Kind::Nightwave if !current_season => continue,
            // Events come and go: show one only while the account has standing there.
            Kind::Other if s.tag == "EventSyndicate" && mine.is_none() => continue,
            _ => {}
        }
        rows.push(syndicate_row(view, s, mine, &held));
    }

    // One row per daily pool, named after the syndicates in it.
    let mut pools: IndexMap<String, Vec<String>> = IndexMap::new();
    for s in cat.syndicates.list.values().filter(|s| s.kind != Kind::Nightwave) {
        if let Some(pool) = &s.pool {
            pools.entry(pool.clone()).or_default().push(s.name.clone());
        }
    }
    let pool_rows: Vec<Value> = pools
        .iter()
        .map(|(pool, names)| {
            let read = merged.daily.as_ref().and_then(|(d, t)| d.get(pool).map(|v| (*v, *t)));
            let (left, fresh) = match read {
                Some((v, t)) => left_today(v, t, cap),
                None => (cap, false),
            };
            let label = if pool == "DailyAffiliation" { "Faction syndicates".to_string() } else { names.join(", ") };
            json!({"key": pool, "name": label, "left": left, "cap": cap.map(|c| c.max(left.unwrap_or(0))), "read": fresh})
        })
        .collect();
    let focus_cap = mastery_rank.map(daily_focus_cap);
    let focus = merged.focus.map_or((focus_cap, false), |(v, t)| left_today(v, t, focus_cap));
    json!({
        "syndicates": rows,
        "pools": pool_rows,
        "focus": {"left": focus.0, "cap": focus_cap.map(|c| c.max(focus.0.unwrap_or(0))), "read": focus.1},
        "masteryRank": mastery_rank, "cap": cap,
        "reset": clock::iso(clock::next_daily_reset(now())),
        "asOf": merged.as_of.map(|(t, _)| t), "source": merged.as_of.map(|(_, s)| s),
        "universal": held(UNIVERSAL_MEDALLION),
        "loaded": !cat.syndicates.list.is_empty(),
    })
}

fn syndicate_row(view: &View, s: &Syndicate, mine: Option<Affiliation>, held: &dyn Fn(&str) -> i64) -> Value {
    let cat = &view.cat;
    let a = mine.unwrap_or_default();
    let p = progress(s, a);
    let rank = s.rank(a.title);
    let next = s.rank(a.title + 1).filter(|_| a.title >= 0).map(|r| {
        json!({"level": r.level, "name": r.name, "credits": r.credits,
               "items": r.items.iter().map(|(uid, n)| {
                   let name = cat.name_of(uid);
                   json!({"name": name, "count": n, "have": held(uid), "image": cat.art_by_name(&name)})
               }).collect::<Vec<_>>()})
    });
    let medal_items: Vec<Value> = s
        .medallions
        .iter()
        .filter(|(uid, _)| uid != UNIVERSAL_MEDALLION)
        .filter_map(|(uid, each)| {
            let n = held(uid);
            (n > 0).then(|| {
                let name = cat.name_of(uid);
                json!({"name": name, "count": n, "each": each, "image": cat.art_by_name(&name)})
            })
        })
        .collect();
    let medal_standing: i64 =
        medal_items.iter().map(|m| m["count"].as_i64().unwrap_or(0) * m["each"].as_i64().unwrap_or(0)).sum();
    let names = &cat.syndicates.list;
    json!({
        "tag": s.tag, "name": s.name, "icon": s.icon, "colour": s.colour,
        "kind": match s.kind { Kind::Faction => "faction", Kind::Nightwave => "nightwave", Kind::Other => "other" },
        "known": mine.is_some(), "steps": s.counts_steps(),
        "standing": a.standing, "title": a.title,
        "rankName": rank.map_or_else(|| if a.title == 0 { "Neutral".to_string() } else { format!("Rank {}", a.title) }, |r| r.name.clone()),
        "rankIcon": rank.and_then(|r| r.icon.clone()),
        "topRank": s.ranks.iter().map(|r| r.level).max(),
        "progress": p.value, "size": p.size, "atCap": p.at_cap, "full": p.full,
        "next": next, "pool": s.pool,
        "medallions": {"standing": medal_standing, "items": medal_items, "capped": s.medallions_capped},
        "allies": s.allies.iter().map(|(tag, share)| json!({"tag": tag, "share": share,
            "name": names.get(tag).map_or(tag.as_str(), |o| o.name.as_str())})).collect::<Vec<_>>(),
        "offers": s.offers.len(),
    })
}

/// One syndicate with its offerings, the ones the account can buy priced in platinum per 1,000
/// standing. Prices of those are asked for; the page polls while they arrive.
pub fn detail(view: &View, tag: &str, mastery_rank: Option<i64>) -> Option<Value> {
    let s = view.cat.syndicates.list.get(tag)?;
    let summary = summary(view, mastery_rank, Some(tag));
    let mut row = summary["syndicates"].as_array()?.iter().find(|r| r["tag"] == tag)?.clone();
    let title = row["title"].as_i64().unwrap_or(0);
    let mut waiting = 0;
    let mut offers: Vec<(f64, Value)> = s
        .offers
        .iter()
        .map(|o| {
            let slug = view.market.slug_for(Some(&o.uid), None, None);
            let available = o.level <= title;
            // Every tradable offering is priced, those of higher ranks too: they show what a
            // rank-up is worth. An item nobody sells has a price entry without a sell order, which
            // is an answer too.
            let entry = slug.as_deref().and_then(|sl| view.market.price(Some(sl), true));
            if slug.is_some() && entry.is_none() {
                waiting += 1;
            }
            let price = entry.and_then(|p| p.sell);
            let per_k = price.filter(|_| o.standing > 0).map(|p| p as f64 * 1000.0 / o.standing as f64);
            let name = slug
                .as_deref()
                .and_then(|sl| view.market.name(sl))
                .unwrap_or_else(|| view.cat.name_of(&o.uid));
            let image = view.market.image(slug.as_deref()).or_else(|| view.image_for(&o.uid));
            // Buyable now first, then the rest; within each, the most platinum per standing.
            let key = per_k.unwrap_or(-1.0) + if available { 1e9 } else { 0.0 };
            (
                key,
                json!({"uid": o.uid, "name": name, "image": image, "standing": o.standing, "level": o.level,
                       "available": available, "slug": slug, "price": price,
                       "perK": per_k.map(|v| crate::difflib::py_round(v, 2))}),
            )
        })
        .collect();
    offers.sort_by(|a, b| b.0.total_cmp(&a.0));
    row["offers"] = Value::Array(offers.into_iter().map(|(_, v)| v).collect());
    row["pricing"] = json!(waiting);
    row["ranks"] = s
        .ranks
        .iter()
        .map(|r| json!({"level": r.level, "name": r.name, "icon": r.icon, "min": r.min, "max": r.max}))
        .collect();
    row["pools"] = summary["pools"].clone();
    Some(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A syndicate as the export describes Steel Meridian, shortened.
    fn export() -> Value {
        json!({
            "SteelMeridianSyndicate": {
                "name": "/Lotus/Language/Syndicates/SteelMeridianName", "dailyLimitBin": "STANDING_LIMIT_BIN_NORMAL",
                "icon": "/Lotus/Interface/Icons/Syndicates/FactionSigilRebels.png", "colour": {"value": "0x00F39579"},
                "alignments": {"PerrinSyndicate": -1, "RedVeilSyndicate": 0.5},
                "medallionsCappedByDailyLimit": false,
                "medallions": [{"itemType": "/Lotus/Types/Items/SyndicateDogTags/SteelMeridianDogTag", "standing": 500},
                               {"itemType": UNIVERSAL_MEDALLION, "standing": 1000}],
                "favours": [{"storeItem": "/Lotus/StoreItems/Upgrades/Mods/Syndicate/Augment", "standingCost": 25000, "requiredLevel": 5}],
                "titles": [
                    {"level": 5, "name": "/T5", "minStanding": 240000, "maxStanding": 372000,
                     "sacrifice": {"items": [{"ItemCount": 3, "ItemType": "/Lotus/Types/Items/MiscItems/SchismKey"}], "credits": 500000}},
                    {"level": 4, "name": "/T4", "minStanding": 141000, "maxStanding": 240000},
                    {"level": 3, "name": "/T3", "minStanding": 71000, "maxStanding": 141000},
                    {"level": 2, "name": "/T2", "minStanding": 27000, "maxStanding": 71000},
                    {"level": 1, "name": "/T1", "minStanding": 5000, "maxStanding": 27000},
                    {"level": -1, "name": "/TN1", "minStanding": -27000, "maxStanding": -5000},
                    {"level": -2, "name": "/TN2", "minStanding": -71000, "maxStanding": -27000}]},
            "CetusSyndicate": {"name": "/Ostron", "dailyLimitBin": "STANDING_LIMIT_BIN_CETUS",
                "titles": [{"level": 1, "name": "/O1", "minStanding": 5000, "maxStanding": 27000}]},
            "KahlSyndicate": {"name": "/Kahl", "dailyLimitBin": "STANDING_LIMIT_BIN_KAHL",
                "titles": [{"level": 5, "minStanding": 4, "maxStanding": 5}, {"level": 1, "minStanding": 0, "maxStanding": 1}]},
            "RadioLegionIntermission16Syndicate": {"name": "/NW", "dailyLimitBin": "STANDING_LIMIT_BIN_NONE"},
        })
    }

    fn dict() -> Value {
        json!({"/Lotus/Language/Syndicates/SteelMeridianName": "Steel Meridian", "/T5": "General", "/T1": "Brave",
               "/TN2": "Enemy", "/Ostron": "Ostron", "/O1": "OFFWORLDER", "/Kahl": "Garrison", "/NW": "Nightwave"})
    }

    #[test]
    fn the_export_is_read_with_its_names() {
        let s = build(&export(), &dict());
        let tags: Vec<&str> = s.list.keys().map(String::as_str).collect();
        assert_eq!(tags, ["SteelMeridianSyndicate", "CetusSyndicate", "KahlSyndicate", "RadioLegionIntermission16Syndicate"]);
        let sm = &s.list["SteelMeridianSyndicate"];
        assert_eq!((sm.name.as_str(), sm.kind.clone(), sm.pool.as_deref()), ("Steel Meridian", Kind::Faction, Some("DailyAffiliation")));
        assert_eq!(sm.colour.as_deref(), Some("#f39579"));
        assert_eq!(sm.ranks.first().map(|r| r.level), Some(-2));
        assert_eq!(sm.rank(5).map(|r| (r.name.as_str(), r.credits, r.items.len())), Some(("General", 500_000, 1)));
        assert_eq!(sm.offers[0].uid, "/Lotus/Upgrades/Mods/Syndicate/Augment");
        assert_eq!(s.list["CetusSyndicate"].rank(1).unwrap().name, "Offworlder", "capitals are tidied");
        assert!(s.list["KahlSyndicate"].counts_steps());
        assert_eq!(s.list["RadioLegionIntermission16Syndicate"].kind, Kind::Nightwave);
    }

    #[test]
    fn pools_are_named_like_the_profile_names_them() {
        // Every bin of the export against the 14 pools a profile has.
        let pairs = [
            ("NORMAL", "DailyAffiliation"), ("PVP", "DailyAffiliationPvp"), ("LIBRARY", "DailyAffiliationLibrary"),
            ("CETUS", "DailyAffiliationCetus"), ("QUILLS", "DailyAffiliationQuills"), ("SOLARIS", "DailyAffiliationSolaris"),
            ("VENTKIDS", "DailyAffiliationVentkids"), ("VOX", "DailyAffiliationVox"), ("ENTRATI", "DailyAffiliationEntrati"),
            ("NECRALOID", "DailyAffiliationNecraloid"), ("ZARIMAN", "DailyAffiliationZariman"), ("KAHL", "DailyAffiliationKahl"),
            ("CAVIA", "DailyAffiliationCavia"), ("HEX", "DailyAffiliationHex"),
        ];
        for (bin, pool) in pairs {
            assert_eq!(pool_name(&format!("STANDING_LIMIT_BIN_{bin}")).as_deref(), Some(pool));
        }
        assert_eq!(pool_name("STANDING_LIMIT_BIN_NONE"), None);
        assert_eq!(daily_cap(36), 34_000);
        assert_eq!(daily_cap(0), 16_000);
        assert_eq!(daily_focus_cap(36), 430_000);
    }

    #[test]
    fn progress_within_a_rank_like_the_game_shows_it() {
        let s = build(&export(), &dict());
        let sm = &s.list["SteelMeridianSyndicate"];
        let at = |standing, title| progress(sm, Affiliation { standing, title });
        // 289,873 at rank 5 is 49,873 of 132,000.
        assert_eq!(at(289_873, 5), Progress { value: 49_873, size: 132_000, at_cap: false, full: false });
        assert_eq!(at(372_000, 5), Progress { value: 132_000, size: 132_000, at_cap: false, full: true });
        assert_eq!(at(27_000, 1), Progress { value: 22_000, size: 22_000, at_cap: true, full: false });
        assert_eq!(at(1_000, 0), Progress { value: 1_000, size: 5_000, at_cap: false, full: false });
        assert_eq!(at(-71_000, -2), Progress { value: -44_000, size: 44_000, at_cap: false, full: false });
        // Kahl's Garrison and Nightcap count ranks in steps: no standing bar, no rank-up to buy.
        let kahl = &s.list["KahlSyndicate"];
        assert_eq!(progress(kahl, Affiliation { standing: 5, title: 5 }), Progress { value: 5, size: 5, at_cap: false, full: false });
        assert_eq!(progress(kahl, Affiliation { standing: 0, title: 0 }), Progress { value: 0, size: 5, at_cap: false, full: false });
    }

    #[test]
    fn the_newer_source_wins() {
        let aff = |standing, title| Affiliation { standing, title };
        let profile = Standing {
            syndicates: IndexMap::from([("A".into(), aff(10, 1)), ("B".into(), aff(20, 1))]),
            daily: IndexMap::from([("DailyAffiliation".into(), 16_000)]),
            daily_focus: Some(250_000),
        };
        let inventory = Standing {
            syndicates: IndexMap::from([("A".into(), aff(15, 1))]),
            daily: IndexMap::new(),
            daily_focus: None,
        };
        let m = merge(&[(&inventory, 200.0, "inventory"), (&profile, 100.0, "profile")]);
        assert_eq!(m.syndicates["A"], aff(15, 1));
        assert_eq!(m.syndicates["B"], aff(20, 1));
        assert_eq!(m.daily.map(|(d, t)| (d["DailyAffiliation"], t)), Some((16_000, 100.0)));
        assert_eq!(m.as_of, Some((200.0, "inventory")));
        // A pool read before today's reset is full again.
        assert_eq!(left_today(0, 0.0, Some(16_000)), (Some(16_000), false));
        assert_eq!(left_today(0, now(), Some(16_000)), (Some(0), true));
    }
}
