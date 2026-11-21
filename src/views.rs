//! What the UI shows, computed from a snapshot of the account so no lock is held meanwhile.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use indexmap::IndexMap;
use serde::Serialize;
use ts_rs::TS;

use crate::accounts::Manual;
use crate::difflib::py_round;
use crate::drops::Drop;
use crate::gamedata::{self, Catalog, ComponentKind, Item, Relic};
use crate::market::Market;
use crate::sources::{Inventory, Profile, Progress};

pub const ERAS: [&str; 5] = ["Lith", "Meso", "Neo", "Axi", "Requiem"];

/// The account at one moment: catalog, profile, inventory, hand-set counts and the market.
pub struct View {
    pub cat: Arc<Catalog>,
    pub profile: Option<Arc<Profile>>,
    pub inv: Option<Arc<Inventory>>,
    pub manual: Arc<Manual>,
    pub market: Arc<Market>,
    /// Recipes in the foundry right now.
    pub foundry: HashSet<String>,
}

#[derive(Clone, Serialize, TS)]
#[ts(export)]
pub struct PartView {
    pub uid: String,
    pub name: String,
    pub need: i64,
    pub have: i64,
    pub slug: Option<String>,
    pub price: Option<i64>,
    pub ducats: Option<i64>,
    pub vaulted: bool,
    pub image: Option<String>,
    pub relics: Vec<gamedata::PartRelic>,
}

/// Where an item stands for the account: levelled, owned, building. Flattened into the rows
/// that show an item.
#[derive(Clone, Serialize, TS)]
#[ts(export)]
pub struct Status {
    pub xp: i64,
    pub rank: i64,
    pub status: &'static str,
    pub manual: bool,
    pub owned: Option<i64>,
    pub building: bool,
}

/// What still to farm for a part: up to three relics, from those that still drop.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct FarmHint {
    pub part: String,
    pub relics: Vec<String>,
    /// Every relic it drops from is vaulted.
    pub vaulted: bool,
}

/// The recipe that crafts a part from its blueprint.
#[derive(Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Recipe {
    pub quantity: i64,
    pub credits: Option<i64>,
    pub seconds: Option<i64>,
    pub ingredients: Vec<Ingredient>,
}

/// A resource a build or a recipe consumes; `have` is None when nothing has counted it.
#[derive(Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct Ingredient {
    pub uid: String,
    pub name: String,
    pub count: i64,
    pub image: Option<String>,
    pub have: Option<i64>,
    pub drops: Vec<Drop>,
}

/// How many of the drop tables were available when the catalog was built.
#[derive(Clone, Copy, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct DropCoverage {
    pub available: usize,
    pub total: usize,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct InventoryView {
    pub items: Vec<InventoryRow>,
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub imported: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub currencies: Option<crate::sources::Currencies>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct InventoryRow {
    pub uid: String,
    pub name: String,
    pub kind: String,
    pub count: i64,
    pub image: Option<String>,
    pub price: Option<i64>,
    pub ducats: Option<i64>,
    pub slug: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Collection {
    pub relics: i64,
    pub relic_kinds: usize,
    pub prime_parts: i64,
    pub sets_complete: usize,
    pub sets_started: usize,
    pub mods: i64,
    pub stacks: usize,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MasteryProgress {
    /// The gear alone, as `possible` and `byCategory`.
    pub points: i64,
    pub possible: i64,
    /// Gear, star chart, Steel Path and intrinsics: the game's own figure.
    pub total: i64,
    /// What is not counted yet: "profile" (no star chart or intrinsics read) or "starChart".
    pub lacks: Option<&'static str>,
    pub rank: i64,
    pub rank_floor: i64,
    pub next_at: i64,
    pub sources: MasterySources,
    pub by_category: Vec<CategoryMastery>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MasterySources {
    pub gear: i64,
    pub star_chart: i64,
    pub steel_path: i64,
    pub railjack: i64,
    pub drifter: i64,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct CategoryMastery {
    pub category: String,
    pub mastered: i64,
    pub total: i64,
    pub points: i64,
    pub possible: i64,
}

/// A row of the Mastery page; what is still to build only for what is not mastered.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct MasteryRow {
    pub uid: String,
    pub name: String,
    pub category: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub is_prime: bool,
    pub vaulted: bool,
    pub max_rank: i64,
    pub mastery_req: i64,
    pub image: Option<String>,
    pub points: i64,
    pub max_points: i64,
    #[serde(flatten)]
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub parts: Option<Vec<MasteryPart>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub buildable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub requires: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub farm: Option<Vec<FarmHint>>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct MasteryPart {
    pub name: String,
    pub need: i64,
    pub have: i64,
    pub vaulted: bool,
    pub image: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PrimeRow {
    pub uid: String,
    pub name: String,
    pub category: String,
    pub vaulted: bool,
    pub image: Option<String>,
    pub status: &'static str,
    pub rank: i64,
    pub owned: Option<i64>,
    pub parts: Vec<PartView>,
    pub have: i64,
    pub need: i64,
    pub complete: bool,
    pub set_price: Option<i64>,
    pub set_slug: Option<String>,
    pub requires: Vec<String>,
}

/// Something the foundry can build: equipment (with its parts) or a build that gives no mastery.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(untagged)]
pub enum FoundryRow {
    Item(CraftableRow),
    Craft(CraftRow),
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CraftableRow {
    pub uid: String,
    pub name: String,
    pub category: String,
    pub image: Option<String>,
    pub is_prime: bool,
    pub vaulted: bool,
    pub mastery_req: i64,
    pub status: &'static str,
    pub owned: Option<i64>,
    pub parts: Vec<PartView>,
    pub have: i64,
    pub need: i64,
    pub complete: bool,
    pub buildable: bool,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct CraftRow {
    pub uid: String,
    pub name: String,
    pub category: String,
    /// Always true: tells the row from equipment.
    pub craft: bool,
    pub pinned: bool,
    pub image: Option<String>,
    pub mastery_req: i64,
    pub quantity: i64,
    pub held: i64,
    /// Needs met, of `need`.
    pub have: usize,
    pub need: usize,
    pub buildable: bool,
    pub parts: Vec<CraftPart>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct CraftPart {
    pub name: String,
    pub need: i64,
    pub have: i64,
    pub image: Option<String>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct FoundryView {
    pub pending: Vec<PendingBuild>,
    pub items: Vec<FoundryRow>,
}

/// Something pinned to the Overview, with how far along it is.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct PinnedRow {
    pub key: String,
    /// "item", "part", "relic" or "build".
    pub kind: &'static str,
    pub name: String,
    /// Whether it has a details panel.
    pub open: bool,
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<&'static str>,
    pub have: i64,
    pub need: i64,
    pub unit: &'static str,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct PendingBuild {
    pub uid: String,
    pub name: String,
    pub done: Option<f64>,
    pub image: Option<String>,
    /// Whether it has a details panel (Forma and the like have none).
    pub known: bool,
}

#[derive(Clone, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RelicView {
    pub name: String,
    pub era: String,
    pub vaulted: bool,
    /// Refinement -> how many are held.
    pub counts: IndexMap<String, i64>,
    pub image: Option<String>,
    pub total: i64,
    /// Refinement -> the game's id.
    pub uids: IndexMap<String, String>,
    pub rewards: Vec<RelicRewardRow>,
    pub needs_count: usize,
    pub slug: Option<String>,
    /// Refinement -> the platinum a crack is worth on average.
    pub ev: IndexMap<String, f64>,
    pub price: Option<i64>,
}

#[derive(Clone, Serialize, TS)]
#[ts(export)]
pub struct RelicRewardRow {
    pub name: String,
    pub uid: String,
    pub rarity: Option<String>,
    /// Refinement -> drop chance in percent.
    pub chance: IndexMap<String, Option<f64>>,
    pub price: Option<i64>,
    pub ducats: Option<i64>,
    pub image: Option<String>,
    pub have: i64,
    pub needed: i64,
}

/// Per era: the relics held and the one most worth cracking.
#[derive(Clone, Serialize, TS)]
#[ts(export)]
pub struct RelicPick {
    pub owned: i64,
    pub kinds: usize,
    pub best: Option<RelicBest>,
}

#[derive(Clone, Serialize, TS)]
#[ts(export)]
pub struct RelicBest {
    pub name: String,
    pub needs: usize,
    pub ev: f64,
    pub image: Option<String>,
    pub total: i64,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TradeRow {
    pub uid: String,
    pub name: String,
    pub have: i64,
    pub spare: i64,
    pub slug: Option<String>,
    pub price: Option<i64>,
    pub ducats: Option<i64>,
    pub vaulted: bool,
    pub ducats_per_plat: Option<f64>,
    pub image: Option<String>,
}

/// A search hit of the command palette. `status`, `price` and `have` only for the kinds that
/// have them.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct FindHit {
    /// "item", "part", "relic", "syndicate", "lookup" or "drop".
    pub kind: &'static str,
    /// What the details panel opens it by.
    pub key: String,
    pub name: String,
    pub sub: String,
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub price: Option<Option<i64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub have: Option<i64>,
}

/// The details panel: one item, part, relic, or the drop sources of anything else.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Detail {
    Drop(DropDetail),
    Item(Box<ItemDetail>),
    Part(PartDetail),
    Relic(RelicDetail),
}

impl Detail {
    /// What the details panel opens it by.
    pub fn key(&self) -> &str {
        match self {
            Detail::Drop(d) => &d.key,
            Detail::Item(d) => &d.key,
            Detail::Part(d) => &d.key,
            Detail::Relic(d) => &d.key,
        }
    }
}

/// `/api/item`: the details, whether they are pinned, and the farming-board task they stand for.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct DetailAnswer {
    #[serde(flatten)]
    pub detail: Detail,
    pub pinned: bool,
    pub task: Option<TaskRef>,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TaskRef {
    pub uid: String,
    pub on_board: bool,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DropDetail {
    pub key: String,
    pub name: String,
    pub image: Option<String>,
    pub drops: Vec<Drop>,
    pub drop_coverage: DropCoverage,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ItemDetail {
    pub key: String,
    pub uid: String,
    pub name: String,
    pub category: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub image: Option<String>,
    pub is_prime: bool,
    pub vaulted: bool,
    pub mastery_req: i64,
    pub max_rank: i64,
    pub points: i64,
    pub max_points: i64,
    pub set_slug: Option<String>,
    pub set_price: Option<i64>,
    pub parts: Vec<DetailPart>,
    pub buildable: bool,
    pub recipes_available: bool,
    pub drops: Vec<Drop>,
    pub drop_coverage: DropCoverage,
    pub requires: Vec<Required>,
    pub resources: Vec<Ingredient>,
    pub disposition: Option<f64>,
    pub dots: Option<i64>,
    #[serde(flatten)]
    pub status: Status,
}

/// A part on an item's details panel.
#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct DetailPart {
    pub uid: String,
    pub name: String,
    pub need: i64,
    pub have: i64,
    pub slug: Option<String>,
    pub price: Option<i64>,
    pub ducats: Option<i64>,
    pub vaulted: bool,
    pub image: Option<String>,
    pub relics: Vec<RelicRef>,
    pub drops: Vec<Drop>,
    pub recipe: Option<Recipe>,
    /// On a task's details: blueprints of it crafting in the foundry.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub crafting: Option<i64>,
    /// On a task's details: runs of its recipe still to do.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub builds: Option<i64>,
    /// On a task's details: it is crafted first and its recipe is not known.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub recipe_unknown: Option<bool>,
}

/// Another item a build consumes (the Lex Prime an Akstiletto Prime needs).
#[derive(Serialize, TS)]
#[ts(export)]
pub struct Required {
    pub uid: String,
    pub name: String,
    pub count: i64,
    pub status: Option<&'static str>,
    pub have: Option<i64>,
}

/// A relic a part drops from, as the details panel lists it.
#[derive(Serialize, TS)]
#[ts(export)]
pub struct RelicRef {
    pub relic: String,
    pub rarity: Option<String>,
    pub chance: Option<f64>,
    pub vaulted: bool,
    pub owned: i64,
    /// A fissure of its era is up and it can be had (it drops, or one is held).
    pub open: bool,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct PartDetail {
    pub key: String,
    pub uid: String,
    pub name: String,
    pub image: Option<String>,
    pub slug: Option<String>,
    pub price: Option<i64>,
    pub ducats: Option<i64>,
    pub vaulted: bool,
    pub have: i64,
    pub needed: i64,
    pub relics: Vec<RelicRef>,
    pub drops: Vec<Drop>,
    pub recipe: Option<Recipe>,
    pub recipes_available: bool,
    pub drop_coverage: DropCoverage,
    pub used_in: Vec<UsedIn>,
}

#[derive(Serialize, TS)]
#[ts(export)]
pub struct UsedIn {
    pub uid: String,
    pub name: String,
    pub image: Option<String>,
    pub status: &'static str,
}

#[derive(Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct RelicDetail {
    #[serde(flatten)]
    pub relic: RelicView,
    pub key: String,
    pub open: bool,
    pub drops: Vec<Drop>,
    pub drop_coverage: DropCoverage,
}

/// Mastery points an item gives per rank: 100 for weapons, 200 for everything else (warframes,
/// companions, archwings, K-drives, Necramechs). Weapons level at 500 x rank^2, the rest at 1000.
fn points_per_rank(it: &Item) -> i64 {
    (it.xp_factor / 5.0) as i64
}

/// The mastery rank a number of mastery points allows, with that rank's threshold and the next one.
/// MR n takes 2,500 x n^2 points up to MR 30; Legendary L takes 2,250,000 + 147,500 x L
/// (wiki.warframe.com/w/Mastery_Rank).
pub fn rank_for(points: i64) -> (i64, i64, i64) {
    let mut rank = 0;
    while rank < 30 && 2500 * (rank + 1) * (rank + 1) <= points {
        rank += 1;
    }
    if rank < 30 {
        return (rank, 2500 * rank * rank, 2500 * (rank + 1) * (rank + 1));
    }
    let legendary = ((points - 2_250_000) / 147_500).max(0);
    (30 + legendary, 2_250_000 + 147_500 * legendary, 2_250_000 + 147_500 * (legendary + 1))
}

/// One rank of any intrinsic gives 1,500 mastery points (wiki.warframe.com/w/Mastery_Rank).
const INTRINSIC_XP: i64 = 1500;

/// Mastery points that do not come from gear.
#[derive(Debug, Default, PartialEq)]
pub struct OtherMastery {
    /// Nodes and junctions.
    pub star_chart: i64,
    /// The same nodes and junctions again, for those done on the Steel Path.
    pub steel_path: i64,
    pub railjack: i64,
    pub drifter: i64,
}

impl OtherMastery {
    pub fn total(&self) -> i64 {
        self.star_chart + self.steel_path + self.railjack + self.drifter
    }
}

pub fn other_mastery(cat: &Catalog, progress: &Progress) -> OtherMastery {
    let mut m = OtherMastery::default();
    for (tag, tier) in &progress.nodes {
        let xp = cat.node_mastery(tag);
        m.star_chart += xp;
        if *tier >= 1 {
            m.steel_path += xp;
        }
    }
    for (skill, rank) in &progress.intrinsics {
        let xp = (*rank).max(0) * INTRINSIC_XP;
        if skill.starts_with("LPS_DRIFT_") {
            m.drifter += xp;
        } else {
            m.railjack += xp;
        }
    }
    m
}

fn farm_hint(parts: &[PartView]) -> Vec<FarmHint> {
    parts
        .iter()
        .filter(|p| p.have < p.need)
        .filter_map(|p| {
            let live: Vec<&gamedata::PartRelic> = p.relics.iter().filter(|r| !r.vaulted).collect();
            let src: Vec<&gamedata::PartRelic> = if live.is_empty() { p.relics.iter().collect() } else { live.clone() };
            (!src.is_empty()).then(|| FarmHint {
                part: p.name.clone(),
                relics: src
                    .iter()
                    .take(3)
                    .map(|r| format!("{} ({})", r.relic, r.rarity.as_deref().unwrap_or("?")))
                    .collect(),
                vaulted: live.is_empty(),
            })
        })
        .collect()
}

/// One relic, priced: the row the relic views and the fissure picks share.
pub struct RelicRow {
    pub value: RelicView,
    pub era: String,
    pub total: i64,
    pub needs: usize,
    pub ev_radiant: f64,
}

/// The counts set by hand with the changes made after them: a reward picked or a build marked
/// done after a count was set by hand goes on top of that count, as it goes on top of an
/// inventory read before it. A count set after a change already includes it.
pub fn hand_counts_after(cat: &Catalog, manual: &Manual, deltas: &[crate::sources::Delta]) -> Manual {
    let mut out = manual.clone();
    for d in deltas.iter().filter(|d| !d.equipment && d.kind != "claimed") {
        // A part's count by hand may be on either twin (blueprint or crafted component).
        let canon = cat.part_alias.get(&d.uid).map_or(d.uid.as_str(), String::as_str);
        let twins = std::iter::once(canon).chain(cat.parts.get(canon).and_then(|p| p.alt.as_deref()));
        let newest = twins
            .filter(|u| out.counts.get(*u).is_some_and(|(_, t)| *t < d.t))
            .max_by(|a, b| out.counts[*a].1.total_cmp(&out.counts[*b].1));
        if let Some(uid) = newest.map(str::to_owned) {
            let (n, t) = out.counts[&uid];
            out.counts.insert(uid, ((n + d.change).max(0), t));
        }
    }
    out
}

/// How well `name` matches a search: lower is better, None is no match. Every word of the query
/// may match the start of any word of the name, so "ash sys" finds "Ash Prime Systems Blueprint".
pub(crate) fn match_rank(name: &str, q: &str, tokens: &[&str]) -> Option<u8> {
    let n = name.to_lowercase();
    if n == q {
        return Some(0);
    }
    if n.starts_with(q) {
        return Some(1);
    }
    let words: Vec<&str> = n.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if tokens.iter().all(|t| words.iter().any(|w| w.starts_with(t))) {
        return Some(2);
    }
    n.contains(q).then_some(3)
}

impl View {
    /// The recipe that crafts a part from its blueprint: keyed by the blueprint, making the
    /// component. A prime weapon part drops ready-made and has none.
    pub(crate) fn recipe_for_part(&self, uid: &str) -> Option<&gamedata::PartRecipe> {
        let part = self.cat.parts.get(uid)?;
        let uids = [Some(uid), part.alt.as_deref()];
        uids.iter()
            .flatten()
            .filter_map(|id| self.cat.part_recipes.get(*id))
            .find(|recipe| uids.contains(&Some(recipe.result.as_str())))
    }

    fn part_recipe(&self, uid: &str) -> Option<Recipe> {
        let recipe = self.recipe_for_part(uid)?;
        Some(Recipe {
            quantity: recipe.quantity,
            credits: recipe.credits,
            seconds: recipe.seconds,
            ingredients: recipe
                .ingredients
                .iter()
                .map(|need| self.ingredient(&need.uid, &need.name, need.count))
                .collect(),
        })
    }

    /// A resource something consumes, with what is held when anything has counted it.
    fn ingredient(&self, uid: &str, name: &str, count: i64) -> Ingredient {
        Ingredient {
            uid: uid.to_string(),
            name: name.to_string(),
            count,
            image: self.icon_of(uid, name),
            have: (self.inv.is_some() || self.manual.counts.contains_key(uid)).then(|| self.count(uid)),
            drops: self.drops_for(name, false),
        }
    }

    fn drops_for(&self, name: &str, include_blueprint: bool) -> Vec<Drop> {
        self.cat.drops.for_name(name, include_blueprint).into_iter().cloned().collect()
    }

    fn drop_coverage(&self) -> DropCoverage {
        DropCoverage { available: self.cat.drops.available, total: self.cat.drops.total }
    }

    /// A resource's picture: WFCD's art for exactly that name, else the game's own icon (which
    /// for some resources is their store bundle's: Polymer Bundle's is a crate of weapons), else
    /// WFCD's art for the start of the name.
    pub(crate) fn icon_of(&self, uid: &str, name: &str) -> Option<String> {
        self.exact_art(name)
            .or_else(|| self.cat.inventory.get(uid).and_then(|e| e.icon.clone()))
            .or_else(|| self.cat.art_by_name(name))
    }

    fn exact_art(&self, name: &str) -> Option<String> {
        gamedata::image_url(self.cat.art.get(&gamedata::art_key(name)).map(String::as_str))
    }

    /// Owned count for any uid. A part counts its blueprint and its crafted component together.
    /// A count set by hand is for the part as a whole (the page shows and steps the total): when
    /// it is newer than the inventory it replaces what the inventory says for both twins, the
    /// newest one should both have one.
    pub fn count(&self, uid: &str) -> i64 {
        let twins = self.twins(uid);
        let hand = twins.iter().filter_map(|u| self.hand_set(u)).max_by(|a, b| a.1.total_cmp(&b.1));
        match hand {
            Some((n, _)) => n,
            None => twins.iter().map(|u| self.held_stack(u)).sum(),
        }
    }

    /// A part's uid and its twin (blueprint and crafted component), the part's own first; any
    /// other uid alone.
    pub fn twins<'a>(&'a self, uid: &'a str) -> Vec<&'a str> {
        let canon = self.cat.part_alias.get(uid).map_or(uid, String::as_str);
        std::iter::once(canon).chain(self.cat.parts.get(canon).and_then(|p| p.alt.as_deref())).collect()
    }

    /// The count set by hand for exactly this uid and when it was set, if it is newer than the
    /// inventory (or there is no inventory): then it replaces what the inventory says.
    fn hand_set(&self, uid: &str) -> Option<(i64, f64)> {
        let (n, t) = self.manual.counts.get(uid)?;
        let inv_t = self.inv.as_deref().map_or(0.0, |i| i.synced_at.filter(|t| *t != 0.0).unwrap_or(i.imported));
        (self.inv.is_none() || *t > inv_t).then_some((*n, *t))
    }

    /// The count set by hand for exactly this uid, when it is newer than the inventory.
    pub fn hand_count(&self, uid: &str) -> Option<i64> {
        self.hand_set(uid).map(|(n, _)| n)
    }

    /// Whether the part (either twin) has a count set by hand that is newer than the inventory.
    pub fn hand_counted(&self, uid: &str) -> bool {
        self.twins(uid).iter().any(|u| self.hand_set(u).is_some())
    }

    /// The inventory's stack of exactly this uid, without hand-set counts or the part's twin.
    pub fn held_stack(&self, uid: &str) -> i64 {
        self.inv.as_deref().map_or(0, |i| i.counts.get(uid).copied().unwrap_or(0))
    }

    pub fn xp(&self, uid: &str) -> i64 {
        let mut best = self.profile.as_ref().and_then(|p| p.xp.get(uid).copied()).unwrap_or(0).max(0);
        if let Some(i) = self.inv.as_deref() {
            best = best.max(i.xp.get(uid).copied().unwrap_or(0)).max(i.equipment.get(uid).map_or(0, |e| e.xp));
        }
        best
    }

    /// Star chart and intrinsics: from the profile and, when an imported file had them, the
    /// inventory. `None` when no source has told us yet.
    pub fn progress(&self) -> Option<Progress> {
        let sources = [
            self.profile.as_ref().and_then(|p| p.progress.as_ref()),
            self.inv.as_ref().and_then(|i| i.progress.as_ref()),
        ];
        let mut merged: Option<Progress> = None;
        for p in sources.into_iter().flatten() {
            merged.get_or_insert_with(Progress::default).merge(p);
        }
        merged
    }

    pub fn owned(&self, uid: &str) -> Option<i64> {
        self.inv.as_ref().map(|i| i.equipment.get(uid).map_or(0, |e| e.count))
    }

    pub fn item_status(&self, it: &Item) -> Status {
        let xp = self.xp(&it.uid);
        let rank = Catalog::rank_from_xp(it, xp);
        let manual = self.manual.mastered.contains_key(&it.uid);
        let status = if manual || rank >= it.max_rank {
            "mastered"
        } else if xp > 0 {
            "leveling"
        } else {
            "missing"
        };
        let main_blueprint = format!("{} Blueprint", it.name);
        let building = it.components.iter().any(|c| {
            c.kind == ComponentKind::Part && self.foundry.contains(&c.uid) && c.name.ends_with(&main_blueprint)
        });
        Status {
            xp,
            rank: rank.max(if manual { it.max_rank } else { 0 }),
            status,
            manual,
            owned: self.owned(&it.uid),
            building,
        }
    }

    /// Everything in the inventory: equipment and every counted stack, with a name, a picture,
    /// the kind it is filed under and, for what can be traded, its cached price and ducats.
    /// Prices are never requested from here: a page of 1,300 rows would flood the queue.
    pub fn inventory(&self) -> InventoryView {
        let Some(inv) = self.inv.as_deref() else {
            return InventoryView { items: Vec::new(), source: None, imported: None, currencies: None };
        };
        let mut rows: Vec<InventoryRow> = Vec::new();
        let mut row = |uid: &str, count: i64| {
            let (name, kind, image) = self.inventory_entry(uid);
            let slug = self.market.slug_for(Some(uid), None, Some(&name));
            let known = slug.as_deref().is_some_and(|s| self.market.name(s).is_some());
            let slug = slug.filter(|_| known);
            rows.push(InventoryRow {
                uid: uid.to_string(),
                name,
                kind,
                count,
                image,
                price: self.price(slug.as_deref(), false),
                ducats: self.market.ducats(slug.as_deref()),
                slug,
            });
        };
        for (uid, e) in &inv.equipment {
            row(uid, e.count);
        }
        // Stacks, and ranked mods and arcanes (held one by one, not as a stack).
        for (uid, count) in inv.held().iter().filter(|(_, c)| **c > 0) {
            if !inv.equipment.contains_key(uid) {
                row(uid, *count);
            }
        }
        InventoryView {
            items: rows,
            source: inv.source.clone(),
            imported: Some(inv.imported),
            currencies: Some(inv.currencies()),
        }
    }

    /// (name, kind, picture) for anything an inventory holds: the catalog's names for equipment,
    /// parts and relics, the game's export for the rest, and the uid's last word when neither
    /// knows it.
    fn inventory_entry(&self, uid: &str) -> (String, String, Option<String>) {
        self.inventory_entry_inner(uid, &mut [""; 4], 0)
    }

    fn inventory_entry_inner<'a>(
        &'a self,
        uid: &'a str,
        seen: &mut [&'a str; 4],
        depth: usize,
    ) -> (String, String, Option<String>) {
        let export = self.cat.inventory.get(uid);
        // Resources only: WFCD's mod pictures are not on its image host.
        let resource = export.filter(|e| matches!(e.kind, "Resources" | "Gems and ores" | "Fish"));
        let named = resource.and_then(|e| e.name.as_deref()).and_then(|n| self.exact_art(n));
        let icon = named.or_else(|| export.and_then(|e| e.icon.clone()));
        if let Some(it) = self.cat.items.get(uid) {
            return (it.name.clone(), it.category.clone(), gamedata::image_url(it.image.as_deref()).or(icon));
        }
        if let Some((e, name)) = export.and_then(|e| Some((e, e.name.clone()?))).filter(|(e, _)| e.result.is_none()) {
            return (name, e.kind.to_string(), icon);
        }
        let canon = self.cat.part_alias.get(uid).map(String::as_str);
        if let Some(part) = canon.and_then(|c| self.cat.parts.get(c)) {
            let name = if uid.ends_with("Blueprint") && !part.name.ends_with("Blueprint") {
                format!("{} Blueprint", part.name)
            } else {
                part.name.clone()
            };
            let kind = if part.name.contains("Prime") { "Prime parts" } else { "Parts" };
            return (name, kind.to_string(), self.image_for(uid).or(icon));
        }
        if let Some((base, refinement)) = self.cat.relic_uids.get(uid) {
            let era = self.cat.relics.get(base).map_or("", |r| r.era.as_str());
            let art = match era {
                "Requiem" => "Immortal",
                e => e,
            };
            let pic = (!art.is_empty()).then(|| format!("{}Icons/Relics/Relic{art}A.png", crate::art::GAME_ART));
            return (format!("{base} {refinement}"), "Relics".to_string(), pic);
        }
        if let Some(e) = export {
            let resolved = if depth < seen.len() && (e.name.is_none() || icon.is_none()) {
                e.result.as_deref().filter(|&r| r != uid && !seen[..depth].contains(&r)).map(|r| {
                    seen[depth] = uid;
                    self.inventory_entry_inner(r, seen, depth + 1)
                })
            } else {
                None
            };
            let name = e.name.clone().or_else(|| resolved.as_ref().map(|(n, _, _)| format!("{n} Blueprint")));
            if let Some(name) = name {
                let pic = icon.or_else(|| resolved.and_then(|(_, _, p)| p));
                return (name, e.kind.to_string(), pic);
            }
        }
        (self.cat.name_of(uid), crate::inventory::kind_by_path(uid).to_string(), icon)
    }

    pub fn price(&self, slug: Option<&str>, request: bool) -> Option<i64> {
        self.market.price(slug, request).and_then(|p| p.sell)
    }

    pub fn part_views(&self, it: &Item, request_price: bool) -> Vec<PartView> {
        it.components
            .iter()
            .filter(|c| c.kind == ComponentKind::Part)
            .map(|c| {
                let p = self.cat.parts.get(&c.uid);
                let slug =
                    self.market.slug_for(Some(&c.uid), p.and_then(|p| p.slug.as_deref()), p.map(|p| p.name.as_str()));
                PartView {
                    uid: c.uid.clone(),
                    name: c.name.clone(),
                    need: c.count,
                    have: self.count(&c.uid),
                    price: slug.as_deref().and_then(|s| self.price(Some(s), request_price)),
                    ducats: self.market.ducats(slug.as_deref()),
                    vaulted: p.is_some_and(|p| p.vaulted),
                    image: self.image_for(&c.uid),
                    relics: p.map(|p| p.relics.iter().take(6).cloned().collect()).unwrap_or_default(),
                    slug,
                }
            })
            .collect()
    }

    pub fn requires(&self, it: &Item) -> Vec<String> {
        it.components.iter().filter(|c| c.kind == ComponentKind::Item).map(|c| c.name.clone()).collect()
    }

    /// Parts still needed: for prime items not mastered, not owned and not in the foundry.
    pub fn needed_parts(&self) -> HashMap<String, i64> {
        let mut needed = HashMap::new();
        for it in self.cat.items.values().filter(|it| it.is_prime) {
            let st = self.item_status(it);
            if st.status == "mastered" || st.owned.unwrap_or(0) > 0 || st.building {
                continue;
            }
            for c in it.components.iter().filter(|c| c.kind == ComponentKind::Part) {
                *needed.entry(c.uid.clone()).or_insert(0) += c.count;
            }
        }
        needed
    }

    /// Best art for any uid: the part's own slot glyph, else the item it belongs to.
    pub fn image_for(&self, uid: &str) -> Option<String> {
        if uid.is_empty() {
            return None;
        }
        let canon = self.cat.part_alias.get(uid).map_or(uid, String::as_str);
        if let Some(part) = self.cat.parts.get(canon) {
            let slug = self.market.slug_for(Some(canon), part.slug.as_deref(), Some(&part.name));
            if let Some(img) = self.market.image(slug.as_deref()) {
                return Some(img);
            }
            // Parts that cannot be traded are not on warframe.market: draw the slot's generic glyph.
            let owners = || part.items.iter().filter_map(|o| self.cat.items.get(o));
            if let Some(img) = owners().find_map(|it| crate::market::slot_glyph(&it.category, canon)) {
                return Some(img);
            }
            if let Some(img) = owners().find_map(|it| it.image.as_deref()) {
                return gamedata::image_url(Some(img));
            }
        }
        self.cat.items.get(uid).and_then(|it| gamedata::image_url(it.image.as_deref()))
    }

    pub fn collection(&self) -> Collection {
        let empty = IndexMap::new();
        let counts = self.inv.as_deref().map_or(&empty, |i| &i.counts);
        let held = |uid: &String, n: &i64| *n > 0 && self.cat.relic_uids.contains_key(uid);
        let relics: i64 = counts.iter().filter(|(u, n)| held(u, n)).map(|(_, n)| n).sum();
        let relic_kinds = counts.iter().filter(|(u, n)| held(u, n)).count();
        // Crafted components are stored under their own uid; the alias maps them to their part.
        let prime_parts: i64 = counts
            .iter()
            .filter(|(uid, n)| {
                **n > 0
                    && self
                        .cat
                        .part_alias
                        .get(*uid)
                        .and_then(|c| self.cat.parts.get(c))
                        .is_some_and(|p| p.name.contains("Prime"))
            })
            .map(|(_, n)| n)
            .sum();
        let (mut done, mut started) = (0, 0);
        for it in self.cat.items.values().filter(|it| it.is_prime) {
            let need: Vec<&gamedata::Component> =
                it.components.iter().filter(|c| c.kind == ComponentKind::Part).collect();
            if need.is_empty() {
                continue;
            }
            let have = need.iter().filter(|c| self.count(&c.uid) >= c.count).count();
            if have >= need.len() {
                done += 1;
            } else if have > 0 {
                started += 1;
            }
        }
        Collection {
            relics,
            relic_kinds,
            prime_parts,
            sets_complete: done,
            sets_started: started,
            mods: self.inv.as_ref().map_or(0, |i| i.mods),
            stacks: counts.len(),
        }
    }

    /// Mastery XP as the game's profile shows it: gear, star chart, Steel Path and intrinsics,
    /// with the rank that total reaches (the rank held can be lower: every rank needs its test).
    /// `points`/`possible`/`byCategory` are the gear alone. `lacks` says what is not counted yet:
    /// "profile" (no star chart or intrinsics read so far) or "starChart" (no node table).
    pub fn mastery_progress(&self) -> MasteryProgress {
        let (mut points, mut possible) = (0i64, 0i64);
        let mut by_cat: IndexMap<&str, (i64, i64, i64, i64)> = IndexMap::new();
        for it in self.cat.items.values() {
            let st = self.item_status(it);
            let per = points_per_rank(it);
            let (p, m) = (st.rank.min(it.max_rank) * per, it.max_rank * per);
            points += p;
            possible += m;
            let e = by_cat.entry(it.category.as_str()).or_default();
            e.0 += i64::from(st.status == "mastered");
            e.1 += 1;
            e.2 += p;
            e.3 += m;
        }
        by_cat.sort_by(|_, a, _, b| b.1.cmp(&a.1));
        let progress = self.progress();
        let other = progress.as_ref().map(|p| other_mastery(&self.cat, p)).unwrap_or_default();
        let lacks = match &progress {
            None => Some("profile"),
            Some(_) if self.cat.node_xp.is_empty() => Some("starChart"),
            Some(_) => None,
        };
        let total = points + other.total();
        let (rank, floor, next) = rank_for(total);
        MasteryProgress {
            points,
            possible,
            total,
            lacks,
            rank,
            rank_floor: floor,
            next_at: next,
            sources: MasterySources {
                gear: points,
                star_chart: other.star_chart,
                steel_path: other.steel_path,
                railjack: other.railjack,
                drifter: other.drifter,
            },
            by_category: by_cat
                .iter()
                .map(|(c, (m, t, p, x))| CategoryMastery {
                    category: c.to_string(),
                    mastered: *m,
                    total: *t,
                    points: *p,
                    possible: *x,
                })
                .collect(),
        }
    }

    pub fn mastered_count(&self) -> (usize, usize) {
        let mastered = self.cat.items.values().filter(|it| self.item_status(it).status == "mastered").count();
        (mastered, self.cat.items.len())
    }

    pub fn mastery(&self) -> Vec<MasteryRow> {
        let mut out: Vec<MasteryRow> = self
            .cat
            .items
            .values()
            .map(|it| {
                let st = self.item_status(it);
                let mut row = MasteryRow {
                    uid: it.uid.clone(),
                    name: it.name.clone(),
                    category: it.category.clone(),
                    kind: it.kind.clone(),
                    is_prime: it.is_prime,
                    vaulted: it.vaulted,
                    max_rank: it.max_rank,
                    mastery_req: it.mastery_req,
                    image: gamedata::image_url(it.image.as_deref()),
                    points: st.rank.min(it.max_rank) * points_per_rank(it),
                    max_points: it.max_rank * points_per_rank(it),
                    status: st.clone(),
                    parts: None,
                    buildable: None,
                    requires: None,
                    farm: None,
                };
                if st.status != "mastered" {
                    let parts: Vec<PartView> = self.part_views(it, false);
                    row.buildable = Some(!parts.is_empty() && parts.iter().all(|p| p.have >= p.need));
                    row.requires = Some(self.requires(it));
                    row.farm = Some(farm_hint(&parts));
                    row.parts = Some(
                        parts
                            .into_iter()
                            .map(|p| MasteryPart {
                                name: p.name,
                                need: p.need,
                                have: p.have,
                                vaulted: p.vaulted,
                                image: p.image,
                            })
                            .collect(),
                    );
                }
                row
            })
            .collect();
        out.sort_by(|a, b| (&a.category, &a.name).cmp(&(&b.category, &b.name)));
        out
    }

    pub fn primes(&self) -> Vec<PrimeRow> {
        let mut out: Vec<(f64, String, PrimeRow)> = Vec::new();
        for it in self.cat.items.values().filter(|it| it.is_prime) {
            let parts = self.part_views(it, true);
            if parts.is_empty() {
                continue;
            }
            let st = self.item_status(it);
            let have: i64 = parts.iter().map(|p| p.have.min(p.need)).sum();
            let need: i64 = parts.iter().map(|p| p.need).sum();
            let set_slug = self.market.slug_for(None, it.set_slug.as_deref(), Some(&format!("{} Set", it.name)));
            out.push((
                -(have as f64 / need as f64),
                it.name.clone(),
                PrimeRow {
                    uid: it.uid.clone(),
                    name: it.name.clone(),
                    category: it.category.clone(),
                    vaulted: it.vaulted,
                    image: gamedata::image_url(it.image.as_deref()),
                    status: st.status,
                    rank: st.rank,
                    owned: st.owned,
                    parts,
                    have,
                    need,
                    complete: have >= need,
                    set_price: self.price(set_slug.as_deref(), true),
                    set_slug,
                    requires: self.requires(it),
                },
            ));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        out.into_iter().map(|(_, _, row)| row).collect()
    }

    /// Everything craftable, with the components held - the collection browser.
    pub fn craftable(&self) -> Vec<CraftableRow> {
        let mut out: Vec<(f64, String, CraftableRow)> = Vec::new();
        for it in self.cat.items.values() {
            let parts = self.part_views(it, false);
            if parts.is_empty() {
                continue;
            }
            let st = self.item_status(it);
            let have: i64 = parts.iter().map(|p| p.have.min(p.need)).sum();
            let need: i64 = parts.iter().map(|p| p.need).sum();
            let buildable = parts.iter().all(|p| p.have >= p.need);
            out.push((
                -(have as f64 / need as f64),
                it.name.clone(),
                CraftableRow {
                    uid: it.uid.clone(),
                    name: it.name.clone(),
                    category: it.category.clone(),
                    image: gamedata::image_url(it.image.as_deref()),
                    is_prime: it.is_prime,
                    vaulted: it.vaulted,
                    mastery_req: it.mastery_req,
                    status: st.status,
                    owned: st.owned,
                    parts,
                    have,
                    need,
                    complete: have >= need,
                    buildable,
                },
            ));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        out.into_iter().map(|(_, _, row)| row).collect()
    }

    /// The Foundry page: what is building, then everything buildable.
    pub fn foundry(&self) -> FoundryView {
        let mut items: Vec<FoundryRow> = self.craftable().into_iter().map(FoundryRow::Item).collect();
        items.extend(self.crafts().into_iter().map(FoundryRow::Craft));
        FoundryView { pending: self.pending_foundry(), items }
    }

    /// Builds that give no mastery (modular parts, gear, forma...), with the blueprint and every
    /// resource checked against the inventory. Closest to buildable first.
    pub fn crafts(&self) -> Vec<CraftRow> {
        let mut out: Vec<(f64, CraftRow)> = self
            .cat
            .crafts
            .iter()
            .map(|c| {
                let parts: Vec<CraftPart> = c
                    .needs
                    .iter()
                    .map(|n| CraftPart {
                        name: n.name.clone(),
                        need: n.count,
                        have: self.count(&n.uid),
                        image: gamedata::image_url(n.image.as_deref()),
                    })
                    .collect();
                let met = parts.iter().filter(|p| p.have >= p.need).count();
                let need = parts.len();
                let row = CraftRow {
                    uid: c.uid.clone(),
                    name: c.name.clone(),
                    category: c.kind.to_string(),
                    craft: true,
                    pinned: self.manual.pinned.contains(&c.uid),
                    image: gamedata::image_url(c.image.as_deref()),
                    mastery_req: c.mastery_req,
                    quantity: c.quantity,
                    held: self.count(&c.uid),
                    have: met,
                    need,
                    buildable: met == need,
                    parts,
                };
                (-(met as f64 / need as f64), row)
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.into_iter().map(|(_, row)| row).collect()
    }

    /// The pinned things with what is still missing for each, in the order they were pinned.
    pub fn pinned(&self) -> Vec<PinnedRow> {
        let needed = self.needed_parts();
        self.manual
            .pinned
            .iter()
            .filter_map(|key| {
                if let Some(it) = self.cat.items.get(key) {
                    let parts = self.part_views(it, false);
                    let have: i64 = parts.iter().map(|p| p.have.min(p.need)).sum();
                    let need: i64 = parts.iter().map(|p| p.need).sum();
                    return Some(PinnedRow {
                        key: key.clone(),
                        kind: "item",
                        name: it.name.clone(),
                        open: true,
                        image: gamedata::image_url(it.image.as_deref()),
                        status: Some(self.item_status(it).status),
                        have,
                        need,
                        unit: "parts",
                    });
                }
                if let Some(p) = self.cat.parts.get(key) {
                    return Some(PinnedRow {
                        key: key.clone(),
                        kind: "part",
                        name: p.name.clone(),
                        open: true,
                        image: self.image_for(key),
                        status: None,
                        have: self.count(key),
                        need: needed.get(key).copied().unwrap_or(0),
                        unit: "held",
                    });
                }
                if let Some(rel) = self.cat.relics.get(key) {
                    let total: i64 = rel.uids.values().map(|u| self.count(u)).sum();
                    return Some(PinnedRow {
                        key: key.clone(),
                        kind: "relic",
                        name: format!("{key} Relic"),
                        open: true,
                        image: gamedata::image_url(rel.image.as_deref()),
                        status: None,
                        have: total,
                        need: 0,
                        unit: "held",
                    });
                }
                let c = self.cat.crafts.iter().find(|c| &c.uid == key)?;
                let met = c.needs.iter().filter(|n| self.count(&n.uid) >= n.count).count();
                Some(PinnedRow {
                    key: key.clone(),
                    kind: "build",
                    name: c.name.clone(),
                    open: false,
                    image: gamedata::image_url(c.image.as_deref()),
                    status: None,
                    have: met as i64,
                    need: c.needs.len() as i64,
                    unit: "needs met",
                })
            })
            .collect()
    }

    /// What is in the foundry now, soonest first.
    pub fn pending_foundry(&self) -> Vec<PendingBuild> {
        let mut rows: Vec<PendingBuild> = self
            .inv
            .as_deref()
            .map(|i| {
                i.foundry
                    .iter()
                    .map(|f| {
                        let uid = f.recipe.as_deref().unwrap_or("");
                        // Forma, consumables and the like have no page to open.
                        let known = self.cat.items.contains_key(uid) || self.cat.part_alias.contains_key(uid);
                        PendingBuild {
                            uid: uid.to_string(),
                            name: self.cat.name_of(uid),
                            done: f.done,
                            image: self.image_for(uid),
                            known,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        rows.sort_by(|a, b| a.done.unwrap_or(f64::MAX).total_cmp(&b.done.unwrap_or(f64::MAX)));
        rows
    }

    pub fn relic_row(&self, base: &str, rel: &Relic, needed: &HashMap<String, i64>) -> RelicRow {
        let counts: IndexMap<String, i64> = rel.uids.iter().map(|(r, uid)| (r.clone(), self.count(uid))).collect();
        let total: i64 = counts.values().sum();
        let mut ev: IndexMap<&str, f64> = IndexMap::from([("Intact", 0.0), ("Radiant", 0.0)]);
        let rewards: Vec<RelicRewardRow> = rel
            .rewards
            .iter()
            .map(|rw| {
                let p = self.cat.parts.get(&rw.uid);
                let slug = self.market.slug_for(
                    Some(&rw.uid),
                    rw.slug.as_deref().or_else(|| p.and_then(|p| p.slug.as_deref())),
                    Some(&rw.name),
                );
                let price = self.price(slug.as_deref(), total > 0);
                for (refinement, sum) in ev.iter_mut() {
                    if let (Some(Some(ch)), Some(price)) = (rw.chance.get(*refinement), price) {
                        if *ch != 0.0 {
                            *sum += ch / 100.0 * price as f64;
                        }
                    }
                }
                RelicRewardRow {
                    name: rw.name.clone(),
                    uid: rw.uid.clone(),
                    rarity: rw.rarity.clone(),
                    chance: rw.chance.clone(),
                    price,
                    ducats: self.market.ducats(slug.as_deref()),
                    image: self.market.image(slug.as_deref()),
                    have: self.count(&rw.uid),
                    needed: needed.get(&rw.uid).copied().unwrap_or(0),
                }
            })
            .collect();
        let needs = rewards.iter().filter(|r| r.needed > r.have).count();
        let relic_slug = self.market.slug_for(None, rel.slug.as_deref(), Some(&format!("{base} Relic")));
        let ev_radiant = py_round(ev["Radiant"], 1);
        let value = RelicView {
            name: base.to_string(),
            era: rel.era.clone(),
            vaulted: rel.vaulted,
            counts,
            image: gamedata::image_url(rel.image.as_deref()),
            total,
            uids: rel.uids.clone(),
            rewards,
            needs_count: needs,
            price: self.price(relic_slug.as_deref(), total > 0),
            slug: relic_slug,
            ev: ev.iter().map(|(k, x)| (k.to_string(), py_round(*x, 1))).collect(),
        };
        RelicRow { value, era: rel.era.clone(), total, needs, ev_radiant }
    }

    pub fn relics(&self, owned_only: bool) -> Vec<RelicView> {
        let needed = self.needed_parts();
        let mut out: Vec<(u8, String, RelicView)> = Vec::new();
        for (base, rel) in &self.cat.relics {
            let row = self.relic_row(base, rel, &needed);
            if owned_only && row.total == 0 {
                continue;
            }
            let era_rank = ERAS.iter().position(|e| *e == rel.era).unwrap_or(9) as u8;
            out.push((era_rank, base.clone(), row.value));
        }
        out.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        out.into_iter().map(|(_, _, row)| row).collect()
    }

    /// Per era, the relics owned and the one most worth cracking: the most drops still needed,
    /// then the highest radiant value.
    pub fn relic_picks(&self) -> IndexMap<String, RelicPick> {
        let needed = self.needed_parts();
        let mut per_era: IndexMap<String, (i64, usize, Option<RelicRow>)> = IndexMap::new();
        for (base, rel) in &self.cat.relics {
            let row = self.relic_row(base, rel, &needed);
            if row.total == 0 {
                continue;
            }
            let e = per_era.entry(row.era.clone()).or_insert((0, 0, None));
            e.0 += row.total;
            e.1 += 1;
            let better = e.2.as_ref().is_none_or(|b| (row.needs, row.ev_radiant) > (b.needs, b.ev_radiant));
            if better {
                e.2 = Some(row);
            }
        }
        per_era
            .into_iter()
            .map(|(era, (owned, kinds, best))| {
                let best = best.map(|b| RelicBest {
                    name: b.value.name,
                    needs: b.needs,
                    ev: b.ev_radiant,
                    image: b.value.image,
                    total: b.total,
                });
                (era, RelicPick { owned, kinds, best })
            })
            .collect()
    }

    /// Prime parts owned beyond what the unfinished sets still need, valued in platinum and ducats.
    pub fn trade(&self) -> Vec<TradeRow> {
        let needed = self.needed_parts();
        let mut rows: Vec<(i64, TradeRow)> = Vec::new();
        for (uid, p) in &self.cat.parts {
            // Relics also drop Kuva, Ayatan stars, Riven slivers, Forma and Requiem mods. They are
            // not prime parts and have no set to complete, so they do not belong here.
            if p.relics.is_empty() || !p.name.contains("Prime") {
                continue;
            }
            let have = self.count(uid);
            let spare = have - needed.get(uid).copied().unwrap_or(0);
            if have <= 0 || spare <= 0 {
                continue;
            }
            let slug = self.market.slug_for(Some(uid), p.slug.as_deref(), Some(&p.name));
            let price = self.price(slug.as_deref(), true);
            let ducats = self.market.ducats(slug.as_deref());
            let per_plat = match (ducats, price) {
                (Some(d), Some(pr)) if d != 0 && pr != 0 => Some(py_round(d as f64 / pr as f64, 1)),
                _ => None,
            };
            rows.push((
                -(price.unwrap_or(0) * spare),
                TradeRow {
                    uid: uid.clone(),
                    name: p.name.clone(),
                    have,
                    spare,
                    image: self.market.image(slug.as_deref()),
                    slug,
                    price,
                    ducats,
                    vaulted: p.vaulted,
                    ducats_per_plat: per_plat,
                },
            ));
        }
        rows.sort_by_key(|(key, _)| *key);
        rows.into_iter().map(|(_, r)| r).collect()
    }

    /// Catalog entries, drop names, relics and syndicates matching `q`, best matches first.
    pub fn find(&self, q: &str) -> Vec<FindHit> {
        let q = q.trim().to_lowercase();
        if q.chars().count() < 2 {
            return Vec::new();
        }
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let hit = |kind, key: String, name: String, sub: String, image| FindHit {
            kind,
            key,
            name,
            sub,
            image,
            status: None,
            price: None,
            have: None,
        };
        let mut hits: Vec<(u8, usize, FindHit)> = Vec::new();
        for it in self.cat.items.values() {
            if let Some(rank) = match_rank(&it.name, &q, &tokens) {
                let st = self.item_status(it);
                hits.push((
                    rank,
                    it.name.len(),
                    FindHit {
                        status: Some(st.status),
                        ..hit(
                            "item",
                            it.uid.clone(),
                            it.name.clone(),
                            it.category.clone(),
                            gamedata::image_url(it.image.as_deref()),
                        )
                    },
                ));
            }
        }
        for (uid, p) in &self.cat.parts {
            if p.relics.is_empty() {
                continue;
            }
            if let Some(rank) = match_rank(&p.name, &q, &tokens) {
                let slug = self.market.slug_for(Some(uid), p.slug.as_deref(), Some(&p.name));
                hits.push((
                    rank,
                    p.name.len(),
                    FindHit {
                        price: Some(self.price(slug.as_deref(), false)),
                        have: Some(self.count(uid)),
                        ..hit(
                            "part",
                            uid.clone(),
                            p.name.clone(),
                            if p.vaulted { "Prime part · vaulted" } else { "Prime part" }.to_string(),
                            self.market.image(slug.as_deref()),
                        )
                    },
                ));
            }
        }
        for (base, rel) in &self.cat.relics {
            if let Some(rank) = match_rank(&format!("{base} relic"), &q, &tokens) {
                let total: i64 = rel.uids.values().map(|u| self.count(u)).sum();
                hits.push((
                    rank,
                    base.len(),
                    FindHit {
                        have: Some(total),
                        ..hit(
                            "relic",
                            base.clone(),
                            format!("{base} Relic"),
                            if rel.vaulted { "Relic · vaulted" } else { "Relic" }.to_string(),
                            gamedata::image_url(rel.image.as_deref()),
                        )
                    },
                ));
            }
        }
        for s in self.cat.syndicates.list.values().filter(|s| s.kind != crate::standing::Kind::Nightwave) {
            if let Some(rank) = match_rank(&s.name, &q, &tokens) {
                hits.push((
                    rank,
                    s.name.len(),
                    hit(
                        "syndicate",
                        format!("syndicate:{}", s.tag),
                        s.name.clone(),
                        "Syndicate · standing and offerings".to_string(),
                        s.icon.clone(),
                    ),
                ));
            }
        }
        let mut represented: HashSet<String> = hits.iter().map(|(_, _, v)| v.name.to_lowercase()).collect();
        for item in self.cat.lookup.values().filter(|it| !self.cat.items.contains_key(&it.uid)) {
            if let Some(rank) = match_rank(&item.name, &q, &tokens) {
                if !represented.insert(item.name.to_lowercase()) {
                    continue;
                }
                hits.push((
                    rank,
                    item.name.len(),
                    hit(
                        "lookup",
                        format!("lookup:{}", item.uid),
                        item.name.clone(),
                        item.category.clone(),
                        gamedata::image_url(item.image.as_deref()),
                    ),
                ));
            }
        }
        for name in self.cat.drops.names() {
            if let Some(rank) = match_rank(name, &q, &tokens) {
                if !represented.insert(name.to_lowercase()) {
                    continue;
                }
                hits.push((
                    rank,
                    name.len(),
                    hit(
                        "drop",
                        format!("drop:{name}"),
                        name.to_string(),
                        "Drop sources".to_string(),
                        self.cat.art_by_name(name),
                    ),
                ));
            }
        }
        hits.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then_with(|| a.2.name.cmp(&b.2.name)));
        hits.into_iter().take(24).map(|(_, _, v)| v).collect()
    }

    /// Everything about one item, part or relic. `open_eras` are the relic eras with a fissure
    /// open right now ("Omnia" opens them all). A relic counts as open when one of its era's
    /// fissures is up and you can get it: it still drops, or you hold one (vaulted relics crack
    /// like any other).
    pub fn detail(&self, key: &str, open_eras: &HashSet<String>) -> Option<Detail> {
        if let Some(uid) = key.strip_prefix("lookup:") {
            let item = self.cat.lookup.get(uid)?;
            return Some(Detail::Drop(DropDetail {
                key: key.to_string(),
                name: item.name.clone(),
                image: gamedata::image_url(item.image.as_deref()),
                drops: self.drops_for(&item.name, false),
                drop_coverage: self.drop_coverage(),
            }));
        }
        if let Some(name) = key.strip_prefix("drop:") {
            let drops = self.drops_for(name, false);
            let first = drops.first()?.item.clone();
            return Some(Detail::Drop(DropDetail {
                key: format!("drop:{first}"),
                image: self.cat.art_by_name(&first),
                name: first,
                drops,
                drop_coverage: self.drop_coverage(),
            }));
        }
        let open = |era: &str| open_eras.contains(era) || open_eras.contains("Omnia");
        let needed = self.needed_parts();
        // Open fissures first, then relics that still drop, then the ones you hold.
        let relic_refs = |relics: &[gamedata::PartRelic]| -> Vec<RelicRef> {
            let mut refs: Vec<RelicRef> = relics
                .iter()
                .map(|r| {
                    let rel = self.cat.relics.get(&r.relic);
                    let owned: i64 = rel.map_or(0, |rel| rel.uids.values().map(|u| self.count(u)).sum());
                    let era = rel.map_or("", |rel| rel.era.as_str());
                    RelicRef {
                        relic: r.relic.clone(),
                        rarity: r.rarity.clone(),
                        chance: r.chance,
                        vaulted: r.vaulted,
                        owned,
                        open: open(era) && (!r.vaulted || owned > 0),
                    }
                })
                .collect();
            let key = |v: &RelicRef| (!v.open, v.vaulted, -v.owned);
            refs.sort_by(|a, b| {
                key(a).cmp(&key(b)).then_with(|| b.chance.unwrap_or(0.0).total_cmp(&a.chance.unwrap_or(0.0)))
            });
            refs
        };
        if let Some(it) = self.cat.items.get(key) {
            let st = self.item_status(it);
            let parts: Vec<DetailPart> = self
                .part_views(it, true)
                .into_iter()
                .map(|p| DetailPart {
                    relics: self.cat.parts.get(&p.uid).map(|part| relic_refs(&part.relics)).unwrap_or_default(),
                    drops: self.drops_for(&p.name, true),
                    recipe: self.part_recipe(&p.uid),
                    uid: p.uid,
                    name: p.name,
                    need: p.need,
                    have: p.have,
                    slug: p.slug,
                    price: p.price,
                    ducats: p.ducats,
                    vaulted: p.vaulted,
                    image: p.image,
                    crafting: None,
                    builds: None,
                    recipe_unknown: None,
                })
                .collect();
            let buildable = !parts.is_empty() && parts.iter().all(|p| p.have >= p.need);
            let set_slug = it
                .set_slug
                .as_ref()
                .and_then(|s| self.market.slug_for(None, Some(s), Some(&format!("{} Set", it.name))));
            let per = points_per_rank(it);
            return Some(Detail::Item(Box::new(ItemDetail {
                key: it.uid.clone(),
                uid: it.uid.clone(),
                name: it.name.clone(),
                category: it.category.clone(),
                kind: it.kind.clone(),
                image: gamedata::image_url(it.image.as_deref()),
                is_prime: it.is_prime,
                vaulted: it.vaulted,
                mastery_req: it.mastery_req,
                max_rank: it.max_rank,
                points: st.rank.min(it.max_rank) * per,
                max_points: it.max_rank * per,
                set_price: self.price(set_slug.as_deref(), true),
                set_slug,
                parts,
                buildable,
                recipes_available: self.cat.recipes_available,
                drops: self.drops_for(&it.name, false),
                drop_coverage: self.drop_coverage(),
                requires: it
                    .components
                    .iter()
                    .filter(|c| c.kind == ComponentKind::Item)
                    .map(|c| Required {
                        uid: c.uid.clone(),
                        name: c.name.clone(),
                        count: c.count,
                        status: self.cat.items.get(&c.uid).map(|i| self.item_status(i).status),
                        have: self.owned(&c.uid),
                    })
                    .collect(),
                resources: it
                    .components
                    .iter()
                    .filter(|c| c.kind == ComponentKind::Resource)
                    .map(|c| self.ingredient(&c.uid, &c.name, c.count))
                    .collect(),
                disposition: it.disposition.map(|d| py_round(d, 3)),
                dots: it.dispo_dots,
                status: st,
            })));
        }
        let canon = self.cat.part_alias.get(key).map_or(key, String::as_str);
        if let Some(p) = self.cat.parts.get(canon) {
            let slug = self.market.slug_for(Some(canon), p.slug.as_deref(), Some(&p.name));
            return Some(Detail::Part(PartDetail {
                key: canon.to_string(),
                uid: canon.to_string(),
                name: p.name.clone(),
                image: self.image_for(canon),
                price: self.price(slug.as_deref(), true),
                ducats: self.market.ducats(slug.as_deref()),
                slug,
                vaulted: p.vaulted,
                have: self.count(canon),
                needed: needed.get(canon).copied().unwrap_or(0),
                relics: relic_refs(&p.relics),
                drops: self.drops_for(&p.name, true),
                recipe: self.part_recipe(canon),
                recipes_available: self.cat.recipes_available,
                drop_coverage: self.drop_coverage(),
                used_in: p
                    .items
                    .iter()
                    .filter_map(|u| self.cat.items.get(u))
                    .map(|it| UsedIn {
                        uid: it.uid.clone(),
                        name: it.name.clone(),
                        image: gamedata::image_url(it.image.as_deref()),
                        status: self.item_status(it).status,
                    })
                    .collect(),
            }));
        }
        // A relic by name ("Lith A1", "Lith A1 Relic"), or by the game's id of one of its
        // refinements, as the Inventory page lists relics.
        let base = match self.cat.relic_uids.get(key) {
            Some((base, _)) => base.as_str(),
            None => key.trim_end_matches(" Relic"),
        };
        let rel = self.cat.relics.get(base)?;
        let row = self.relic_row(base, rel, &needed);
        Some(Detail::Relic(RelicDetail {
            open: open(&rel.era) && (!rel.vaulted || row.total > 0),
            relic: row.value,
            key: base.to_string(),
            drops: self.drops_for(&format!("{base} Relic"), false),
            drop_coverage: self.drop_coverage(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    /// An answer as the page receives it.
    fn json<T: serde::Serialize>(v: T) -> Value {
        serde_json::to_value(v).unwrap()
    }

    #[test]
    fn item_part_and_relic_details_expose_their_drop_sources() {
        let tables = json!({"missionRewards": {"Venus": {"Fossa": {"gameMode": "Assassination",
            "rewards": [{"itemName": "Rhino Chassis Blueprint", "chance": 38.72},
                        {"itemName": "Vitality", "chance": 8.0},
                        {"itemName": "Lith A1 Relic", "chance": 5.0}]}}}});
        let mut cat = gamedata::build(&IndexMap::from([
            (
                "Warframes".to_string(),
                json!([{"uniqueName": "/Rhino", "name": "Rhino", "imageName": "rhino.png", "masterable": true,
                "components": [{"uniqueName": "/Lotus/Types/Recipes/WarframeRecipes/RhinoChassisComponent"}]}]),
            ),
            ("Relics".to_string(), json!([{"uniqueName": "/LithA1", "name": "Lith A1 Intact", "vaulted": false}])),
            (
                "Mods".to_string(),
                json!([
                    {"uniqueName": "/Vitality", "name": "Vitality", "imageName": "vitality.png"},
                    {"uniqueName": "/VendorMod", "name": "Vendor Mod"},
                    {"uniqueName": "/Variant", "name": "Vitality"}
                ]),
            ),
            (
                "Resources".to_string(),
                json!([
                    {"uniqueName": "/Cell", "name": "Orokin Cell"},
                    {"uniqueName": "/Rubedo", "name": "Rubedo"}
                ]),
            ),
            (
                "ExportRecipes".to_string(),
                json!({
                    "/Lotus/Types/Recipes/WarframeRecipes/RhinoChassisBlueprint": {
                        "resultType": "/Lotus/Types/Recipes/WarframeRecipes/RhinoChassisComponent",
                        "num": 1, "buildPrice": 15000, "buildTime": 43200,
                        "ingredients": [{"ItemType": "/Rubedo", "ItemCount": 400}]
                    }
                }),
            ),
        ]));
        cat.drops = crate::drops::Drops::from_tables(&[("missionRewards", &tables)], 2);
        let mut view = View {
            cat: Arc::new(cat),
            profile: None,
            inv: None,
            manual: Arc::new(Manual::default()),
            market: crate::market::Market::for_tests(),
            foundry: HashSet::new(),
        };
        let mastery = json(view.mastery());
        let rhino = mastery.as_array().unwrap().iter().find(|r| r["uid"] == "/Rhino").unwrap();
        assert_eq!(rhino["parts"][0]["need"], 1);
        let item = view.detail("/Rhino", &HashSet::new()).map(json).unwrap();
        assert!(rhino["parts"][0]["image"].is_string());
        assert_eq!(rhino["parts"][0]["image"], item["parts"][0]["image"]);
        assert_eq!(item["dropCoverage"], json!({"available": 1, "total": 2}));
        assert_eq!(item["parts"][0]["drops"][0]["item"], "Rhino Chassis Blueprint");
        assert_eq!(item["parts"][0]["drops"][0]["chance"], 38.72);
        assert_eq!(item["recipesAvailable"], true);
        assert_eq!(item["parts"][0]["recipe"]["ingredients"][0]["name"], "Rubedo");
        assert_eq!(item["parts"][0]["recipe"]["ingredients"][0]["count"], 400);
        assert!(item["parts"][0]["recipe"]["ingredients"][0]["have"].is_null());
        assert_eq!(item["parts"][0]["recipe"]["seconds"], 43200);
        let uid = item["parts"][0]["uid"].as_str().unwrap();
        let part = view.detail(uid, &HashSet::new()).map(json).unwrap();
        assert_eq!(part["drops"][0]["location"], "Venus · Fossa");
        assert_eq!(part["recipe"], item["parts"][0]["recipe"]);
        Arc::make_mut(&mut view.manual).counts.insert("/Rubedo".into(), (250, 1.0));
        Arc::make_mut(&mut view.manual).counts.insert(uid.into(), (1, 1.0));
        let held = view.detail("/Rhino", &HashSet::new()).map(json).unwrap();
        assert_eq!(held["parts"][0]["have"], 1);
        assert_eq!(held["parts"][0]["recipe"]["ingredients"][0]["have"], 250);
        assert_eq!(held["parts"][0]["recipe"]["ingredients"][0]["count"], 400);
        let relic = view.detail("Lith A1", &HashSet::new()).map(json).unwrap();
        // The Inventory page asks by the relic's id.
        assert_eq!(view.detail("/LithA1", &HashSet::new()).map(json).unwrap()["name"], relic["name"]);
        assert_eq!(relic["drops"][0]["chance"], 5.0);
        let hits = json(view.find("vitality"));
        assert_eq!(hits.as_array().unwrap().iter().filter(|hit| hit["name"] == "Vitality").count(), 1);
        assert_eq!(hits[0]["key"], "lookup:/Vitality");
        let lookup = view.detail("lookup:/Vitality", &HashSet::new()).map(json).unwrap();
        assert_eq!(lookup["drops"][0]["chance"], 8.0);
        assert!(json(view.find("vendor mod")).as_array().unwrap().iter().any(|hit| hit["key"] == "lookup:/VendorMod"));
        assert_eq!(view.detail("lookup:/VendorMod", &HashSet::new()).map(json).unwrap()["drops"], json!([]));
        assert!(json(view.find("orokin cell")).as_array().unwrap().iter().any(|hit| hit["key"] == "lookup:/Cell"));
        let mod_detail = view.detail("drop:Vitality", &HashSet::new()).map(json).unwrap();
        assert_eq!(mod_detail["kind"], "drop");
        assert_eq!(mod_detail["drops"][0]["chance"], 8.0);
        assert!(view.detail("drop:unknown", &HashSet::new()).map(json).is_none());
    }

    #[test]
    fn a_change_after_a_count_set_by_hand_goes_on_top_of_it() {
        let cat = Catalog::default();
        let mut manual = Manual::default();
        manual.counts.insert("/Cell".into(), (4, 120.0));
        let change = |t, change| crate::sources::Delta {
            t,
            kind: "pick".into(),
            uid: "/Cell".into(),
            change,
            equipment: false,
            reward: None,
        };
        assert_eq!(hand_counts_after(&cat, &manual, &[change(200.0, 1)]).counts["/Cell"], (5, 120.0));
        assert_eq!(hand_counts_after(&cat, &manual, &[change(100.0, 1)]).counts["/Cell"], (4, 120.0), "set after it");
        assert_eq!(hand_counts_after(&cat, &manual, &[change(200.0, -9)]).counts["/Cell"].0, 0, "never below zero");
    }

    #[test]
    fn mastery_rank_thresholds() {
        assert_eq!(rank_for(0), (0, 0, 2500));
        assert_eq!(rank_for(2499), (0, 0, 2500));
        assert_eq!(rank_for(2500), (1, 2500, 10_000));
        assert_eq!(rank_for(1_960_000), (28, 1_960_000, 2_102_500));
        assert_eq!(rank_for(2_250_000), (30, 2_250_000, 2_397_500));
        assert_eq!(rank_for(2_397_500), (31, 2_397_500, 2_545_000), "Legendary 1");
    }

    #[test]
    fn mastery_outside_the_arsenal() {
        let cat = Catalog {
            node_xp: gamedata::node_xp_table(&json!({
                "SolNode27": {"masteryExp": 63}, "SolNode1": {"masteryExp": 24},
                "SolNode94": {"masteryExp": 0}, "EarthToVenusJunction": {"masteryExp": 0}})),
            ..Catalog::default()
        };
        assert_eq!(cat.node_xp.len(), 2, "only nodes that give XP are kept");
        let progress = Progress {
            // SolNode1 and the junction were also done on the Steel Path; SolNode94 gives nothing;
            // an event node is not in the table.
            nodes: IndexMap::from(
                [("SolNode27", 0), ("SolNode1", 1), ("SolNode94", 0), ("EarthToVenusJunction", 1), ("EventNode25", 0)]
                    .map(|(tag, tier)| (tag.to_string(), tier)),
            ),
            intrinsics: IndexMap::from(
                [("LPS_PILOTING", 7), ("LPS_COMMAND", 4), ("LPS_DRIFT_RIDING", 2)].map(|(k, v)| (k.to_string(), v)),
            ),
        };
        let m = other_mastery(&cat, &progress);
        assert_eq!(
            m,
            OtherMastery { star_chart: 63 + 24 + 1000, steel_path: 24 + 1000, railjack: 16_500, drifter: 3000 }
        );
        assert_eq!(m.total(), 21_611);
    }

    #[test]
    fn search_matches_word_starts() {
        let tokens = ["ash", "sys"];
        assert_eq!(match_rank("Ash Prime Systems Blueprint", "ash sys", &tokens), Some(2));
        assert_eq!(match_rank("Ash Prime", "ash", &["ash"]), Some(1));
        assert_eq!(match_rank("Ash", "ash", &["ash"]), Some(0));
        assert_eq!(match_rank("Trash Prime", "ash", &["ash"]), Some(3));
        assert_eq!(match_rank("Braton", "ash", &["ash"]), None);
    }

    #[test]
    fn cyclic_recipes_do_not_overflow_stack() {
        let mut cat = Catalog::default();
        cat.inventory.insert(
            "/SelfBp".into(),
            crate::inventory::Entry {
                name: None,
                icon: None,
                kind: "Blueprints",
                result: Some("/SelfBp".into()),
                rarity: None,
                rank: None,
                hidden: false,
            },
        );
        cat.inventory.insert(
            "/CycleA".into(),
            crate::inventory::Entry {
                name: None,
                icon: None,
                kind: "Blueprints",
                result: Some("/CycleB".into()),
                rarity: None,
                rank: None,
                hidden: false,
            },
        );
        cat.inventory.insert(
            "/CycleB".into(),
            crate::inventory::Entry {
                name: None,
                icon: None,
                kind: "Blueprints",
                result: Some("/CycleA".into()),
                rarity: None,
                rank: None,
                hidden: false,
            },
        );
        let view = View {
            cat: Arc::new(cat),
            profile: None,
            inv: None,
            manual: Arc::new(Manual::default()),
            market: crate::market::Market::for_tests(),
            foundry: HashSet::new(),
        };
        let (self_name, _, _) = view.inventory_entry("/SelfBp");
        assert!(!self_name.is_empty());
        let (cycle_name, _, _) = view.inventory_entry("/CycleA");
        assert!(cycle_name.ends_with("Blueprint"));
    }
}
