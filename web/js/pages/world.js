// World: the timers and rotations worth knowing before logging in, with the game's own art.

import { $, api, art, countdown, duc, esc, fmt, plat } from "../lib.js";
import { ICON, PORTRAIT, archonPortrait, cycleTile, emblem, factionColor, factionIcon, factionTag, gimg, syndicateIcon } from "../game.js";

const portrait = (path) => gimg(path, 64, "portrait");

/** The head of a card: a picture, a name, a line under it, and a countdown on the right. */
const lead = (pic, title, sub, right = "") => `<div class="lead">${pic}
    <div class="grow"><div class="big">${title}</div>${sub ? `<div class="sub">${sub}</div>` : ""}</div>
    ${right ? `<div class="when">${right}</div>` : ""}</div>`;

const missions = (list) => list.map((m, i) => `<div class="list-row"><span class="step">${i + 1}</span>
    <div class="main"><div class="t">${esc(m.type)}</div><div class="s">${esc(m.node || "")}${m.modifier ? ` · ${esc(m.modifier)}` : ""}</div></div></div>`).join("");

function baroCard(baro) {
  const stock = baro.active && baro.inventory?.length
    ? `<div class="list scroll">${baro.inventory.map((i) => `<div class="list-row">${art(i.image, "sm")}
        <span class="grow ellipsis">${esc(i.item)}</span>${duc(i.ducats)}<span class="muted num">${fmt(i.credits)} cr</span></div>`).join("")}</div>`
    : "";
  return `<div class="card"><div class="card-head"><h2>Baro Ki'Teer</h2></div>
    ${lead(portrait(PORTRAIT.baro), baro.active ? "Here now" : "Away", esc(baro.location || ""),
      baro.active ? `leaves in<b>${countdown(baro.expiry, "now")}</b>` : `arrives in<b>${countdown(baro.activation, "now")}</b>`)}
    ${stock || `<p class="muted" style="margin:0">${baro.active ? "No stock listed." : "The Void Trader comes every two weeks and takes ducats and credits."}</p>`}</div>`;
}

function sortieCard(s) {
  if (!s?.boss) return `<div class="card"><div class="card-head"><h2>Sortie</h2></div><div class="muted">Not listed.</div></div>`;
  return `<div class="card"><div class="card-head"><h2>Sortie</h2></div>
    ${lead(emblem(factionIcon(s.faction)), esc(s.boss), esc(s.faction || ""), `ends in<b>${countdown(s.expiry)}</b>`)}
    <div class="list">${missions(s.missions || [])}</div></div>`;
}

function archonCard(a) {
  if (!a?.boss) return `<div class="card"><div class="card-head"><h2>Archon hunt</h2></div><div class="muted">Not listed.</div></div>`;
  const pic = archonPortrait(a.boss);
  return `<div class="card"><div class="card-head"><h2>Archon hunt</h2></div>
    ${lead(pic ? portrait(pic) : emblem(ICON.archon), esc(a.boss), "Narmer", `ends in<b>${countdown(a.expiry)}</b>`)}
    <div class="list">${missions(a.missions || [])}</div></div>`;
}

function bountiesCard(w) {
  const rows = (w.bounties || []).map((b) => `<div class="list-row">${emblem(syndicateIcon(b.syndicate), "sm")}
      <div class="main"><div class="t">${esc(b.syndicate)}</div><div class="s">${b.jobs} bounties</div></div>
      <span class="muted">new in ${countdown(b.expiry)}</span></div>`);
  if (w.varzia?.expiry) {
    rows.push(`<div class="list-row">${emblem(ICON.vault, "sm")}
      <div class="main"><div class="t">Prime Resurgence</div><div class="s">${esc(w.varzia.location || "")}</div></div>
      <span class="muted">rotates in ${countdown(w.varzia.expiry)}</span></div>`);
  }
  for (const e of w.events || []) {
    const share = e.goal ? Math.min(100, (100 * e.score) / e.goal) : null;
    rows.push(`<div class="list-row">${emblem(ICON.alert, "sm")}
      <div class="main"><div class="t">${esc(e.name)}</div><div class="s">${esc(e.node || "")}${e.goal ? ` · ${fmt(e.score)} / ${fmt(e.goal)}` : ""}</div>
        ${share != null ? `<div class="bar thin"><i style="width:${share.toFixed(1)}%"></i></div>` : ""}</div>
      <span class="muted">${countdown(e.expiry, "ending")}</span></div>`);
  }
  return `<div class="card"><div class="card-head"><h2>Bounties and events</h2></div>
    <div class="list">${rows.join("") || `<div class="muted">None listed.</div>`}</div></div>`;
}

function circuitCard(w) {
  const group = (c) => `<div class="sublabel">${c.steel ? "Steel Path · Incarnon adapters" : "Normal · Warframes"}</div>
    <div class="picks">${c.choices.map((x) => `<div class="pick">${art(x.image)}<span>${esc(x.name)}</span></div>`).join("")}</div>`;
  return `<div class="card"><div class="card-head"><h2>The Circuit</h2><span class="grow"></span><span class="muted">${countdown(w.resets?.weekly)}</span></div>
    ${(w.circuit || []).map(group).join("") || `<div class="muted">Not listed.</div>`}</div>`;
}

function archimedeaCard(w) {
  const one = (a) => {
    const foe = a.missions.find((m) => m.faction)?.faction;
    return `${lead(emblem(factionIcon(foe)), esc(a.name), esc(foe || ""), `ends in<b>${countdown(a.expiry)}</b>`)}
      <div class="list" style="margin-bottom:12px">${a.missions.map((m, i) => `<div class="list-row"><span class="step">${i + 1}</span>
        <div class="main"><div class="t">${esc(m.type)}</div>${m.deviation ? `<div class="s">${esc(m.deviation)}</div>` : ""}</div></div>`).join("")}</div>`;
  };
  return `<div class="card"><div class="card-head"><h2>Archimedea</h2></div>
    ${(w.archimedea || []).map(one).join("") || `<div class="muted">Not listed.</div>`}</div>`;
}

function alertsCard(alerts) {
  const row = (a) => {
    const rewards = a.rewards?.length ? a.rewards : [{ name: a.reward || a.what || "", count: 1 }];
    const pic = rewards.find((r) => r.image)?.image;
    return `<div class="list-row">${pic ? art(pic) : emblem(ICON.alert)}
      <div class="main"><div class="t wrap">${rewards.map((r) => `${r.count > 1 ? `${r.count}× ` : ""}${esc(r.name)}`).join(" · ")}</div>
        <div class="s">${esc(a.type)} · ${esc(a.node)}</div></div>
      <div class="side">${factionTag(a.faction)}<span class="muted">${countdown(a.expiry, "ending")}</span></div></div>`;
  };
  return `<div class="card"><div class="card-head"><h2>Alerts <span class="count">${alerts.length}</span></h2></div>
    <div class="list">${alerts.map(row).join("") || `<div class="muted">No alerts.</div>`}</div></div>`;
}

function invasionsCard(invasions) {
  const side = (who, reward, image, right) => `<div class="inv-side ${right ? "right" : ""}">
      ${gimg(factionIcon(who), 22)}<div><b>${esc(who)}</b><span>${reward ? esc(reward) : "no reward"}</span></div>${image ? art(image, "sm") : ""}</div>`;
  const row = (i) => `<div class="inv">
      <div class="row"><span class="t grow">${esc(i.node)}</span><span class="muted">${esc(i.what || "")}</span></div>
      <div class="inv-bar" style="--a:${factionColor(i.attacker)};--d:${factionColor(i.defender)}" title="${Math.round(i.completion)}% to the attackers">
        <i style="width:${Math.max(0, Math.min(100, i.completion)).toFixed(1)}%"></i></div>
      <div class="inv-sides">${side(i.attacker, i.attackerReward, i.attackerImage, false)}${side(i.defender, i.defenderReward, i.defenderImage, true)}</div></div>`;
  return `<div class="card"><div class="card-head"><h2>Invasions <span class="count">${invasions.length}</span></h2></div>
    ${invasions.map(row).join("") || `<div class="muted">No invasions.</div>`}</div>`;
}

function nightwaveCard(nw) {
  const challenges = [...(nw.challenges || [])].sort((a, b) => (b.daily - a.daily) || (a.elite - b.elite));
  const kind = (c) => (c.daily ? ["daily", "Daily"] : c.elite ? ["elite", "Elite weekly"] : ["weekly", "Weekly"]);
  return `<div class="card"><div class="card-head"><h2>Nightwave</h2></div>
    ${lead(portrait(PORTRAIT.nora), nw.season != null ? `Season ${esc(nw.season)}` : "Nora Night", `${challenges.length} acts`, nw.expiry ? `ends in<b>${countdown(nw.expiry)}</b>` : "")}
    <div class="list">${challenges.map((c) => `<div class="list-row">${emblem(ICON[kind(c)[0]], `sm ${kind(c)[0]}`, kind(c)[1])}
      <div class="main"><div class="t">${esc(c.title)}</div><div class="s">${esc(c.desc)}</div></div>
      <div class="side"><span class="num">${fmt(c.reputation)}</span><span class="faint">${countdown(c.expiry, "ending")}</span></div>
    </div>`).join("") || `<div class="muted">No acts listed.</div>`}</div></div>`;
}

function steelPathCard(sp) {
  return `<div class="card"><div class="card-head"><h2>Steel Path</h2></div>
    ${lead(portrait(PORTRAIT.teshin), "Teshin's offer", "Steel Path honors", `changes in<b>${countdown(sp?.expiry)}</b>`)}
    <div class="row">${art(sp?.image)}<span class="grow" style="font:700 19px/1.2 var(--display)">${esc(sp?.reward || "")}</span>
      <span class="ftag">${gimg(ICON.steel, 18)}${fmt(sp?.cost)}</span></div></div>`;
}

function darvoCard(d) {
  const left = d.stock - d.sold;
  return `<div class="card"><div class="card-head"><h2>Darvo's deal</h2></div>
    ${lead(d.image ? art(d.image, "lg") : emblem(ICON.darvo), esc(d.item), left > 0 ? `${fmt(left)} of ${fmt(d.stock)} left` : `<span class="pill warn">Sold out</span>`, `ends in<b>${countdown(d.expiry)}</b>`)}
    <div class="row">${gimg(ICON.darvo, 20)}<span class="muted grow">Darvo, daily</span>${plat(d.price)}
      <span class="faint" style="text-decoration:line-through">${fmt(d.originalPrice)}</span><span class="pill gold">−${esc(d.discount)}%</span></div></div>`;
}

function render(w) {
  if (!w || (w.error && !w.cycles)) return `<div class="callout">${esc(w?.error || "World state unavailable.")}</div>`;
  return `
    <div class="cycles scenic" style="margin-bottom:16px">${(w.cycles || []).map((c) => cycleTile(c)).join("")}</div>
    <div class="grid cols-3">${baroCard(w.baro || {})}${sortieCard(w.sortie)}${archonCard(w.archon)}</div>
    <div class="grid cols-3" style="margin-top:16px">${bountiesCard(w)}${circuitCard(w)}${archimedeaCard(w)}</div>
    <div class="grid cols-even" style="margin-top:16px">${alertsCard(w.alerts || [])}${invasionsCard(w.invasions || [])}</div>
    <div class="grid cols-2" style="margin-top:16px">
      ${nightwaveCard(w.nightwave || {})}
      <div class="stack">
        ${steelPathCard(w.steelPath)}
        ${w.darvo ? darvoCard(w.darvo) : ""}
        <div class="card"><div class="card-head"><h2>Resets</h2></div><div class="list">
          <div class="list-row">${emblem(ICON.daily, "sm")}<div class="main"><div class="t">Daily</div><div class="s">Standing caps, login reward</div></div>${countdown(w.resets?.daily)}</div>
          <div class="list-row">${emblem(ICON.weekly, "sm")}<div class="main"><div class="t">Weekly</div><div class="s">Archon hunt, Steel Path, Nightwave</div></div>${countdown(w.resets?.weekly)}</div></div></div>
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
    // The countdowns tick on their own; redrawing replaces every picture, so only do it for news.
    const news = (x) => JSON.stringify({ ...x, fetched: null });
    let shown = news(w);
    app.every(async () => {
      const n = await api("world").catch(() => null);
      if (n && $("#world") && news(n) !== shown) {
        shown = news(n);
        $("#world").innerHTML = render(n);
      }
    }, 60000);
  },
};
