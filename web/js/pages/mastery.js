// Mastery: everything that gives mastery, what is left, and what to do about it.

import { $, api, art, esc, fmt, partPips, statusPill } from "../lib.js";
import * as drawer from "../drawer.js";

const st = { q: "", status: "todo", cat: "", prime: "", layout: "table", unvaulted: false, buildable: false, maxMr: "", limit: 300 };

const STATUS_FILTERS = [["todo", "To do"], ["missing", "Not levelled"], ["leveling", "Levelling"], ["mastered", "Mastered"], ["", "All"]];

function next(r) {
  if (r.status === "mastered") return "";
  if (r.building) return `<span class="lotus">In the foundry</span>`;
  if (r.owned) return "Owned - level it";
  const parts = r.parts || [];
  if (r.status === "leveling") return `${r.rank}/${r.maxRank}, keep levelling`;
  if (!parts.length) return r.requires?.length ? `Needs ${esc(r.requires.join(", "))}` : "";
  const have = parts.filter((p) => p.have >= p.need).length;
  if (r.buildable) return `<span class="lotus">Parts collected</span>`;
  const farm = (r.farm || [])[0];
  return `${have}/${parts.length} parts${farm ? ` · <span class="muted">${esc(farm.relics[0] || "")}</span>` : ""}`;
}

export default {
  id: "mastery",
  title: "Mastery",
  icon: "mastery",
  async render(view, params, app) {
    const data = await api("mastery");
    if (data.loading) {
      view.innerHTML = `<div class="page"><div class="empty"><b>Downloading game data…</b>${esc(data.error || "")}</div></div>`;
      setTimeout(() => app.refresh(), 2500);
      return;
    }
    const cats = (app.state.mastery?.byCategory || []);
    const allCats = [...new Set(data.map((r) => r.category))].sort();
    view.innerHTML = `<div class="page">
      <div class="page-head"><div><h1>Mastery</h1><p>${fmt(app.state.mastered)} of ${fmt(app.state.total)} mastered · ${fmt(app.state.mastery?.total)}${app.state.mastery?.lacks ? "+" : ""} mastery XP · ${fmt(app.state.mastery?.points)} of ${fmt(app.state.mastery?.possible)} from gear<br>Choose an item to see its parts.</p></div></div>
      <div class="tiles" style="margin-bottom:16px">${cats.map((c) => `<div class="tile" style="cursor:pointer" data-cat="${esc(c.category)}">
          <div class="row spread"><span class="muted" style="font-size:12.5px">${esc(c.category)}</span><span class="num" style="font-size:12.5px">${c.mastered}/${c.total}</span></div>
          <div class="bar thin" style="margin-top:8px"><i style="width:${(100 * c.mastered / Math.max(1, c.total)).toFixed(1)}%"></i></div></div>`).join("")}</div>
      <div class="toolbar">
        <input type="search" id="q" placeholder="Search" value="${esc(st.q)}" aria-label="Search">
        <div class="seg" id="status">${STATUS_FILTERS.map(([v, l]) => `<button data-v="${v}" class="${st.status === v ? "on" : ""}">${l}</button>`).join("")}</div>
        <select id="cat" aria-label="Category"><option value="">All categories</option>${allCats.map((c) => `<option ${st.cat === c ? "selected" : ""}>${esc(c)}</option>`).join("")}</select>
        <select id="mr" aria-label="Mastery rank needed"><option value="">Any MR</option>${Array.from({ length: 19 }, (_, i) => `<option value="${i}" ${String(st.maxMr) === String(i) ? "selected" : ""}>MR ${i} or lower</option>`).join("")}</select>
        <select id="prime" aria-label="Prime filter"><option value="" ${st.prime === "" ? "selected" : ""}>All items</option><option value="prime" ${st.prime === "prime" ? "selected" : ""}>Primes only</option><option value="nonprime" ${st.prime === "nonprime" ? "selected" : ""}>Non-Primes only</option></select>
        <label class="check"><input type="checkbox" id="unv" ${st.unvaulted ? "checked" : ""}>Not vaulted</label>
        <label class="check" title="Parts may include unbuilt blueprints; resources and required equipment are not checked"><input type="checkbox" id="bld" ${st.buildable ? "checked" : ""}>Parts collected</label>
        <div class="seg" id="layout" aria-label="Mastery display">
          <button data-v="table" aria-pressed="${st.layout === "table"}" class="${st.layout === "table" ? "on" : ""}">Table</button>
          <button data-v="cards" aria-pressed="${st.layout === "cards"}" class="${st.layout === "cards" ? "on" : ""}">Cards</button>
        </div>
        <span class="count" id="cnt"></span>
      </div>
      <div id="results"><div class="table-wrap" id="table-view"><table class="table">
        <thead><tr><th>Item</th><th>Category</th><th class="r">MR</th><th style="width:160px">Rank</th><th>Status</th><th>Next</th></tr></thead>
        <tbody id="tb"></tbody></table></div><div class="cards" id="card-view" hidden></div></div>
      <div id="more" style="margin-top:12px"></div></div>`;
    const draw = () => {
      const q = st.q.toLowerCase();
      const rows = data.filter((r) =>
        (st.status === "" || (st.status === "todo" ? r.status !== "mastered" : r.status === st.status)) &&
        (!st.cat || r.category === st.cat) && (!st.prime || (st.prime === "prime" ? r.isPrime : !r.isPrime)) && (!st.unvaulted || !r.vaulted) &&
        (!st.buildable || r.buildable) && (!q || r.name.toLowerCase().includes(q)) &&
        (st.maxMr === "" || (r.masteryReq || 0) <= Number(st.maxMr)));
      $("#cnt").textContent = `${fmt(rows.length)} items`;
      $("#table-view").hidden = st.layout !== "table";
      $("#card-view").hidden = st.layout !== "cards";
      const visible = rows.slice(0, st.limit);
      $("#tb").innerHTML = st.layout === "table" ? visible.map((r) => `<tr class="click" data-uid="${esc(r.uid)}">
          <td><div class="row">${art(r.image, "sm")}<span class="ellipsis" style="font-weight:600">${esc(r.name)}</span>
            ${r.vaulted ? `<span class="pill vaulted">vaulted</span>` : ""}${r.manual ? `<span class="pill">marked</span>` : ""}</div></td>
          <td class="muted">${esc(r.category)}</td><td class="r num">${r.masteryReq || ""}</td>
          <td><div class="row"><div class="bar thin grow ${r.status === "mastered" ? "" : "lotus"}"><i style="width:${(100 * r.rank / Math.max(1, r.maxRank)).toFixed(0)}%"></i></div>
            <span class="num muted" style="font-size:12.5px;width:44px;text-align:right">${r.rank}/${r.maxRank}</span></div></td>
          <td>${statusPill(r.status)}</td><td>${next(r)}</td></tr>`).join("")
        || `<tr><td colspan="6"><div class="empty"><b>Nothing matches</b>Try another filter.</div></td></tr>` : "";
      $("#card-view").innerHTML = st.layout === "cards" ? visible.map((r) => `<button type="button" class="icard mastery-card ${r.status === "mastered" ? "done" : ""}" data-uid="${esc(r.uid)}">
          ${art(r.image, "lg")}<span class="body">
            <span class="row"><span class="name grow">${esc(r.name)}</span>${r.masteryReq ? `<span class="pill">MR ${r.masteryReq}</span>` : ""}</span>
            <span class="muted">${esc(r.category)}</span>
            ${(r.parts || []).length ? `<span class="pips">${partPips(r.parts)}</span>` : ""}
            <span class="row"><span class="bar thin grow ${r.status === "mastered" ? "" : "lotus"}"><i style="width:${(100 * r.rank / Math.max(1, r.maxRank)).toFixed(0)}%"></i></span>
              <span class="num muted">${r.rank}/${r.maxRank}</span></span>
            <span class="meta">${statusPill(r.status)}${r.owned ? `<span class="pill good">owned</span>` : ""}${r.vaulted ? `<span class="pill vaulted">vaulted</span>` : ""}${r.manual ? `<span class="pill">marked</span>` : ""}</span>
            ${next(r) ? `<span class="muted">${next(r)}</span>` : ""}
          </span></button>`).join("") || `<div class="empty"><b>Nothing matches</b>Try another filter.</div>` : "";
      $("#more").innerHTML = rows.length > st.limit ? `<button id="moreBtn">Show ${fmt(Math.min(300, rows.length - st.limit))} more</button>` : "";
      const mb = $("#moreBtn");
      if (mb) mb.onclick = () => { st.limit += 300; draw(); };
    };
    draw();
    $("#q").oninput = (e) => { st.q = e.target.value; st.limit = 300; draw(); };
    for (const b of view.querySelectorAll("#status button")) {
      b.onclick = () => { st.status = b.dataset.v; for (const x of view.querySelectorAll("#status button")) x.classList.toggle("on", x === b); draw(); };
    }
    $("#cat").onchange = (e) => { st.cat = e.target.value; draw(); };
    $("#mr").onchange = (e) => { st.maxMr = e.target.value; draw(); };
    $("#prime").onchange = (e) => { st.prime = e.target.value; draw(); };
    for (const b of view.querySelectorAll("#layout button")) {
      b.onclick = () => {
        st.layout = b.dataset.v;
        for (const x of view.querySelectorAll("#layout button")) {
          x.classList.toggle("on", x === b);
          x.setAttribute("aria-pressed", String(x === b));
        }
        draw();
      };
    }
    $("#unv").onchange = (e) => { st.unvaulted = e.target.checked; draw(); };
    $("#bld").onchange = (e) => { st.buildable = e.target.checked; draw(); };
    for (const t of view.querySelectorAll("[data-cat]")) {
      t.onclick = () => { st.cat = st.cat === t.dataset.cat ? "" : t.dataset.cat; $("#cat").value = st.cat; draw(); };
    }
    $("#results").onclick = (e) => { const item = e.target.closest("[data-uid]"); if (item) drawer.open(item.dataset.uid); };
  },
};
