//! The farming board: what each task is about, how far along it is and what it still needs.
//!
//! A task names equipment, a mod, an arcane or a resource, and the board colours it:
//! * `missing` (red): parts are missing - or copies, for a mod, an arcane or a resource;
//! * `resources` (yellow): every part is there, but not the resources or credits to build it,
//!   or a part still to craft has a recipe neither export knows;
//! * `done` (green): owned, building, or everything to build it is there.
//!
//! Parts are counted the way the rest of the app counts them, blueprint and crafted part
//! together. What the builds still consume leaves out the parts already crafted or crafting in
//! the foundry, which the inventory tells apart.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};

use indexmap::IndexMap;
use serde_json::{Value, json};

use crate::accounts::Task;
use crate::gamedata::{self, ComponentKind, Item, PartRecipe};
use crate::views::{View, match_rank};

/// What a task can be about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Masterable equipment, built from parts and resources.
    Item,
    Mod,
    /// Copies of an arcane fuse into ranks: rank r takes (r + 1)(r + 2) / 2 of them.
    Arcane,
    /// Anything a build consumes, counted.
    Resource,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Item => "item",
            Kind::Mod => "mod",
            Kind::Arcane => "arcane",
            Kind::Resource => "resource",
        }
    }
}

/// What the catalog knows about the thing a task is about.
pub struct Thing {
    pub kind: Kind,
    pub name: String,
    pub image: Option<String>,
    /// The line above the name on the board: "Warframes", "Mod · Rare", "Resource".
    pub sub: String,
    /// A mod's or arcane's top rank.
    pub top_rank: Option<i64>,
}

/// The equipment, mod, arcane or resource a uid names; `None` for anything else (a part, a
/// relic) and for what the loaded data does not know.
pub fn thing(view: &View, uid: &str) -> Option<Thing> {
    let cat = &view.cat;
    if let Some(it) = cat.items.get(uid) {
        return Some(Thing {
            kind: Kind::Item,
            name: it.name.clone(),
            image: gamedata::image_url(it.image.as_deref()),
            sub: it.category.clone(),
            top_rank: None,
        });
    }
    if let Some(e) = cat.inventory.get(uid).filter(|e| matches!(e.kind, "Mods" | "Arcanes") && !e.hidden) {
        let kind = if e.kind == "Arcanes" { Kind::Arcane } else { Kind::Mod };
        let label = if kind == Kind::Arcane { "Arcane" } else { "Mod" };
        return Some(Thing {
            kind,
            name: e.name.clone().or_else(|| cat.lookup.get(uid).map(|l| l.name.clone()))?,
            image: e.icon.clone(),
            sub: e.rarity.as_ref().map_or_else(|| label.to_string(), |r| format!("{label} · {r}")),
            top_rank: e.rank,
        });
    }
    // WFCD's mod list, for when the game's export could not be loaded. Its pictures of mods are
    // not on its image host.
    if let Some(l) = cat.lookup.get(uid).filter(|l| l.category == "Mods") {
        return Some(Thing { kind: Kind::Mod, name: l.name.clone(), image: None, sub: "Mod".into(), top_rank: None });
    }
    let name = resource_name(view, uid)?;
    Some(Thing { kind: Kind::Resource, image: view.icon_of(uid, &name), name, sub: "Resource".into(), top_rank: None })
}

/// A resource's name: WFCD's, which the drop tables use too, else the game's own (WFCD lacks
/// some fish and gems), else the one made from its uid.
fn resource_name(view: &View, uid: &str) -> Option<String> {
    let cat = &view.cat;
    let made = cat.resources.get(uid)?;
    let own = cat.inventory.get(uid).and_then(|e| e.name.clone());
    Some(cat.names.get(uid).cloned().or(own).unwrap_or_else(|| made.clone()))
}

/// The game keeps other copies of some mods under the same name: Flawed ones in `/Beginner/`,
/// more in `/Intermediate/`, and in `/Expert/`, beside the Primed mods (which have names of their
/// own), unreleased ones. The ordinary one is meant.
fn variant(uid: &str) -> bool {
    ["/Beginner/", "/Intermediate/", "/Expert/"].iter().any(|v| uid.contains(v))
}

/// Copies an arcane of rank `rank` was fused from: 1, 3, 6, 10, 15, 21.
pub fn arcane_copies(rank: i64) -> i64 {
    let r = rank.max(0);
    (r + 1) * (r + 2) / 2
}

/// The rank `copies` fuse to, up to `top`; `None` without a copy.
fn arcane_rank(copies: i64, top: i64) -> Option<i64> {
    (0..=top.max(0)).rev().find(|r| arcane_copies(*r) <= copies)
}

/// How many a count task asks for unless set: one copy, or for an arcane its top rank.
fn default_target(t: &Thing) -> i64 {
    match (t.kind, t.top_rank) {
        (Kind::Arcane, Some(top)) => arcane_copies(top),
        _ => 1,
    }
}

/// 15000 -> "15,000", as the page writes numbers.
fn grouped(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

// ------------------------------------------------------------------ copies held

/// How many copies of a mod, an arcane or a resource the account holds.
struct Held {
    /// For an arcane, a ranked copy counts as the copies fused into it.
    count: i64,
    /// The rank of each ranked copy, when the inventory says.
    ranks: Option<Vec<i64>>,
    /// A count set by hand, newer than the inventory, stands for all of it.
    by_hand: bool,
}

fn held(view: &View, uid: &str, kind: Kind) -> Held {
    if let Some(n) = view.hand_count(uid) {
        return Held { count: n, ranks: None, by_hand: true };
    }
    let ranks: Option<Vec<i64>> = match kind {
        Kind::Mod | Kind::Arcane => {
            view.inv.as_ref().and_then(|i| i.upgrades.as_ref()).map(|u| u.get(uid).cloned().unwrap_or_default())
        }
        _ => None,
    };
    let ranked: i64 = ranks.iter().flatten().map(|r| if kind == Kind::Arcane { arcane_copies(*r) } else { 1 }).sum();
    Held { count: view.held_stack(uid) + ranked, ranks, by_hand: false }
}

// ------------------------------------------------------------------ what equipment needs

/// One part of an equipment task.
struct PlanPart {
    uid: String,
    name: String,
    need: i64,
    /// Held, blueprint and crafted part together, plus any crafting in the foundry.
    have: i64,
    /// Crafting in the foundry now.
    crafting: i64,
    /// Blueprint-to-part builds still to do.
    builds: i64,
    /// Still to craft, but its recipe is in neither export: what that costs is not known.
    recipe_unknown: bool,
}

/// Other equipment the build consumes (the Lex Primes of an Aklex Prime).
struct Required {
    name: String,
    count: i64,
    owned: Option<i64>,
}

/// What an equipment task still needs.
struct Plan {
    parts: Vec<PlanPart>,
    requires: Vec<Required>,
    /// Resources the builds still consume: uid -> (name, count).
    materials: IndexMap<String, (String, i64)>,
    credits: i64,
}

/// The recipe that builds the item itself, from its main blueprint.
fn final_recipe<'a>(view: &'a View, it: &Item) -> Option<&'a PartRecipe> {
    it.components.iter().filter(|c| c.kind == ComponentKind::Part).find_map(|c| {
        let alt = view.cat.parts.get(&c.uid).and_then(|p| p.alt.as_deref());
        [Some(c.uid.as_str()), alt]
            .into_iter()
            .flatten()
            .filter_map(|u| view.cat.part_recipes.get(u))
            .find(|r| r.result == it.uid)
    })
}

/// The crafted half of a part that drops as a blueprint: of a part and its twin, the one named
/// "...Component" (`gamedata::alt_uid`). A prime weapon part drops ready-made and has none.
fn crafted_half<'a>(view: &'a View, uid: &'a str) -> Option<&'a str> {
    let twin = view.cat.parts.get(uid).and_then(|p| p.alt.as_deref());
    [Some(uid), twin].into_iter().flatten().find(|u| u.ends_with("Component"))
}

fn plan(view: &View, it: &Item) -> Plan {
    // Blueprints in the foundry: gone from the inventory, their part is on the way.
    let mut pending: HashMap<&str, i64> = HashMap::new();
    for f in view.inv.iter().flat_map(|i| &i.foundry) {
        if let Some(r) = f.recipe.as_deref() {
            *pending.entry(r).or_insert(0) += 1;
        }
    }
    let mut materials: IndexMap<String, (String, i64)> = IndexMap::new();
    let mut add = |uid: &str, name: &str, n: i64| {
        if n > 0 {
            materials.entry(uid.to_string()).or_insert_with(|| (name.to_string(), 0)).1 += n;
        }
    };
    let mut credits = 0;
    let mut parts = Vec::new();
    let main_blueprint = format!("{} Blueprint", it.name);
    for c in it.components.iter().filter(|c| c.kind == ComponentKind::Part) {
        let alt = view.cat.parts.get(&c.uid).and_then(|p| p.alt.as_deref());
        let crafting: i64 =
            [Some(c.uid.as_str()), alt].into_iter().flatten().map(|u| pending.get(u).copied().unwrap_or(0)).sum();
        // Only the inventory tells crafted parts from blueprints; a count set by hand does not,
        // so then every part is taken to need crafting.
        let crafted = |result: &str| {
            let held = if view.hand_count(&c.uid).is_some() { 0 } else { view.held_stack(result) };
            held + crafting
        };
        let mut builds = 0;
        let mut recipe_unknown = false;
        if let Some(r) = view.recipe_for_part(&c.uid) {
            let left = (c.count - crafted(&r.result)).max(0);
            builds = (left + r.quantity - 1) / r.quantity;
            for need in &r.ingredients {
                add(&need.uid, &need.name, need.count * builds);
            }
            credits += r.credits.unwrap_or(0) * builds;
        } else if let Some(component) = crafted_half(view, &c.uid).filter(|_| !c.name.ends_with(&main_blueprint)) {
            // A part that drops as a blueprint is crafted before the build, and without its
            // recipe nobody can say what that takes: the task must not read "ready". (The main
            // blueprint is the build itself; the twin its uid suggests does not exist.)
            recipe_unknown = crafted(component) < c.count;
        }
        parts.push(PlanPart {
            uid: c.uid.clone(),
            name: c.name.clone(),
            need: c.count,
            have: view.count(&c.uid) + crafting,
            crafting,
            builds,
            recipe_unknown,
        });
    }
    for c in it.components.iter().filter(|c| c.kind == ComponentKind::Resource) {
        add(&c.uid, &c.name, c.count);
    }
    credits += final_recipe(view, it).and_then(|r| r.credits).unwrap_or(0);
    materials.sort_by(|_, a, _, b| a.0.cmp(&b.0));
    let requires = it
        .components
        .iter()
        .filter(|c| c.kind == ComponentKind::Item)
        .map(|c| Required { name: c.name.clone(), count: c.count, owned: view.owned(&c.uid) })
        .collect();
    Plan { parts, requires, materials, credits }
}

fn credits_held(view: &View) -> Option<i64> {
    view.inv.as_ref().and_then(|i| i.currencies.credits)
}

/// What the builds need more of than the account holds, by name. A resource no source has
/// counted reads as none held.
fn shortages(view: &View, plan: &Plan) -> Vec<String> {
    let mut short: Vec<String> =
        plan.materials.iter().filter(|(uid, (_, n))| view.count(uid) < *n).map(|(_, (name, _))| name.clone()).collect();
    if credits_held(view).is_some_and(|c| c < plan.credits) {
        short.push("Credits".to_string());
    }
    short
}

/// "Volt Prime Neuroptics" on Volt Prime's card is "Neuroptics".
fn short_name<'a>(item: &str, part: &'a str) -> &'a str {
    part.strip_prefix(item).map(str::trim_start).filter(|s| !s.is_empty()).unwrap_or(part)
}

// ------------------------------------------------------------------ how far a task is

/// A task's colour, the line under its name and its bar.
struct Progress {
    state: &'static str,
    note: String,
    have: i64,
    need: i64,
}

fn item_progress(view: &View, it: &Item, plan: &Plan) -> Progress {
    let st = view.item_status(it);
    let total: i64 =
        plan.parts.iter().map(|p| p.need).sum::<i64>() + plan.requires.iter().map(|r| r.count).sum::<i64>();
    let have: i64 = plan.parts.iter().map(|p| p.have.min(p.need)).sum::<i64>()
        + plan.requires.iter().map(|r| r.owned.unwrap_or(0).min(r.count)).sum::<i64>();
    let done = |note: &str| Progress { state: "done", note: note.to_string(), have: 1, need: 1 };
    if st.owned.unwrap_or(0) > 0 {
        return done("Owned");
    }
    if st.building {
        return done("Building in the foundry");
    }
    if total == 0 {
        return Progress { state: "missing", note: "Not owned".into(), have: 0, need: 1 };
    }
    let missing: Vec<&str> = plan
        .parts
        .iter()
        .filter(|p| p.have < p.need)
        .map(|p| short_name(&it.name, &p.name))
        .chain(plan.requires.iter().filter(|r| r.owned.unwrap_or(0) < r.count).map(|r| r.name.as_str()))
        .collect();
    if let [one] = missing.as_slice() {
        return Progress { state: "missing", note: format!("Missing {one}"), have, need: total };
    }
    if !missing.is_empty() {
        return Progress {
            state: "missing",
            note: format!("Missing {} of {total} parts", total - have),
            have,
            need: total,
        };
    }
    let short = shortages(view, plan);
    if !short.is_empty() {
        let need = plan.materials.len() as i64 + i64::from(plan.credits > 0);
        let note = match short.as_slice() {
            [one] => format!("Short of {one}"),
            many => format!("Short of {} resources", many.len()),
        };
        return Progress { state: "resources", note, have: (need - short.len() as i64).max(0), need: need.max(1) };
    }
    // Every known cost is covered, but a part still to craft has no known recipe.
    let unknown: Vec<&str> =
        plan.parts.iter().filter(|p| p.recipe_unknown).map(|p| short_name(&it.name, &p.name)).collect();
    if !unknown.is_empty() {
        let note = match unknown.as_slice() {
            [one] => format!("{one} recipe unknown"),
            many => format!("Recipes of {} parts unknown", many.len()),
        };
        let known = plan.materials.len() as i64 + i64::from(plan.credits > 0);
        return Progress { state: "resources", note, have: known, need: known + unknown.len() as i64 };
    }
    if plan.parts.iter().any(|p| p.crafting > 0) {
        return done("Ready once its parts are crafted");
    }
    done("Ready to build")
}

fn count_progress(kind: Kind, held: &Held, target: i64) -> Progress {
    let note = match kind {
        Kind::Mod | Kind::Arcane if target == 1 => (if held.count > 0 { "Owned" } else { "Not owned" }).to_string(),
        Kind::Mod | Kind::Arcane => format!("{} of {} copies", grouped(held.count), grouped(target)),
        _ => format!("{} of {}", grouped(held.count), grouped(target)),
    };
    let state = if held.count >= target { "done" } else { "missing" };
    Progress { state, note, have: held.count.min(target), need: target }
}

/// A task as the board shows it.
pub fn row(view: &View, task: &Task) -> Value {
    let Some(t) = thing(view, &task.uid) else {
        return json!({"uid": task.uid, "kind": "unknown", "name": view.cat.name_of(&task.uid), "image": null,
                      "sub": "Unknown", "state": "missing", "note": "Not in the loaded game data", "have": 0, "need": 1});
    };
    let p = match (t.kind, view.cat.items.get(&task.uid)) {
        (Kind::Item, Some(it)) => item_progress(view, it, &plan(view, it)),
        _ => count_progress(t.kind, &held(view, &task.uid, t.kind), task.target.unwrap_or_else(|| default_target(&t))),
    };
    json!({"uid": task.uid, "kind": t.kind.name(), "name": t.name, "image": t.image, "sub": t.sub,
           "state": p.state, "note": p.note, "have": p.have, "need": p.need})
}

fn signature(rows: &[Value]) -> String {
    let mut h = DefaultHasher::new();
    for r in rows {
        r.to_string().hash(&mut h);
    }
    format!("{:016x}", h.finish())
}

/// The board of the account shown, in the order the tasks were added. `sig` changes whenever
/// a card would.
pub fn board(view: &View) -> Value {
    let rows: Vec<Value> = view.manual.tasks.iter().map(|t| row(view, t)).collect();
    let done = rows.iter().filter(|r| r["state"] == "done").count();
    json!({"tasks": rows, "done": done, "sig": signature(&rows), "inventoryKnown": view.inv.is_some(),
           "ranksKnown": view.inv.as_ref().is_some_and(|i| i.upgrades.is_some())})
}

/// For the sidebar: how many tasks there are and how many are done, and the board's `sig`.
pub fn summary(view: &View) -> Value {
    let rows: Vec<Value> = view.manual.tasks.iter().map(|t| row(view, t)).collect();
    let done = rows.iter().filter(|r| r["state"] == "done").count();
    json!({"total": rows.len(), "done": done, "sig": signature(&rows)})
}

/// The uids of the tasks that are done.
pub fn done(view: &View) -> HashSet<String> {
    view.manual.tasks.iter().filter(|t| row(view, t)["state"] == "done").map(|t| t.uid.clone()).collect()
}

/// Everything the details panel shows for a task, on the board or not. `open_eras` as for
/// `View::detail`.
pub fn detail(view: &View, uid: &str, open_eras: &HashSet<String>) -> Option<Value> {
    let t = thing(view, uid)?;
    let on_board = view.manual.tasks.iter().find(|x| x.uid == uid).cloned();
    let task = on_board.clone().unwrap_or_else(|| Task { uid: uid.to_string(), ..Task::default() });
    let mut v = row(view, &task);
    v["onBoard"] = json!(on_board.is_some());
    v["inventoryKnown"] = json!(view.inv.is_some());
    if let (Kind::Item, Some(it)) = (t.kind, view.cat.items.get(uid)) {
        let plan = plan(view, it);
        let mut item = view.detail(uid, open_eras)?;
        for p in item["parts"].as_array_mut().into_iter().flatten() {
            if let Some(pp) = plan.parts.iter().find(|pp| p["uid"] == pp.uid.as_str()) {
                p["have"] = json!(pp.have);
                p["crafting"] = json!(pp.crafting);
                p["builds"] = json!(pp.builds);
                p["recipeUnknown"] = json!(pp.recipe_unknown);
            }
        }
        v["item"] = item;
        let known = |uid: &str| view.inv.is_some() || view.manual.counts.contains_key(uid);
        v["materials"] = plan
            .materials
            .iter()
            .map(|(uid, (name, count))| {
                json!({"uid": uid, "name": name, "count": count,
                       "have": if known(uid) { Some(view.count(uid)) } else { None },
                       "image": view.icon_of(uid, name), "drops": view.cat.drops.for_name(name, false)})
            })
            .collect();
        v["credits"] = json!({"need": plan.credits, "have": credits_held(view)});
        return Some(v);
    }
    let held = held(view, uid, t.kind);
    let target = task.target.unwrap_or_else(|| default_target(&t));
    v["held"] = json!({"count": held.count, "ranks": held.ranks, "byHand": held.by_hand});
    v["target"] = json!(target);
    v["targetDefault"] = json!(default_target(&t));
    v["topRank"] = json!(t.top_rank);
    if t.kind == Kind::Arcane {
        v["rank"] = json!(arcane_rank(held.count, t.top_rank.unwrap_or(0)));
    }
    v["drops"] = json!(view.cat.drops.for_name(&t.name, false));
    v["dropCoverage"] = json!({"available": view.cat.drops.available, "total": view.cat.drops.total});
    Some(v)
}

// ------------------------------------------------------------------ finding things to add

/// The task uid for something known only by its name, as a drop table writes it: a mod, an
/// arcane or a resource of exactly that name.
pub fn uid_by_name(view: &View, name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let mut best: Option<(bool, &str)> = None;
    for (uid, e) in &view.cat.inventory {
        if matches!(e.kind, "Mods" | "Arcanes")
            && !e.hidden
            && e.name.as_deref().is_some_and(|n| n.to_lowercase() == lower)
        {
            let key = (variant(uid), uid.as_str());
            if best.is_none_or(|b| key < b) {
                best = Some(key);
            }
        }
    }
    if let Some((_, uid)) = best {
        return Some(uid.to_string());
    }
    let resource =
        view.cat.resources.keys().find(|uid| resource_name(view, uid).is_some_and(|n| n.to_lowercase() == lower));
    resource.cloned()
}

/// Equipment, mods, arcanes and resources matching `q`, best matches first, for adding to the
/// board. A mod or arcane name the game uses more than once is offered once.
pub fn find(view: &View, q: &str) -> Value {
    let q = q.trim().to_lowercase();
    if q.chars().count() < 2 {
        return json!([]);
    }
    let tokens: Vec<&str> = q.split_whitespace().collect();
    let cat = &view.cat;
    let mut found: Vec<(u8, &str)> = Vec::new();
    for it in cat.items.values() {
        if let Some(rank) = match_rank(&it.name, &q, &tokens) {
            found.push((rank, it.uid.as_str()));
        }
    }
    // One uid per mod or arcane name: the ordinary copy, then the first by uid.
    let mut by_name: HashMap<String, (u8, bool, &str)> = HashMap::new();
    let export_mods = cat.inventory.values().any(|e| e.kind == "Mods");
    let named = cat
        .inventory
        .iter()
        .filter(|(_, e)| matches!(e.kind, "Mods" | "Arcanes") && !e.hidden)
        .filter_map(|(uid, e)| Some((uid, e.name.as_deref()?)))
        .chain(cat.lookup.values().filter(|l| !export_mods && l.category == "Mods").map(|l| (&l.uid, l.name.as_str())));
    for (uid, name) in named {
        let Some(rank) = match_rank(name, &q, &tokens) else { continue };
        let key = (rank, variant(uid), uid.as_str());
        let slot = by_name.entry(name.to_lowercase()).or_insert(key);
        if (key.1, key.2) < (slot.1, slot.2) {
            *slot = key;
        }
    }
    found.extend(by_name.into_values().map(|(rank, _, uid)| (rank, uid)));
    for uid in cat.resources.keys() {
        if let Some(rank) = resource_name(view, uid).and_then(|n| match_rank(&n, &q, &tokens)) {
            found.push((rank, uid.as_str()));
        }
    }
    // A resource could also be a mod; each uid once.
    let mut seen = HashSet::new();
    found.retain(|(_, uid)| seen.insert(*uid));
    let mut hits: Vec<(u8, Value)> = found
        .into_iter()
        .filter_map(|(rank, uid)| {
            let t = thing(view, uid)?;
            Some((
                rank,
                json!({"key": uid, "kind": t.kind.name(), "name": t.name, "sub": t.sub, "image": t.image,
                               "added": view.manual.has_task(uid)}),
            ))
        })
        .collect();
    let name_of = |v: &Value| v["name"].as_str().unwrap_or("").to_string();
    hits.sort_by(|a, b| {
        let (na, nb) = (name_of(&a.1), name_of(&b.1));
        (a.0, na.len(), &na).cmp(&(b.0, nb.len(), &nb))
    });
    Value::Array(hits.into_iter().take(30).map(|(_, v)| v).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::accounts::Manual;
    use crate::inventory::Entry;
    use crate::market::Market;
    use crate::sources::{FoundryEntry, Inventory};

    const ASH: &str = "/Lotus/Powersuits/Ninja/AshPrime";
    const BP: &str = "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeBlueprint";
    const CHASSIS_BP: &str = "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisBlueprint";
    const CHASSIS: &str = "/Lotus/Types/Recipes/WarframeRecipes/AshPrimeChassisComponent";
    const CELL: &str = "/Lotus/Types/Items/MiscItems/OrokinCell";
    const RUBEDO: &str = "/Lotus/Types/Items/MiscItems/Rubedo";
    const ENERGIZE: &str = "/Lotus/Upgrades/CosmeticEnhancers/Utility/EnergyOnKill";
    const FIBER: &str = "/Lotus/Upgrades/Mods/Warframe/AvatarArmourMod";
    const FLAWED: &str = "/Lotus/Upgrades/Mods/Warframe/Beginner/AvatarArmourModBeginner";

    /// Ash Prime from its blueprint, a chassis (crafted from a blueprint with Rubedo) and an
    /// Orokin Cell, as WFCD and the game's recipes describe it; Steel Fiber twice (one Flawed);
    /// Arcane Energize.
    fn catalog() -> gamedata::Catalog {
        let mut cat = gamedata::build(&IndexMap::from([
            (
                "Warframes".to_string(),
                json!([{"uniqueName": ASH, "name": "Ash Prime", "masterable": true, "isPrime": true,
                        "components": [{"uniqueName": BP}, {"uniqueName": CHASSIS},
                                       {"uniqueName": CELL, "itemCount": 2}]}]),
            ),
            (
                "Misc".to_string(),
                json!([{"uniqueName": CELL, "name": "Orokin Cell"}, {"uniqueName": RUBEDO, "name": "Rubedo"}]),
            ),
            (
                "ExportRecipes".to_string(),
                json!({
                    BP: {"resultType": ASH, "buildPrice": 25000, "ingredients": [{"ItemType": CHASSIS, "ItemCount": 1}]},
                    CHASSIS_BP: {"resultType": CHASSIS, "buildPrice": 15000,
                                 "ingredients": [{"ItemType": RUBEDO, "ItemCount": 400}]},
                }),
            ),
        ]));
        let entry = |name: &str, kind: &'static str, rank: i64| Entry {
            name: Some(name.to_string()),
            icon: None,
            kind,
            result: None,
            rank: Some(rank),
            rarity: Some("Rare".into()),
            hidden: false,
        };
        cat.inventory.insert(FIBER.into(), entry("Steel Fiber", "Mods", 10));
        cat.inventory.insert(FLAWED.into(), entry("Steel Fiber", "Mods", 3));
        cat.inventory.insert(ENERGIZE.into(), entry("Arcane Energize", "Arcanes", 5));
        cat
    }

    fn view(inv: Option<Inventory>, tasks: &[&str]) -> View {
        view_of(catalog(), inv, tasks)
    }

    fn view_of(cat: gamedata::Catalog, inv: Option<Inventory>, tasks: &[&str]) -> View {
        let manual = Manual {
            tasks: tasks.iter().map(|uid| Task { uid: uid.to_string(), target: None, added: 1.0 }).collect(),
            ..Manual::default()
        };
        View {
            cat: Arc::new(cat),
            profile: None,
            inv: inv.map(Arc::new),
            manual: Arc::new(manual),
            market: Market::new(Arc::new(std::sync::RwLock::new(crate::config::Settings::default()))),
            foundry: HashSet::new(),
        }
    }

    fn inventory(counts: &[(&str, i64)]) -> Inventory {
        Inventory {
            counts: counts.iter().map(|(u, n)| (u.to_string(), *n)).collect(),
            upgrades: Some(IndexMap::new()),
            imported: 10.0,
            ..Inventory::default()
        }
    }

    /// "state: note" of a task for `uid`.
    fn state(v: &View, uid: &str) -> String {
        let r = row(v, &Task { uid: uid.into(), ..Task::default() });
        format!("{}: {}", r["state"].as_str().unwrap(), r["note"].as_str().unwrap())
    }

    #[test]
    fn the_resources_of_a_build_are_what_recipes_consume() {
        let cat = catalog();
        assert_eq!(cat.resources.keys().collect::<Vec<_>>(), [CELL, RUBEDO], "sorted by name; parts are not resources");
        assert_eq!(cat.part_alias.get(CHASSIS_BP).map(String::as_str), Some(CHASSIS));
    }

    #[test]
    fn equipment_turns_yellow_then_green() {
        // Nothing held: red, both parts missing.
        let v = view(Some(inventory(&[])), &[ASH]);
        assert_eq!(state(&v, ASH), "missing: Missing 2 of 2 parts");
        // The chassis blueprint only: one part missing, named short.
        let v = view(Some(inventory(&[(CHASSIS_BP, 1)])), &[ASH]);
        assert_eq!(state(&v, ASH), "missing: Missing Blueprint");
        // Both parts, but the chassis still to craft and no Rubedo: yellow.
        let v = view(Some(inventory(&[(BP, 1), (CHASSIS_BP, 1), (CELL, 2)])), &[ASH]);
        assert_eq!(state(&v, ASH), "resources: Short of Rubedo");
        // With the Rubedo: green.
        let v = view(Some(inventory(&[(BP, 1), (CHASSIS_BP, 1), (CELL, 2), (RUBEDO, 400)])), &[ASH]);
        assert_eq!(state(&v, ASH), "done: Ready to build");
        // A crafted chassis needs no Rubedo, but the cells are short.
        let v = view(Some(inventory(&[(BP, 1), (CHASSIS, 1), (CELL, 1)])), &[ASH]);
        assert_eq!(state(&v, ASH), "resources: Short of Orokin Cell");
        // The chassis crafting in the foundry counts as held and crafted.
        let mut inv = inventory(&[(BP, 1), (CELL, 2)]);
        inv.foundry.push(FoundryEntry { recipe: Some(CHASSIS_BP.into()), done: None });
        let v = view(Some(inv), &[ASH]);
        assert_eq!(state(&v, ASH), "done: Ready once its parts are crafted");
        // Credits count when the inventory has them: 15,000 for the chassis, 25,000 for Ash.
        let mut inv = inventory(&[(BP, 1), (CHASSIS_BP, 1), (CELL, 2), (RUBEDO, 400)]);
        inv.currencies.credits = Some(39_999);
        let v = view(Some(inv), &[ASH]);
        assert_eq!(state(&v, ASH), "resources: Short of Credits");
        // Owned: done, whatever the parts.
        let mut inv = inventory(&[]);
        inv.equipment.insert(ASH.into(), crate::sources::Equipment { count: 1, xp: 0 });
        assert_eq!(state(&view(Some(inv), &[ASH]), ASH), "done: Owned");
    }

    #[test]
    fn a_part_whose_recipe_is_unknown_is_never_ready() {
        // Citrine Prime's part recipes were missing from the export-plus copy and read as free
        // (issue #11, 2026-11-09). Here the chassis recipe is missing.
        let without_recipe = |counts: &[(&str, i64)]| {
            let mut cat = catalog();
            cat.part_recipes.remove(CHASSIS_BP);
            view_of(cat, Some(inventory(counts)), &[ASH])
        };
        let v = without_recipe(&[(BP, 1), (CHASSIS_BP, 1), (CELL, 2)]);
        assert_eq!(state(&v, ASH), "resources: Chassis recipe unknown");
        let d = detail(&v, ASH, &HashSet::new()).unwrap();
        let chassis = d["item"]["parts"].as_array().unwrap().iter().find(|p| p["uid"] == CHASSIS).unwrap();
        assert_eq!((chassis["recipeUnknown"].as_bool(), chassis["builds"].as_i64()), (Some(true), Some(0)));
        // A crafted chassis needs no recipe.
        assert_eq!(state(&without_recipe(&[(BP, 1), (CHASSIS, 1), (CELL, 2)]), ASH), "done: Ready to build");
        // A known shortage is named first.
        let v = without_recipe(&[(BP, 1), (CHASSIS_BP, 1), (CELL, 1)]);
        assert_eq!(state(&v, ASH), "resources: Short of Orokin Cell");
    }

    #[test]
    fn the_details_list_what_the_builds_still_take() {
        let mut inv = inventory(&[(BP, 1), (CHASSIS_BP, 1), (CELL, 1)]);
        inv.currencies.credits = Some(100_000);
        let v = view(Some(inv), &[ASH]);
        let d = detail(&v, ASH, &HashSet::new()).unwrap();
        assert_eq!(d["onBoard"], true);
        assert_eq!(d["credits"], json!({"need": 40_000, "have": 100_000}));
        let materials: Vec<(&str, i64, i64)> = d["materials"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| (m["name"].as_str().unwrap(), m["count"].as_i64().unwrap(), m["have"].as_i64().unwrap()))
            .collect();
        assert_eq!(materials, [("Orokin Cell", 2, 1), ("Rubedo", 400, 0)]);
        let chassis = d["item"]["parts"].as_array().unwrap().iter().find(|p| p["uid"] == CHASSIS).unwrap();
        assert_eq!((chassis["have"].as_i64(), chassis["builds"].as_i64()), (Some(1), Some(1)));
    }

    #[test]
    fn mods_and_arcanes_count_their_ranked_copies() {
        let mut inv = inventory(&[(ENERGIZE, 3)]);
        inv.upgrades = Some(IndexMap::from([(ENERGIZE.to_string(), vec![2]), (FIBER.to_string(), vec![10])]));
        let v = view(Some(inv), &[ENERGIZE, FIBER]);
        // Rank 2 took 6 copies, plus 3 unranked: 9 of the 21 rank 5 takes.
        assert_eq!(state(&v, ENERGIZE), "missing: 9 of 21 copies");
        assert_eq!(state(&v, FIBER), "done: Owned");
        let d = detail(&v, ENERGIZE, &HashSet::new()).unwrap();
        assert_eq!((d["target"].as_i64(), d["rank"].as_i64(), d["topRank"].as_i64()), (Some(21), Some(2), Some(5)));
        // A lower goal is met.
        let mut m = (*v.manual).clone();
        m.tasks[0].target = Some(9);
        let v = View { manual: Arc::new(m), ..v };
        assert_eq!(row(&v, &v.manual.tasks[0])["state"], "done");
        // Unranked copies only, from an inventory that kept no ranks: not owned yet.
        let mut old = inventory(&[]);
        old.upgrades = None;
        assert_eq!(state(&view(Some(old), &[]), FIBER), "missing: Not owned");
        assert_eq!(arcane_copies(5), 21);
        assert_eq!(arcane_copies(3), 10);
        assert_eq!(arcane_rank(0, 5), None);
        assert_eq!(arcane_rank(20, 5), Some(4));
        assert_eq!(arcane_rank(500, 5), Some(5));
    }

    #[test]
    fn resources_count_to_their_goal() {
        let v = view(Some(inventory(&[(RUBEDO, 1500)])), &[RUBEDO]);
        assert_eq!(state(&v, RUBEDO), "done: 1,500 of 1");
        let mut m = (*v.manual).clone();
        m.tasks[0].target = Some(20_000);
        let v = View { manual: Arc::new(m), ..v };
        let r = row(&v, &v.manual.tasks[0]);
        assert_eq!((r["state"].as_str(), r["note"].as_str()), (Some("missing"), Some("1,500 of 20,000")));
        assert_eq!(grouped(1_234_567), "1,234,567");
        assert_eq!(grouped(-15), "-15");
    }

    #[test]
    fn the_search_offers_each_kind_once() {
        let v = view(None, &[FIBER]);
        let hits = find(&v, "steel");
        let names: Vec<(&str, &str, bool)> = hits
            .as_array()
            .unwrap()
            .iter()
            .map(|h| (h["key"].as_str().unwrap(), h["kind"].as_str().unwrap(), h["added"].as_bool().unwrap()))
            .collect();
        assert_eq!(names, [(FIBER, "mod", true)], "the Flawed copy is not offered");
        assert_eq!(find(&v, "energ")[0]["kind"], "arcane");
        assert_eq!(find(&v, "rubed")[0]["key"], RUBEDO);
        assert_eq!(find(&v, "ash")[0]["key"], ASH);
        assert!(find(&v, "ash prime chassis").as_array().unwrap().is_empty(), "parts are not tasks");
        assert_eq!(uid_by_name(&v, "Steel Fiber").as_deref(), Some(FIBER));
        assert_eq!(uid_by_name(&v, "orokin cell").as_deref(), Some(CELL));
        assert_eq!(uid_by_name(&v, "Lith A1 Relic"), None);
    }

    #[test]
    fn the_board_counts_what_is_done() {
        let mut inv = inventory(&[(RUBEDO, 5)]);
        inv.upgrades = Some(IndexMap::from([(FIBER.to_string(), vec![0])]));
        let v = view(Some(inv), &[ASH, FIBER, RUBEDO, "/Gone/Item"]);
        let b = board(&v);
        let states: Vec<&str> = b["tasks"].as_array().unwrap().iter().map(|t| t["state"].as_str().unwrap()).collect();
        assert_eq!(states, ["missing", "done", "done", "missing"]);
        assert_eq!(b["done"], 2);
        assert_eq!(b["tasks"][3]["kind"], "unknown");
        assert_eq!(summary(&v)["sig"], b["sig"]);
        assert_eq!(done(&v), HashSet::from([FIBER.to_string(), RUBEDO.to_string()]));
    }
}
