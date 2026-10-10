// Foundry: what is building, and everything you could build with the parts you hold.

import { $, api, art, countdown, esc, fmt, icon, statusPill } from "../lib.js";
import * as drawer from "../drawer.js";

const st = { q: "", cat: "", show: "buildable" };
const SHOW = [["buildable", "Ready to build"], ["started", "Started"], ["missing", "Not built yet"], ["", "Everything"]];

export default {
  id: "foundry",
  title: "Foundry",
  icon: "foundry",
  async render(view, params, app) {
    const d = await api("foundry");
    if (d.loading) { setTimeout(() => app.refresh(), 2500); throw new Error("Downloading game data…"); }
    const cats = [...new Set(d.items.map((r) => r.category))].sort();
    const ready = d.items.filter((r) => r.buildable && r.status !== "mastered").length;
    view.innerHTML = `<div class="page">
      <div class="page-head"><div><h1>Foundry</h1><p>${d.pending.length} building · ${ready} ready to build from parts you hold</p></div></div>
      ${d.pending.length ? `<div class="card" style="margin-bottom:16px"><div class="card-head"><h2>Building</h2></div>
        <div class="cards">${d.pending.map((f) => `<div class="icard" ${f.known ? `data-uid="${esc(f.uid)}"` : `style="cursor:default"`}>${art(f.image)}
          <div class="body"><span class="name">${esc(f.name)}</span>
          <div class="meta">${f.done ? (f.done <= Date.now() / 1000 ? `<span class="pill good">Ready to claim</span>` : `Ready in ${countdown(f.done, "now")}`) : "building"}</div></div></div>`).join("")}</div></div>` : ""}
      <div class="toolbar">
        <input type="search" id="q" placeholder="Search" value="${esc(st.q)}" aria-label="Search">
        <div class="seg" id="show">${SHOW.map(([v, l]) => `<button data-v="${v}" class="${st.show === v ? "on" : ""}">${l}</button>`).join("")}</div>
        <select id="cat" aria-label="Category"><option value="">All categories</option>${cats.map((c) => `<option ${st.cat === c ? "selected" : ""}>${esc(c)}</option>`).join("")}</select>
        <span class="count" id="cnt"></span>
      </div>
      <div class="cards" id="grid"></div></div>`;
    const draw = () => {
      const q = st.q.toLowerCase();
      const rows = d.items.filter((r) => (!q || r.name.toLowerCase().includes(q)) && (!st.cat || r.category === st.cat) &&
        (st.show === "" || (st.show === "buildable" && r.buildable && r.status !== "mastered") ||
          (st.show === "started" && r.have > 0 && !r.buildable) || (st.show === "missing" && r.status !== "mastered")));
      $("#cnt").textContent = `${fmt(rows.length)} items`;
      $("#grid").innerHTML = rows.slice(0, 300).map((r) => `<div class="icard ${r.buildable ? "done" : ""}" data-uid="${esc(r.uid)}">
          ${art(r.image, "lg")}
          <div class="body"><div class="row"><span class="name grow">${esc(r.name)}</span>${r.masteryReq ? `<span class="faint" style="font-size:12px">MR ${r.masteryReq}</span>` : ""}</div>
            <div class="pips">${r.parts.map((p) => `<span class="pip ${p.have >= p.need ? "ok" : ""}" title="${esc(p.name)} - own ${p.have} of ${p.need}">
              ${p.image ? `<img alt="" src="${esc(p.image)}" onerror="this.remove()">` : p.have >= p.need ? icon("check") : ""}${p.have > 1 ? `<b>${p.have}</b>` : ""}</span>`).join("")}</div>
            <div class="meta"><span>${r.have}/${r.need} parts</span>${statusPill(r.status)}
              ${r.buildable && r.status !== "mastered" ? `<span class="pill good">ready</span>` : ""}${r.vaulted ? `<span class="pill vaulted">vaulted</span>` : ""}</div>
          </div></div>`).join("") || `<div class="empty"><b>Nothing here</b>${st.show === "buildable" ? "No item has all its parts yet." : "Try another filter."}</div>`;
    };
    draw();
    $("#q").oninput = (e) => { st.q = e.target.value; draw(); };
    for (const b of view.querySelectorAll("#show button")) {
      b.onclick = () => { st.show = b.dataset.v; for (const x of view.querySelectorAll("#show button")) x.classList.toggle("on", x === b); draw(); };
    }
    $("#cat").onchange = (e) => { st.cat = e.target.value; draw(); };
    view.querySelector(".page").onclick = (e) => { const c = e.target.closest("[data-uid]"); if (c && c.dataset.uid) drawer.open(c.dataset.uid); };
  },
};
