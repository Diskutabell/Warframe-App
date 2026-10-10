// Relics: what you hold, what each is worth, and which can be cracked right now.

import { $, api, art, era, esc, fmt, icon, plat } from "../lib.js";
import * as drawer from "../drawer.js";

const st = { owned: true, era: "", q: "", sort: "ev", unvaulted: false };
const ERAS = ["Lith", "Meso", "Neo", "Axi", "Requiem"];
const SORTS = [["ev", "Value"], ["need", "Needed drops"], ["name", "Name"]];

export default {
  id: "relics",
  title: "Relics",
  icon: "relics",
  async render(view, params, app) {
    const [data, fis] = await Promise.all([api("relics" + (st.owned ? "?owned=1" : "")), api("fissures").catch(() => null)]);
    if (data.loading) { setTimeout(() => app.refresh(), 2500); throw new Error("Downloading game data…"); }
    const open = new Set((fis?.fissures || []).filter((f) => !f.storm).map((f) => f.tier));
    // Vaulted relics you hold crack like any other.
    const isOpen = (r) => (!r.vaulted || r.total > 0) && (open.has(r.era) || open.has("Omnia"));
    const total = data.reduce((a, r) => a + r.total, 0);
    view.innerHTML = `<div class="page">
      <div class="page-head"><div><h1>Relics</h1><p>${st.owned ? `${fmt(total)} relics of ${fmt(data.length)} kinds` : `${fmt(data.length)} relics`} · value is the chance-weighted platinum of the drops</p></div></div>
      <div class="toolbar">
        <input type="search" id="q" placeholder="Relic or drop" value="${esc(st.q)}" aria-label="Search relics">
        <div class="seg" id="era"><button data-v="" class="${st.era === "" ? "on" : ""}">All</button>${ERAS.map((e) => `<button data-v="${e}" class="${st.era === e ? "on" : ""}">${e}</button>`).join("")}</div>
        <div class="seg" id="sort">${SORTS.map(([v, l]) => `<button data-v="${v}" class="${st.sort === v ? "on" : ""}">${l}</button>`).join("")}</div>
        <label class="check"><input type="checkbox" id="own" ${st.owned ? "checked" : ""}>Only mine</label>
        <label class="check"><input type="checkbox" id="unv" ${st.unvaulted ? "checked" : ""}>Not vaulted</label>
        <span class="count" id="cnt"></span>
      </div>
      <div class="cards" id="list"></div></div>`;
    const draw = () => {
      const q = st.q.toLowerCase();
      let rows = data.filter((r) => (!st.era || r.era === st.era) && (!st.unvaulted || !r.vaulted) &&
        (!q || (r.name + " " + r.rewards.map((x) => x.name).join(" ")).toLowerCase().includes(q)));
      if (st.sort === "ev") rows = [...rows].sort((a, b) => b.ev.Radiant - a.ev.Radiant);
      if (st.sort === "need") rows = [...rows].sort((a, b) => b.needsCount - a.needsCount || b.ev.Radiant - a.ev.Radiant);
      $("#cnt").textContent = `${fmt(rows.length)} relics`;
      $("#list").innerHTML = rows.slice(0, 400).map((r) => {
        const owned = Object.entries(r.counts).filter(([, n]) => n).map(([k, n]) => `${n}× ${k}`).join(", ");
        return `<div class="icard" data-relic="${esc(r.name)}">
          <div style="position:relative">${art(r.image, "lg")}${r.total ? `<span class="pill gold" style="position:absolute;right:-6px;bottom:-6px">${r.total}</span>` : ""}</div>
          <div class="body">
            <div class="row"><span class="name grow">${esc(r.name)}</span>${era(r.era)}</div>
            <div class="pips">${r.rewards.map((x) => `<span class="pip ${x.have > 0 ? "ok" : ""} ${x.needed > x.have ? "want" : ""}"
              title="${esc(x.name)} - ${esc(x.rarity || "")} · ${x.price != null ? x.price + "p" : "no price"} · own ${x.have}${x.needed > x.have ? " · needed" : ""}">
              ${x.image ? `<img alt="" src="${esc(x.image)}" onerror="this.remove()">` : ""}</span>`).join("")}</div>
            <div class="meta"><span>${plat(r.ev.Radiant)} radiant · ${plat(r.ev.Intact)} intact</span>
              ${r.needsCount ? `<span class="pill lotus">${r.needsCount} needed</span>` : ""}
              ${isOpen(r) ? `<span class="pill lotus">${icon("fissures")} open now</span>` : ""}
              ${r.vaulted ? `<span class="pill vaulted">vaulted</span>` : ""}</div>
            ${owned ? `<div class="meta">${esc(owned)}</div>` : ""}
          </div></div>`;
      }).join("") || `<div class="empty"><b>${st.owned ? "No relics tracked" : "No relics match"}</b>${st.owned ? "Import an inventory or set counts in a relic's details, or untick “Only mine”." : "Try another filter."}</div>`;
    };
    draw();
    $("#q").oninput = (e) => { st.q = e.target.value; draw(); };
    const seg = (id, key) => {
      for (const b of view.querySelectorAll(`#${id} button`)) {
        b.onclick = () => { st[key] = b.dataset.v; for (const x of view.querySelectorAll(`#${id} button`)) x.classList.toggle("on", x === b); draw(); };
      }
    };
    seg("era", "era");
    seg("sort", "sort");
    $("#own").onchange = (e) => { st.owned = e.target.checked; app.refresh(); };
    $("#unv").onchange = (e) => { st.unvaulted = e.target.checked; draw(); };
    $("#list").onclick = (e) => { const c = e.target.closest("[data-relic]"); if (c) drawer.open(c.dataset.relic); };
  },
};
