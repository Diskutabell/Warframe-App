//! Static game data (items, prime parts, relics) from the community WFCD/warframe-items dataset.
//!
//! Nothing here touches the game: the files are downloaded from GitHub and cached.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, anyhow};
use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;

use crate::{config, net, now};

const BASE_URL: &str = "https://raw.githubusercontent.com/WFCD/warframe-items/master/data/json/";
pub const IMG_URL: &str = "https://cdn.warframestat.us/img/";

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
    pub loaded_at: Option<f64>,
}

pub fn image_url(image: Option<&str>) -> Option<String> {
    image.filter(|s| !s.is_empty()).map(|s| format!("{IMG_URL}{s}"))
}

/// Downloads (or revalidates) every file and builds the catalog.
pub fn load(force: bool) -> Result<Catalog> {
    let names: Vec<&str> = MASTERY_FILES.iter().chain(EXTRA_FILES.iter()).copied().collect();
    let raw: Vec<Result<Value>> = std::thread::scope(|s| {
        let jobs: Vec<_> = names.iter().map(|n| s.spawn(move || download(n, force))).collect();
        jobs.into_iter().map(|j| j.join().unwrap_or_else(|_| Err(anyhow!("download thread panicked")))).collect()
    });
    let mut files = IndexMap::new();
    for (name, data) in names.into_iter().zip(raw) {
        files.insert(name.to_string(), data.with_context(|| format!("Could not download game data ({name})"))?);
    }
    let mut catalog = build(&files);
    catalog.loaded_at = Some(now());
    Ok(catalog)
}

fn cache_path(name: &str) -> PathBuf {
    config::dirs().cache.join(format!("wfcd_{name}.json"))
}

fn download(name: &str, force: bool) -> Result<Value> {
    let path = cache_path(name);
    let etag_path = path.with_extension("etag");
    let age =
        fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
    let cached = || -> Result<Value> { Ok(serde_json::from_slice(&fs::read(&path)?)?) };
    if !force && age.is_some_and(|a| a < MAX_AGE) {
        if let Ok(v) = cached() {
            return Ok(v);
        }
    }
    let etag = if force { None } else { fs::read_to_string(&etag_path).ok() };
    let mut headers = Vec::new();
    if let (Some(tag), true) = (&etag, path.exists()) {
        headers.push(("If-None-Match", tag.as_str()));
    }
    let url = format!("{BASE_URL}{name}.json");
    match net::get(&url, &headers, Duration::from_secs(60)) {
        Ok(resp) if resp.status == 304 => {
            // Unchanged upstream: mark the copy fresh again.
            if let Ok(f) = fs::File::options().append(true).open(&path) {
                let _ = f.set_modified(SystemTime::now());
            }
            cached()
        }
        Ok(resp) if (200..300).contains(&resp.status) => {
            let data: Value = serde_json::from_slice(&resp.body)?;
            config::write_atomic(&path, &resp.body)?;
            match resp.etag {
                Some(tag) => {
                    let _ = fs::write(&etag_path, tag);
                }
                None => {
                    let _ = fs::remove_file(&etag_path);
                }
            }
            Ok(data)
        }
        // Offline or an error upstream: a stale copy beats no data at all.
        other => cached().map_err(|cache_err| match other {
            Ok(resp) => anyhow!("HTTP {}", resp.status),
            Err(e) => e.context(cache_err),
        }),
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

    let mut names: HashMap<String, String> = HashMap::new();
    for data in raw.values() {
        for x in data.as_array().into_iter().flatten() {
            if let (Some(uid), Some(name)) = (str_of(&x["uniqueName"]), str_of(&x["name"])) {
                names.insert(uid.to_string(), name.to_string());
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
            let leaf = canonical.rsplit('/').next().unwrap_or("");
            let default_name = if canonical.ends_with("Blueprint")
                && leaf.replace("Blueprint", "") == uid.rsplit('/').next().unwrap_or("")
            {
                format!("{item_name} Blueprint")
            } else {
                names.get(&canonical).cloned().unwrap_or_else(|| camel_name(&canonical))
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
        if fname == "Relics" {
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

    Catalog { items, parts, part_alias: alias, relics, relic_uids, names, crafts, loaded_at: None }
}

impl Catalog {
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
