// Foundry: what is building, and everything you could build with the parts you hold.

import { $, act, artUrl, api, art, countdown, esc, fmt, icon, statusPill } from "../lib.js";
import { ICON, gimg } from "../game.js";
import * as drawer from "../drawer.js";

const st = { q: "", cat: "", show: "buildable", kind: "", myMr: false };
const KINDS = [["", "All"], ["gear", "Equipment"], ["craft", "Parts and gear"]];
const SHOW = [["buildable", "Requirements collected"], ["started", "Started"], ["missing", "Not built yet"], ["", "Everything"]];

export default {
  id: "foundry",
  title: "Foundry",
  icon: "foundry",
  async render(view, params, app) {
    const d = await api("foundry");
    if (d.loading) { setTimeout(() => app.refresh(), 2500); throw new Error("Downloading game data…"); }
    const cats = [...new Set(d.items.map((r) => r.category))].sort();
    const ready = d.items.filter((r) => r.buildable && r.status !== "mastered").length;
    // The low end of a badge band ("16-18") is the rank that is certain.
    const mr = app.state.masteryRank;
    view.innerHTML = `<div class="page">
      <div class="page-head"><div><h1>Foundry</h1><p>${d.pending.length} building · ${ready} with listed requirements collected. Equipment counts may include unbuilt blueprints; check resources and the foundry.</p></div></div>
      ${d.pending.length ? `<div class="card" style="margin-bottom:16px"><div class="card-head"><h2>Building</h2></div>
        <div class="cards">${d.pending.map((f) => `<div class="icard" ${f.known ? `data-uid="${esc(f.uid)}"` : `style="cursor:default"`}>${art(f.image)}
          <div class="body"><span class="name">${esc(f.name)}</span>
          <div class="meta">${f.done ? (f.done <= Date.now() / 1000 ? `<span class="pill good">Ready to claim</span>` : `Ready in ${countdown(f.done, "now")}`) : "building"}</div></div></div>`).join("")}</div></div>` : ""}
      <div class="toolbar">
        <input type="search" id="q" placeholder="Search" value="${esc(st.q)}" aria-label="Search">
        <div class="seg" id="show">${SHOW.map(([v, l]) => `<button data-v="${v}" class="${st.show === v ? "on" : ""}">${l}</button>`).join("")}</div>
        <div class="seg" id="kind" title="Equipment gives mastery; parts and gear are modular parts (amps, zaws, kitguns...), ciphers, restores and the like">${KINDS.map(([v, l]) => `<button data-v="${v}" class="${st.kind === v ? "on" : ""}">${l}</button>`).join("")}</div>
        <select id="cat" aria-label="Category"><option value="">All categories</option>${cats.map((c) => `<option ${st.cat === c ? "selected" : ""}>${esc(c)}</option>`).join("")}</select>
        ${mr != null ? `<label class="check" title="Hide what needs a higher mastery rank than yours"><input type="checkbox" id="mymr" ${st.myMr ? "checked" : ""}>Buildable at MR ${mr}</label>` : ""}
        <span class="count" id="cnt"></span>
      </div>
      <div class="cards" id="grid"></div></div>`;
    const draw = () => {
      const q = st.q.toLowerCase();
      const rows = d.items.filter((r) => (!q || r.name.toLowerCase().includes(q)) && (!st.cat || r.category === st.cat) &&
        (st.kind === "" || (st.kind === "craft") === !!r.craft) &&
        (!st.myMr || mr == null || (r.masteryReq || 0) <= mr) &&
        (st.show === "" || (st.show === "buildable" && r.buildable && r.status !== "mastered") ||
          (st.show === "started" && r.have > 0 && !r.buildable) ||
          (st.show === "missing" && (r.craft ? !r.held : r.status !== "mastered"))));
      $("#cnt").textContent = `${fmt(rows.length)} items`;
      $("#grid").innerHTML = rows.slice(0, 300).map((r) => `<div class="icard ${r.buildable ? "done" : ""}" ${r.craft ? `style="cursor:default"` : `data-uid="${esc(r.uid)}"`}>
          ${art(r.image, "lg")}
          <div class="body"><div class="row"><span class="name grow">${esc(r.name)}</span>${r.craft ? `<button class="ghost icon pin ${r.pinned ? "on" : ""}" data-pin="${esc(r.uid)}" title="${r.pinned ? "Unpin from the Overview" : "Pin to the Overview"}" style="padding:2px">${icon("star")}</button>` : ""}${r.masteryReq ? `<span class="mrreq" title="Needs mastery rank ${r.masteryReq}">${gimg(ICON.mastery, 14)}${r.masteryReq}</span>` : ""}</div>
            <div class="pips">${r.parts.map((p) => `<span class="pip ${p.have >= p.need ? "ok" : ""}" title="${esc(p.name)} - own ${fmt(p.have)} of ${fmt(p.need)}">
              ${p.image ? `<img alt="" src="${esc(artUrl(p.image, 24))}" onerror="this.remove()">` : p.have >= p.need ? icon("check") : ""}${!r.craft && p.have > 1 ? `<b>${p.have}</b>` : ""}</span>`).join("")}</div>
            <div class="meta"><span>${r.have}/${r.need} ${r.craft ? "needs met" : "parts"}</span>${r.craft
                ? `<span class="pill">${esc(r.category)}</span>${r.held ? `<span class="pill lotus">have ${fmt(r.held)}</span>` : ""}${r.quantity > 1 ? `<span class="faint">makes ${r.quantity}</span>` : ""}`
                : statusPill(r.status)}
              ${r.buildable && r.status !== "mastered" ? `<span class="pill good">${r.craft ? "needs met" : "parts collected"}</span>` : ""}${r.vaulted ? `<span class="pill vaulted">vaulted</span>` : ""}${r.craft ? `<a href="https://wiki.warframe.com/w/${encodeURIComponent(r.name.replace(/ /g, "_"))}" target="_blank" rel="noopener">Where to get</a>` : ""}</div>
          </div></div>`).join("") || `<div class="empty"><b>Nothing here</b>${st.show === "buildable" ? "No item has all its parts yet." : "Try another filter."}</div>`;
    };
    draw();
    $("#q").oninput = (e) => { st.q = e.target.value; draw(); };
    for (const b of view.querySelectorAll("#show button")) {
      b.onclick = () => { st.show = b.dataset.v; for (const x of view.querySelectorAll("#show button")) x.classList.toggle("on", x === b); draw(); };
    }
    $("#cat").onchange = (e) => { st.cat = e.target.value; draw(); };
    for (const b of view.querySelectorAll("#kind button")) {
      b.onclick = () => { st.kind = b.dataset.v; for (const x of view.querySelectorAll("#kind button")) x.classList.toggle("on", x === b); draw(); };
    }
    const my = $("#mymr");
    if (my) my.onchange = (e) => { st.myMr = e.target.checked; draw(); };
    $("#grid").addEventListener("click", async (e) => {
      const b = e.target.closest("[data-pin]");
      if (!b) return;
      e.stopPropagation();
      const row = d.items.find((r) => r.uid === b.dataset.pin);
      if (row && await act("manual/pin", { uid: row.uid, pinned: !row.pinned }, row.pinned ? "Unpinned" : "Pinned to the Overview")) { row.pinned = !row.pinned; draw(); }
    });
    view.querySelector(".page").onclick = (e) => { const c = e.target.closest("[data-uid]"); if (c && c.dataset.uid) drawer.open(c.dataset.uid); };
  },
};
