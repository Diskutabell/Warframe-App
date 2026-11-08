// The detail drawer: one item, part or relic, from anywhere in the app.

import { $, artUrl, act, api, art, duc, era, esc, fmt, hydrateIcons, icon, plat, statusPill, toast } from "./lib.js";
import { ICON, gimg } from "./game.js";

let stack = [];
let requestVersion = 0;

export const isOpen = () => !$("#drawer").hidden;

/** Something was changed from the drawer: pages listening re-render. */
const changed = () => window.dispatchEvent(new CustomEvent("wfp:changed"));

export function close() {
  requestVersion++;
  $("#drawer").hidden = true;
  $("#scrim").hidden = true;
  stack = [];
  // Back to the page, so its keyboard shortcuts work again.
  $("#view").focus();
  if (location.hash.includes("item=")) history.replaceState(null, "", location.hash.replace(/[?&]item=[^&]*/, ""));
}

export async function open(key, push = true) {
  const version = ++requestVersion;
  if (push) stack.push(key);
  const d = $("#drawer");
  d.hidden = false;
  $("#scrim").hidden = false;
  d.innerHTML = `<div class="drawer-top"><span class="grow"></span><button class="ghost icon" data-close>${icon("x")}</button></div>
    <div class="stack"><div class="skeleton" style="width:60%"></div><div class="skeleton"></div><div class="skeleton" style="width:80%"></div></div>`;
  d.querySelector("[data-close]").onclick = close;
  if (key.startsWith("syndicate:")) return openSyndicate(key, version);
  let x;
  try {
    x = await api("item?key=" + encodeURIComponent(key));
  } catch (e) {
    if (version !== requestVersion) return;
    d.querySelector(".stack").innerHTML = `<div class="callout">${esc(e.message)}</div>`;
    return;
  }
  if (version !== requestVersion) return;
  const body = x.kind === "item" ? item(x) : x.kind === "part" ? part(x) : x.kind === "drop"
    ? `<div class="dhead">${art(x.image, "xl")}<div class="grow"><h2>${esc(x.name)}</h2></div></div>${acquisition(x.name, x.drops, x.dropCoverage)}`
    : relic(x);
  const market = x.kind === "item" ? x.setSlug : x.slug;
  d.innerHTML = `<div class="drawer-top">
      ${stack.length > 1 ? `<button class="ghost" data-back>${icon("back")} Back</button>` : ""}
      <span class="grow"></span>
      ${market ? `<a class="btn ghost" href="https://warframe.market/items/${esc(market)}" target="_blank" rel="noopener">${icon("link")} warframe.market</a>` : ""}
      ${x.kind !== "drop" ? `<button class="ghost icon pin ${x.pinned ? "on" : ""}" data-pin title="${x.pinned ? "Unpin from the Overview" : "Pin to the Overview"}">${icon("star")}</button>` : ""}
      <button class="ghost icon" data-close title="Close (Esc)">${icon("x")}</button>
    </div>${body}`;
  hydrateIcons(d);
  wire(d, x, version);
  // Keyboard users land in the drawer, on its close button.
  d.querySelector("[data-close]").focus();
}

$("#scrim").onclick = close;

// ------------------------------------------------------------------ a syndicate

const syndicateArt = (url, px) => (url ? `<img class="gi" alt="" width="${px}" height="${px}" src="${esc(artUrl(url, px))}" onerror="this.remove()">` : "");

function syndicate(x) {
  const offers = x.offers || [];
  const priced = offers.filter((o) => o.available && o.perK != null);
  // What sells on warframe.market is the point here; the rest (cosmetics, consumables, parts that
  // cannot be traded) is folded away.
  const tradable = offers.filter((o) => o.slug);
  const other = offers.filter((o) => !o.slug);
  const row = (o) => `<div class="list-row ${o.available ? "" : "dim"}">${art(o.image, "sm")}
      <div class="main"><div class="t">${esc(o.name)}</div>
        <div class="s">${fmt(o.standing)} standing${o.level > x.title ? ` · needs rank ${o.level}` : ""}</div></div>
      <div style="text-align:right">${o.slug ? plat(o.price) : `<span class="faint">not tradable</span>`}
        ${o.perK != null ? `<div class="faint num" style="font-size:12.5px">${o.perK}p / 1k</div>` : ""}</div></div>`;
  const ranks = (x.ranks || []).filter((r) => r.level > 0).map((r) => `<div class="list-row ${r.level === x.title ? "here" : ""}">
      ${syndicateArt(r.icon, 28)}<div class="main"><div class="t">${esc(r.name)}</div></div><span class="faint num">rank ${r.level}</span></div>`).join("");
  return `<div class="dhead">${syndicateArt(x.rankIcon || x.icon, 96)}<div class="grow">
      <h2>${esc(x.name)}</h2>
      <div class="row wrap"><span class="pill" style="${x.colour ? `color:${esc(x.colour)}` : ""}">${esc(x.rankName)}</span>
        ${x.atCap ? `<span class="pill gold">Rank up ready</span>` : ""}${x.full ? `<span class="pill warn">Full - spend it</span>` : ""}</div></div></div>
    <div class="kv">
      <div><span>Standing</span><b>${fmt(x.standing)}</b></div>
      <div><span>This rank</span><b>${x.steps ? `${x.progress} / ${x.size}` : x.size ? `${fmt(x.progress)} / ${fmt(x.size)}` : "–"}</b></div>
      <div><span>Best value</span><b>${priced.length ? `${priced[0].perK}p / 1k` : "–"}</b></div>
    </div>
    ${(x.allies || []).length ? `<div class="dsec"><h2>Standing earned here also changes</h2><div class="chips">${x.allies.map((a) =>
      `<span class="chip res ${a.share > 0 ? "owned" : ""}">${a.share > 0 ? "+" : "−"}${Math.abs(Math.round(a.share * 100))}% ${esc(a.name)}</span>`).join("")}</div></div>` : ""}
    <div class="dsec"><h2>What standing buys <span class="count">${tradable.length} tradable</span>${x.pricing ? ` <span class="muted" style="letter-spacing:0;text-transform:none;font-weight:400">pricing ${x.pricing}…</span>` : ""}</h2>
      <p class="muted" style="margin:-4px 0 8px;font-size:13px">What you can buy at your rank first, by platinum per 1,000 standing (the lowest sell order on warframe.market).</p>
      <div class="list">${tradable.map(row).join("") || `<div class="muted">Nothing this syndicate offers sells on warframe.market.</div>`}</div>
      ${other.length ? `<details class="more"><summary>${other.length} more that cannot be traded</summary><div class="list">${other.map(row).join("")}</div></details>` : ""}</div>
    ${ranks ? `<div class="dsec"><h2>Ranks</h2><div class="list">${ranks}</div></div>` : ""}`;
}

async function openSyndicate(key, version, again = 0) {
  const d = $("#drawer");
  const scroll = again ? d.scrollTop : 0;
  const tag = key.slice("syndicate:".length);
  let x;
  try {
    x = await api("syndicate?tag=" + encodeURIComponent(tag));
  } catch (e) {
    if (version !== requestVersion) return;
    if (again) toast(e.message, true);
    else d.querySelector(".stack").innerHTML = `<div class="callout">${esc(e.message)}</div>`;
    return;
  }
  if (version !== requestVersion) return;
  d.innerHTML = `<div class="drawer-top">
      ${stack.length > 1 ? `<button class="ghost" data-back>${icon("back")} Back</button>` : ""}
      <span class="grow"></span><button class="ghost icon" data-close title="Close (Esc)">${icon("x")}</button>
    </div>${syndicate(x)}`;
  hydrateIcons(d);
  d.querySelector("[data-close]").onclick = close;
  const back = d.querySelector("[data-back]");
  if (back) back.onclick = () => { stack.pop(); open(stack[stack.length - 1], false); };
  if (again) d.scrollTop = scroll;
  else d.querySelector("[data-close]").focus();
  // Prices arrive one by one from the market queue: look again while some are missing, for two
  // minutes at most.
  if (x.pricing && again < 30) {
    setTimeout(() => { if (version === requestVersion && isOpen()) openSyndicate(key, version, again + 1); }, 4000);
  }
}

function stepper(uid, n) {
  return `<span class="stepper"><button data-count="${esc(uid)}" data-c="${n - 1}" aria-label="One fewer">−</button><b>${n}</b><button data-count="${esc(uid)}" data-c="${n + 1}" aria-label="One more">+</button></span>`;
}

function relicChips(relics) {
  return relics.map((r) => `<span class="chip ${r.vaulted ? "vaulted" : ""} ${r.open ? "open" : ""} ${r.owned ? "owned" : ""}" data-open="${esc(r.relic)}"
      title="${esc(r.rarity || "")}${r.chance != null ? ` · ${r.chance}% intact` : ""}${r.vaulted ? " · vaulted" : ""}${r.open ? " · fissure open now" : ""}">
      ${esc(r.relic)}${r.owned ? ` <b class="lotus">×${r.owned}</b>` : ""}</span>`).join("");
}

function wiki(name) {
  return `https://wiki.warframe.com/w/${encodeURIComponent(name.replace(/ /g, "_"))}`;
}

export function dropRows(drops) {
  return drops.map((d) => `<div class="list-row">
    <div class="main"><div class="t">${esc(d.location)}</div>
      <div class="s">${esc(d.item)}${d.mode ? ` · ${esc(d.mode)}` : ""}${d.rotation ? ` · rotation ${esc(d.rotation)}` : ""}${d.stage ? ` · ${esc(d.stage)}` : ""}${d.rarity ? ` · ${esc(d.rarity)}` : ""}</div></div>
    <span class="num" style="text-align:right">${d.chance}%${d.enemyDropChance != null ? `<div class="faint" style="font-size:12px">of ${d.enemyDropChance}% enemy ${esc(d.enemyDropKind || "blueprint")} drop</div>` : ""}</span></div>`).join("");
}

export function resourceRows(resources) {
  return resources.map((r) => `<div class="list-row"><div class="main"><div class="t">${esc(r.name)}</div>
    <div class="s">${r.have == null ? "Owned: unknown" : `${fmt(r.have)} held`} · ${fmt(r.count)} required${r.have != null && r.have < r.count ? ` · ${fmt(r.count - r.have)} short` : ""}</div></div>
    ${r.drops?.length ? `<button data-open="${esc(`drop:${r.name}`)}">Sources</button>` : `<a href="${esc(wiki(r.name))}" target="_blank" rel="noopener">Wiki</a>`}</div>`).join("");
}

export function recipeDetails(recipe) {
  const costs = [
    recipe.credits != null ? `${fmt(recipe.credits)} credits` : "",
    recipe.seconds != null ? `${fmt(recipe.seconds / 3600)} hours` : "",
    recipe.quantity !== 1 ? `makes ${fmt(recipe.quantity)}` : "",
  ].filter(Boolean).join(" · ");
  return `<div class="dsec"><h3>Component recipe (per build)</h3>
    ${costs ? `<p class="muted">${esc(costs)}</p>` : ""}
    ${recipe.ingredients.length ? `<div class="list">${resourceRows(recipe.ingredients)}</div>` : `<p class="muted">No ingredients listed.</p>`}</div>`;
}

function acquisition(name, drops, coverage, extra = "") {
  const rows = drops || [];
  const note = coverage?.available < coverage?.total
    ? `Some drop tables are unavailable (${coverage.available}/${coverage.total} loaded).`
    : "Drop tables do not include every purchase, quest or time-limited source.";
  return `<div class="dsec"><h2>Where to get it</h2>
    <p class="muted" style="font-size:13px">${esc(note)} Chances are per listed reward roll; enemy chances are conditional on a mod or blueprint drop. Data from <a href="https://drops.warframestat.us/" target="_blank" rel="noopener">WFCD's drop tables</a> may be cached.</p>
    ${extra}${rows.length ? `<div class="list">${dropRows(rows.slice(0, 8))}</div>
      ${rows.length > 8 ? `<details class="more"><summary>Show ${rows.length - 8} more sources</summary><div class="list">${dropRows(rows.slice(8))}</div></details>` : ""}`
      : `<div class="muted">No matching direct drops in the available tables. Check any parts and the wiki for other sources.</div>`}
    <a class="btn ghost" style="margin-top:8px" href="${esc(wiki(name))}" target="_blank" rel="noopener">${icon("link")} ${esc(name)} on the Warframe Wiki</a></div>`;
}

function item(x) {
  const pct = x.maxRank ? Math.round((x.rank / x.maxRank) * 100) : 0;
  const parts = x.parts.map((p) => `<div class="part">
      <div class="top">${art(p.image, "sm")}
        <div class="grow"><div class="ellipsis" data-open="${esc(p.uid)}" style="cursor:pointer;font-weight:600">${esc(p.name)}</div>
          <div class="meta">${plat(p.price)} ${duc(p.ducats)} ${p.vaulted ? `<span class="pill vaulted">vaulted</span>` : ""}</div></div>
        <div class="row"><span class="faint">need ${p.need}</span>${stepper(p.uid, p.have)}</div>
      </div>
      ${p.relics.length ? `<div class="chips">${relicChips(p.relics)}</div>` : ""}
      ${p.drops?.length ? `<details class="more"><summary>Where to get this part (${p.drops.length})</summary><div class="list">${dropRows(p.drops)}</div></details>` : ""}
    </div>`).join("");
  return `<div class="dhead">${art(x.image, "xl")}<div class="grow">
      <h2>${esc(x.name)}</h2>
      <div class="row wrap">${statusPill(x.status)}<span class="pill">${esc(x.category)}</span>
        ${x.masteryReq ? `<span class="pill" title="Mastery rank ${x.masteryReq} to build or trade it">${gimg(ICON.mastery, 14)}MR ${x.masteryReq}</span>` : ""}
        ${x.vaulted ? `<span class="pill vaulted">Vaulted</span>` : ""}${x.building ? `<span class="pill lotus">In the foundry</span>` : ""}
        ${x.owned ? `<span class="pill good">Owned</span>` : ""}</div></div></div>
    <div class="kv">
      <div><span>Rank</span><b>${x.rank} / ${x.maxRank}</b></div>
      <div><span>Mastery</span><b>${fmt(x.points)} / ${fmt(x.maxPoints)}</b></div>
      <div><span>${x.setPrice != null ? "Set price" : "Affinity"}</span><b>${x.setPrice != null ? plat(x.setPrice) : fmt(x.xp)}</b></div>
    </div>
    <div class="bar lotus" style="margin-top:10px"><i style="width:${pct}%"></i></div>
    <div class="row" style="margin-top:12px">
      <button data-mastered="${x.manual ? 0 : 1}">${icon("check")} ${x.manual ? "Unmark as mastered" : "Mark as mastered"}</button>
      <span class="muted" style="font-size:13px">${x.manual ? "Marked by hand." : "If the profile is out of date."}</span>
    </div>
    ${acquisition(x.name, x.drops, x.dropCoverage)}
    ${x.parts.length ? `<div class="dsec"><h2>Parts ${x.buildable ? `<span class="pill good" title="Counts may include unbuilt blueprints; check the foundry and resources">parts collected</span>` : ""}</h2>${parts}</div>` : ""}
    ${x.requires.length ? `<div class="dsec"><h2>Also needs</h2><div class="list">${x.requires.map((r) => `
      <div class="list-row click" data-open="${esc(r.uid)}"><div class="main"><div class="t">${r.count > 1 ? `${r.count} × ` : ""}${esc(r.name)}</div></div>${r.status ? statusPill(r.status) : ""}</div>`).join("")}</div></div>` : ""}
    ${x.resources.length ? `<div class="dsec"><h2>Resources</h2><div class="chips">${x.resources.map((r) => `<a class="chip res" href="${esc(wiki(r.name))}" target="_blank" rel="noopener" title="Find ${esc(r.name)} on the Warframe Wiki">${r.image ? `<img alt="" width="22" height="22" src="${esc(artUrl(r.image, 22))}" onerror="this.remove()">` : ""}${fmt(r.count)} × ${esc(r.name)}</a>`).join("")}</div>
      ${x.resources.filter((r) => r.drops?.length).map((r) => `<details class="more"><summary>${esc(r.name)} drop sources (${r.drops.length})</summary><div class="list">${dropRows(r.drops)}</div></details>`).join("")}</div>` : ""}
    ${x.disposition ? `<div class="dsec"><h2>Riven disposition</h2><span class="gold">${"●".repeat(x.dots || 0)}${"○".repeat(5 - (x.dots || 0))}</span> <span class="muted">${x.disposition}×</span></div>` : ""}`;
}

function part(x) {
  return `<div class="dhead">${art(x.image, "xl")}<div class="grow"><h2>${esc(x.name)}</h2>
      <div class="row wrap">${x.vaulted ? `<span class="pill vaulted">Vaulted</span>` : x.relics.length ? `<span class="pill good">Drops now</span>` : ""}
        ${x.needed > x.have ? `<span class="pill lotus">Needed ×${x.needed}</span>` : ""}</div></div></div>
    <div class="kv">
      <div><span>Price</span><b>${plat(x.price)}</b></div>
      <div><span>Ducats</span><b>${x.ducats ? duc(x.ducats) : "–"}</b></div>
      <div><span>Owned</span><b>${stepper(x.uid, x.have)}</b></div>
    </div>
    ${acquisition(x.name, x.drops, x.dropCoverage)}
    ${x.recipe ? recipeDetails(x.recipe) : !x.recipesAvailable ? `<p class="muted">Component recipes are unavailable; check the wiki.</p>` : ""}
    ${x.relics.length ? `<div class="dsec"><h2>Dropped by <span class="count">${x.relics.length}</span></h2><div class="list">${x.relics.map((r) => `
      <div class="list-row click" data-open="${esc(r.relic)}"><div class="main"><div class="t">${esc(r.relic)} Relic</div>
        <div class="s">${esc(r.rarity || "")}${r.chance != null ? ` · ${r.chance}% intact` : ""}${r.owned ? ` · you own ${r.owned}` : ""}</div></div>
        ${r.open ? `<span class="pill lotus">Fissure open</span>` : ""}${r.vaulted ? `<span class="pill vaulted">Vaulted</span>` : ""}</div>`).join("")}</div></div>` : ""}
    ${x.usedIn.length ? `<div class="dsec"><h2>Builds</h2><div class="list">${x.usedIn.map((i) => `
      <div class="list-row click" data-open="${esc(i.uid)}">${art(i.image, "sm")}<div class="main"><div class="t">${esc(i.name)}</div></div>${statusPill(i.status)}</div>`).join("")}</div></div>` : ""}`;
}

function relic(x) {
  const counts = Object.entries(x.uids).map(([ref, uid]) => `<div><span>${esc(ref)}</span><b>${stepper(uid, x.counts[ref] || 0)}</b></div>`).join("");
  const pct = (v) => (v == null ? "?" : `${Math.round(v * 10) / 10}%`);
  const rows = x.rewards.map((r) => `<div class="list-row click" data-open="${esc(r.uid)}">${art(r.image, "sm")}
      <div class="main"><div class="t">${esc(r.name)}${r.needed > r.have ? ` <span class="pill lotus">needed</span>` : ""}</div>
        <div class="s">${esc(r.rarity || "")} · ${pct(r.chance.Intact)} intact, ${pct(r.chance.Radiant)} radiant${r.have ? ` · you have ${r.have}` : ""}</div></div>
      <div style="text-align:right">${plat(r.price)}<div>${duc(r.ducats)}</div></div></div>`).join("");
  return `<div class="dhead">${art(x.image, "xl")}<div class="grow"><h2>${esc(x.name)} Relic</h2>
      <div class="row wrap">${era(x.era)}${x.vaulted ? `<span class="pill vaulted">Vaulted</span>` : ""}
        ${x.open ? `<span class="pill lotus">${icon("fissures")} Fissure open now</span>` : ""}</div></div></div>
    <div class="kv">
      <div><span>Owned</span><b>${fmt(x.total)}</b></div>
      <div><span>Value radiant</span><b>${plat(x.ev.Radiant)}</b></div>
      <div><span>Value intact</span><b>${plat(x.ev.Intact)}</b></div>
    </div>
    <div class="dsec"><h2>Owned by refinement</h2><div class="kv" style="grid-template-columns:repeat(4,minmax(0,1fr))">${counts}</div></div>
    ${acquisition(`${x.name} Relic`, x.drops, x.dropCoverage, x.vaulted ? `<p class="muted">This relic is vaulted; current sources may be time-limited.</p>` : "")}
    <div class="dsec"><h2>Drops</h2><div class="list">${rows}</div></div>`;
}

function wire(d, x, version) {
  const refresh = () => { changed(); if (version === requestVersion) open(x.key, false); };
  d.querySelector("[data-close]").onclick = close;
  const back = d.querySelector("[data-back]");
  if (back) back.onclick = () => { stack.pop(); open(stack[stack.length - 1], false); };
  for (const el of d.querySelectorAll("[data-open]")) {
    el.onclick = (e) => { e.stopPropagation(); open(el.dataset.open); };
  }
  for (const b of d.querySelectorAll("[data-count]")) {
    b.onclick = async (e) => {
      e.stopPropagation();
      const count = Math.max(0, +b.dataset.c);
      if (await act("manual/count", { uid: b.dataset.count, count })) refresh();
    };
  }
  const pin = d.querySelector("[data-pin]");
  if (pin) pin.onclick = async () => {
    if (await act("manual/pin", { uid: x.key, pinned: !x.pinned }, x.pinned ? "Unpinned" : "Pinned to the Overview")) refresh();
  };
  const m = d.querySelector("[data-mastered]");
  if (m) m.onclick = async () => {
    if (await act("manual/mastered", { uid: x.uid, mastered: m.dataset.mastered === "1" }, "Saved")) refresh();
  };
}
