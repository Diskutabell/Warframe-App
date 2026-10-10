// Overview: who you are, what to do next, and what is going on in the world.

import { $, ago, api, art, countdown, duc, era, esc, fmt, icon, plat } from "../lib.js";
import * as drawer from "../drawer.js";
import { ICON, PORTRAIT, cycleTile, gimg, mrBadge } from "../game.js";

const ICONS = ["platinum", "ducats", "endo", "credits"];

function hero(S) {
  const m = S.mastery || {};
  // The rank held: from an imported inventory, else the log's badge (a band such as 16-18 for
  // higher ranks). XP proves nothing about it: every rank needs its test, and bundles hand out
  // mastery-locked gear early.
  const mr = S.masteryRank;
  const top = S.masteryRankMax ?? mr;
  const shown = mr == null ? "?" : top !== mr ? `${mr}-${top}` : `${mr}`;
  const share = m.possible ? Math.min(1, (m.points || 0) / m.possible) : 0;
  const cur = S.currencies || {};
  const purse = [["platinum", "Platinum", cur.platinum], ["ducats", "Ducats", cur.ducats],
    ["endo", "Endo", cur.endo], ["credits", "Credits", cur.credits]]
    .filter(([, , v]) => v != null)
    .map(([k, label, v]) => `<div class="cur"><img class="ci" alt="" src="icons/${ICONS.includes(k) ? k : "platinum"}.png"><div><b>${fmt(v)}</b><span>${label}</span></div></div>`).join("");
  const rankName = (r) => (r > 30 ? `Legendary ${r - 30}` : `MR ${r}`);
  const ringTitle = mr == null ? "Mastery rank: not known yet (it comes from the game log)" : "Mastery rank";
  // m.total is the mastery XP of the game's profile screen: gear, star chart, Steel Path and
  // intrinsics. m.rank is the rank that much XP reaches once the tests are passed.
  const src = m.sources || {};
  const breakdown = [["Gear", src.gear], ["Star chart", src.starChart], ["Steel Path", src.steelPath],
    ["Railjack intrinsics", src.railjack], ["Drifter intrinsics", src.drifter]]
    .filter(([, v]) => v).map(([label, v]) => `${label} ${fmt(v)}`).join(" · ");
  const toNext = `<b>${fmt((m.nextAt || 0) - (m.total || 0))}</b> more for ${rankName((m.rank || 0) + 1)}`;
  let hint = "";
  if (m.lacks === "profile") {
    hint = `This is gear only. The star chart and intrinsics are added once your profile has been read (link the account in Settings).`;
  } else if (m.lacks === "starChart") {
    hint = `The star chart is not counted yet: its data could not be downloaded.`;
  } else if (mr == null || m.rank > top) {
    hint = `Your mastery XP is enough for <b>${rankName(m.rank)}</b>${mr != null ? `; you hold MR ${shown}` : ""}. ${toNext}.`;
  } else if (m.rank === top) {
    hint = `${toNext}.`;
  }
  return `<div class="card hero">
    <div class="mr-badge" title="${ringTitle}">
      ${gimg(mr == null ? ICON.mastery : mrBadge(top), 116)}
      <b>${mr == null ? "?" : `MR ${shown}`}</b><span>Mastery rank</span>
    </div>
    <div class="grow">
      <h1>${esc(S.account.name || S.account.logName || "Tenno")}</h1>
      <div class="facts">
        <span><b>${fmt(S.mastered)}</b>of ${fmt(S.total)} mastered</span>
        <span><b>${fmt(S.total - S.mastered)}</b>to go</span>
        <span title="${esc(breakdown)}"><b>${fmt(m.total)}${m.lacks ? "+" : ""}</b>mastery XP</span>
      </div>
      <div class="row" style="margin-top:10px;max-width:560px">
        <div class="bar grow"><i style="width:${(share * 100).toFixed(1)}%"></i></div>
        <span class="muted num" style="font-size:12.5px">${(share * 100).toFixed(1)}% of all item mastery · ${fmt((m.possible || 0) - (m.points || 0))} points left in gear</span>
      </div>
      ${hint ? `<div class="hint">${icon("mastery")}<span>${hint}</span></div>` : ""}
      ${breakdown && !m.lacks ? `<div class="muted num" style="font-size:12.5px;margin-top:6px">${esc(breakdown)}</div>` : ""}
    </div>
    ${purse ? `<div class="purse">${purse}</div>` : ""}
  </div>`;
}

function crackNow(data) {
  const groups = new Map();
  for (const f of data?.fissures || []) {
    if (f.storm || !f.relics || !f.relics.owned || !f.relics.best) continue;
    const key = `${f.tier}|${f.hard}`;
    if (!groups.has(key)) groups.set(key, { tier: f.tier, hard: f.hard, best: f.relics.best, owned: f.relics.owned, missions: [] });
    groups.get(key).missions.push(f);
  }
  const rows = [...groups.values()]
    .sort((a, b) => (b.best.needs || 0) - (a.best.needs || 0) || (b.best.ev || 0) - (a.best.ev || 0))
    .slice(0, 6);
  if (!rows.length) {
    return `<div class="empty"><b>Nothing to crack right now</b>${data?.error ? esc(data.error) : "No open fissure matches a relic you own."}</div>`;
  }
  return `<div class="list">${rows.map((g) => `<div class="list-row click" data-relic="${esc(g.best.name)}">
      ${art(g.best.image, "sm")}${era(g.tier)}<div class="main"><div class="t">${esc(g.best.name)} Relic${g.hard ? ` <span class="pill warn">Steel Path</span>` : ""}
          <span class="muted" style="font-weight:400;font-size:13px"> · ${g.best.needs ? `${g.best.needs} needed drops · ` : ""}${plat(g.best.ev)} radiant</span></div>
        <div class="s">${g.missions.map((f) => `${esc(f.mission)} <span class="faint">(${esc(f.node.replace(/ \(.*\)$/, ""))}, ${countdown(f.expiry, "ending")})</span>`).join(" · ")}</div></div>
      <span class="faint num" style="font-size:12.5px">${fmt(g.owned)} ${g.tier === "Omnia" ? "relics" : esc(g.tier)}</span>
    </div>`).join("")}</div>`;
}

function rewards(h) {
  const entries = (h?.entries || []).slice(0, 5);
  const s = h?.session || {};
  const head = `<div class="tiles" style="margin-bottom:10px">
      <div class="tile"><b>${fmt(s.screens || 0)}</b><span>reward screens this session</span></div>
      <div class="tile"><b>${plat(s.bestValue || 0)}</b><span>best picks, at market price</span></div></div>`;
  if (!entries.length) return head + `<div class="empty"><b>No reward screens yet</b>Crack a relic; the reward strip and this list fill in by themselves.</div>`;
  return head + `<div class="list">${entries.map((e) => `<div class="list-row">
      <span class="faint num" style="width:72px;flex:none;font-size:12.5px">${ago(e.t)}</span>
      <div class="chips grow" style="margin:0">${e.drops.map((d) => `<span class="chip ${d.name === e.best ? "open" : ""}" data-part="${esc(d.uid)}">
        ${esc(d.name)} ${d.price != null ? plat(d.price) : ""}${d.needed ? ` <span class="lotus">●</span>` : ""}</span>`).join("")}</div></div>`).join("")}</div>`;
}

function worldCard(w) {
  if (!w || w.error && !w.cycles) return `<div class="empty"><b>World state unavailable</b>${esc(w?.error || "")}</div>`;
  const baro = w.baro || {};
  return `<div class="cycles scenic small">${(w.cycles || []).map((c) => cycleTile(c, false)).join("")}</div>
    <div class="list" style="margin-top:8px">
      <div class="list-row">${gimg(PORTRAIT.baro, 30, "portrait mini")}<span class="grow">Baro Ki'Teer${baro.location ? ` <span class="muted">· ${esc(baro.location)}</span>` : ""}</span>
        <span class="${baro.active ? "lotus" : "muted"}">${baro.active ? `here for ${countdown(baro.expiry, "leaving")}` : `arrives in ${countdown(baro.activation, "now")}`}</span></div>
      <div class="list-row"><span class="grow">Daily reset</span><span class="muted">${countdown(w.resets?.daily)}</span></div>
      <div class="list-row"><span class="grow">Weekly reset</span><span class="muted">${countdown(w.resets?.weekly)}</span></div>
    </div>`;
}

function foundry(S) {
  const f = S.foundry || [];
  if (!f.length) return `<div class="empty"><b>Nothing building</b>${S.sources.inventory.source ? "The foundry is empty." : "Needs an inventory (Settings)."}</div>`;
  return `<div class="list">${f.map((x) => `<div class="list-row ${x.known ? "click" : ""}" ${x.known ? `data-part="${esc(x.uid)}"` : ""}>${art(x.image, "sm")}
      <div class="main"><div class="t">${esc(x.name)}</div></div>
      ${x.done ? (x.done <= Date.now() / 1000 ? `<span class="pill good">Ready</span>` : `<span class="muted">${countdown(x.done, "ready")}</span>`) : `<span class="muted">building</span>`}
    </div>`).join("")}</div>`;
}

function collection(S) {
  const c = S.collection || {};
  return `<div class="tiles">
    <div class="tile"><b>${fmt(c.setsComplete)}</b><span>prime sets complete</span></div>
    <div class="tile"><b>${fmt(c.setsStarted)}</b><span>sets in progress</span></div>
    <div class="tile"><b>${fmt(c.primeParts)}</b><span>prime parts held</span></div>
    <div class="tile"><b>${fmt(c.relics)}</b><span>relics (${fmt(c.relicKinds)} kinds)</span></div>
    <div class="tile"><b>${fmt(c.mods)}</b><span>mods</span></div></div>`;
}

function activity(S) {
  const ev = (S.events || []).map((e) => `<li class="${esc(e.kind)}"><time>${new Date(e.t * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>${esc(e.text)}</li>`).join("");
  return `<ul class="events">${ev || `<li>Nothing yet.</li>`}</ul>`;
}

const card = (title, body, link, id) => `<div class="card"><div class="card-head"><h2>${title}</h2><span class="grow"></span>
    ${link ? `<a class="card-link" href="#${link}">Open ${icon("next")}</a>` : ""}</div><div id="${id}">${body}</div></div>`;

let lastScreens = null;

function pinned(S) {
  return `<div class="list">${(S.pinned || []).map((p) => {
    const done = p.need > 0 && p.have >= p.need;
    return `<div class="list-row ${p.open ? "click" : ""}" ${p.open ? `data-part="${esc(p.key)}"` : ""}>${art(p.image, "sm")}
      <div class="main"><div class="t">${esc(p.name)}</div>
        ${p.need > 0 && p.unit !== "held" ? `<div class="bar thin lotus" style="margin-top:5px"><i style="width:${Math.min(100, (100 * p.have) / p.need).toFixed(0)}%"></i></div>` : ""}</div>
      <span class="${done ? "lotus" : "muted"} num" style="font-size:13px">${p.unit === "held" ? `${fmt(p.have)} held${p.need ? ` · need ${p.need}` : ""}` : `${p.have}/${p.need} ${esc(p.unit)}`}</span>
    </div>`;
  }).join("")}</div>`;
}

function wire(view) {
  for (const el of view.querySelectorAll("[data-relic]")) el.onclick = () => el.dataset.relic && drawer.open(el.dataset.relic);
  for (const el of view.querySelectorAll("[data-part]")) el.onclick = () => drawer.open(el.dataset.part);
}

export default {
  id: "home",
  title: "Overview",
  icon: "home",
  async render(view, params, app) {
    const S = app.state;
    const [fis, world, hist] = await Promise.all([
      api("fissures").catch(() => null), api("world").catch(() => null), api("history").catch(() => null)]);
    lastScreens = S.session?.screens;
    view.innerHTML = `<div class="page">
      ${S.dataError ? `<div class="callout" style="margin-bottom:16px">${esc(S.dataError)}</div>` : ""}
      <div id="hero">${hero(S)}</div>
      <div class="grid cols-2" style="margin-top:16px">
        <div class="stack">
          ${card("Crack now", crackNow(fis), "fissures", "crack")}
          ${card("Relic rewards", rewards(hist), null, "rewards")}
          ${card("Collection", collection(S), "primes", "collection")}
        </div>
        <div class="stack">
          <div id="pinnedCard" ${S.pinned?.length ? "" : "hidden"}>${card("Pinned", pinned(S), null, "pinned")}</div>
          ${card("World", worldCard(world), "world", "world")}
          ${card("Foundry", foundry(S), "foundry", "foundry")}
          ${card("Activity", activity(S), null, "activity")}
        </div>
      </div></div>`;
    wire(view);
    app.every(async () => {
      const [f, w] = await Promise.all([api("fissures").catch(() => null), api("world").catch(() => null)]);
      const c = $("#crack"), wo = $("#world");
      if (c && f) { c.innerHTML = crackNow(f); wire(c); }
      if (wo && w) wo.innerHTML = worldCard(w);
    }, 60000);
  },
  async onState(S) {
    const h = $("#hero");
    if (!h) return;
    h.innerHTML = hero(S);
    $("#foundry").innerHTML = foundry(S);
    $("#activity").innerHTML = activity(S);
    $("#collection").innerHTML = collection(S);
    $("#pinnedCard").hidden = !S.pinned?.length;
    $("#pinned").innerHTML = pinned(S);
    wire($("#foundry"));
    wire($("#pinned"));
    if (S.session?.screens !== lastScreens) {
      lastScreens = S.session?.screens;
      const hist = await api("history").catch(() => null);
      if (hist && $("#rewards")) { $("#rewards").innerHTML = rewards(hist); wire($("#rewards")); }
    }
  },
};
