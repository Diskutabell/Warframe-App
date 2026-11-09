// Inventory: everything the last memory scan (or imported file) holds - resources, mods, arcanes,
// relics, parts, fish, decorations, equipment - searchable, with the game's own icons.

import { $, api, artUrl, debounce, esc, fmt, ago } from "../lib.js";
import * as drawer from "../drawer.js";

const st = { q: "", kind: "", sort: "name", tradable: false, limit: 240 };
// Kinds the details panel knows; the rest have nothing more to show.
const DETAILS = new Set(["Warframes", "Primary", "Secondary", "Melee", "Sentinels", "Archwing", "Arch-Gun", "Arch-Melee",
  "Pets", "Misc", "Prime parts", "Parts", "Relics"]);
// Equipment kinds from the catalog, gathered under one chip.
const EQUIPMENT = new Set(["Warframes", "Primary", "Secondary", "Melee", "Sentinels", "Archwing", "Arch-Gun", "Arch-Melee",
  "Pets", "Misc", "Weapons", "Amps", "Companions"]);
const group = (kind) => (EQUIPMENT.has(kind) ? "Equipment" : kind);

const SORTS = [["name", "Name"], ["count", "Most held"], ["value", "Value"]];
const value = (x) => (x.price || 0) * x.count;

const pic = (x) => (x.image
  ? `<img class="inv-pic" loading="lazy" decoding="async" alt="" src="${esc(artUrl(x.image, 52))}" onerror="this.replaceWith(Object.assign(document.createElement('span'),{className:'inv-pic none'}))">`
  : `<span class="inv-pic none"></span>`);

function tile(x) {
  const open = DETAILS.has(x.kind);
  const money = [
    x.price != null ? `<span class="plat" title="Lowest sell order, each">${fmt(x.price)}</span>` : "",
    x.ducats ? `<span class="duc" title="Ducats each">${fmt(x.ducats)}</span>` : "",
  ].join("");
  return `<${open ? "button type=\"button\"" : "div"} class="inv-tile${open ? " click" : ""}" ${open ? `data-uid="${esc(x.uid)}"` : ""} title="${esc(x.name)}">
    ${pic(x)}<span class="inv-body"><span class="inv-name">${esc(x.name)}</span>
      <span class="inv-meta"><span class="inv-kind">${esc(x.kind)}</span>${money}</span></span>
    <span class="inv-count">${x.count > 1 || !EQUIPMENT.has(x.kind) ? fmt(x.count) : ""}</span></${open ? "button" : "div"}>`;
}

export default {
  id: "inventory",
  title: "Inventory",
  icon: "inventory",
  async render(view) {
    const data = await api("inventory");
    const items = data.items || [];
    if (!items.length) {
      view.innerHTML = `<div class="page"><div class="page-head"><div><h1>Inventory</h1></div></div>
        <div class="empty"><b>No inventory loaded yet</b>Switch on the memory scan in Settings and it is read while the game runs,
        after login and after each mission. An exported inventory file can be imported there too.</div></div>`;
      return;
    }
    const c = data.currencies || {};
    const cur = (icon, n, label) => (n == null ? "" : `<div class="tile cur"><img src="icons/${icon}.png" alt="" width="26" height="26"><div><b>${fmt(n)}</b><span>${label}</span></div></div>`);
    const kinds = new Map();
    for (const x of items) kinds.set(group(x.kind), (kinds.get(group(x.kind)) || 0) + 1);
    const chips = [...kinds].sort((a, b) => b[1] - a[1]);
    view.innerHTML = `<div class="page">
      <div class="page-head"><div><h1>Inventory</h1><p>${fmt(items.length)} stacks · ${esc(data.source || "")} ${ago(data.imported)}</p></div></div>
      <div class="tiles cur-row">${cur("platinum", c.platinum, "Platinum")}${cur("credits", c.credits, "Credits")}${cur("endo", c.endo, "Endo")}${cur("ducats", c.ducats, "Ducats")}</div>
      <div class="toolbar">
        <input type="search" id="q" placeholder="Search the inventory" value="${esc(st.q)}" aria-label="Search">
        <div class="seg" id="sort">${SORTS.map(([v, l]) => `<button data-v="${v}" class="${st.sort === v ? "on" : ""}">${l}</button>`).join("")}</div>
        <label class="check"><input type="checkbox" id="trade" ${st.tradable ? "checked" : ""}>Tradable</label>
        <span class="count" id="cnt"></span>
      </div>
      <div class="kind-chips" id="kinds"><button class="chip ${st.kind ? "" : "on"}" data-kind="">All <span>${fmt(items.length)}</span></button>
        ${chips.map(([k, n]) => `<button class="chip ${st.kind === k ? "on" : ""}" data-kind="${esc(k)}">${esc(k)} <span>${fmt(n)}</span></button>`).join("")}</div>
      <div class="inv-grid" id="grid"></div>
      <div id="more" style="margin-top:12px"></div></div>`;
    const draw = () => {
      const q = st.q.toLowerCase();
      const rows = items.filter((x) => (!st.kind || group(x.kind) === st.kind) && (!st.tradable || x.slug)
        && (!q || x.name.toLowerCase().includes(q) || x.kind.toLowerCase().includes(q)));
      rows.sort(st.sort === "count" ? (a, b) => b.count - a.count
        : st.sort === "value" ? (a, b) => value(b) - value(a) || (b.ducats || 0) - (a.ducats || 0)
          : (a, b) => a.name.localeCompare(b.name));
      $("#cnt").textContent = `${fmt(rows.length)} shown`;
      $("#grid").innerHTML = rows.slice(0, st.limit).map(tile).join("")
        || `<div class="empty" style="grid-column:1/-1"><b>Nothing matches</b>Try another word or kind.</div>`;
      $("#more").innerHTML = rows.length > st.limit ? `<button id="moreBtn">Show ${fmt(Math.min(240, rows.length - st.limit))} more</button>` : "";
      const mb = $("#moreBtn");
      if (mb) mb.onclick = () => { st.limit += 240; draw(); };
    };
    draw();
    $("#q").oninput = debounce((e) => { st.q = e.target.value; st.limit = 240; draw(); }, 120);
    $("#trade").onchange = (e) => { st.tradable = e.target.checked; draw(); };
    for (const b of view.querySelectorAll("#sort button")) {
      b.onclick = () => { st.sort = b.dataset.v; for (const x of view.querySelectorAll("#sort button")) x.classList.toggle("on", x === b); draw(); };
    }
    $("#kinds").onclick = (e) => {
      const b = e.target.closest("[data-kind]");
      if (!b) return;
      st.kind = b.dataset.kind;
      st.limit = 240;
      for (const x of view.querySelectorAll("#kinds .chip")) x.classList.toggle("on", x === b);
      draw();
    };
    $("#grid").onclick = (e) => { const t = e.target.closest("[data-uid]"); if (t) drawer.open(t.dataset.uid); };
  },
};
