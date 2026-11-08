// Farming goals: one account's selected equipment and its known requirements.

import { $, act, api, art, debounce, esc, fmt } from "../lib.js";
import * as drawer from "../drawer.js";

let query = "";

function sources(part) {
  const relics = part.relics || [];
  const drops = part.drops || [];
  if (!relics.length && !drops.length) {
    return `<div class="muted">No source in the loaded drop tables. Open the part for the wiki link.</div>`;
  }
  return `${relics.length ? `<div class="chips">${relics.map((r) =>
    `<button class="chip ${r.vaulted ? "vaulted" : ""} ${r.owned ? "owned" : ""}" data-open="${esc(r.relic)}">
      ${esc(r.relic)}${r.owned ? ` · owned ${fmt(r.owned)}` : ""}${r.open ? " · fissure open" : ""}${r.vaulted ? " · vaulted" : ""}
      ${r.chance != null ? ` · ${r.chance}% intact` : ""}</button>`).join("")}</div>` : ""}
    ${drops.length ? `<details class="more"><summary>Direct drops (${drops.length})</summary><div class="list">${drawer.dropRows(drops)}</div></details>` : ""}`;
}

function resourceTotals(parts, resources) {
  const totals = new Map();
  const add = (r, times) => {
    const existing = totals.get(r.uid);
    if (existing) existing.count += r.count * times;
    else totals.set(r.uid, { ...r, count: r.count * times });
  };
  for (const resource of resources) add(resource, 1);
  for (const part of parts) {
    if (!part.recipe) continue;
    const builds = Math.ceil(part.need / part.recipe.quantity);
    for (const ingredient of part.recipe.ingredients) add(ingredient, builds);
  }
  return [...totals.values()].sort((a, b) => a.name.localeCompare(b.name));
}

function goalCard(x, inventoryKnown) {
  const parts = x.parts || [];
  const resources = x.resources || [];
  const requires = x.requires || [];
  const totals = resourceTotals(parts, resources);
  const missing = parts.filter((p) => p.have < p.need);
  const priority = (p) => p.relics?.some((r) => r.owned && r.open) ? 0
    : p.relics?.some((r) => !r.vaulted) ? 1 : p.drops?.length ? 2 : 3;
  missing.sort((a, b) => priority(a) - priority(b));
  const next = x.owned > 0 ? "You own this item."
    : x.building ? "This item is in the foundry."
      : missing.length ? `Consider starting with ${missing[0].name}.`
        : parts.length ? "Check the foundry: collected parts may still need crafting."
          : "Check the item’s acquisition sources.";
  return `<section class="card" style="margin-top:16px">
    <div class="card-head"><div class="row">${art(x.image, "sm")}<div><h2>${esc(x.name)}</h2><span class="muted">${esc(next)}</span></div></div>
      <div class="row"><button data-open="${esc(x.uid)}">Item details</button><button data-remove="${esc(x.uid)}">Remove goal</button></div></div>
    ${!inventoryKnown ? `<p class="muted">No inventory loaded: counts below only include manual entries and may be incomplete.</p>` : ""}
    ${parts.length ? `<p class="muted">Part counts combine blueprints and crafted parts. Recipes are shown per build, even if a part is already crafted; check your foundry.</p>
      ${!x.recipesAvailable ? `<p class="callout">Component recipes are unavailable. Connect to download the recipe data or check the wiki.</p>` : ""}
      <div class="stack">${parts.map((p) => `<div class="part">
        <div class="row"><button class="ghost grow" data-open="${esc(p.uid)}" style="text-align:left"><b>${esc(p.name)}</b></button>
          <span class="${p.have >= p.need ? "lotus" : "warn-text"}">${fmt(p.have)} / ${fmt(p.need)} held${p.have < p.need ? ` · ${fmt(p.need - p.have)} missing` : ""}</span></div>
        ${p.have < p.need ? sources(p) : ""}
        ${p.recipe ? drawer.recipeDetails(p.recipe) : ""}
      </div>`).join("")}</div>` : ""}
    ${requires.length ? `<div class="dsec"><h2>Equipment required</h2><div class="list">${requires.map((r) =>
      `<div class="list-row"><button class="ghost grow" data-open="${esc(r.uid)}" style="text-align:left">${esc(r.name)}</button>
      <span>${r.have == null ? "Owned: unknown" : `${fmt(r.have)} / ${fmt(r.count)} owned`}</span></div>`).join("")}</div></div>` : ""}
    ${totals.length ? `<div class="dsec"><h2>Known materials total</h2>
      <p class="muted">Final build plus listed component recipes, assuming every component still needs crafting. Already crafted parts, missing recipes and other equipment requirements are not accounted for.</p>
      <div class="list">${drawer.resourceRows(totals)}</div></div>` : ""}
    <p class="muted" style="font-size:13px">Recipes and sources may be incomplete or cached. Quest, purchase and time-limited sources may be missing; check item details or the wiki.</p>
  </section>`;
}

export default {
  id: "farming",
  title: "Farming",
  icon: "search",
  async render(view, params, app) {
    const data = await api("goals");
    if (data.loading) {
      view.innerHTML = `<div class="page"><div class="empty"><b>Loading game data…</b>${esc(data.error || "")}</div></div>`;
      setTimeout(() => app.refresh(), 2500);
      return;
    }
    const goals = await Promise.all(data.goals.map(async (uid) => {
      try { return { uid, item: await api("item?key=" + encodeURIComponent(uid)) }; }
      catch (error) { return { uid, error: error.message }; }
    }));
    view.innerHTML = `<div class="page"><div class="page-head"><div><h1>Farming planner</h1>
      <p>Choose equipment to see its parts, what you hold and where to farm the rest. Goals are saved per account.</p></div></div>
      <div class="toolbar"><input id="goalSearch" type="search" aria-label="Find equipment to farm" placeholder="Find equipment, e.g. Volt Prime" value="${esc(query)}"></div>
      <div id="goalResults"></div>
      <div id="goals">${goals.map((g) => g.item ? goalCard(g.item, data.inventoryKnown)
        : `<div class="callout">${esc(g.uid)}: ${esc(g.error)} <button data-remove="${esc(g.uid)}">Remove goal</button></div>`).join("")
        || `<div class="empty"><b>No farming goals yet</b>Search for equipment above or choose “Set farming goal” in an item’s details.</div>`}</div></div>`;
    const search = async () => {
      const current = query;
      if (current.trim().length < 2) { $("#goalResults").innerHTML = ""; return; }
      try {
        const found = await api("find-equipment?q=" + encodeURIComponent(current));
        if (!view.isConnected || $("#goalSearch")?.value !== current) return;
        if (found.loading) throw new Error(found.error || "Game data is still loading.");
        const items = found;
        $("#goalResults").innerHTML = items.length ? `<div class="list">${items.map((r) =>
          `<div class="list-row">${art(r.image, "sm")}<div class="main"><div class="t">${esc(r.name)}</div><div class="s">${esc(r.sub)}</div></div>
          <button data-add="${esc(r.key)}" ${data.goals.includes(r.key) ? "disabled" : ""}>${data.goals.includes(r.key) ? "Added" : "Add goal"}</button></div>`).join("")}</div>`
          : `<div class="muted">No matching equipment. Use Ctrl+K to search drop sources, including mods.</div>`;
      } catch (e) {
        if ($("#goalSearch")?.value === current) $("#goalResults").innerHTML = `<div class="callout">${esc(e.message)}</div>`;
      }
    };
    $("#goalSearch").oninput = (e) => { query = e.target.value; searchLater(); };
    const searchLater = debounce(search, 150);
    if (query) search();
    view.querySelector(".page").onclick = async (e) => {
      const open = e.target.closest("[data-open]");
      if (open) { drawer.open(open.dataset.open); return; }
      const add = e.target.closest("[data-add]");
      const remove = e.target.closest("[data-remove]");
      if (add || remove) {
        if (await act("manual/goal", { uid: add ? add.dataset.add : remove.dataset.remove, selected: !!add })) app.refresh();
      }
    };
  },
};
