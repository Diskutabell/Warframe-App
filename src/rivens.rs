//! Riven maths: base values, disposition, the roll window and letter grades.
//!
//! value = base x disposition x multiplier, and a roll lands anywhere in 90%..110% of that.
//! The multipliers per (positives, negatives) and the grade bands follow wiki.warframe.com's Riven
//! Mods page.
//!
//! A grade only says how the roll compares with an average roll - never whether the stats are
//! any good on the weapon.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

use crate::config::{self, Platform};
use crate::difflib::py_round;
use crate::gamedata::{Catalog, Item};
use crate::net;
use crate::now;

/// How a stat's number is shown on the card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    Percent,
    Metres,
    Seconds,
    Count,
    /// Faction damage. The base values here are percentages (45 means x1.45); the card may show
    /// either form, and the OCR reader converts multipliers to percentages.
    Multiplier,
}

impl Unit {
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Percent | Unit::Multiplier => "%",
            Unit::Metres => "m",
            Unit::Seconds => "s",
            Unit::Count => "",
        }
    }
}

/// Which way a stat can roll.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Either,
    PositiveOnly,
    NegativeOnly,
}

pub struct Stat {
    pub name: &'static str,
    pub unit: Unit,
    /// Base value per weapon class: rifle, shotgun, pistol, archgun, melee.
    pub base: [Option<f64>; 5],
    pub sign: Sign,
    /// Only reachable through the Riven Splicer.
    pub spliced: bool,
}

/// Every riven attribute and its base value, from wiki.warframe.com's Riven Mods page ("Base
/// Values" and "Spliced Values", both taken from DE's Public Export ExportUpgrades_en.json).
#[rustfmt::skip]
pub static STATS: &[Stat] = &[
    // The 32 regular attributes.
    Stat { name: "Damage", unit: Unit::Percent, base: [Some(165.0), Some(164.7), Some(219.6), Some(99.9), Some(164.7)], sign: Sign::PositiveOnly, spliced: false },
    Stat { name: "Multishot", unit: Unit::Percent, base: [Some(90.0), Some(119.7), Some(119.7), Some(60.3), None], sign: Sign::Either, spliced: false },
    Stat { name: "Critical Chance", unit: Unit::Percent, base: [Some(149.99), Some(90.0), Some(149.99), Some(99.9), Some(180.0)], sign: Sign::Either, spliced: false },
    Stat { name: "Critical Damage", unit: Unit::Percent, base: [Some(119.997), Some(90.0), Some(90.0), Some(80.1), Some(90.0)], sign: Sign::Either, spliced: false },
    Stat { name: "Fire Rate / Attack Speed", unit: Unit::Percent, base: [Some(60.03), Some(90.0), Some(74.7), Some(60.03), Some(54.9)], sign: Sign::Either, spliced: false },
    Stat { name: "Status Chance", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(60.3), Some(90.0)], sign: Sign::Either, spliced: false },
    Stat { name: "Status Duration", unit: Unit::Percent, base: [Some(99.99), Some(99.99), Some(99.99), Some(99.99), Some(99.99)], sign: Sign::Either, spliced: false },
    Stat { name: "Magazine Capacity", unit: Unit::Percent, base: [Some(49.995), Some(49.995), Some(49.995), Some(60.3), None], sign: Sign::Either, spliced: false },
    Stat { name: "Ammo Maximum", unit: Unit::Percent, base: [Some(49.95), Some(90.0), Some(90.0), Some(99.9), None], sign: Sign::Either, spliced: false },
    Stat { name: "Reload Speed", unit: Unit::Percent, base: [Some(49.995), Some(49.995), Some(49.995), Some(99.9), None], sign: Sign::Either, spliced: false },
    Stat { name: "Punch Through", unit: Unit::Metres, base: [Some(2.7), Some(2.7), Some(2.7), Some(2.7), None], sign: Sign::PositiveOnly, spliced: false },
    Stat { name: "Projectile Speed", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), None, None], sign: Sign::Either, spliced: false },
    Stat { name: "Recoil", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(90.0), None], sign: Sign::Either, spliced: false },
    Stat { name: "Zoom", unit: Unit::Percent, base: [Some(59.99), Some(41.994), Some(80.1), Some(59.99), None], sign: Sign::Either, spliced: false },
    Stat { name: "Heat Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(119.7), Some(90.0)], sign: Sign::PositiveOnly, spliced: false },
    Stat { name: "Cold Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(119.7), Some(90.0)], sign: Sign::PositiveOnly, spliced: false },
    Stat { name: "Electricity Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(119.7), Some(90.0)], sign: Sign::PositiveOnly, spliced: false },
    Stat { name: "Toxin Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(119.7), Some(90.0)], sign: Sign::PositiveOnly, spliced: false },
    Stat { name: "Impact Damage", unit: Unit::Percent, base: [Some(119.97), Some(119.97), Some(119.97), Some(90.0), Some(119.7)], sign: Sign::Either, spliced: false },
    Stat { name: "Puncture Damage", unit: Unit::Percent, base: [Some(119.997), Some(119.997), Some(119.997), Some(90.0), Some(119.997)], sign: Sign::Either, spliced: false },
    Stat { name: "Slash Damage", unit: Unit::Percent, base: [Some(119.97), Some(119.97), Some(119.97), Some(90.0), Some(119.7)], sign: Sign::Either, spliced: false },
    Stat { name: "Damage to Grineer", unit: Unit::Multiplier, base: [Some(45.0), Some(45.0), Some(45.0), Some(45.0), Some(45.0)], sign: Sign::Either, spliced: false },
    Stat { name: "Damage to Corpus", unit: Unit::Multiplier, base: [Some(45.0), Some(45.0), Some(45.0), Some(45.0), Some(45.0)], sign: Sign::Either, spliced: false },
    Stat { name: "Damage to Infested", unit: Unit::Multiplier, base: [Some(45.0), Some(45.0), Some(45.0), Some(45.0), Some(45.0)], sign: Sign::Either, spliced: false },
    Stat { name: "Range", unit: Unit::Metres, base: [None, None, None, None, Some(1.94)], sign: Sign::Either, spliced: false },
    Stat { name: "Combo Duration", unit: Unit::Seconds, base: [None, None, None, None, Some(8.1)], sign: Sign::Either, spliced: false },
    Stat { name: "Finisher Damage", unit: Unit::Percent, base: [None, None, None, None, Some(119.7)], sign: Sign::Either, spliced: false },
    Stat { name: "Heavy Attack Efficiency", unit: Unit::Percent, base: [None, None, None, None, Some(73.44)], sign: Sign::Either, spliced: false },
    Stat { name: "Initial Combo", unit: Unit::Count, base: [None, None, None, None, Some(24.5)], sign: Sign::Either, spliced: false },
    Stat { name: "Slide Critical Chance", unit: Unit::Percent, base: [None, None, None, None, Some(120.0)], sign: Sign::Either, spliced: false },
    Stat { name: "Additional Combo Count Chance", unit: Unit::Percent, base: [None, None, None, None, Some(58.77)], sign: Sign::PositiveOnly, spliced: false },
    Stat { name: "Chance to Gain Combo Count", unit: Unit::Percent, base: [None, None, None, None, Some(104.85)], sign: Sign::NegativeOnly, spliced: false },
    // The 18 spliced attributes (Riven Splicer).
    Stat { name: "Ammo Efficiency", unit: Unit::Percent, base: [Some(45.0), Some(45.0), Some(45.0), Some(45.0), None], sign: Sign::Either, spliced: true },
    Stat { name: "Blast Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(90.0), Some(90.0)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Corrosive Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(90.0), Some(90.0)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Damage to Orokin", unit: Unit::Multiplier, base: [Some(45.0), Some(45.0), Some(45.0), Some(45.0), Some(45.0)], sign: Sign::Either, spliced: true },
    Stat { name: "Damage to Scaldra", unit: Unit::Multiplier, base: [Some(45.0), Some(45.0), Some(45.0), Some(45.0), Some(45.0)], sign: Sign::Either, spliced: true },
    Stat { name: "Damage to Techrot", unit: Unit::Multiplier, base: [Some(45.0), Some(45.0), Some(45.0), Some(45.0), Some(45.0)], sign: Sign::Either, spliced: true },
    Stat { name: "Gas Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(90.0), Some(90.0)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Heavy Attack Wind Up Speed", unit: Unit::Percent, base: [None, None, None, None, Some(119.7)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Magnetic Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(90.0), Some(90.0)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Melee Damage On Heavy Attack", unit: Unit::Percent, base: [None, None, None, None, Some(119.7)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Parry Angle", unit: Unit::Percent, base: [None, None, None, None, Some(81.0)], sign: Sign::Either, spliced: true },
    Stat { name: "Radiation Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(90.0), Some(90.0)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Reload While Holstered", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), None, None], sign: Sign::Either, spliced: true },
    Stat { name: "Slam Attack Damage", unit: Unit::Percent, base: [None, None, None, None, Some(119.7)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Status Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(90.0), Some(90.0)], sign: Sign::Either, spliced: true },
    Stat { name: "Viral Damage", unit: Unit::Percent, base: [Some(90.0), Some(90.0), Some(90.0), Some(90.0), Some(90.0)], sign: Sign::PositiveOnly, spliced: true },
    Stat { name: "Weakpoint Damage", unit: Unit::Percent, base: [Some(225.0), Some(225.0), Some(225.0), Some(225.0), None], sign: Sign::Either, spliced: true },
    Stat { name: "Weakpoint Critical Chance", unit: Unit::Percent, base: [Some(247.5), Some(247.5), Some(247.5), Some(247.5), None], sign: Sign::Either, spliced: true },
];

/// How the card words each stat, lower-case and letters only, as the OCR reader sees it. Several
/// wordings per stat: older cards, wrapped lines and the wiki's own names all occur.
#[rustfmt::skip]
pub static WORDING: &[(&str, &str)] = &[
    ("damage", "Damage"), ("melee damage", "Damage"), ("multishot", "Multishot"),
    ("critical chance", "Critical Chance"), ("critical damage", "Critical Damage"),
    ("critical chance for slide attack", "Slide Critical Chance"),
    ("critical chance on slide attack", "Slide Critical Chance"),
    ("slide critical chance", "Slide Critical Chance"),
    ("fire rate", "Fire Rate / Attack Speed"), ("attack speed", "Fire Rate / Attack Speed"),
    ("status chance", "Status Chance"), ("status duration", "Status Duration"),
    ("magazine capacity", "Magazine Capacity"), ("ammo maximum", "Ammo Maximum"),
    ("reload speed", "Reload Speed"), ("punch through", "Punch Through"),
    ("projectile speed", "Projectile Speed"), ("projectile flight speed", "Projectile Speed"),
    ("flight speed", "Projectile Speed"),
    ("recoil", "Recoil"), ("weapon recoil", "Recoil"), ("zoom", "Zoom"),
    ("heat", "Heat Damage"), ("cold", "Cold Damage"), ("electricity", "Electricity Damage"),
    ("toxin", "Toxin Damage"), ("impact", "Impact Damage"), ("puncture", "Puncture Damage"),
    ("slash", "Slash Damage"),
    ("damage to grineer", "Damage to Grineer"), ("damage to corpus", "Damage to Corpus"),
    ("damage to infested", "Damage to Infested"), ("damage vs grineer", "Damage to Grineer"),
    ("damage vs corpus", "Damage to Corpus"), ("damage vs infested", "Damage to Infested"),
    ("range", "Range"), ("melee range", "Range"), ("combo duration", "Combo Duration"),
    ("finisher damage", "Finisher Damage"), ("initial combo", "Initial Combo"),
    ("heavy attack efficiency", "Heavy Attack Efficiency"),
    ("channeling efficiency", "Heavy Attack Efficiency"),
    ("additional combo count chance", "Additional Combo Count Chance"),
    ("chance to gain combo count", "Chance to Gain Combo Count"),
    ("chance to gain extra combo count", "Chance to Gain Combo Count"),
    // Spliced attributes.
    ("ammo efficiency", "Ammo Efficiency"), ("blast", "Blast Damage"), ("corrosive", "Corrosive Damage"),
    ("gas", "Gas Damage"), ("magnetic", "Magnetic Damage"), ("radiation", "Radiation Damage"),
    ("viral", "Viral Damage"),
    ("damage to orokin", "Damage to Orokin"), ("damage to scaldra", "Damage to Scaldra"),
    ("damage to techrot", "Damage to Techrot"), ("damage vs orokin", "Damage to Orokin"),
    ("damage vs scaldra", "Damage to Scaldra"), ("damage vs techrot", "Damage to Techrot"),
    ("heavy attack wind up speed", "Heavy Attack Wind Up Speed"),
    ("melee damage on heavy attack", "Melee Damage On Heavy Attack"),
    ("parry angle", "Parry Angle"), ("reload while holstered", "Reload While Holstered"),
    ("slam attack damage", "Slam Attack Damage"), ("status damage", "Status Damage"),
    ("weakpoint damage", "Weakpoint Damage"), ("weak point damage", "Weakpoint Damage"),
    ("weakpoint critical chance", "Weakpoint Critical Chance"),
    ("weak point critical chance", "Weakpoint Critical Chance"),
];

pub fn stat(name: &str) -> Option<&'static Stat> {
    STATS.iter().find(|s| s.name.eq_ignore_ascii_case(name))
}

pub const VARIANCE: f64 = 0.10;

/// warframe.market's slug for a stat (its `/v2/riven/attributes`, checked 2026-11-09: the same 50
/// attributes as `STATS`).
pub fn market_slug(stat: &str) -> Option<&'static str> {
    Some(match stat {
        "Damage" => "base_damage_/_melee_damage",
        "Multishot" => "multishot",
        "Critical Chance" => "critical_chance",
        "Critical Damage" => "critical_damage",
        "Fire Rate / Attack Speed" => "fire_rate_/_attack_speed",
        "Status Chance" => "status_chance",
        "Status Duration" => "status_duration",
        "Magazine Capacity" => "magazine_capacity",
        "Ammo Maximum" => "ammo_maximum",
        "Reload Speed" => "reload_speed",
        "Punch Through" => "punch_through",
        "Projectile Speed" => "projectile_speed",
        "Recoil" => "recoil",
        "Zoom" => "zoom",
        "Heat Damage" => "heat_damage",
        "Cold Damage" => "cold_damage",
        "Electricity Damage" => "electric_damage",
        "Toxin Damage" => "toxin_damage",
        "Impact Damage" => "impact_damage",
        "Puncture Damage" => "puncture_damage",
        "Slash Damage" => "slash_damage",
        "Damage to Grineer" => "damage_vs_grineer",
        "Damage to Corpus" => "damage_vs_corpus",
        "Damage to Infested" => "damage_vs_infested",
        "Range" => "range",
        "Combo Duration" => "combo_duration",
        "Finisher Damage" => "finisher_damage",
        "Heavy Attack Efficiency" => "channeling_efficiency",
        "Initial Combo" => "channeling_damage",
        "Slide Critical Chance" => "critical_chance_on_slide_attack",
        "Additional Combo Count Chance" => "chance_to_gain_extra_combo_count",
        "Chance to Gain Combo Count" => "chance_to_gain_combo_count",
        "Ammo Efficiency" => "ammo_efficiency",
        "Blast Damage" => "blast",
        "Corrosive Damage" => "corrosive",
        "Damage to Orokin" => "damage_to_orokin",
        "Damage to Scaldra" => "damage_to_scaldra",
        "Damage to Techrot" => "damage_to_techrot",
        "Gas Damage" => "gas",
        "Heavy Attack Wind Up Speed" => "heavy_attack_wind_up_speed",
        "Magnetic Damage" => "magnetic",
        "Melee Damage On Heavy Attack" => "melee_damage_on_heavy_attack",
        "Parry Angle" => "parry_angle",
        "Radiation Damage" => "radiation",
        "Reload While Holstered" => "magazine_reloaded_s_when_holstered",
        "Slam Attack Damage" => "slam_attack_damage",
        "Status Damage" => "status_damage",
        "Viral Damage" => "viral",
        "Weakpoint Damage" => "weak_point_damage",
        "Weakpoint Critical Chance" => "weak_point_critical_chance",
        _ => return None,
    })
}

/// (bonus, malus) per (positives, negatives).
pub fn multipliers(positives: u8, negatives: u8) -> Option<(f64, f64)> {
    match (positives, negatives) {
        (2, 0) => Some((0.99, 0.0)),
        (2, 1) => Some((1.2375, -0.495)),
        (3, 0) => Some((0.75, 0.0)),
        (3, 1) => Some((0.9375, -0.75)),
        _ => None,
    }
}

/// 2.5-point bands across the +-10% roll window.
pub fn grade_for(deviation_pct: f64) -> &'static str {
    const BANDS: [(f64, &str); 7] =
        [(7.5, "A+"), (5.0, "A"), (2.5, "A-"), (0.0, "B+"), (-2.5, "B"), (-5.0, "B-"), (-7.5, "C")];
    BANDS.iter().find(|(lo, _)| deviation_pct >= *lo).map_or("C-", |(_, g)| g)
}

fn class_index(item: &Item) -> usize {
    match item.riven_class.as_str() {
        "shotgun" => 1,
        "pistol" => 2,
        "archgun" => 3,
        "melee" => 4,
        _ => 0,
    }
}

#[derive(Serialize)]
pub struct StatRow {
    pub stat: &'static str,
    pub unit: &'static str,
    pub min: f64,
    pub centre: f64,
    pub max: f64,
    pub spliced: bool,
    #[serde(rename = "negMin", skip_serializing_if = "Option::is_none")]
    pub neg_min: Option<f64>,
    #[serde(rename = "negCentre", skip_serializing_if = "Option::is_none")]
    pub neg_centre: Option<f64>,
    #[serde(rename = "negMax", skip_serializing_if = "Option::is_none")]
    pub neg_max: Option<f64>,
}

/// Every stat this weapon can roll, with its min / centre / max value.
pub fn stat_table(item: &Item, positives: u8, negatives: u8) -> Vec<StatRow> {
    let (Some(dispo), Some((bonus, malus))) = (item.disposition, multipliers(positives, negatives)) else {
        return Vec::new();
    };
    let ix = class_index(item);
    STATS
        .iter()
        .filter_map(|s| {
            let base = s.base[ix]?;
            let centre = base * dispo * bonus;
            let neg = (malus != 0.0).then(|| base * dispo * malus.abs());
            Some(StatRow {
                stat: s.name,
                unit: s.unit.suffix(),
                min: py_round(centre * (1.0 - VARIANCE), 2),
                centre: py_round(centre, 2),
                max: py_round(centre * (1.0 + VARIANCE), 2),
                spliced: s.spliced,
                neg_min: neg.map(|c| py_round(c * (1.0 - VARIANCE), 2)),
                neg_centre: neg.map(|c| py_round(c, 2)),
                neg_max: neg.map(|c| py_round(c * (1.0 + VARIANCE), 2)),
            })
        })
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct Graded {
    pub stat: &'static str,
    pub value: f64,
    pub centre: f64,
    pub deviation: f64,
    pub grade: &'static str,
    pub negative: bool,
    /// A real roll always lands within +-10% of the centre. Outside that, the stat count or the
    /// weapon is wrong - worth saying so rather than printing a confident C-.
    #[serde(rename = "outOfRange")]
    pub out_of_range: bool,
    /// The stat cannot roll with this sign at all, so the sign was misread.
    #[serde(rename = "signConflict")]
    pub sign_conflict: bool,
}

/// Grades one rolled stat the way a riven grader does: its deviation from the centre roll.
pub fn grade_roll(
    item: &Item,
    stat_name: &str,
    value: f64,
    positives: u8,
    negatives: u8,
    is_negative: bool,
) -> Option<Graded> {
    let s = stat(stat_name)?;
    let base = s.base[class_index(item)]?;
    let dispo = item.disposition?;
    let (bonus, malus) = multipliers(positives, negatives)?;
    let centre = base * dispo * if is_negative { malus.abs() } else { bonus };
    if centre == 0.0 {
        return None;
    }
    let mag = value.abs();
    // A negative is better when it is smaller, so the sign of the deviation flips.
    let dev = if is_negative { (centre - mag) / centre } else { (mag - centre) / centre } * 100.0;
    let sign_conflict = matches!((s.sign, is_negative), (Sign::PositiveOnly, true) | (Sign::NegativeOnly, false));
    Some(Graded {
        stat: s.name,
        value,
        centre: py_round(centre, 2),
        deviation: py_round(dev, 3),
        grade: grade_for(dev),
        negative: is_negative,
        out_of_range: dev.abs() > VARIANCE * 100.0 + 0.01,
        sign_conflict,
    })
}

/// The (positives, negatives) a riven was rolled with, from the stats read off its card, and how
/// many of its stats were not read at all.
///
/// The multiplier depends on that count, and OCR can lose a whole stat line on a busy background:
/// then the read count picks the wrong multiplier and every grade comes out ~20% off. The values
/// settle it - normally exactly one count puts every reading inside the +-10% roll window. A line
/// can go missing but cannot appear, so only counts at least as large as the read one are tried.
pub fn infer_counts(item: &Item, stats: &[(&str, f64, bool)]) -> (u8, u8, u8) {
    let neg = stats.iter().filter(|s| s.2).count();
    let pos = stats.len() - neg;
    let read = (pos.clamp(2, 3) as u8, neg.min(1) as u8);
    let fits = |(p, n): (u8, u8)| {
        stats.iter().all(|&(stat, value, negative)| {
            grade_roll(item, stat, value, p, n, negative).is_some_and(|g| !g.out_of_range)
        })
    };
    if fits(read) {
        return (read.0, read.1, 0);
    }
    let options: Vec<(u8, u8)> = [(2, 0), (2, 1), (3, 0), (3, 1)]
        .into_iter()
        .filter(|&(p, n)| p as usize >= pos && n as usize >= neg && fits((p, n)))
        .collect();
    match options[..] {
        [(p, n)] => (p, n, (p + n).saturating_sub(stats.len() as u8)),
        _ => (read.0, read.1, 0),
    }
}

// ------------------------------------------------------------------ traded prices

const RIVEN_API: &str = "https://api.warframestat.us/{platform}/rivens";
const MARKET_MAX_AGE: f64 = 6.0 * 3600.0;

const DOTS: [&str; 6] = ["", "●○○○○", "●●○○○", "●●●○○", "●●●●○", "●●●●●"];

static MARKET_LOCK: Mutex<()> = Mutex::new(());

/// {weapon name: {unrolled: {...}, rerolled: {...}}} of traded riven prices, cached 6 h.
fn market(platform: Platform) -> (serde_json::Map<String, Value>, Option<String>) {
    let _guard = MARKET_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = config::dirs().cache.join("rivens.json");
    let cached: Option<Value> = config::read_json(&file);
    if let Some(c) = &cached {
        if now() - c["t"].as_f64().unwrap_or(0.0) < MARKET_MAX_AGE {
            return (c["data"].as_object().cloned().unwrap_or_default(), None);
        }
    }
    let url = RIVEN_API.replace("{platform}", platform.as_str());
    match net::get_json(&url, &[], Duration::from_secs(25)) {
        Ok(raw) => {
            let mut data = serde_json::Map::new();
            for group in raw.as_object().into_iter().flat_map(|o| o.values()) {
                for (weapon, kinds) in group.as_object().into_iter().flatten() {
                    if kinds.is_object() {
                        data.insert(weapon.clone(), kinds.clone());
                    }
                }
            }
            let _ = config::write_json(&file, &json!({"t": now(), "data": data}));
            (data, None)
        }
        Err(e) => (
            cached.and_then(|c| c["data"].as_object().cloned()).unwrap_or_default(),
            Some(format!("Riven prices unavailable: {e}")),
        ),
    }
}

/// What `weapon`'s rivens sold for lately, from the cached prices (whatever their age; a stale
/// copy is refreshed in the background, so a riven read never waits for it).
pub fn traded(weapon: &str, platform: Platform) -> Option<Value> {
    let cached: Option<Value> = config::read_json(&config::dirs().cache.join("rivens.json"));
    if cached.as_ref().is_none_or(|c| now() - c["t"].as_f64().unwrap_or(0.0) >= MARKET_MAX_AGE) {
        std::thread::spawn(move || market(platform));
    }
    let p = cached?["data"].get(weapon)?.clone();
    let pick = |v: &Value| v.as_object().filter(|o| !o.is_empty()).map(|_| v.clone());
    let (unrolled, rerolled) = (pick(&p["unrolled"]), pick(&p["rerolled"]));
    (unrolled.is_some() || rerolled.is_some()).then(|| json!({"unrolled": unrolled, "rerolled": rerolled}))
}

/// Every weapon that takes a riven, with its traded riven prices.
pub fn rows(catalog: &Catalog, platform: Platform) -> (Vec<Value>, Option<String>) {
    let (prices, err) = market(platform);
    let pick = |v: &Value| {
        v.as_object().filter(|o| !o.is_empty()).map(|o| {
            json!({"median": o.get("median"), "avg": o.get("avg"), "min": o.get("min"),
                   "max": o.get("max"), "pop": o.get("pop")})
        })
    };
    let mut out: Vec<(f64, Value)> = catalog
        .items
        .values()
        .filter_map(|it| {
            let dispo = it.disposition.filter(|d| *d != 0.0)?;
            let p = prices.get(&it.name).cloned().unwrap_or(Value::Null);
            let dots = it.dispo_dots.unwrap_or(0) as usize;
            Some((
                py_round(dispo, 3),
                json!({
                    "name": it.name, "category": it.category, "image": crate::gamedata::image_url(it.image.as_deref()),
                    "disposition": py_round(dispo, 3), "dots": it.dispo_dots,
                    "dotStr": DOTS.get(dots).copied().unwrap_or(""),
                    "unrolled": pick(&p["unrolled"]), "rerolled": pick(&p["rerolled"]),
                }),
            ))
        })
        .collect();
    out.sort_by(|a, b| b.0.total_cmp(&a.0));
    (out.into_iter().map(|(_, v)| v).collect(), err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn weapon(class: &str, dispo: f64) -> Item {
        Item { riven_class: class.into(), disposition: Some(dispo), ..Item::default() }
    }

    #[test]
    fn every_stat_has_a_market_slug() {
        let slugs: std::collections::HashSet<_> = STATS.iter().map(|s| market_slug(s.name).expect(s.name)).collect();
        assert_eq!(slugs.len(), STATS.len());
    }

    #[test]
    fn table_is_complete_and_unique() {
        assert_eq!(STATS.len(), 50);
        assert_eq!(STATS.iter().filter(|s| s.spliced).count(), 18);
        for (i, s) in STATS.iter().enumerate() {
            assert!(STATS[i + 1..].iter().all(|o| o.name != s.name), "duplicate {}", s.name);
        }
        for (_, name) in WORDING {
            assert!(stat(name).is_some(), "wording points at unknown stat {name}");
        }
        for s in STATS {
            assert!(WORDING.iter().any(|(_, n)| n == &s.name), "no wording reads {}", s.name);
        }
    }

    #[test]
    fn a_centre_roll_grades_b_plus() {
        let w = weapon("pistol", 1.2);
        // Damage on a pistol: 219.6 base, 3 positives + 1 negative -> 0.9375.
        let centre = 219.6 * 1.2 * 0.9375;
        let g = grade_roll(&w, "Damage", centre, 3, 1, false).unwrap();
        assert_eq!(g.grade, "B+");
        assert!(g.deviation.abs() < 1e-9 && !g.out_of_range && !g.sign_conflict);
    }

    #[test]
    fn negatives_grade_better_when_smaller() {
        let w = weapon("rifle", 1.0);
        let centre = 59.99 * 0.75; // Zoom, 3+1 malus
        let small = grade_roll(&w, "Zoom", -centre * 0.92, 3, 1, true).unwrap();
        assert_eq!(small.grade, "A+");
        let big = grade_roll(&w, "Zoom", -centre * 1.08, 3, 1, true).unwrap();
        assert_eq!(big.grade, "C-");
    }

    #[test]
    fn zero_negatives_use_their_own_multiplier() {
        // 3+0 uses 0.75, not the 3+1 multiplier 0.9375.
        let w = weapon("rifle", 1.0);
        let g = grade_roll(&w, "Multishot", 90.0 * 0.75, 3, 0, false).unwrap();
        assert_eq!(g.grade, "B+");
    }

    #[test]
    fn a_missing_stat_line_is_inferred_from_the_values() {
        // A Latron (disposition 1.4), a 3+0 roll; OCR lost "+57.5% Magazine Capacity".
        let w = weapon("rifle", 1.4);
        let read = [("Ammo Maximum", 56.9, false), ("Critical Chance", 167.4, false)];
        assert_eq!(infer_counts(&w, &read), (3, 0, 1));
        let all =
            [("Ammo Maximum", 56.9, false), ("Critical Chance", 167.4, false), ("Magazine Capacity", 57.5, false)];
        assert_eq!(infer_counts(&w, &all), (3, 0, 0));
        // Readings that fit no count keep the read one (and stay flagged out of range).
        assert_eq!(infer_counts(&w, &[("Ammo Maximum", 500.0, false), ("Critical Chance", 1.0, false)]), (2, 0, 0));
    }

    #[test]
    fn out_of_range_and_sign_conflicts_are_flagged() {
        let w = weapon("melee", 1.0);
        assert!(grade_roll(&w, "Damage", 500.0, 2, 0, false).unwrap().out_of_range);
        assert!(grade_roll(&w, "Damage", -20.0, 2, 1, true).unwrap().sign_conflict);
        assert!(grade_roll(&w, "Multishot", 50.0, 2, 0, false).is_none(), "melee cannot roll multishot");
    }
}
