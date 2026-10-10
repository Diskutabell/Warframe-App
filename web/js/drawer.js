// The detail drawer: one item, part or relic, from anywhere in the app.

import { $, act, api, art, duc, era, esc, fmt, hydrateIcons, icon, plat, statusPill } from "./lib.js";

let stack = [];

export const isOpen = () => !$("#drawer").hidden;

/** Something was changed from the drawer: pages listening re-render. */
const changed = () => window.dispatchEvent(new CustomEvent("wfp:changed"));

export function close() {
  $("#drawer").hidden = true;
  $("#scrim").hidden = true;
  stack = [];
  // Back to the page, so its keyboard shortcuts work again.
  $("#view").focus();
  if (location.hash.includes("item=")) history.replaceState(null, "", location.hash.replace(/[?&]item=[^&]*/, ""));
}

export async function open(key, push = true) {
  if (push) stack.push(key);
  const d = $("#drawer");
  d.hidden = false;
  $("#scrim").hidden = false;
  d.innerHTML = `<div class="drawer-top"><span class="grow"></span><button class="ghost icon" data-close>${icon("x")}</button></div>
    <div class="stack"><div class="skeleton" style="width:60%"></div><div class="skeleton"></div><div class="skeleton" style="width:80%"></div></div>`;
  d.querySelector("[data-close]").onclick = close;
  let x;
  try {
    x = await api("item?key=" + encodeURIComponent(key));
  } catch (e) {
    d.querySelector(".stack").innerHTML = `<div class="callout">${esc(e.message)}</div>`;
    return;
  }
  const body = x.kind === "item" ? item(x) : x.kind === "part" ? part(x) : relic(x);
  const market = x.kind === "item" ? x.setSlug : x.slug;
  d.innerHTML = `<div class="drawer-top">
      ${stack.length > 1 ? `<button class="ghost" data-back>${icon("back")} Back</button>` : ""}
      <span class="grow"></span>
      ${market ? `<a class="btn ghost" href="https://warframe.market/items/${esc(market)}" target="_blank" rel="noopener">${icon("link")} warframe.market</a>` : ""}
      <button class="ghost icon" data-close title="Close (Esc)">${icon("x")}</button>
    </div>${body}`;
  hydrateIcons(d);
  wire(d, x);
  // Keyboard users land in the drawer, on its close button.
  d.querySelector("[data-close]").focus();
}

$("#scrim").onclick = close;

function stepper(uid, n) {
  return `<span class="stepper"><button data-count="${esc(uid)}" data-c="${n - 1}" aria-label="One fewer">−</button><b>${n}</b><button data-count="${esc(uid)}" data-c="${n + 1}" aria-label="One more">+</button></span>`;
}

function relicChips(relics) {
  return relics.map((r) => `<span class="chip ${r.vaulted ? "vaulted" : ""} ${r.open ? "open" : ""} ${r.owned ? "owned" : ""}" data-open="${esc(r.relic)}"
      title="${esc(r.rarity || "")}${r.chance != null ? ` · ${r.chance}% intact` : ""}${r.vaulted ? " · vaulted" : ""}${r.open ? " · fissure open now" : ""}">
      ${esc(r.relic)}${r.owned ? ` <b class="lotus">×${r.owned}</b>` : ""}</span>`).join("");
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
    </div>`).join("");
  return `<div class="dhead">${art(x.image, "xl")}<div class="grow">
      <h2>${esc(x.name)}</h2>
      <div class="row wrap">${statusPill(x.status)}<span class="pill">${esc(x.category)}</span>
        ${x.masteryReq ? `<span class="pill">MR ${x.masteryReq}</span>` : ""}
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
    ${x.parts.length ? `<div class="dsec"><h2>Parts ${x.buildable ? `<span class="pill good">ready to build</span>` : ""}</h2>${parts}</div>` : ""}
    ${x.requires.length ? `<div class="dsec"><h2>Also needs</h2><div class="list">${x.requires.map((r) => `
      <div class="list-row click" data-open="${esc(r.uid)}"><div class="main"><div class="t">${r.count > 1 ? `${r.count} × ` : ""}${esc(r.name)}</div></div>${r.status ? statusPill(r.status) : ""}</div>`).join("")}</div></div>` : ""}
    ${x.resources.length ? `<div class="dsec"><h2>Resources</h2><div class="chips">${x.resources.map((r) => `<span class="chip">${fmt(r.count)} × ${esc(r.name)}</span>`).join("")}</div></div>` : ""}
    ${x.disposition ? `<div class="dsec"><h2>Riven disposition</h2><span class="gold">${"●".repeat(x.dots || 0)}${"○".repeat(5 - (x.dots || 0))}</span> <span class="muted">${x.disposition}×</span></div>` : ""}`;
}

function part(x) {
  return `<div class="dhead">${art(x.image, "xl")}<div class="grow"><h2>${esc(x.name)}</h2>
      <div class="row wrap">${x.vaulted ? `<span class="pill vaulted">Vaulted</span>` : `<span class="pill good">Drops now</span>`}
        ${x.needed > x.have ? `<span class="pill lotus">Needed ×${x.needed}</span>` : ""}</div></div></div>
    <div class="kv">
      <div><span>Price</span><b>${plat(x.price)}</b></div>
      <div><span>Ducats</span><b>${x.ducats ? duc(x.ducats) : "–"}</b></div>
      <div><span>Owned</span><b>${stepper(x.uid, x.have)}</b></div>
    </div>
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
    <div class="dsec"><h2>Drops</h2><div class="list">${rows}</div></div>`;
}

function wire(d, x) {
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
      if (await act("manual/count", { uid: b.dataset.count, count })) { changed(); open(stack[stack.length - 1], false); }
    };
  }
  const m = d.querySelector("[data-mastered]");
  if (m) m.onclick = async () => {
    if (await act("manual/mastered", { uid: x.uid, mastered: m.dataset.mastered === "1" }, "Saved")) { changed(); open(stack[stack.length - 1], false); }
  };
}
