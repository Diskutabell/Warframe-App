//! Static game data (items, prime parts, relics) from the community WFCD/warframe-items dataset.
//!
//! Nothing here touches the game: the files are downloaded from GitHub and cached.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

use crate::{config, net, now};

const BASE_URL: &str = "https://raw.githubusercontent.com/WFCD/warframe-items/master/data/json/";
pub const IMG_URL: &str = "https://cdn.warframestat.us/img/";
/// The star chart, exported from the game's own files: the only source found that has the mastery
/// XP of each node (`masteryExp`). WFCD has no node data. Its nodes add up to 14,569, the total
/// the wiki gives for all missions.
const EXPORT_URL: &str = "https://raw.githubusercontent.com/calamity-inc/warframe-public-export-plus/senpai/";
const REGIONS_URL: &str =
    "https://raw.githubusercontent.com/calamity-inc/warframe-public-export-plus/senpai/ExportRegions.json";
/// Syndicates from the same export: ranks, their standing ranges and costs, daily pools,
/// offerings and medallions (`standing.rs`). Names are keys into `dict.en.json` (3.9 MB).
const SYNDICATE_FILES: [&str; 2] = ["ExportSyndicates.json", "dict.en.json"];
/// Relics and their reward tables from the same export, for when WFCD's relics come without their
/// drops (`relics.rs`).
const RELIC_EXPORTS: [&str; 2] = ["ExportRelics.json", "ExportRewards.json"];
/// WFCD's copy of DE's drop tables: the names the game prints for relic drops, and every table a
/// relic drops from today.
const DROP_URL: &str = "https://drops.warframestat.us/data/";
const DROP_TABLES: [&str; 12] = [
    "relics",
    "missionRewards",
    "transientRewards",
    "cetusBountyRewards",
    "solarisBountyRewards",
    "deimosRewards",
    "zarimanRewards",
    "entratiLabRewards",
    "hexRewards",
    "sortieRewards",
    "blueprintLocations",
    "modLocations",
];
/// A junction gives 1,000 mastery points (wiki.warframe.com/w/Mastery_Rank); the export lists
/// every junction with 0.
const JUNCTION_XP: i64 = 1000;
/// The Plexus gives mastery like a warframe (6,000 at rank 30) but is in none of WFCD's files.
/// The profile lists its XP under the Railjack harness.
pub const PLEXUS_UID: &str = "/Lotus/Types/Game/CrewShip/RailJack/DefaultHarness";

pub const MASTERY_FILES: [&str; 11] = [
    "Warframes",
    "Primary",
    "Secondary",
    "Melee",
    "Archwing",
    "Arch-Gun",
    "Arch-Melee",
    "Sentinels",
    "SentinelWeapons",
    "Pets",
    "Misc",
];
/// "Gear" holds what the foundry builds for the gear wheel: ciphers, restores, specters, bait.
const EXTRA_FILES: [&str; 3] = ["Relics", "Resources", "Gear"];
const WEAPON_CATEGORIES: [&str; 6] = ["Primary", "Secondary", "Melee", "Arch-Gun", "Arch-Melee", "SentinelWeapons"];
pub const REFINEMENTS: [&str; 4] = ["Intact", "Exceptional", "Flawless", "Radiant"];
/// WFCD publishes several times a week. A day-old copy is revalidated with its ETag, which costs
/// a 304 when nothing changed.
const MAX_AGE: Duration = Duration::from_secs(24 * 3600);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentKind {
    /// Another masterable item (e.g. the Lex Prime a Akstiletto Prime needs).
    Item,
    /// Plain resources: Orokin Cells, Forma...
    #[default]
    Resource,
    /// A blueprint or crafted part: what relics drop and the foundry tracks.
    Part,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Component {
    pub uid: String,
    pub count: i64,
    pub kind: ComponentKind,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct Item {
    pub uid: String,
    pub name: String,
    pub category: String,
    pub kind: String,
    pub is_prime: bool,
    pub vaulted: bool,
    pub mastery_req: i64,
    pub max_rank: i64,
    /// Affinity per rank squared: weapons level at 500 * r^2, everything else at 1000 * r^2.
    pub xp_factor: f64,
    pub image: Option<String>,
    /// Riven strength: the multiplier every rolled stat is scaled by.
    pub disposition: Option<f64>,
    /// The 1-5 dot display of the same thing.
    pub dispo_dots: Option<i64>,
    pub riven_class: String,
    pub components: Vec<Component>,
    pub set_slug: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PartRelic {
    pub relic: String,
    pub rarity: Option<String>,
    pub chance: Option<f64>,
    pub vaulted: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Part {
    pub uid: String,
    pub name: String,
    pub slug: Option<String>,
    pub items: Vec<String>,
    pub relics: Vec<PartRelic>,
    /// The blueprint/component twin of this uid, when the game has one.
    pub alt: Option<String>,
    pub vaulted: bool,
}

#[derive(Clone, Debug)]
pub struct RelicReward {
    pub uid: String,
    pub name: String,
    pub rarity: Option<String>,
    pub slug: Option<String>,
    /// Drop chance in percent per refinement.
    pub chance: IndexMap<String, Option<f64>>,
}

#[derive(Clone, Debug)]
pub struct Relic {
    pub era: String,
    pub vaulted: bool,
    pub slug: Option<String>,
    /// refinement -> uid
    pub uids: IndexMap<String, String>,
    pub rewards: Vec<RelicReward>,
    pub image: Option<String>,
}

/// One thing a build consumes: its blueprint or a resource.
#[derive(Clone, Debug)]
pub struct CraftNeed {
    pub uid: String,
    pub name: String,
    pub count: i64,
    pub image: Option<String>,
}

/// Something the foundry builds that gives no mastery by itself: the parts of modular gear (amps,
/// zaws, kitguns, K-drives, companions), gear such as ciphers and restores, forma, lenses.
#[derive(Clone, Debug)]
pub struct Craft {
    pub uid: String,
    pub name: String,
    /// WFCD's type: "Amp", "Zaw Component", "Gear", ...
    pub kind: String,
    pub image: Option<String>,
    pub mastery_req: i64,
    /// How many one build makes (ciphers 1, small restores 50).
    pub quantity: i64,
    pub needs: Vec<CraftNeed>,
}

#[derive(Clone, Debug)]
pub struct PartRecipe {
    pub result: String,
    pub quantity: i64,
    pub credits: Option<i64>,
    pub seconds: Option<i64>,
    pub ingredients: Vec<CraftNeed>,
}

#[derive(Clone, Debug)]
pub struct LookupItem {
    pub uid: String,
    pub name: String,
    pub category: String,
    pub image: Option<String>,
}

#[derive(Default)]
pub struct Catalog {
    pub items: IndexMap<String, Item>,
    pub parts: IndexMap<String, Part>,
    /// Any uid (blueprint or component) -> canonical part uid.
    pub part_alias: HashMap<String, String>,
    /// Base name ("Axi A1") -> relic.
    pub relics: IndexMap<String, Relic>,
    /// Relic uid -> (base name, refinement).
    pub relic_uids: HashMap<String, (String, String)>,
    pub names: HashMap<String, String>,
    /// Builds that give no mastery, by kind and name.
    pub crafts: Vec<Craft>,
    /// Blueprint uid -> recipe, from the public game export.
    pub part_recipes: HashMap<String, PartRecipe>,
    pub recipes_available: bool,
    /// Non-equipment catalog entries searchable even when they have no known drops.
    pub lookup: IndexMap<String, LookupItem>,
    /// Display name (as `art_key` spells it) -> image file, for everything in the WFCD files.
    pub art: HashMap<String, String>,
    /// Star chart node tag ("SolNode27") -> the mastery XP its first completion gives. Empty
    /// until the region export has been downloaded once.
    pub node_xp: HashMap<String, i64>,
    /// Every syndicate with its ranks and offerings. Empty until the export has been downloaded.
    pub syndicates: crate::standing::Syndicates,
    /// Reward and enemy blueprint sources indexed by the exact item name.
    pub drops: crate::drops::Drops,
    /// Names, icons and kinds of everything an inventory holds (`inventory.rs`). Empty until the
    /// export has been downloaded once.
    pub inventory: HashMap<String, crate::inventory::Entry>,
    pub loaded_at: Option<f64>,
}

pub fn image_url(image: Option<&str>) -> Option<String> {
    image.filter(|s| !s.is_empty()).map(|s| format!("{IMG_URL}{s}"))
}

/// Downloads (or revalidates) every file and builds the catalog.
// One download per source, all side by side; the tuple names what each thread fetched.
#[allow(clippy::type_complexity)]
pub fn load(force: bool) -> Result<Catalog> {
    let names: Vec<&str> = MASTERY_FILES.iter().chain(EXTRA_FILES.iter()).copied().collect();
    let regions_path = config::dirs().cache.join("regions.json");
    let (raw, mods, recipes, regions, syndicates, drops, held): (
        Vec<Result<Value>>,
        Result<Value>,
        Result<Value>,
        Result<Value>,
        Vec<Result<Value>>,
        Vec<Result<Value>>,
        Vec<Result<Value>>,
    ) = std::thread::scope(|s| {
        let jobs: Vec<_> = names.iter().map(|n| s.spawn(move || download(n, force))).collect();
        let mods = s.spawn(|| download("Mods", force));
        let recipes = s.spawn(|| {
            download_to(
                &format!("{EXPORT_URL}ExportRecipes.json"),
                &config::dirs().cache.join("ExportRecipes.json"),
                force,
            )
        });
        let regions = s.spawn(|| download_to(REGIONS_URL, &regions_path, force));
        let syndicates: Vec<_> = SYNDICATE_FILES
            .iter()
            .map(|f| s.spawn(move || download_to(&format!("{EXPORT_URL}{f}"), &config::dirs().cache.join(f), force)))
            .collect();
        let drops: Vec<_> = DROP_TABLES
            .iter()
            .map(|t| {
                s.spawn(move || {
                    download_to(
                        &format!("{DROP_URL}{t}.json"),
                        &config::dirs().cache.join(format!("drops_{t}.json")),
                        force,
                    )
                })
            })
            .collect();
        let held: Vec<_> = crate::inventory::EXPORTS
            .iter()
            .map(|f| s.spawn(move || download_to(&format!("{EXPORT_URL}{f}"), &config::dirs().cache.join(f), force)))
            .collect();
        let panicked = || Err(anyhow!("download thread panicked"));
        (
            jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| panicked())).collect(),
            mods.join().unwrap_or_else(|_| panicked()),
            recipes.join().unwrap_or_else(|_| panicked()),
            regions.join().unwrap_or_else(|_| panicked()),
            syndicates.into_iter().map(|j| j.join().unwrap_or_else(|_| panicked())).collect(),
            drops.into_iter().map(|j| j.join().unwrap_or_else(|_| panicked())).collect::<Vec<_>>(),
            held.into_iter().map(|j| j.join().unwrap_or_else(|_| panicked())).collect::<Vec<_>>(),
        )
    });
    let mut files = IndexMap::new();
    for (name, data) in names.into_iter().zip(raw) {
        files.insert(name.to_string(), data.with_context(|| format!("Could not download game data ({name})"))?);
    }
    match mods {
        Ok(data) if data.is_array() => {
            files.insert("Mods".into(), data);
        }
        Ok(_) => eprintln!("mod catalog has an unexpected format"),
        Err(e) => eprintln!("mod catalog unavailable: {e:#}"),
    }
    match recipes {
        Ok(data) if data.as_object().is_some_and(|o| !o.is_empty()) => {
            files.insert("ExportRecipes".into(), data);
        }
        Ok(_) => eprintln!("component recipe export has an unexpected format"),
        Err(e) => eprintln!("component recipes unavailable: {e:#}"),
    }
    if files.get("Relics").is_some_and(crate::relics::lacks_rewards) {
        repair_relics(&mut files, force, &drops);
    }
    let mut catalog = build(&files);
    let mut drop_tables = Vec::new();
    for (name, data) in DROP_TABLES.iter().zip(&drops).skip(1) {
        match data {
            Ok(data) if data.get(*name).is_some_and(|v| v.is_array() || v.is_object()) => {
                drop_tables.push((*name, data))
            }
            Ok(_) => eprintln!("drop table {name} has an unexpected format"),
            Err(e) => eprintln!("drop table {name} unavailable: {e:#}"),
        }
    }
    catalog.drops = crate::drops::Drops::from_tables(&drop_tables, DROP_TABLES.len() - 1);
    // The app works without the star chart: only the mastery total is then short of it.
    match regions {
        Ok(raw) => catalog.node_xp = node_xp_table(&raw),
        Err(e) => eprintln!("star chart data unavailable, mastery will leave it out: {e:#}"),
    }
    // Standing works without it too: the page then says the syndicate data is missing.
    match &syndicates[..] {
        [Ok(export), Ok(dict)] => catalog.syndicates = crate::standing::build(export, dict),
        _ => {
            let why: Vec<String> =
                syndicates.iter().filter_map(|r| r.as_ref().err()).map(|e| format!("{e:#}")).collect();
            eprintln!("syndicate data unavailable: {}", why.join("; "));
        }
    }
    // The inventory page names what it can without it.
    if let Some(Ok(dict)) = syndicates.get(1) {
        let exports: Vec<(&str, &Value)> = crate::inventory::EXPORTS
            .iter()
            .zip(&held)
            .filter_map(|(f, r)| match r {
                Ok(v) => Some((*f, v)),
                Err(e) => {
                    eprintln!("{f} unavailable, the inventory page names less: {e:#}");
                    None
                }
            })
            .collect();
        catalog.inventory = crate::inventory::index(&exports, files.get("ExportRecipes"), dict);
    }
    catalog.loaded_at = Some(now());
    Ok(catalog)
}

/// Fills in the drop tables WFCD's relics lack, from the game's export and DE's drop tables (see
/// `relics.rs`). Every source is downloaded and cached like the WFCD files; without the export the
/// relics stay empty, without the drop tables the names fall back to the ids' words and "vaulted"
/// to the export's own dates.
fn repair_relics(files: &mut IndexMap<String, Value>, force: bool, drops: &[Result<Value>]) {
    let cache = &config::dirs().cache;
    let exports: Vec<Result<Value>> = std::thread::scope(|s| {
        let exports: Vec<_> = RELIC_EXPORTS
            .iter()
            .map(|f| s.spawn(move || download_to(&format!("{EXPORT_URL}{f}"), &cache.join(f), force)))
            .collect();
        let panicked = || Err(anyhow!("download thread panicked"));
        exports.into_iter().map(|j| j.join().unwrap_or_else(|_| panicked())).collect()
    });
    let [Ok(export_relics), Ok(export_rewards)] = &exports[..] else {
        eprintln!("WFCD's relics have no drop tables and the game's export could not be read: relics stay empty");
        return;
    };
    let names = drops.first().and_then(|r| r.as_ref().ok());
    let tables: Vec<Value> = drops.iter().skip(1).take(8).filter_map(|r| r.as_ref().ok().cloned()).collect();
    let dropping = (tables.len() == 8).then(|| crate::relics::dropping(&tables));
    if let Some(relics) = files.get_mut("Relics") {
        let filled = crate::relics::patch(relics, export_relics, export_rewards, names, dropping.as_ref());
        eprintln!("WFCD's relics had no drop tables: {filled} filled in from the game's export");
    }
}

/// Node tag -> mastery XP, for the nodes that give any.
pub fn node_xp_table(regions: &Value) -> HashMap<String, i64> {
    let nodes = regions.as_object().into_iter().flatten();
    nodes.filter_map(|(tag, node)| Some((tag.clone(), node["masteryExp"].as_i64().filter(|xp| *xp > 0)?))).collect()
}

fn cache_path(name: &str) -> PathBuf {
    config::dirs().cache.join(format!("wfcd_{name}.json"))
}

fn download(name: &str, force: bool) -> Result<Value> {
    download_to(&format!("{BASE_URL}{name}.json"), &cache_path(name), force)
}

/// Fetches a JSON file into the cache. A copy younger than `MAX_AGE` is used as it is, an older
/// one is revalidated with its ETag, and when the source cannot be reached the old copy will do.
fn download_to(url: &str, path: &Path, force: bool) -> Result<Value> {
    download_to_with(url, path, force, net::get)
}

fn download_to_with(
    url: &str,
    path: &Path,
    force: bool,
    get: impl FnOnce(&str, &[(&str, &str)], Duration) -> Result<net::Response>,
) -> Result<Value> {
    let etag_path = path.with_extension("etag");
    let age = fs::metadata(path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
    let cached: Result<Value> = fs::read(path)
        .with_context(|| format!("reading cached {}", path.display()))
        .and_then(|bytes| serde_json::from_slice(&bytes).with_context(|| format!("parsing cached {}", path.display())));
    if !force && age.is_some_and(|a| a < MAX_AGE) {
        if let Ok(v) = cached {
            return Ok(v);
        }
    }
    let etag = if !force && cached.is_ok() { fs::read_to_string(&etag_path).ok() } else { None };
    let mut headers = Vec::new();
    if let Some(tag) = &etag {
        headers.push(("If-None-Match", tag.as_str()));
    }
    match get(url, &headers, Duration::from_secs(60)) {
        Ok(resp) if resp.status == 304 => {
            let data = cached.context("server returned HTTP 304 without a usable cached copy")?;
            if let Ok(f) = fs::File::options().append(true).open(path) {
                let _ = f.set_modified(SystemTime::now());
            }
            Ok(data)
        }
        Ok(resp) if (200..300).contains(&resp.status) => {
            let data: Value = match serde_json::from_slice(&resp.body) {
                Ok(data) => data,
                Err(e) => {
                    return match cached {
                        Ok(data) => {
                            eprintln!("invalid JSON from {url}, using cached {}: {e}", path.display());
                            Ok(data)
                        }
                        Err(cache_err) => Err(anyhow!("invalid JSON from {url}: {e}; {cache_err:#}")),
                    };
                }
            };
            match fs::remove_file(&etag_path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e).with_context(|| format!("removing {}", etag_path.display())),
            }
            config::write_atomic(path, &resp.body)?;
            if let Some(tag) = resp.etag {
                if let Err(e) = config::write_atomic(&etag_path, tag.as_bytes()) {
                    eprintln!("could not save ETag for {}: {e}", path.display());
                }
            }
            Ok(data)
        }
        // Offline or an error upstream: a stale copy beats no data at all.
        other => match cached {
            Ok(data) => Ok(data),
            Err(cache_err) => Err(match other {
                Ok(resp) => anyhow!("HTTP {}; {cache_err:#}", resp.status),
                Err(e) => e.context(cache_err),
            }),
        },
    }
}

// Riven base values are per weapon class, not per WFCD category: shotguns sit inside Primary,
// zaws report productCategory "Pistols", archwing guns are "SpaceGuns".
fn riven_class(kind: Option<&str>, product: Option<&str>, category: &str) -> &'static str {
    if matches!(category, "Arch-Gun" | "Arch-Melee") || matches!(product, Some("SpaceGuns" | "SpaceMelee")) {
        "archgun"
    } else if category == "Melee" || product == Some("Melee") || kind == Some("Zaw Component") {
        "melee"
    } else if kind == Some("Shotgun") {
        "shotgun"
    } else if product == Some("Pistols") || matches!(kind, Some("Pistol" | "Dual Pistols" | "Throwing")) {
        "pistol"
    } else {
        "rifle"
    }
}

/// Splits an internal part name into what it belongs to and its slot as players know it:
/// "IronframeHelmet" -> ("ironframe", "Neuroptics"), "FuraxWraithLeftGauntlet" ->
/// ("furaxwraith", "Left Gauntlet"). `None` for a name that ends in no slot.
fn split_slot(stem: &str) -> Option<(String, String)> {
    const SLOTS: [(&str, &str); 33] = [
        ("helmet", "Neuroptics"),
        ("chassis", "Chassis"),
        ("systems", "Systems"),
        ("barrel", "Barrel"),
        ("receiver", "Receiver"),
        ("stock", "Stock"),
        ("blade", "Blade"),
        ("handle", "Handle"),
        ("hilt", "Hilt"),
        ("grip", "Grip"),
        ("string", "String"),
        ("limb", "Limb"),
        ("link", "Link"),
        ("gauntlet", "Gauntlet"),
        ("glove", "Glove"),
        ("guard", "Guard"),
        ("head", "Head"),
        ("disc", "Disc"),
        ("wings", "Wings"),
        ("carapace", "Carapace"),
        ("cerebrum", "Cerebrum"),
        ("harness", "Harness"),
        ("heatsink", "Heatsink"),
        ("motor", "Motor"),
        ("engine", "Engine"),
        ("pouch", "Pouch"),
        ("stars", "Stars"),
        ("ornament", "Ornament"),
        ("aegis", "Aegis"),
        ("rivet", "Rivet"),
        ("hook", "Hook"),
        ("chain", "Chain"),
        ("core", "Core"),
    ];
    let lower = stem.to_ascii_lowercase();
    let lower = lower.strip_suffix("item").unwrap_or(&lower);
    let (end, slot) = SLOTS.iter().find(|(end, _)| lower.ends_with(end))?;
    let mut base = &lower[..lower.len() - end.len()];
    let mut label = slot.to_string();
    for (side, word) in [("left", "Left"), ("right", "Right"), ("upper", "Upper"), ("lower", "Lower")] {
        if let Some(rest) = base.strip_suffix(side) {
            base = rest;
            label = format!("{word} {slot}");
            break;
        }
    }
    Some((base.strip_suffix("part").unwrap_or(base).to_string(), label))
}

/// "/Lotus/.../AshPrimeHelmetBlueprint" -> "Ash Prime Neuroptics".
pub fn camel_name(uid: &str) -> String {
    let leaf = uid.rsplit('/').next().unwrap_or(uid);
    let leaf = leaf.strip_suffix("Component").or_else(|| leaf.strip_suffix("Blueprint")).unwrap_or(leaf);
    let mut words = String::with_capacity(leaf.len() + 8);
    let mut prev: Option<char> = None;
    for c in leaf.chars() {
        if c.is_ascii_uppercase() && prev.is_some_and(|p| p.is_ascii_lowercase() || p.is_ascii_digit()) {
            words.push(' ');
        }
        words.push(c);
        prev = Some(c);
    }
    let mut words = words.trim().to_string();
    if uid.contains("/WarframeRecipes/") {
        if let Some(stem) = words.strip_suffix("Helmet") {
            if stem.chars().last().is_none_or(|c| !(c.is_alphanumeric() || c == '_')) {
                words = format!("{stem}Neuroptics");
            }
        }
    }
    words
}

pub fn slugify(name: &str) -> String {
    let lower = name.to_lowercase().replace('&', "and");
    let mut out = String::with_capacity(lower.len());
    let mut gap = false;
    for c in lower.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

/// Prime parts exist as a Blueprint (the relic drop) and a Component (the crafted part).
pub fn alt_uid(uid: &str) -> Option<String> {
    if let Some(stem) = uid.strip_suffix("Component") {
        return Some(format!("{stem}Blueprint"));
    }
    if let Some(stem) = uid.strip_suffix("Blueprint") {
        if uid.contains("/WarframeRecipes/") {
            return Some(format!("{stem}Component"));
        }
    }
    None
}

fn str_of(v: &Value) -> Option<&str> {
    v.as_str().filter(|s| !s.is_empty())
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn rarity_rank(r: Option<&str>) -> u8 {
    match r {
        Some("Common") => 0,
        Some("Uncommon") => 1,
        Some("Rare") => 2,
        _ => 3,
    }
}

/// Builds the catalog from the raw WFCD files, keyed by file name.
pub fn build(raw: &IndexMap<String, Value>) -> Catalog {
    let empty = Vec::new();
    let file = |name: &str| raw.get(name).and_then(Value::as_array).unwrap_or(&empty);

    let mut lookup = IndexMap::new();
    for (category, data) in raw {
        if category == "Relics" {
            continue;
        }
        for item in data.as_array().into_iter().flatten() {
            if let (Some(uid), Some(name)) = (str_of(&item["uniqueName"]), str_of(&item["name"])) {
                lookup.entry(uid.to_string()).or_insert_with(|| LookupItem {
                    uid: uid.to_string(),
                    name: name.to_string(),
                    category: category.to_string(),
                    image: str_of(&item["imageName"]).map(str::to_owned),
                });
            }
        }
    }

    let mut names: HashMap<String, String> = HashMap::new();
    for data in raw.values() {
        for x in data.as_array().into_iter().flatten() {
            if let (Some(uid), Some(name)) = (str_of(&x["uniqueName"]), str_of(&x["name"])) {
                names.insert(uid.to_string(), name.to_string());
            }
        }
    }
    let recipes_available = raw.get("ExportRecipes").and_then(Value::as_object).is_some_and(|r| !r.is_empty());
    let mut part_recipes = HashMap::new();
    if let Some(recipes) = raw.get("ExportRecipes").and_then(Value::as_object) {
        for (uid, recipe) in recipes {
            let Some(result) = str_of(&recipe["resultType"]) else { continue };
            let Some(ingredients) = recipe["ingredients"].as_array() else { continue };
            let needs: Option<Vec<CraftNeed>> = ingredients
                .iter()
                .map(|ingredient| {
                    let id = str_of(&ingredient["ItemType"])?;
                    let count = ingredient["ItemCount"].as_i64().filter(|n| *n > 0)?;
                    Some(CraftNeed {
                        uid: id.to_string(),
                        name: names.get(id).cloned().unwrap_or_else(|| camel_name(id)),
                        count,
                        image: None,
                    })
                })
                .collect();
            let Some(ingredients) = needs else {
                eprintln!("invalid ingredients in component recipe {uid}");
                continue;
            };
            part_recipes.insert(
                uid.clone(),
                PartRecipe {
                    result: result.to_string(),
                    quantity: recipe["num"].as_i64().filter(|n| *n > 0).unwrap_or(1),
                    credits: recipe["buildPrice"].as_i64().filter(|n| *n >= 0),
                    seconds: recipe["buildTime"].as_i64().filter(|n| *n >= 0),
                    ingredients,
                },
            );
        }
    }
    // Pictures by name, for things the world state only names. The files come in catalog order,
    // so a weapon wins over a resource that happens to share its name.
    let mut art: HashMap<String, String> = HashMap::new();
    for (fname, data) in raw {
        if fname == "Relics" {
            continue;
        }
        for x in data.as_array().into_iter().flatten() {
            if let (Some(name), Some(img)) = (str_of(&x["name"]), str_of(&x["imageName"])) {
                art.entry(art_key(name)).or_insert_with(|| img.to_string());
            }
        }
    }

    // Relics first: they carry the nice part names and warframe.market slugs.
    struct RelicBuild {
        relic: Relic,
        rewards: IndexMap<String, RelicReward>,
    }
    let mut relics: IndexMap<String, RelicBuild> = IndexMap::new();
    let mut relic_uids = HashMap::new();
    let mut part_info: HashMap<String, (String, Option<String>)> = HashMap::new();
    for r in file("Relics") {
        let full = r["name"].as_str().unwrap_or("");
        let Some((base, refinement)) = full.rsplit_once(' ') else { continue };
        if !REFINEMENTS.contains(&refinement) {
            continue;
        }
        let Some(ruid) = str_of(&r["uniqueName"]) else { continue };
        let entry = relics.entry(base.to_string()).or_insert_with(|| RelicBuild {
            relic: Relic {
                era: base.split(' ').next().unwrap_or("").to_string(),
                vaulted: true,
                slug: None,
                uids: IndexMap::new(),
                rewards: Vec::new(),
                image: None,
            },
            rewards: IndexMap::new(),
        });
        entry.relic.uids.insert(refinement.to_string(), ruid.to_string());
        relic_uids.insert(ruid.to_string(), (base.to_string(), refinement.to_string()));
        if refinement == "Intact" {
            entry.relic.vaulted = r.get("vaulted").is_none_or(truthy);
            entry.relic.slug = Some(
                str_of(&r["marketInfo"]["urlName"])
                    .map(str::to_owned)
                    .unwrap_or_else(|| slugify(&format!("{base} relic"))),
            );
            entry.relic.image = str_of(&r["imageName"]).map(str::to_owned);
        }
        for rw in r["rewards"].as_array().into_iter().flatten() {
            let it = &rw["item"];
            let Some(uid) = str_of(&it["uniqueName"]) else { continue };
            let rarity = rw["rarity"].as_str().map(str::to_owned);
            let reward = entry.rewards.entry(uid.to_string()).or_insert_with(|| RelicReward {
                uid: uid.to_string(),
                name: str_of(&it["name"]).map(str::to_owned).unwrap_or_else(|| camel_name(uid)),
                rarity: rarity.clone(),
                slug: str_of(&it["warframeMarket"]["urlName"]).map(str::to_owned),
                chance: IndexMap::new(),
            });
            reward.chance.insert(refinement.to_string(), rw["chance"].as_f64());
            if refinement == "Intact" {
                reward.rarity = rarity;
            }
            part_info.entry(uid.to_string()).or_insert_with(|| (reward.name.clone(), reward.slug.clone()));
            names.entry(uid.to_string()).or_insert_with(|| reward.name.clone());
        }
    }
    let mut relics: IndexMap<String, Relic> = relics
        .into_iter()
        .map(|(base, mut b)| {
            let mut rewards: Vec<RelicReward> = b.rewards.into_values().collect();
            rewards.sort_by_key(|e| rarity_rank(e.rarity.as_deref()));
            b.relic.rewards = rewards;
            (base, b.relic)
        })
        .collect();

    // Masterable items and their build components.
    let mut items: IndexMap<String, Item> = IndexMap::new();
    let mut raw_components: HashMap<String, IndexMap<String, i64>> = HashMap::new();
    for fname in MASTERY_FILES {
        for x in file(fname) {
            if !truthy(&x["masterable"]) {
                continue;
            }
            let Some(uid) = str_of(&x["uniqueName"]) else { continue };
            let category = str_of(&x["category"]).unwrap_or(fname).to_string();
            let kind = str_of(&x["type"]);
            let mut max_rank = x["maxLevelCap"].as_i64().filter(|&r| r != 0).unwrap_or(30);
            // WFCD leaves Necramechs at the default 30; they rank (and give mastery) to 40.
            if kind == Some("Necramech") && max_rank < 40 {
                max_rank = 40;
            }
            // Kitgun chambers sit in "Misc" but level like the guns they are.
            let weapon = WEAPON_CATEGORIES.contains(&category.as_str()) || kind == Some("Kitgun Component");
            let mut comps: IndexMap<String, i64> = IndexMap::new();
            for c in x["components"].as_array().into_iter().flatten() {
                let Some(cuid) = str_of(&c["uniqueName"]) else { continue };
                let n = c.get("itemCount").and_then(Value::as_i64).filter(|&n| n != 0).unwrap_or(1);
                *comps.entry(cuid.to_string()).or_insert(0) += n;
            }
            raw_components.insert(uid.to_string(), comps);
            items.insert(
                uid.to_string(),
                Item {
                    uid: uid.to_string(),
                    name: x["name"].as_str().unwrap_or("").to_string(),
                    kind: kind.unwrap_or(&category).to_string(),
                    is_prime: truthy(&x["isPrime"]),
                    vaulted: truthy(&x["vaulted"]),
                    mastery_req: x["masteryReq"].as_i64().unwrap_or(0),
                    max_rank,
                    xp_factor: if weapon { 500.0 } else { 1000.0 },
                    image: str_of(&x["imageName"]).map(str::to_owned),
                    disposition: x["omegaAttenuation"].as_f64(),
                    dispo_dots: x["disposition"].as_i64(),
                    riven_class: riven_class(kind, str_of(&x["productCategory"]), &category).to_string(),
                    category,
                    components: Vec::new(),
                    set_slug: None,
                },
            );
        }
    }

    items.entry(PLEXUS_UID.to_string()).or_insert_with(|| Item {
        uid: PLEXUS_UID.to_string(),
        name: "Plexus".to_string(),
        category: "Misc".to_string(),
        kind: "Plexus".to_string(),
        max_rank: 30,
        xp_factor: 1000.0,
        ..Item::default()
    });

    // Each item's own blueprint, by its internal name ("ironframe" for Hildryn). Parts are named
    // after the item whose blueprint shares their internal name - not always the item that needs
    // them: Chroma takes a Volt helmet, Pride a blade of Wrath.
    let mut own_blueprint: HashMap<String, String> = HashMap::new();
    for (uid, comps) in &raw_components {
        let blueprints: Vec<&str> = comps
            .keys()
            .filter(|c| c.contains("/Recipes/"))
            .filter_map(|c| c.rsplit('/').next().and_then(|leaf| leaf.strip_suffix("Blueprint")))
            .collect();
        // The only blueprint is the item's own; among several, the one that names no slot.
        let own = match blueprints.as_slice() {
            [one] => Some(*one),
            many => many.iter().copied().find(|stem| split_slot(stem).is_none()),
        };
        if let Some(stem) = own {
            own_blueprint.insert(uid.clone(), stem.to_ascii_lowercase());
        }
    }
    let owners: Vec<(String, String)> =
        own_blueprint.iter().filter_map(|(uid, stem)| Some((stem.clone(), items.get(uid)?.name.clone()))).collect();
    let related =
        |base: &str, blueprint: &str| !base.is_empty() && (blueprint.starts_with(base) || base.starts_with(blueprint));

    // Resolve components into parts, required items and resources.
    let mut parts: IndexMap<String, Part> = IndexMap::new();
    let mut alias: HashMap<String, String> = HashMap::new();
    let uids: Vec<String> = items.keys().cloned().collect();
    for uid in &uids {
        let (item_name, is_prime) = (items[uid].name.clone(), items[uid].is_prime);
        let mut comps = Vec::new();
        for (cuid, count) in raw_components.remove(uid).unwrap_or_default() {
            if let Some(other) = items.get(&cuid) {
                comps.push(Component { name: other.name.clone(), uid: cuid, count, kind: ComponentKind::Item });
                continue;
            }
            if !cuid.contains("/Recipes/") {
                let name = names.get(&cuid).cloned().unwrap_or_else(|| camel_name(&cuid));
                comps.push(Component { uid: cuid, count, kind: ComponentKind::Resource, name });
                continue;
            }
            let alt = alt_uid(&cuid);
            let canonical = match &alt {
                Some(a) if !part_info.contains_key(&cuid) && part_info.contains_key(a) => a.clone(),
                _ => cuid.clone(),
            };
            let info = part_info.get(&canonical);
            // A part without a relic has no display name in the data, and its uid uses the game's
            // internal names (Hildryn is "Ironframe", the Brakk a "GrineerHandShotgun"). So name
            // it after the item it belongs to: "Hildryn Chassis", "Hildryn Blueprint".
            let leaf = canonical.rsplit('/').next().unwrap_or("");
            let stem = leaf.trim_end_matches("Blueprint").trim_end_matches("Component");
            let is_blueprint = canonical.ends_with("Blueprint");
            let mine = own_blueprint.get(uid);
            let is_own_blueprint = is_blueprint && mine.is_some_and(|bp| *bp == stem.to_ascii_lowercase());
            let default_name = if is_own_blueprint || (is_blueprint && stem == uid.rsplit('/').next().unwrap_or("")) {
                format!("{item_name} Blueprint")
            } else if let Some(name) = names.get(&canonical) {
                name.clone()
            } else if let Some((base, slot)) = split_slot(stem) {
                let owner = match mine {
                    Some(bp) if related(&base, bp) => Some(item_name.as_str()),
                    // Someone else's part: only the item whose blueprint has exactly this name. A
                    // looser match made Broken War's "War Blade" a Gunsen ("Warfan") blade.
                    Some(_) => owners.iter().find(|(bp, _)| *bp == base).map(|(_, name)| name.as_str()),
                    None => Some(item_name.as_str()),
                };
                owner.map_or_else(|| camel_name(&canonical), |o| format!("{o} {slot}"))
            } else if is_blueprint && mine.is_none() {
                format!("{item_name} Blueprint")
            } else {
                camel_name(&canonical)
            };
            let pname = info.map(|i| i.0.clone()).filter(|n| !n.is_empty()).unwrap_or(default_name);
            let part = parts.entry(canonical.clone()).or_insert_with(|| Part {
                uid: canonical.clone(),
                name: pname.clone(),
                slug: info.and_then(|i| i.1.clone()),
                alt: if canonical == cuid { alt.clone() } else { Some(cuid.clone()) },
                ..Part::default()
            });
            if !part.items.contains(uid) {
                part.items.push(uid.clone());
            }
            alias.insert(canonical.clone(), canonical.clone());
            alias.insert(cuid.clone(), canonical.clone());
            if let Some(a) = alt {
                alias.entry(a).or_insert_with(|| canonical.clone());
            }
            comps.push(Component { uid: canonical, count, kind: ComponentKind::Part, name: pname });
        }
        let item = items.get_mut(uid).expect("uid from keys");
        item.components = comps;
        item.set_slug = is_prime.then(|| slugify(&format!("{item_name} set")));
    }

    // Relic rewards that are not item parts (Forma blueprints, Kuva, Ayatan...) still get a part.
    for (base, rel) in relics.iter_mut() {
        for rw in rel.rewards.iter_mut() {
            let canonical = alias.get(&rw.uid).cloned().unwrap_or_else(|| rw.uid.clone());
            rw.uid = canonical.clone();
            let part = parts.entry(canonical.clone()).or_insert_with(|| {
                let alt = alt_uid(&canonical);
                alias.insert(canonical.clone(), canonical.clone());
                if let Some(a) = &alt {
                    alias.entry(a.clone()).or_insert_with(|| canonical.clone());
                }
                Part { uid: canonical.clone(), name: rw.name.clone(), slug: rw.slug.clone(), alt, ..Part::default() }
            });
            part.relics.push(PartRelic {
                relic: base.clone(),
                rarity: rw.rarity.clone(),
                chance: rw.chance.get("Intact").copied().flatten(),
                vaulted: rel.vaulted,
            });
        }
    }
    for p in parts.values_mut() {
        p.relics.sort_by(|a, b| {
            a.vaulted.cmp(&b.vaulted).then_with(|| b.chance.unwrap_or(0.0).total_cmp(&a.chance.unwrap_or(0.0)))
        });
        p.vaulted = !p.relics.is_empty() && p.relics.iter().all(|r| r.vaulted);
    }

    // Everything else with a recipe. WFCD lists the blueprint and the resources as components;
    // a blueprint has no entry of its own, so it gets a plain name.
    let mut images: HashMap<&str, &str> = HashMap::new();
    for (fname, data) in raw {
        if fname == "Relics" {
            continue;
        }
        for x in data.as_array().into_iter().flatten() {
            if let (Some(uid), Some(img)) = (str_of(&x["uniqueName"]), str_of(&x["imageName"])) {
                images.insert(uid, img);
            }
        }
    }
    let mut crafts: Vec<Craft> = Vec::new();
    for (fname, data) in raw {
        if fname == "Relics" || fname == "Mods" {
            continue;
        }
        for x in data.as_array().into_iter().flatten() {
            let (Some(uid), Some(name)) = (str_of(&x["uniqueName"]), str_of(&x["name"])) else { continue };
            if items.contains_key(uid) || crafts.iter().any(|c| c.uid == uid) {
                continue;
            }
            let needs: Vec<CraftNeed> = x["components"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    let cuid = str_of(&c["uniqueName"])?;
                    let known = names.get(cuid).cloned();
                    let name = known.unwrap_or_else(|| {
                        if cuid.ends_with("Blueprint") { "Blueprint".to_string() } else { camel_name(cuid) }
                    });
                    Some(CraftNeed {
                        uid: cuid.to_string(),
                        name,
                        count: c.get("itemCount").and_then(Value::as_i64).filter(|&n| n != 0).unwrap_or(1),
                        image: images.get(cuid).map(|s| s.to_string()),
                    })
                })
                .collect();
            if needs.is_empty() {
                continue;
            }
            crafts.push(Craft {
                uid: uid.to_string(),
                name: name.to_string(),
                kind: str_of(&x["type"]).unwrap_or(fname).to_string(),
                image: str_of(&x["imageName"]).map(str::to_owned),
                mastery_req: x["masteryReq"].as_i64().unwrap_or(0),
                quantity: x["buildQuantity"].as_i64().filter(|&n| n > 0).unwrap_or(1),
                needs,
            });
        }
    }
    crafts.sort_by(|a, b| (&a.kind, &a.name).cmp(&(&b.kind, &b.name)));

    Catalog {
        items,
        parts,
        part_alias: alias,
        relics,
        relic_uids,
        names,
        crafts,
        part_recipes,
        recipes_available,
        lookup,
        art,
        node_xp: HashMap::new(),
        syndicates: crate::standing::Syndicates::default(),
        drops: crate::drops::Drops::default(),
        inventory: HashMap::new(),
        loaded_at: None,
    }
}

/// A name reduced to what both spellings share: the world state writes "Ack And Brunt" for the
/// catalog's "Ack & Brunt".
pub fn art_key(name: &str) -> String {
    let lower = name.to_lowercase();
    lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && *w != "and").collect()
}

impl Catalog {
    /// A picture for something known only by its display name: a reward, a Circuit pick, Darvo's
    /// deal. A leading count goes ("3x Forma"), and a part falls back to the thing it builds
    /// ("Dera Vandal Blueprint", "Twin Vipers Wraith Barrel") by dropping words from the end.
    pub fn art_by_name(&self, name: &str) -> Option<String> {
        let mut words: Vec<&str> = name.split_whitespace().collect();
        let is_count = |w: &str| w.trim_end_matches(['x', 'X']).parse::<u32>().is_ok();
        if words.len() > 1 && is_count(words[0]) {
            words.remove(0);
        }
        while !words.is_empty() {
            if let Some(img) = self.art.get(&art_key(&words.join(" "))) {
                return image_url(Some(img));
            }
            words.pop();
        }
        None
    }

    /// Mastery XP for completing a star chart node once. Steel Path gives the same again.
    pub fn node_mastery(&self, tag: &str) -> i64 {
        if tag.ends_with("Junction") {
            return JUNCTION_XP;
        }
        self.node_xp.get(tag).copied().unwrap_or(0)
    }

    pub fn name_of(&self, uid: &str) -> String {
        if let Some(it) = self.items.get(uid) {
            return it.name.clone();
        }
        if let Some(p) = self.parts.get(uid) {
            return p.name.clone();
        }
        if let Some((base, refinement)) = self.relic_uids.get(uid) {
            return format!("{base} {refinement}");
        }
        self.names.get(uid).cloned().unwrap_or_else(|| camel_name(uid))
    }

    /// Names the OCR matcher looks for on the reward screen: everything a relic can drop.
    pub fn reward_names(&self) -> IndexMap<String, String> {
        let mut out = IndexMap::new();
        for p in self.parts.values() {
            if !p.relics.is_empty() {
                out.insert(p.name.clone(), p.uid.clone());
            }
        }
        out.entry("Forma Blueprint".to_string())
            .or_insert_with(|| "/Lotus/Types/Recipes/Components/FormaBlueprint".to_string());
        out
    }

    pub fn rank_from_xp(item: &Item, xp: i64) -> i64 {
        let r = (xp.max(0) as f64 / item.xp_factor).sqrt() as i64;
        r.min(item.max_rank)
    }

    /// Weapons that can carry a riven.
    pub fn riven_weapons(&self) -> Vec<String> {
        self.items.values().filter(|i| i.disposition.is_some_and(|d| d != 0.0)).map(|i| i.name.clone()).collect()
    }

    pub fn item_by_name(&self, name: &str) -> Option<&Item> {
        let lower = name.to_lowercase();
        self.items.values().find(|i| i.name.to_lowercase() == lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct TestCache(PathBuf);

    impl TestCache {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "warframeproject-gamedata-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> PathBuf {
            self.0.join("data.json")
        }
    }

    impl Drop for TestCache {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn response(status: u16, body: &str, etag: Option<&str>) -> net::Response {
        net::Response { status, body: body.as_bytes().to_vec(), etag: etag.map(str::to_owned) }
    }

    #[test]
    fn cached_download_revalidates_only_usable_data() {
        let cache = TestCache::new();
        let path = cache.path();
        fs::write(&path, "broken").unwrap();
        fs::write(path.with_extension("etag"), "\"old\"").unwrap();
        let data = download_to_with("test", &path, false, |_, headers, _| {
            assert!(headers.is_empty(), "a corrupt cache must not send an ETag");
            Ok(response(200, r#"{"updated":true}"#, Some("\"new\"")))
        })
        .unwrap();
        assert_eq!(data, json!({"updated": true}));
        assert_eq!(fs::read_to_string(path.with_extension("etag")).unwrap(), "\"new\"");
        let data = download_to_with("test", &path, false, |_, _, _| panic!("fresh cache should not fetch")).unwrap();
        assert_eq!(data, json!({"updated": true}));

        fs::File::options()
            .append(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - MAX_AGE - Duration::from_secs(60))
            .unwrap();
        let data = download_to_with("test", &path, false, |_, headers, _| {
            assert_eq!(headers, &[("If-None-Match", "\"new\"")]);
            Ok(response(304, "", None))
        })
        .unwrap();
        assert_eq!(data, json!({"updated": true}));
        assert!(fs::metadata(&path).unwrap().modified().unwrap().elapsed().unwrap() < MAX_AGE);
    }

    #[test]
    fn cached_download_preserves_valid_data_on_bad_response() {
        let cache = TestCache::new();
        let path = cache.path();
        fs::write(&path, r#"{"cached":true}"#).unwrap();
        fs::write(path.with_extension("etag"), "\"old\"").unwrap();
        let data = download_to_with("test", &path, true, |_, headers, _| {
            assert!(headers.is_empty(), "forced refresh must not send an ETag");
            Ok(response(200, "not json", Some("\"bad\"")))
        })
        .unwrap();
        assert_eq!(data, json!({"cached": true}));
        assert_eq!(fs::read_to_string(&path).unwrap(), r#"{"cached":true}"#);
        assert_eq!(fs::read_to_string(path.with_extension("etag")).unwrap(), "\"old\"");
        let data = download_to_with("test", &path, true, |_, _, _| Ok(response(503, "", None))).unwrap();
        assert_eq!(data, json!({"cached": true}));
        let data = download_to_with("test", &path, true, |_, _, _| Err(anyhow!("network unavailable"))).unwrap();
        assert_eq!(data, json!({"cached": true}));
        let data =
            download_to_with("test", &path, true, |_, _, _| Ok(response(200, r#"{"next":true}"#, None))).unwrap();
        assert_eq!(data, json!({"next": true}));
        assert!(!path.with_extension("etag").exists());
        fs::write(&path, "broken").unwrap();
        let err = download_to_with("test", &path, true, |_, _, _| Ok(response(200, "not json", None))).unwrap_err();
        assert!(format!("{err:#}").contains("invalid JSON"));
        let err = download_to_with("test", &path, false, |_, _, _| Ok(response(304, "", None))).unwrap_err();
        assert!(format!("{err:#}").contains("without a usable cached copy"));
    }

    #[test]
    fn camel_names() {
        assert_eq!(camel_name("/Lotus/Types/Recipes/WarframeRecipes/AshPrimeHelmetBlueprint"), "Ash Prime Neuroptics");
        assert_eq!(camel_name("/Lotus/Types/Recipes/Weapons/WeaponParts/BratonPrimeReceiver"), "Braton Prime Receiver");
        assert_eq!(camel_name("/Lotus/Types/Items/MiscItems/OrokinCell"), "Orokin Cell");
    }

    #[test]
    fn slugs_and_twins() {
        assert_eq!(slugify("Ack & Brunt Set"), "ack_and_brunt_set");
        assert_eq!(slugify("  Axi A1 Relic!"), "axi_a1_relic");
        assert_eq!(
            alt_uid("/X/WarframeRecipes/AshPrimeChassisBlueprint").as_deref(),
            Some("/X/WarframeRecipes/AshPrimeChassisComponent")
        );
        assert_eq!(alt_uid("/X/Weapons/BratonPrimeBlueprint"), None);
    }

    fn raw(files: &[(&str, Value)]) -> IndexMap<String, Value> {
        let mut m: IndexMap<String, Value> =
            MASTERY_FILES.iter().chain(EXTRA_FILES.iter()).map(|f| (f.to_string(), json!([]))).collect();
        for (k, v) in files {
            m.insert(k.to_string(), v.clone());
        }
        m
    }

    #[test]
    fn mastery_curves() {
        let c = build(&raw(&[
            (
                "Misc",
                json!([{"uniqueName": "/K", "name": "Catchmoon", "masterable": true, "category": "Misc", "type": "Kitgun Component"},
                            {"uniqueName": "/D", "name": "Flatbelly", "masterable": true, "category": "Misc", "type": "K-Drive Component"}]),
            ),
            (
                "Warframes",
                json!([{"uniqueName": "/V", "name": "Voidrig", "masterable": true, "category": "Warframes", "type": "Necramech"}]),
            ),
        ]));
        assert_eq!(c.items["/K"].xp_factor, 500.0, "kitgun chambers level like weapons");
        assert_eq!(c.items["/D"].xp_factor, 1000.0);
        assert_eq!(c.items["/V"].max_rank, 40);
        assert_eq!(Catalog::rank_from_xp(&c.items["/K"], 450_000), 30);
        assert_eq!(Catalog::rank_from_xp(&c.items["/V"], 1_600_000), 40);
    }

    #[test]
    fn parts_without_a_relic_are_named_after_their_item() {
        let r = "/Lotus/Types/Recipes/WarframeRecipes/";
        let c = build(&raw(&[
            (
                "Warframes",
                json!([{"uniqueName": "/Lotus/Powersuits/IronFrame/IronFrame", "name": "Hildryn", "masterable": true,
                        "components": [{"uniqueName": format!("{r}IronframeBlueprint")},
                                       {"uniqueName": format!("{r}IronframeHelmetComponent")},
                                       {"uniqueName": format!("{r}IronframeChassisComponent")}]}]),
            ),
            (
                "Primary",
                json!([{"uniqueName": "/Lotus/Weapons/Tenno/Rifle/Rifle", "name": "Braton", "masterable": true,
                        "components": [{"uniqueName": "/Lotus/Types/Recipes/Weapons/BratonBlueprint"}]},
                       {"uniqueName": "/Lotus/Weapons/Grineer/Pistols/HandShotgun", "name": "Brakk", "masterable": true,
                        "components": [{"uniqueName": "/Lotus/Types/Recipes/Weapons/WeaponParts/GrineerHandShotgunBarrel"}]}]),
            ),
        ]));
        let names = |uid: &str| c.items[uid].components.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names("/Lotus/Powersuits/IronFrame/IronFrame"),
            ["Hildryn Blueprint", "Hildryn Neuroptics", "Hildryn Chassis"]
        );
        assert_eq!(names("/Lotus/Weapons/Tenno/Rifle/Rifle"), ["Braton Blueprint"]);
        assert_eq!(names("/Lotus/Weapons/Grineer/Pistols/HandShotgun"), ["Brakk Barrel"]);
    }

    #[test]
    fn a_part_of_another_item_keeps_that_items_name() {
        let r = "/Lotus/Types/Recipes/";
        let c = build(&raw(&[
            (
                "Warframes",
                json!([{"uniqueName": "/Lotus/Powersuits/Dragon/Dragon", "name": "Chroma", "masterable": true,
                        "components": [{"uniqueName": format!("{r}WarframeRecipes/ChromaBlueprint")},
                                       {"uniqueName": format!("{r}WarframeRecipes/ChromaHelmetComponent")},
                                       {"uniqueName": format!("{r}WarframeRecipes/VOLTHelmetComponent")}]},
                       {"uniqueName": "/Lotus/Powersuits/Volt/Volt", "name": "Volt", "masterable": true,
                        "components": [{"uniqueName": format!("{r}WarframeRecipes/VoltBlueprint")},
                                       {"uniqueName": format!("{r}WarframeRecipes/VOLTHelmetComponent")}]},
                       {"uniqueName": "/Lotus/Powersuits/Volt/VoltPrime", "name": "Volt Prime", "masterable": true,
                        "components": [{"uniqueName": format!("{r}WarframeRecipes/VoltPrimeBlueprint")}]}]),
            ),
            (
                "Melee",
                json!([{"uniqueName": "/Lotus/Weapons/Tenno/Melee/Swords/StalkerTwoSmallSword", "name": "Broken War", "masterable": true,
                        "components": [{"uniqueName": format!("{r}Weapons/BrokenWarBlueprint")},
                                       {"uniqueName": format!("{r}Weapons/WeaponParts/WarBlade")}]},
                       {"uniqueName": "/Lotus/Weapons/Tenno/Melee/Warfan/WarfanWeapon", "name": "Gunsen", "masterable": true,
                        "components": [{"uniqueName": format!("{r}Weapons/WarfanBlueprint")}]},
                       {"uniqueName": "/Lotus/Weapons/Tenno/Melee/Fist/FuraxWraith", "name": "Furax Wraith", "masterable": true,
                        "components": [{"uniqueName": format!("{r}Weapons/FuraxWraithBlueprint")},
                                       {"uniqueName": format!("{r}Weapons/WeaponParts/FuraxWraithLeftGauntlet")},
                                       {"uniqueName": format!("{r}Weapons/WeaponParts/FuraxWraithRightGauntlet")}]}]),
            ),
        ]));
        let names = |uid: &str| c.items[uid].components.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
        assert_eq!(
            names("/Lotus/Powersuits/Dragon/Dragon"),
            ["Chroma Blueprint", "Chroma Neuroptics", "Volt Neuroptics"]
        );
        assert_eq!(names("/Lotus/Powersuits/Volt/Volt"), ["Volt Blueprint", "Volt Neuroptics"]);
        // No item's blueprint is called "War": the part keeps its own name.
        assert_eq!(
            names("/Lotus/Weapons/Tenno/Melee/Swords/StalkerTwoSmallSword"),
            ["Broken War Blueprint", "War Blade"]
        );
        assert_eq!(
            names("/Lotus/Weapons/Tenno/Melee/Fist/FuraxWraith"),
            ["Furax Wraith Blueprint", "Furax Wraith Left Gauntlet", "Furax Wraith Right Gauntlet"]
        );
    }

    #[test]
    fn pictures_by_display_name() {
        let c = build(&raw(&[
            (
                "Melee",
                json!([{"uniqueName": "/AB", "name": "Ack & Brunt", "masterable": true, "imageName": "ack.png"}]),
            ),
            (
                "Primary",
                json!([{"uniqueName": "/DV", "name": "Dera Vandal", "masterable": true, "imageName": "dera.png"}]),
            ),
            ("Resources", json!([{"uniqueName": "/F", "name": "Forma", "imageName": "forma.png"}])),
        ]));
        let art = |name| c.art_by_name(name).map(|u| u.trim_start_matches(IMG_URL).to_string());
        assert_eq!(art("Ack And Brunt").as_deref(), Some("ack.png"), "the Circuit's spelling");
        assert_eq!(art("Dera Vandal Blueprint").as_deref(), Some("dera.png"));
        assert_eq!(art("Dera Vandal Barrel").as_deref(), Some("dera.png"));
        assert_eq!(art("3x Forma").as_deref(), Some("forma.png"));
        assert_eq!(art("Conquera Kuaka Floof"), None);
        assert_eq!(art("10000 credits"), None);
    }

    #[test]
    fn mastery_wfcd_does_not_list() {
        let c = build(&raw(&[]));
        let plexus = &c.items[PLEXUS_UID];
        assert_eq!((plexus.name.as_str(), plexus.max_rank, plexus.xp_factor), ("Plexus", 30, 1000.0));
        assert_eq!(Catalog::rank_from_xp(plexus, 1_146_166), 30, "the XP in the user's profile");
        // Junctions are in the region export with 0; the wiki gives 1,000 each.
        assert_eq!(c.node_mastery("EarthToVenusJunction"), 1000);
        assert_eq!(c.node_mastery("SolNode27"), 0, "unknown until the region export is loaded");
    }

    #[test]
    fn parts_link_relics_to_items() {
        let bp = "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisBlueprint";
        let c = build(&raw(&[
            (
                "Warframes",
                json!([{"uniqueName": "/Ash", "name": "Ash Prime", "masterable": true, "isPrime": true,
                                  "components": [{"uniqueName": "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent", "itemCount": 1},
                                                 {"uniqueName": "/Lotus/Types/Items/MiscItems/OrokinCell", "itemCount": 2}]}]),
            ),
            (
                "Relics",
                json!([{"uniqueName": "/R/A1", "name": "Axi A1 Intact", "vaulted": false,
                               "rewards": [{"rarity": "Rare", "chance": 2.0, "item": {"uniqueName": bp, "name": "Ash Prime Chassis Blueprint"}}]}]),
            ),
        ]));
        let ash = &c.items["/Ash"];
        assert_eq!(ash.components[0].kind, ComponentKind::Part);
        assert_eq!(ash.components[0].uid, bp, "the component resolves to the relic's blueprint");
        assert_eq!(ash.components[1].kind, ComponentKind::Resource);
        assert_eq!(ash.set_slug.as_deref(), Some("ash_prime_set"));
        let part = &c.parts[bp];
        assert_eq!(part.relics[0].relic, "Axi A1");
        assert!(!part.vaulted);
        assert_eq!(c.reward_names().get("Ash Prime Chassis Blueprint").map(String::as_str), Some(bp));
    }
}
