// Market: what your spare parts are worth, and a price check for anything on warframe.market.

import { $, api, art, debounce, duc, esc, fmt, plat } from "../lib.js";
import * as drawer from "../drawer.js";

const st = { sort: "plat", q: "" };
const SORTS = [["plat", "Platinum"], ["ducats", "Ducats"], ["ratio", "Ducats per plat"]];

function spareRows(rows) {
  const key = { plat: (r) => (r.price || 0) * r.spare, ducats: (r) => (r.ducats || 0) * r.spare, ratio: (r) => r.ducatsPerPlat || 0 }[st.sort];
  return [...rows].sort((a, b) => key(b) - key(a)).map((r) => `<tr class="click" data-uid="${esc(r.uid)}">
      <td><div class="row">${art(r.image, "sm")}<span class="ellipsis" style="font-weight:600">${esc(r.name)}</span>${r.vaulted ? `<span class="pill vaulted">vaulted</span>` : ""}</div></td>
      <td class="r num">${r.spare}<span class="faint"> / ${r.have}</span></td>
      <td class="r">${plat(r.price)}</td><td class="r">${r.ducats ? duc(r.ducats) : `<span class="faint">–</span>`}</td>
      <td class="r num">${r.ducatsPerPlat ?? `<span class="faint">–</span>`}</td>
      <td class="r">${plat(r.price != null ? r.price * r.spare : null)}</td></tr>`).join("");
}

function checkRows(rows) {
  return rows.map((r) => `<tr>
      <td><div class="row">${art(r.image, "sm")}<div><a href="https://warframe.market/items/${esc(r.slug)}" target="_blank" rel="noopener">${esc(r.name)}</a>
        <a class="muted" href="https://wiki.warframe.com/w/${encodeURIComponent(r.name.replace(/ /g, "_"))}" target="_blank" rel="noopener" title="Find ${esc(r.name)} on the Warframe Wiki"> · sources</a></div></div></td>
      <td class="r">${r.sell != null ? plat(r.sell) : `<span class="faint">…</span>`}</td>
      <td class="r">${r.buy != null ? plat(r.buy) : `<span class="faint">…</span>`}</td>
      <td class="r">${r.ducats ? duc(r.ducats) : ""}</td></tr>`).join("") || `<tr><td colspan="4" class="muted">Nothing on warframe.market matches.</td></tr>`;
}

const UNITS = { "Punch Through": "m", "Combo Duration": "s", "Initial Combo": "" };
const gradeClass = (g) => (g?.[0] === "A" ? "a" : g?.[0] === "B" ? "b" : "c");

/** The last riven the hotkey read: the same grades and prices as the panel over the game. */
function rivenCard(r) {
  if (!r || (!r.weapon && !r.error)) {
    return `<div class="empty"><b>No riven read yet</b>Open a riven in the game so its card shows (cycling it, or its own screen) and press the riven hotkey (F10 unless you changed it in Settings).</div>`;
  }
  if (r.error) return `<div class="callout">${esc(r.error)}</div>`;
  const m = r.market;
  const med = (k) => r.traded?.[k]?.median;
  return `<div class="riv-head">${art(r.image, "lg")}<div class="grow"><div class="riv-weapon">${esc(r.weapon)}</div>
      <div class="muted">${r.positives} positives${r.negatives ? " + 1 negative" : ""}${r.unread ? ` · ${r.unread} not readable` : ""} · ${esc(new Date(r.t * 1000).toLocaleString())}</div></div>
      ${r.overall ? `<span class="riv-overall ${gradeClass(r.overall)}" title="The average roll">${esc(r.overall)}</span>` : ""}</div>
    <div class="riv-stats">${r.stats.map((g) => `<div class="riv-stat"><span class="riv-grade ${gradeClass(g.grade)}">${esc(g.grade)}</span>
      <span class="grow">${g.negative ? "−" : "+"}<b>${Math.abs(g.value)}${UNITS[g.stat] ?? "%"}</b> ${esc(g.stat)}</span><span class="muted num">${g.deviation > 0 ? "+" : ""}${g.deviation.toFixed(1)}%</span>
      <div class="riv-roll ${gradeClass(g.grade)}"><i style="width:${Math.max(0, Math.min(100, (g.deviation + 10) * 5))}%"></i></div></div>`).join("")}</div>
    <div class="tiles" style="margin-top:12px">
      <div class="tile"><b>${m?.count ? plat(m.lowest) : "–"}</b><span>${m ? `lowest of ${fmt(m.count)} listed with these positives` : r.marketError ? "warframe.market did not answer" : "looking up listings…"}</span></div>
      <div class="tile"><b>${m?.median != null ? plat(m.median) : "–"}</b><span>median listing</span></div>
      <div class="tile"><b>${med("rerolled") != null ? plat(Math.round(med("rerolled"))) : "–"}</b><span>sold lately, rerolled</span></div>
      <div class="tile"><b>${med("unrolled") != null ? plat(Math.round(med("unrolled"))) : "–"}</b><span>sold lately, unrolled</span></div></div>
    ${m?.url ? `<p style="margin:10px 0 0"><a href="${esc(m.url)}" target="_blank" rel="noopener">See these listings on warframe.market</a></p>` : ""}`;
}

export default {
  id: "market",
  title: "Market",
  icon: "market",
  async render(view, params, app) {
    const [d, riven] = await Promise.all([api("trade"), api("riven/result").catch(() => null)]);
    if (d.loading) { setTimeout(() => app.refresh(), 2500); throw new Error("Downloading game data…"); }
    const rows = d.rows;
    const platTotal = rows.reduce((a, r) => a + (r.price || 0) * r.spare, 0);
    const ducTotal = rows.reduce((a, r) => a + (r.ducats || 0) * r.spare, 0);
    const parts = rows.reduce((a, r) => a + r.spare, 0);
    view.innerHTML = `<div class="page">
      <div class="page-head"><div><h1>Market</h1><p>Prices are the lowest sell and highest buy orders from players online on warframe.market.</p></div></div>
      <div class="grid cols-2">
        <div class="card"><div class="card-head"><h2>Spare prime parts</h2><span class="grow"></span>
            <div class="seg" id="sort">${SORTS.map(([v, l]) => `<button data-v="${v}" class="${st.sort === v ? "on" : ""}">${l}</button>`).join("")}</div></div>
          <p class="muted" style="margin:-4px 0 12px;font-size:13.5px">Parts you own beyond what your unfinished sets still need${d.source === "manual" ? " - counted from what you entered by hand; an inventory gives complete numbers" : ""}.</p>
          ${rows.length ? `<div class="tiles" style="margin-bottom:12px">
              <div class="tile"><b>${fmt(parts)}</b><span>spare parts</span></div>
              <div class="tile"><b>${plat(platTotal)}</b><span>at the lowest sell price</span></div>
              <div class="tile"><b>${duc(ducTotal) || "–"}</b><span>if all go to Baro</span></div></div>
            <div class="table-wrap" style="max-height:calc(100vh - 330px)"><table class="table">
              <thead><tr><th>Part</th><th class="r">Spare</th><th class="r">Each</th><th class="r">Ducats</th><th class="r" title="Ducats per platinum: a high value is worth more to Baro than to other players">Ducats/p</th><th class="r">Total</th></tr></thead>
              <tbody id="spare">${spareRows(rows)}</tbody></table></div>`
            : `<div class="empty"><b>No spare parts</b>This needs part counts: an inventory (Settings), or counts set by hand on an item's details.</div>`}
        </div>
        <div class="card"><div class="card-head"><h2>Price check</h2></div>
          <input type="search" id="q" placeholder="Any tradable item, e.g. Saryn Prime Set" value="${esc(st.q)}" aria-label="Item name" style="width:100%;margin-bottom:12px">
          <div class="table-wrap" style="max-height:calc(100vh - 290px)"><table class="table">
            <thead><tr><th>Item</th><th class="r">Sell</th><th class="r">Buy</th><th class="r">Ducats</th></tr></thead>
            <tbody id="check"><tr><td colspan="4" class="muted">Type two letters or more.</td></tr></tbody></table></div>
        </div>
      </div>
      <div class="card" style="margin-top:16px"><div class="card-head"><h2>Last riven</h2></div>${rivenCard(riven)}</div></div>`;
    for (const b of view.querySelectorAll("#sort button")) {
      b.onclick = () => {
        st.sort = b.dataset.v;
        for (const x of view.querySelectorAll("#sort button")) x.classList.toggle("on", x === b);
        if ($("#spare")) $("#spare").innerHTML = spareRows(rows);
      };
    }
    const spare = $("#spare");
    if (spare) spare.onclick = (e) => { const tr = e.target.closest("[data-uid]"); if (tr) drawer.open(tr.dataset.uid); };
    // Prices arrive in the background; ask again until every row has one (or a few tries passed).
    let token = 0;
    const check = async (q, tries = 0) => {
      const mine = ++token;
      if (q.trim().length < 2) { $("#check").innerHTML = `<tr><td colspan="4" class="muted">Type two letters or more.</td></tr>`; return; }
      const res = await api("search?q=" + encodeURIComponent(q)).catch((e) => ({ error: e.message }));
      if (mine !== token || !$("#check")) return;
      if (res.error) { $("#check").innerHTML = `<tr><td colspan="4" class="warn-text">${esc(res.error)}</td></tr>`; return; }
      $("#check").innerHTML = checkRows(res);
      if (res.some((r) => r.sell == null) && tries < 6) setTimeout(() => mine === token && check(q, tries + 1), 1500);
    };
    const later = debounce((q) => check(q), 250);
    $("#q").oninput = (e) => { st.q = e.target.value; later(st.q); };
    if (st.q) check(st.q);
  },
};
