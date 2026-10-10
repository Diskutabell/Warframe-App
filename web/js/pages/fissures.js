// Fissures: what is open, which of your relics fit, and the ones you watch.

import { $, act, api, countdown, era, esc, fmt, icon, plat } from "../lib.js";
import * as drawer from "../drawer.js";

const st = { kind: "normal", tier: "" };
const KINDS = [["normal", "Normal"], ["steel", "Steel Path"], ["storm", "Void Storms"]];
const TIERS = ["Lith", "Meso", "Neo", "Axi", "Requiem", "Omnia"];

const kindOf = (f) => (f.storm ? "storm" : f.hard ? "steel" : "normal");

function mine(f) {
  const r = f.relics;
  if (f.storm) return `<span class="mine none">Railjack</span>`;
  if (!r || !r.owned) return `<span class="mine none">No ${esc(f.tier)} relics</span>`;
  const b = r.best;
  return `<div class="mine"><span class="click-relic" data-relic="${esc(b?.name || "")}" style="cursor:pointer;font-weight:600">${esc(b?.name || "")}</span>
    <span class="muted"> · ${fmt(r.owned)} owned${b?.needs ? ` · ${b.needs} needed` : ""} · ${plat(b?.ev)}</span></div>`;
}

export default {
  id: "fissures",
  title: "Fissures",
  icon: "fissures",
  async render(view, params, app) {
    const [data, settings] = await Promise.all([api("fissures"), api("settings")]);
    let watch = new Set(data.watch || []);
    view.innerHTML = `<div class="page">
      <div class="page-head"><div class="grow"><h1>Fissures</h1><p>Live void fissures, with the relics you own for each.</p></div>
        <label class="row" style="gap:10px;cursor:pointer"><span class="switch"><input type="checkbox" id="notify" ${settings.notify_fissures ? "checked" : ""}><span></span></span>
          <span>Notify me about watched fissures ${icon("bell")}</span></label></div>
      ${data.error ? `<div class="callout" style="margin-bottom:12px">${esc(data.error)}</div>` : ""}
      <div class="toolbar">
        <div class="seg" id="kind">${KINDS.map(([v, l]) => `<button data-v="${v}" class="${st.kind === v ? "on" : ""}">${l}</button>`).join("")}</div>
        <div class="seg" id="tier"><button data-v="" class="${st.tier === "" ? "on" : ""}">All tiers</button>${TIERS.map((t) => `<button data-v="${t}" class="${st.tier === t ? "on" : ""}">${t}</button>`).join("")}</div>
        <span class="count" id="cnt"></span>
      </div>
      <div class="card flush" id="list"></div>
      <p class="faint" style="font-size:13px;margin-top:10px">${icon("bell")} watches a tier, mission and kind - "Axi Capture, Steel Path" - not one fissure, so you hear about the next one too.</p>
    </div>`;
    const draw = (d) => {
      const rows = (d.fissures || []).filter((f) => kindOf(f) === st.kind && (!st.tier || f.tier === st.tier));
      $("#cnt").textContent = `${rows.length} open`;
      $("#list").innerHTML = rows.map((f) => `<div class="fis">
          <div>${era(f.tier)}</div>
          <div><div class="mission">${esc(f.mission)}</div><div class="node">${esc(f.node)}</div></div>
          <div class="muted">${esc(f.enemy)}</div>
          <div class="num">${countdown(f.expiry, "ending")}</div>
          ${mine(f)}
          <button class="ghost icon watch ${watch.has(f.watch) ? "on" : ""}" data-watch="${esc(f.watch)}" title="${watch.has(f.watch) ? "Stop watching" : "Watch"} ${esc(f.tier)} ${esc(f.mission)}">${icon("bell")}</button>
        </div>`).join("") || `<div class="empty" style="margin:12px"><b>None open</b>Nothing of this kind right now.</div>`;
    };
    draw(data);
    const seg = (id, key) => {
      for (const b of view.querySelectorAll(`#${id} button`)) {
        b.onclick = () => { st[key] = b.dataset.v; for (const x of view.querySelectorAll(`#${id} button`)) x.classList.toggle("on", x === b); draw(data); };
      }
    };
    seg("kind", "kind");
    seg("tier", "tier");
    $("#list").onclick = async (e) => {
      const w = e.target.closest("[data-watch]");
      if (w) {
        const key = w.dataset.watch;
        watch.has(key) ? watch.delete(key) : watch.add(key);
        const ok = await act("settings", { fissure_watch: [...watch] }, watch.has(key) ? "Watching" : "Stopped watching");
        if (!ok) watch = new Set(data.watch || []);
        draw(data);
        return;
      }
      const r = e.target.closest("[data-relic]");
      if (r && r.dataset.relic) drawer.open(r.dataset.relic);
    };
    $("#notify").onchange = (e) => act("settings", { notify_fissures: e.target.checked }, e.target.checked ? "Notifications on" : "Notifications off");
    app.every(async () => {
      const d = await api("fissures").catch(() => null);
      if (d && $("#list")) { Object.assign(data, d); draw(data); }
    }, 60000);
  },
};
