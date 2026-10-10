// World: the timers and rotations worth knowing before logging in.

import { $, api, countdown, duc, esc, fmt, plat } from "../lib.js";

function render(w) {
  if (!w || (w.error && !w.cycles)) return `<div class="callout">${esc(w?.error || "World state unavailable.")}</div>`;
  const baro = w.baro || {};
  const nw = w.nightwave || {};
  const challenges = [...(nw.challenges || [])].sort((a, b) => (b.daily - a.daily) || (a.elite - b.elite));
  const cycles = (w.cycles || []).map((c) => `<div class="cycle"><div class="n">${esc(c.name)}</div><div class="s">${esc(c.state)}</div>
      <div class="t">${countdown(c.expiry, "changing")} <span class="faint">· ${esc(c.place)}</span></div></div>`).join("");
  const missions = (list) => list.map((m, i) => `<div class="list-row"><span class="faint num" style="width:18px">${i + 1}</span>
      <div class="main"><div class="t">${esc(m.type)}</div><div class="s">${esc(m.node || "")}${m.modifier ? ` · ${esc(m.modifier)}` : ""}</div></div></div>`).join("");
  return `
    <div class="card" style="margin-bottom:16px"><div class="card-head"><h2>Cycles</h2></div><div class="cycles">${cycles}</div></div>
    <div class="grid cols-3">
      <div class="card"><div class="card-head"><h2>Baro Ki'Teer</h2></div>
        <div style="font:700 22px/1.2 var(--display)">${baro.active ? "Here now" : "Away"}</div>
        <div class="muted">${esc(baro.location || "")}</div>
        <div style="margin-top:8px">${baro.active ? `Leaves in ${countdown(baro.expiry, "now")}` : `Arrives in ${countdown(baro.activation, "now")}`}</div>
        ${baro.active && baro.inventory?.length ? `<div class="list" style="margin-top:10px">${baro.inventory.map((i) => `<div class="list-row">
          <span class="grow ellipsis">${esc(i.item)}</span>${duc(i.ducats)}<span class="muted num">${fmt(i.credits)} cr</span></div>`).join("")}</div>` : ""}</div>
      <div class="card"><div class="card-head"><h2>Sortie</h2><span class="grow"></span><span class="muted">${countdown(w.sortie?.expiry)}</span></div>
        <div style="font:700 20px/1.2 var(--display)">${esc(w.sortie?.boss || "")}</div><div class="muted">${esc(w.sortie?.faction || "")}</div>
        <div class="list" style="margin-top:6px">${missions(w.sortie?.missions || [])}</div></div>
      <div class="card"><div class="card-head"><h2>Archon hunt</h2><span class="grow"></span><span class="muted">${countdown(w.archon?.expiry)}</span></div>
        <div style="font:700 20px/1.2 var(--display)">${esc(w.archon?.boss || "")}</div>
        <div class="list" style="margin-top:6px">${missions(w.archon?.missions || [])}</div></div>
    </div>
    <div class="grid cols-3" style="margin-top:16px">
      <div class="card"><div class="card-head"><h2>Bounties</h2></div><div class="list">
        ${(w.bounties || []).map((b) => `<div class="list-row"><span class="grow">${esc(b.syndicate)} <span class="faint">· ${b.jobs} bounties</span></span><span class="muted">new in ${countdown(b.expiry)}</span></div>`).join("") || `<div class="muted">None listed.</div>`}
        ${w.varzia?.expiry ? `<div class="list-row"><span class="grow">Prime Resurgence <span class="faint">· ${esc(w.varzia.location || "")}</span></span><span class="muted">rotates in ${countdown(w.varzia.expiry)}</span></div>` : ""}
        ${(w.events || []).map((e) => `<div class="list-row"><div class="main"><div class="t">${esc(e.name)}</div><div class="s">${esc(e.node || "")}${e.goal ? ` · ${fmt(e.score)} / ${fmt(e.goal)}` : ""}</div></div><span class="muted">${countdown(e.expiry, "ending")}</span></div>`).join("")}
      </div></div>
      <div class="card"><div class="card-head"><h2>The Circuit</h2><span class="grow"></span><span class="muted">${countdown(w.resets?.weekly)}</span></div>
        ${(w.circuit || []).map((c) => `<div style="margin-bottom:10px"><div class="muted" style="font-size:12.5px;margin-bottom:5px">${c.steel ? "Steel Path: Incarnon adapters" : "Normal: Warframes"}</div>
          <div class="chips" style="margin:0">${c.choices.map((x) => `<span class="chip" style="cursor:default">${esc(x)}</span>`).join("")}</div></div>`).join("") || `<div class="muted">Not listed.</div>`}
      </div>
      <div class="card"><div class="card-head"><h2>Archimedea</h2></div>
        ${(w.archimedea || []).map((a) => `<div style="margin-bottom:10px"><div class="row"><span class="grow" style="font:700 17px/1.2 var(--display)">${esc(a.name)}</span><span class="muted">${countdown(a.expiry)}</span></div>
          <div class="muted" style="font-size:13px">${a.missions.map((m) => `${esc(m.type)}${m.deviation ? ` (${esc(m.deviation)})` : ""}`).join(" · ")}</div></div>`).join("") || `<div class="muted">Not listed.</div>`}
      </div>
    </div>
    <div class="grid cols-2" style="margin-top:16px">
      <div class="card"><div class="card-head"><h2>Alerts <span class="count">${(w.alerts || []).length}</span></h2></div><div class="list">
        ${(w.alerts || []).map((a) => `<div class="list-row"><div class="main"><div class="t">${esc(a.reward || a.what || "")}</div>
          <div class="s">${esc(a.type)} · ${esc(a.node)} · ${esc(a.faction)}</div></div><span class="muted">${countdown(a.expiry, "ending")}</span></div>`).join("") || `<div class="muted">No alerts.</div>`}
      </div></div>
      <div class="card"><div class="card-head"><h2>Invasions <span class="count">${(w.invasions || []).length}</span></h2></div><div class="list">
        ${(w.invasions || []).map((i) => `<div class="list-row"><div class="main"><div class="t">${esc(i.node)}</div>
          <div class="s" style="white-space:normal">${esc(i.attacker)}${i.attackerReward ? `: ${esc(i.attackerReward)}` : ""} <span class="faint">vs</span> ${esc(i.defender)}${i.defenderReward ? `: ${esc(i.defenderReward)}` : ""}</div></div>
          <span class="muted num">${Math.round(i.completion)}%</span></div>`).join("") || `<div class="muted">No invasions.</div>`}
      </div></div>
    </div>
    <div class="grid cols-2" style="margin-top:16px">
      <div class="card"><div class="card-head"><h2>Nightwave${nw.season != null ? ` <span class="count">season ${esc(nw.season)}</span>` : ""}</h2></div>
        <div class="list">${challenges.map((c) => `<div class="list-row">
          <span class="pill ${c.daily ? "lotus" : c.elite ? "gold" : ""}" style="width:64px;justify-content:center">${c.daily ? "Daily" : c.elite ? "Elite" : "Weekly"}</span>
          <div class="main"><div class="t">${esc(c.title)}</div><div class="s">${esc(c.desc)}</div></div>
          <div style="text-align:right"><div class="num">${fmt(c.reputation)}</div><div class="faint" style="font-size:12px">${countdown(c.expiry, "ending")}</div></div>
        </div>`).join("") || `<div class="muted">No challenges listed.</div>`}</div></div>
      <div class="stack">
        <div class="card"><div class="card-head"><h2>Steel Path</h2><span class="grow"></span><span class="muted">${countdown(w.steelPath?.expiry)}</span></div>
          <div class="row"><span class="grow" style="font:700 20px/1.2 var(--display)">${esc(w.steelPath?.reward || "")}</span>
          <span class="muted">${fmt(w.steelPath?.cost)} Steel Essence</span></div></div>
        ${w.darvo ? `<div class="card"><div class="card-head"><h2>Darvo's deal</h2><span class="grow"></span><span class="muted">${countdown(w.darvo.expiry)}</span></div>
          <div class="row"><span class="grow" style="font:700 20px/1.2 var(--display)">${esc(w.darvo.item)}</span>
            ${plat(w.darvo.price)}<span class="faint" style="text-decoration:line-through">${fmt(w.darvo.originalPrice)}</span>
            <span class="pill gold">−${esc(w.darvo.discount)}%</span></div>
          <div class="muted" style="margin-top:6px">${w.darvo.sold >= w.darvo.stock ? `<span class="pill warn">Sold out</span>` : `${fmt(w.darvo.stock - w.darvo.sold)} of ${fmt(w.darvo.stock)} left`}</div></div>` : ""}
        <div class="card"><div class="card-head"><h2>Resets</h2></div><div class="list">
          <div class="list-row"><span class="grow">Daily (standing caps, login reward)</span>${countdown(w.resets?.daily)}</div>
          <div class="list-row"><span class="grow">Weekly (archon hunt, Steel Path, Nightwave)</span>${countdown(w.resets?.weekly)}</div></div></div>
      </div>
    </div>`;
}

export default {
  id: "world",
  title: "World",
  icon: "world",
  async render(view, params, app) {
    const w = await api("world");
    view.innerHTML = `<div class="page"><div class="page-head"><div><h1>World</h1><p>Cycles, vendors and rotations. Times update live.</p></div></div>
      <div id="world">${render(w)}</div></div>`;
    app.every(async () => {
      const n = await api("world").catch(() => null);
      if (n && $("#world")) $("#world").innerHTML = render(n);
    }, 60000);
  },
};
