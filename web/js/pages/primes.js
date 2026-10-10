// Prime sets: each set, its parts, and how close it is.

import { $, api, artUrl, art, esc, fmt, plat, statusPill } from "../lib.js";
import * as drawer from "../drawer.js";

const st = { q: "", show: "started", unvaulted: false, unmastered: false };
const SHOW = [["started", "In progress"], ["complete", "Complete"], ["missing", "Not started"], ["", "All"]];

function pips(parts) {
  return parts.flatMap((p) => Array.from({ length: p.need }, (_, i) =>
    `<span class="pip ${i < p.have ? "ok" : ""}" title="${esc(p.name)} - own ${p.have} of ${p.need}">${p.image ? `<img alt="" src="${esc(artUrl(p.image, 24))}" onerror="this.remove()">` : ""}${i === 0 && p.have > p.need ? `<b>${p.have}</b>` : ""}</span>`)).join("");
}

export default {
  id: "primes",
  title: "Prime sets",
  icon: "primes",
  async render(view, params, app) {
    const data = await api("primes");
    if (data.loading) { setTimeout(() => app.refresh(), 2500); throw new Error("Downloading game data…"); }
    const complete = data.filter((r) => r.complete).length;
    const started = data.filter((r) => !r.complete && r.have > 0).length;
    view.innerHTML = `<div class="page">
      <div class="page-head"><div><h1>Prime sets</h1><p>${complete} complete · ${started} in progress · ${data.length} in total</p></div></div>
      <div class="toolbar">
        <input type="search" id="q" placeholder="Search sets" value="${esc(st.q)}" aria-label="Search sets">
        <div class="seg" id="show">${SHOW.map(([v, l]) => `<button data-v="${v}" class="${st.show === v ? "on" : ""}">${l}</button>`).join("")}</div>
        <label class="check"><input type="checkbox" id="unv" ${st.unvaulted ? "checked" : ""}>Not vaulted</label>
        <label class="check"><input type="checkbox" id="unm" ${st.unmastered ? "checked" : ""}>Not mastered</label>
        <span class="count" id="cnt"></span>
      </div>
      <div class="cards" id="sets"></div></div>`;
    const draw = () => {
      const q = st.q.toLowerCase();
      const rows = data.filter((r) => (!q || r.name.toLowerCase().includes(q)) && (!st.unvaulted || !r.vaulted) &&
        (!st.unmastered || r.status !== "mastered") &&
        (st.show === "" || (st.show === "complete" && r.complete) || (st.show === "started" && !r.complete && r.have > 0) ||
          (st.show === "missing" && r.have === 0)));
      $("#cnt").textContent = `${fmt(rows.length)} sets`;
      $("#sets").innerHTML = rows.map((r) => `<div class="icard ${r.complete ? "done" : ""}" data-uid="${esc(r.uid)}">
          ${art(r.image, "lg")}
          <div class="body">
            <div class="row"><span class="name grow">${esc(r.name)}</span>${r.setPrice != null ? plat(r.setPrice) : ""}</div>
            <div class="pips">${pips(r.parts)}</div>
            <div class="meta"><span>${r.have}/${r.need} parts</span>${statusPill(r.status)}
              ${r.owned ? `<span class="pill good">owned</span>` : ""}${r.vaulted ? `<span class="pill vaulted">vaulted</span>` : ""}</div>
          </div></div>`).join("") || `<div class="empty"><b>No sets match</b>Try another filter.</div>`;
    };
    draw();
    $("#q").oninput = (e) => { st.q = e.target.value; draw(); };
    for (const b of view.querySelectorAll("#show button")) {
      b.onclick = () => { st.show = b.dataset.v; for (const x of view.querySelectorAll("#show button")) x.classList.toggle("on", x === b); draw(); };
    }
    $("#unv").onchange = (e) => { st.unvaulted = e.target.checked; draw(); };
    $("#unm").onchange = (e) => { st.unmastered = e.target.checked; draw(); };
    $("#sets").onclick = (e) => { const c = e.target.closest("[data-uid]"); if (c) drawer.open(c.dataset.uid); };
  },
};
