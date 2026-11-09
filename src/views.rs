//! What the UI shows, computed from a snapshot of the account so no lock is held meanwhile.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::{Value, json};

use crate::accounts::Manual;
use crate::difflib::py_round;
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

#[derive(Serialize)]
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

pub struct Status {
    pub xp: i64,
    pub rank: i64,
    pub status: &'static str,
    pub manual: bool,
    pub owned: Option<i64>,
    pub building: bool,
}

impl Status {
    fn merge_into(&self, row: &mut Value) {
        for (k, v) in [
            ("xp", json!(self.xp)),
            ("rank", json!(self.rank)),
            ("status", json!(self.status)),
            ("manual", json!(self.manual)),
            ("owned", json!(self.owned)),
            ("building", json!(self.building)),
        ] {
            row[k] = v;
        }
    }
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

fn farm_hint(parts: &[PartView]) -> Value {
    let hints: Vec<Value> = parts
        .iter()
        .filter(|p| p.have < p.need)
        .filter_map(|p| {
            let live: Vec<&gamedata::PartRelic> = p.relics.iter().filter(|r| !r.vaulted).collect();
            let src: Vec<&gamedata::PartRelic> = if live.is_empty() { p.relics.iter().collect() } else { live.clone() };
            (!src.is_empty()).then(|| {
                json!({"part": p.name,
                       "relics": src.iter().take(3).map(|r| format!("{} ({})", r.relic, r.rarity.as_deref().unwrap_or("?"))).collect::<Vec<_>>(),
                       "vaulted": live.is_empty()})
            })
        })
        .collect();
    Value::Array(hints)
}

/// One relic, priced: the row the relic views and the fissure picks share.
pub struct RelicRow {
    pub value: Value,
    pub era: String,
    pub total: i64,
    pub needs: usize,
    pub ev_radiant: f64,
}

/// How well `name` matches a search: lower is better, None is no match. Every word of the query
/// may match the start of any word of the name, so "ash sys" finds "Ash Prime Systems Blueprint".
fn match_rank(name: &str, q: &str, tokens: &[&str]) -> Option<u8> {
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
    fn part_recipe(&self, uid: &str) -> Option<Value> {
        let part = self.cat.parts.get(uid)?;
        let uids = [Some(uid), part.alt.as_deref()];
        let recipe = uids
            .iter()
            .flatten()
            .filter_map(|id| self.cat.part_recipes.get(*id))
            .find(|recipe| uids.contains(&Some(recipe.result.as_str())))?;
        Some(json!({
            "quantity": recipe.quantity, "credits": recipe.credits, "seconds": recipe.seconds,
            "ingredients": recipe.ingredients.iter().map(|need| json!({
                "uid": need.uid, "name": need.name, "count": need.count, "image": self.icon_of(&need.uid, &need.name),
                "have": if self.inv.is_some() || self.manual.counts.contains_key(&need.uid) { Some(self.count(&need.uid)) } else { None },
                "drops": self.cat.drops.for_name(&need.name, false)
            })).collect::<Vec<_>>()
        }))
    }

    /// A resource's picture: WFCD's art for exactly that name, else the game's own icon (which
    /// for some resources is their store bundle's: Polymer Bundle's is a crate of weapons), else
    /// WFCD's art for the start of the name.
    fn icon_of(&self, uid: &str, name: &str) -> Option<String> {
        self.exact_art(name)
            .or_else(|| self.cat.inventory.get(uid).and_then(|e| e.icon.clone()))
            .or_else(|| self.cat.art_by_name(name))
    }

    fn exact_art(&self, name: &str) -> Option<String> {
        gamedata::image_url(self.cat.art.get(&gamedata::art_key(name)).map(String::as_str))
    }

    /// Owned count for any uid. Prime parts combine the blueprint and the crafted component, and a
    /// hand-set count wins over the inventory only if it is newer than the inventory.
    pub fn count(&self, uid: &str) -> i64 {
        let inv = self.inv.as_deref();
        let inv_t = inv.map_or(0.0, |i| i.synced_at.filter(|t| *t != 0.0).unwrap_or(i.imported));
        let mut uids = vec![uid];
        if let Some(alt) = self.cat.parts.get(uid).and_then(|p| p.alt.as_deref()) {
            uids.push(alt);
        }
        uids.iter()
            .map(|u| match (self.manual.counts.get(*u), inv) {
                (Some((n, t)), i) if i.is_none() || *t > inv_t => *n,
                (_, Some(i)) => i.counts.get(*u).copied().unwrap_or(0),
                _ => 0,
            })
            .sum()
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
    pub fn inventory(&self) -> Value {
        let Some(inv) = self.inv.as_deref() else {
            return json!({"items": [], "source": null});
        };
        let mut rows: Vec<Value> = Vec::new();
        let mut row = |uid: &str, count: i64| {
            let (name, kind, image) = self.inventory_entry(uid);
            let slug = self.market.slug_for(Some(uid), None, Some(&name));
            let known = slug.as_deref().is_some_and(|s| self.market.name(s).is_some());
            let slug = slug.filter(|_| known);
            rows.push(json!({
                "uid": uid, "name": name, "kind": kind, "count": count, "image": image,
                "price": self.price(slug.as_deref(), false), "ducats": self.market.ducats(slug.as_deref()),
                "slug": slug,
            }));
        };
        for (uid, e) in &inv.equipment {
            row(uid, e.count);
        }
        for (uid, count) in inv.counts.iter().filter(|(_, c)| **c > 0) {
            if !inv.equipment.contains_key(uid) {
                row(uid, *count);
            }
        }
        json!({"items": rows, "source": inv.source, "imported": inv.imported, "currencies": inv.currencies})
    }

    /// (name, kind, picture) for anything an inventory holds: the catalog's names for equipment,
    /// parts and relics, the game's export for the rest, and the uid's last word when neither
    /// knows it.
    fn inventory_entry(&self, uid: &str) -> (String, String, Option<String>) {
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
            let name = e
                .name
                .clone()
                .or_else(|| e.result.as_deref().map(|r| format!("{} Blueprint", self.inventory_entry(r).0)));
            if let Some(name) = name {
                let pic = icon.or_else(|| e.result.as_deref().and_then(|r| self.inventory_entry(r).2));
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

    pub fn collection(&self) -> Value {
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
        json!({"relics": relics, "relicKinds": relic_kinds, "primeParts": prime_parts, "setsComplete": done,
               "setsStarted": started, "mods": self.inv.as_ref().map_or(0, |i| i.mods), "stacks": counts.len()})
    }

    /// Mastery XP as the game's profile shows it: gear, star chart, Steel Path and intrinsics,
    /// with the rank that total reaches (the rank held can be lower: every rank needs its test).
    /// `points`/`possible`/`byCategory` are the gear alone. `lacks` says what is not counted yet:
    /// "profile" (no star chart or intrinsics read so far) or "starChart" (no node table).
    pub fn mastery_progress(&self) -> Value {
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
        json!({
            "points": points, "possible": possible,
            "total": total, "lacks": lacks, "rank": rank, "rankFloor": floor, "nextAt": next,
            "sources": {"gear": points, "starChart": other.star_chart, "steelPath": other.steel_path,
                        "railjack": other.railjack, "drifter": other.drifter},
            "byCategory": by_cat.iter().map(|(c, (m, t, p, x))| json!({"category": c, "mastered": m, "total": t, "points": p, "possible": x}))
                .collect::<Vec<_>>(),
        })
    }

    pub fn mastered_count(&self) -> (usize, usize) {
        let mastered = self.cat.items.values().filter(|it| self.item_status(it).status == "mastered").count();
        (mastered, self.cat.items.len())
    }

    pub fn mastery(&self) -> Value {
        let mut out: Vec<Value> = self
            .cat
            .items
            .values()
            .map(|it| {
                let st = self.item_status(it);
                let mut row = json!({
                    "uid": it.uid, "name": it.name, "category": it.category, "type": it.kind,
                    "isPrime": it.is_prime, "vaulted": it.vaulted, "maxRank": it.max_rank,
                    "masteryReq": it.mastery_req, "image": gamedata::image_url(it.image.as_deref()),
                    "points": st.rank.min(it.max_rank) * points_per_rank(it), "maxPoints": it.max_rank * points_per_rank(it),
                });
                st.merge_into(&mut row);
                if st.status != "mastered" {
                    let parts: Vec<PartView> = self.part_views(it, false);
                    row["parts"] = parts
                        .iter()
                        .map(|p| json!({"name": p.name, "need": p.need, "have": p.have, "vaulted": p.vaulted, "image": p.image}))
                        .collect();
                    row["buildable"] = json!(!parts.is_empty() && parts.iter().all(|p| p.have >= p.need));
                    row["requires"] = json!(self.requires(it));
                    row["farm"] = farm_hint(&parts);
                }
                row
            })
            .collect();
        out.sort_by(|a, b| {
            (a["category"].as_str(), a["name"].as_str()).cmp(&(b["category"].as_str(), b["name"].as_str()))
        });
        Value::Array(out)
    }

    pub fn primes(&self) -> Value {
        let mut out: Vec<(f64, String, Value)> = Vec::new();
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
                json!({
                    "uid": it.uid, "name": it.name, "category": it.category, "vaulted": it.vaulted,
                    "image": gamedata::image_url(it.image.as_deref()), "status": st.status, "rank": st.rank,
                    "owned": st.owned, "parts": parts, "have": have, "need": need, "complete": have >= need,
                    "setPrice": self.price(set_slug.as_deref(), true), "setSlug": set_slug, "requires": self.requires(it),
                }),
            ));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        Value::Array(out.into_iter().map(|(_, _, row)| row).collect())
    }

    /// Everything craftable, with the components held - the collection browser.
    pub fn craftable(&self) -> Value {
        let mut out: Vec<(f64, String, Value)> = Vec::new();
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
                json!({
                    "uid": it.uid, "name": it.name, "category": it.category,
                    "image": gamedata::image_url(it.image.as_deref()), "isPrime": it.is_prime,
                    "vaulted": it.vaulted, "masteryReq": it.mastery_req, "status": st.status, "owned": st.owned,
                    "parts": parts, "have": have, "need": need, "complete": have >= need, "buildable": buildable,
                }),
            ));
        }
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        Value::Array(out.into_iter().map(|(_, _, row)| row).collect())
    }

    /// Builds that give no mastery (modular parts, gear, forma...), with the blueprint and every
    /// resource checked against the inventory. Closest to buildable first.
    pub fn crafts(&self) -> Vec<Value> {
        let mut out: Vec<(f64, Value)> = self
            .cat
            .crafts
            .iter()
            .map(|c| {
                let parts: Vec<Value> = c
                    .needs
                    .iter()
                    .map(|n| {
                        json!({"name": n.name, "need": n.count, "have": self.count(&n.uid),
                               "image": gamedata::image_url(n.image.as_deref())})
                    })
                    .collect();
                let met = parts.iter().filter(|p| p["have"].as_i64() >= p["need"].as_i64()).count();
                let row = json!({
                    "uid": c.uid, "name": c.name, "category": c.kind, "craft": true,
                    "pinned": self.manual.pinned.contains(&c.uid),
                    "image": gamedata::image_url(c.image.as_deref()), "masteryReq": c.mastery_req,
                    "quantity": c.quantity, "held": self.count(&c.uid),
                    "have": met, "need": parts.len(), "buildable": met == parts.len(), "parts": parts,
                });
                (-(met as f64 / parts.len() as f64), row)
            })
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out.into_iter().map(|(_, row)| row).collect()
    }

    /// The pinned things with what is still missing for each, in the order they were pinned.
    pub fn pinned(&self) -> Vec<Value> {
        let needed = self.needed_parts();
        self.manual
            .pinned
            .iter()
            .filter_map(|key| {
                if let Some(it) = self.cat.items.get(key) {
                    let parts = self.part_views(it, false);
                    let have: i64 = parts.iter().map(|p| p.have.min(p.need)).sum();
                    let need: i64 = parts.iter().map(|p| p.need).sum();
                    return Some(json!({"key": key, "kind": "item", "name": it.name, "open": true,
                        "image": gamedata::image_url(it.image.as_deref()), "status": self.item_status(it).status,
                        "have": have, "need": need, "unit": "parts"}));
                }
                if let Some(p) = self.cat.parts.get(key) {
                    return Some(json!({"key": key, "kind": "part", "name": p.name, "open": true,
                        "image": self.image_for(key), "have": self.count(key),
                        "need": needed.get(key).copied().unwrap_or(0), "unit": "held"}));
                }
                if let Some(rel) = self.cat.relics.get(key) {
                    let total: i64 = rel.uids.values().map(|u| self.count(u)).sum();
                    return Some(json!({"key": key, "kind": "relic", "name": format!("{key} Relic"), "open": true,
                        "image": gamedata::image_url(rel.image.as_deref()), "have": total, "need": 0, "unit": "held"}));
                }
                let c = self.cat.crafts.iter().find(|c| &c.uid == key)?;
                let met = c.needs.iter().filter(|n| self.count(&n.uid) >= n.count).count();
                Some(json!({"key": key, "kind": "build", "name": c.name, "open": false,
                    "image": gamedata::image_url(c.image.as_deref()), "have": met, "need": c.needs.len(),
                    "unit": "needs met"}))
            })
            .collect()
    }

    /// What is in the foundry now, soonest first.
    pub fn pending_foundry(&self) -> Value {
        let mut rows: Vec<Value> = self
            .inv
            .as_deref()
            .map(|i| {
                i.foundry
                    .iter()
                    .map(|f| {
                        let uid = f.recipe.as_deref().unwrap_or("");
                        // Forma, consumables and the like have no page to open.
                        let known = self.cat.items.contains_key(uid) || self.cat.part_alias.contains_key(uid);
                        json!({"uid": uid, "name": self.cat.name_of(uid), "done": f.done, "image": self.image_for(uid),
                               "known": known})
                    })
                    .collect()
            })
            .unwrap_or_default();
        rows.sort_by(|a, b| a["done"].as_f64().unwrap_or(f64::MAX).total_cmp(&b["done"].as_f64().unwrap_or(f64::MAX)));
        Value::Array(rows)
    }

    pub fn relic_row(&self, base: &str, rel: &Relic, needed: &HashMap<String, i64>) -> RelicRow {
        let counts: IndexMap<&String, i64> = rel.uids.iter().map(|(r, uid)| (r, self.count(uid))).collect();
        let total: i64 = counts.values().sum();
        let mut ev: IndexMap<&str, f64> = IndexMap::from([("Intact", 0.0), ("Radiant", 0.0)]);
        let rewards: Vec<Value> = rel
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
                json!({"name": rw.name, "uid": rw.uid, "rarity": rw.rarity, "chance": rw.chance, "price": price,
                       "ducats": self.market.ducats(slug.as_deref()), "image": self.market.image(slug.as_deref()),
                       "have": self.count(&rw.uid), "needed": needed.get(&rw.uid).copied().unwrap_or(0)})
            })
            .collect();
        let needs = rewards.iter().filter(|r| r["needed"].as_i64() > r["have"].as_i64()).count();
        let relic_slug = self.market.slug_for(None, rel.slug.as_deref(), Some(&format!("{base} Relic")));
        let ev_radiant = py_round(ev["Radiant"], 1);
        let value = json!({
            "name": base, "era": rel.era, "vaulted": rel.vaulted, "counts": counts,
            "image": gamedata::image_url(rel.image.as_deref()), "total": total, "uids": rel.uids,
            "rewards": rewards, "needsCount": needs, "slug": relic_slug,
            "ev": ev.iter().map(|(k, x)| (k.to_string(), json!(py_round(*x, 1)))).collect::<serde_json::Map<_, _>>(),
            "price": self.price(relic_slug.as_deref(), total > 0),
        });
        RelicRow { value, era: rel.era.clone(), total, needs, ev_radiant }
    }

    pub fn relics(&self, owned_only: bool) -> Value {
        let needed = self.needed_parts();
        let mut out: Vec<(u8, String, Value)> = Vec::new();
        for (base, rel) in &self.cat.relics {
            let row = self.relic_row(base, rel, &needed);
            if owned_only && row.total == 0 {
                continue;
            }
            let era_rank = ERAS.iter().position(|e| *e == rel.era).unwrap_or(9) as u8;
            out.push((era_rank, base.clone(), row.value));
        }
        out.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        Value::Array(out.into_iter().map(|(_, _, row)| row).collect())
    }

    /// Per era, the relics owned and the one most worth cracking: the most drops still needed,
    /// then the highest radiant value.
    pub fn relic_picks(&self) -> IndexMap<String, Value> {
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
                let best = best.map(|b| {
                    json!({"name": b.value["name"], "needs": b.needs, "ev": b.ev_radiant, "image": b.value["image"],
                           "total": b.total})
                });
                (era, json!({"owned": owned, "kinds": kinds, "best": best}))
            })
            .collect()
    }

    /// Prime parts owned beyond what the unfinished sets still need, valued in platinum and ducats.
    pub fn trade(&self) -> Vec<Value> {
        let needed = self.needed_parts();
        let mut rows: Vec<(i64, Value)> = Vec::new();
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
                json!({"uid": uid, "name": p.name, "have": have, "spare": spare, "slug": slug, "price": price,
                       "ducats": ducats, "vaulted": p.vaulted, "ducatsPerPlat": per_plat,
                       "image": self.market.image(slug.as_deref())}),
            ));
        }
        rows.sort_by_key(|(key, _)| *key);
        rows.into_iter().map(|(_, r)| r).collect()
    }

    /// Catalog entries, drop names, relics and syndicates matching `q`, best matches first.
    pub fn find(&self, q: &str) -> Value {
        self.find_matching(q, false)
    }

    pub fn find_equipment(&self, q: &str) -> Value {
        self.find_matching(q, true)
    }

    fn find_matching(&self, q: &str, equipment_only: bool) -> Value {
        let q = q.trim().to_lowercase();
        if q.chars().count() < 2 {
            return json!([]);
        }
        let tokens: Vec<&str> = q.split_whitespace().collect();
        let mut hits: Vec<(u8, usize, Value)> = Vec::new();
        for it in self.cat.items.values() {
            if let Some(rank) = match_rank(&it.name, &q, &tokens) {
                let st = self.item_status(it);
                hits.push((
                    rank,
                    it.name.len(),
                    json!({"kind": "item", "key": it.uid, "name": it.name,
                    "sub": it.category, "image": gamedata::image_url(it.image.as_deref()), "status": st.status}),
                ));
            }
        }
        if equipment_only {
            hits.sort_by(|a, b| {
                (a.0, a.1).cmp(&(b.0, b.1)).then_with(|| a.2["name"].as_str().cmp(&b.2["name"].as_str()))
            });
            return Value::Array(hits.into_iter().take(24).map(|(_, _, v)| v).collect());
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
                    json!({"kind": "part", "key": uid, "name": p.name,
                    "sub": if p.vaulted { "Prime part · vaulted" } else { "Prime part" },
                    "image": self.market.image(slug.as_deref()), "price": self.price(slug.as_deref(), false),
                    "have": self.count(uid)}),
                ));
            }
        }
        for (base, rel) in &self.cat.relics {
            if let Some(rank) = match_rank(&format!("{base} relic"), &q, &tokens) {
                let total: i64 = rel.uids.values().map(|u| self.count(u)).sum();
                hits.push((
                    rank,
                    base.len(),
                    json!({"kind": "relic", "key": base, "name": format!("{base} Relic"),
                    "sub": if rel.vaulted { "Relic · vaulted" } else { "Relic" },
                    "image": gamedata::image_url(rel.image.as_deref()), "have": total}),
                ));
            }
        }
        for s in self.cat.syndicates.list.values().filter(|s| s.kind != crate::standing::Kind::Nightwave) {
            if let Some(rank) = match_rank(&s.name, &q, &tokens) {
                hits.push((
                    rank,
                    s.name.len(),
                    json!({"kind": "syndicate", "key": format!("syndicate:{}", s.tag), "name": s.name,
                    "sub": "Syndicate · standing and offerings", "image": s.icon}),
                ));
            }
        }
        let mut represented: HashSet<String> =
            hits.iter().filter_map(|(_, _, v)| v["name"].as_str().map(str::to_lowercase)).collect();
        for item in self.cat.lookup.values().filter(|it| !self.cat.items.contains_key(&it.uid)) {
            if let Some(rank) = match_rank(&item.name, &q, &tokens) {
                if !represented.insert(item.name.to_lowercase()) {
                    continue;
                }
                hits.push((
                    rank,
                    item.name.len(),
                    json!({"kind": "lookup", "key": format!("lookup:{}", item.uid), "name": item.name,
                           "sub": item.category, "image": gamedata::image_url(item.image.as_deref())}),
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
                    json!({"kind": "drop", "key": format!("drop:{name}"), "name": name,
                           "sub": "Drop sources", "image": self.cat.art_by_name(name)}),
                ));
            }
        }
        hits.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then_with(|| a.2["name"].as_str().cmp(&b.2["name"].as_str())));
        Value::Array(hits.into_iter().take(24).map(|(_, _, v)| v).collect())
    }

    /// Everything about one item, part or relic. `open_eras` are the relic eras with a fissure
    /// open right now ("Omnia" opens them all). A relic counts as open when one of its era's
    /// fissures is up and you can get it: it still drops, or you hold one (vaulted relics crack
    /// like any other).
    pub fn detail(&self, key: &str, open_eras: &HashSet<String>) -> Option<Value> {
        if let Some(uid) = key.strip_prefix("lookup:") {
            let item = self.cat.lookup.get(uid)?;
            return Some(json!({
                "kind": "drop", "key": key, "name": item.name,
                "image": gamedata::image_url(item.image.as_deref()),
                "drops": self.cat.drops.for_name(&item.name, false),
                "dropCoverage": {"available": self.cat.drops.available, "total": self.cat.drops.total}
            }));
        }
        if let Some(name) = key.strip_prefix("drop:") {
            let drops = self.cat.drops.for_name(name, false);
            let first = drops.first()?;
            return Some(json!({
                "kind": "drop", "key": format!("drop:{}", first.item), "name": first.item,
                "image": self.cat.art_by_name(&first.item), "drops": drops,
                "dropCoverage": {"available": self.cat.drops.available, "total": self.cat.drops.total}
            }));
        }
        let open = |era: &str| open_eras.contains(era) || open_eras.contains("Omnia");
        let needed = self.needed_parts();
        // Open fissures first, then relics that still drop, then the ones you hold.
        let relic_refs = |relics: &[gamedata::PartRelic]| -> Vec<Value> {
            let mut refs: Vec<Value> = relics
                .iter()
                .map(|r| {
                    let rel = self.cat.relics.get(&r.relic);
                    let owned: i64 = rel.map_or(0, |rel| rel.uids.values().map(|u| self.count(u)).sum());
                    let era = rel.map_or("", |rel| rel.era.as_str());
                    json!({"relic": r.relic, "rarity": r.rarity, "chance": r.chance, "vaulted": r.vaulted,
                           "owned": owned, "open": open(era) && (!r.vaulted || owned > 0)})
                })
                .collect();
            let key = |v: &Value| (v["open"] != true, v["vaulted"] == true, -v["owned"].as_i64().unwrap_or(0));
            refs.sort_by(|a, b| {
                key(a)
                    .cmp(&key(b))
                    .then_with(|| b["chance"].as_f64().unwrap_or(0.0).total_cmp(&a["chance"].as_f64().unwrap_or(0.0)))
            });
            refs
        };
        if let Some(it) = self.cat.items.get(key) {
            let st = self.item_status(it);
            let parts: Vec<Value> = self
                .part_views(it, true)
                .into_iter()
                .map(|p| {
                    let relics = self.cat.parts.get(&p.uid).map(|part| relic_refs(&part.relics)).unwrap_or_default();
                    let mut v = serde_json::to_value(&p).unwrap_or_default();
                    v["relics"] = json!(relics);
                    v["drops"] = json!(self.cat.drops.for_name(&p.name, true));
                    v["recipe"] = json!(self.part_recipe(&p.uid));
                    v
                })
                .collect();
            let buildable = !parts.is_empty() && parts.iter().all(|p| p["have"].as_i64() >= p["need"].as_i64());
            let set_slug = it
                .set_slug
                .as_ref()
                .and_then(|s| self.market.slug_for(None, Some(s), Some(&format!("{} Set", it.name))));
            let per = points_per_rank(it);
            let mut v = json!({
                "kind": "item", "key": it.uid, "uid": it.uid, "name": it.name, "category": it.category, "type": it.kind,
                "image": gamedata::image_url(it.image.as_deref()), "isPrime": it.is_prime, "vaulted": it.vaulted,
                "masteryReq": it.mastery_req, "maxRank": it.max_rank,
                "points": st.rank.min(it.max_rank) * per, "maxPoints": it.max_rank * per,
                "setSlug": set_slug, "setPrice": self.price(set_slug.as_deref(), true),
                "parts": parts, "buildable": buildable,
                "recipesAvailable": self.cat.recipes_available,
                "drops": self.cat.drops.for_name(&it.name, false),
                "dropCoverage": {"available": self.cat.drops.available, "total": self.cat.drops.total},
                "requires": it.components.iter().filter(|c| c.kind == ComponentKind::Item).map(|c| {
                    let status = self.cat.items.get(&c.uid).map(|i| self.item_status(i).status);
                    json!({"uid": c.uid, "name": c.name, "count": c.count, "status": status,
                           "have": self.owned(&c.uid)})
                }).collect::<Vec<_>>(),
                "resources": it.components.iter().filter(|c| c.kind == ComponentKind::Resource)
                    .map(|c| json!({"uid": c.uid, "name": c.name, "count": c.count,
                        "have": if self.inv.is_some() || self.manual.counts.contains_key(&c.uid) { Some(self.count(&c.uid)) } else { None },
                        "image": self.icon_of(&c.uid, &c.name),
                        "drops": self.cat.drops.for_name(&c.name, false)}))
                    .collect::<Vec<_>>(),
                "disposition": it.disposition.map(|d| py_round(d, 3)), "dots": it.dispo_dots,
            });
            st.merge_into(&mut v);
            return Some(v);
        }
        let canon = self.cat.part_alias.get(key).map_or(key, String::as_str);
        if let Some(p) = self.cat.parts.get(canon) {
            let slug = self.market.slug_for(Some(canon), p.slug.as_deref(), Some(&p.name));
            return Some(json!({
                "kind": "part", "key": canon, "uid": canon, "name": p.name, "image": self.image_for(canon),
                "slug": slug, "price": self.price(slug.as_deref(), true), "ducats": self.market.ducats(slug.as_deref()),
                "vaulted": p.vaulted, "have": self.count(canon), "needed": needed.get(canon).copied().unwrap_or(0),
                "relics": relic_refs(&p.relics),
                "drops": self.cat.drops.for_name(&p.name, true),
                "recipe": self.part_recipe(canon), "recipesAvailable": self.cat.recipes_available,
                "dropCoverage": {"available": self.cat.drops.available, "total": self.cat.drops.total},
                "usedIn": p.items.iter().filter_map(|u| self.cat.items.get(u)).map(|it| json!({
                    "uid": it.uid, "name": it.name, "image": gamedata::image_url(it.image.as_deref()),
                    "status": self.item_status(it).status})).collect::<Vec<_>>(),
            }));
        }
        let base = key.trim_end_matches(" Relic");
        let rel = self.cat.relics.get(base)?;
        let row = self.relic_row(base, rel, &needed);
        let mut v = row.value;
        v["kind"] = json!("relic");
        v["key"] = json!(base);
        v["open"] = json!(open(&rel.era) && (!rel.vaulted || row.total > 0));
        v["drops"] = json!(self.cat.drops.for_name(&format!("{base} Relic"), false));
        v["dropCoverage"] = json!({"available": self.cat.drops.available, "total": self.cat.drops.total});
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            market: Market::new(Arc::new(std::sync::RwLock::new(crate::config::Settings::default()))),
            foundry: HashSet::new(),
        };
        let mastery = view.mastery();
        let rhino = mastery.as_array().unwrap().iter().find(|r| r["uid"] == "/Rhino").unwrap();
        assert_eq!(rhino["parts"][0]["need"], 1);
        let item = view.detail("/Rhino", &HashSet::new()).unwrap();
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
        let part = view.detail(uid, &HashSet::new()).unwrap();
        assert_eq!(part["drops"][0]["location"], "Venus · Fossa");
        assert_eq!(part["recipe"], item["parts"][0]["recipe"]);
        Arc::make_mut(&mut view.manual).counts.insert("/Rubedo".into(), (250, 1.0));
        Arc::make_mut(&mut view.manual).counts.insert(uid.into(), (1, 1.0));
        let held = view.detail("/Rhino", &HashSet::new()).unwrap();
        assert_eq!(held["parts"][0]["have"], 1);
        assert_eq!(held["parts"][0]["recipe"]["ingredients"][0]["have"], 250);
        assert_eq!(held["parts"][0]["recipe"]["ingredients"][0]["count"], 400);
        let relic = view.detail("Lith A1", &HashSet::new()).unwrap();
        assert_eq!(relic["drops"][0]["chance"], 5.0);
        let hits = view.find("vitality");
        assert_eq!(hits.as_array().unwrap().iter().filter(|hit| hit["name"] == "Vitality").count(), 1);
        assert_eq!(hits[0]["key"], "lookup:/Vitality");
        let lookup = view.detail("lookup:/Vitality", &HashSet::new()).unwrap();
        assert_eq!(lookup["drops"][0]["chance"], 8.0);
        assert!(view.find("vendor mod").as_array().unwrap().iter().any(|hit| hit["key"] == "lookup:/VendorMod"));
        assert_eq!(view.detail("lookup:/VendorMod", &HashSet::new()).unwrap()["drops"], json!([]));
        assert!(view.find("orokin cell").as_array().unwrap().iter().any(|hit| hit["key"] == "lookup:/Cell"));
        let mod_detail = view.detail("drop:Vitality", &HashSet::new()).unwrap();
        assert_eq!(mod_detail["kind"], "drop");
        assert_eq!(mod_detail["drops"][0]["chance"], 8.0);
        assert!(view.detail("drop:unknown", &HashSet::new()).is_none());
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
    fn equipment_search_is_not_crowded_out_by_other_results() {
        let mods: Vec<Value> =
            (0..30).map(|n| json!({"uniqueName": format!("/mod/{n}"), "name": format!("Rh{n:02}")})).collect();
        let cat = gamedata::build(&IndexMap::from([
            ("Warframes".to_string(), json!([{"uniqueName": "/Rhino", "name": "Rhino", "masterable": true}])),
            ("Mods".to_string(), json!(mods)),
        ]));
        let view = View {
            cat: Arc::new(cat),
            profile: None,
            inv: None,
            manual: Arc::new(Manual::default()),
            market: Market::new(Arc::new(std::sync::RwLock::new(crate::config::Settings::default()))),
            foundry: HashSet::new(),
        };
        assert!(!view.find("rh").as_array().unwrap().iter().any(|hit| hit["key"] == "/Rhino"));
        let equipment = view.find_equipment("rh");
        assert_eq!(equipment.as_array().unwrap().len(), 1);
        assert_eq!(equipment[0]["key"], "/Rhino");
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
}
