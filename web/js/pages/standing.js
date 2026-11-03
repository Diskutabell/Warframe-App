// Standing: every syndicate's rank and what the next one costs, how much standing is left today,
// and the medallions waiting to be handed in. A syndicate opens in the side panel with what its
// standing buys.

import { $, ago, api, artUrl, countdown, esc, fmt } from "../lib.js";
import * as drawer from "../drawer.js";

/** A picture from the game's files, already a full URL from the engine. */
export const sigil = (url, px, cls = "") => (url
  ? `<img class="gi ${cls}" alt="" width="${px}" height="${px}" loading="lazy" src="${esc(artUrl(url, px))}" onerror="this.remove()">`
  : "");

const SOURCE = { profile: "your profile", inventory: "the inventory read from the game" };

/** A rank's range is drawn as a bar; the event syndicate's last rank spans 494,000 and is no cap. */
const hasBar = (r) => !r.steps && r.size > 0 && r.size <= 200000;

/** "18,143 / 22,000", or for a negative rank how deep into it the account is. */
function amount(r) {
  if (r.steps) return `rank ${fmt(r.progress)} of ${fmt(r.size)}`;
  if (!r.size) return `${fmt(r.standing)} standing`;
  if (r.title < 0) return `${fmt(r.progress)} of −${fmt(r.size)}`;
  return `${fmt(r.progress)} / ${fmt(r.size)}`;
}

/** What to do next with this syndicate, as a pill. */
function status(r) {
  if (r.atCap) return `<span class="pill gold" title="This rank's standing is full: rank up to earn more">Rank up ready</span>`;
  if (r.full) return `<span class="pill warn" title="Top rank and full standing: anything earned now is lost unless you spend some">Full - spend it</span>`;
  if (r.title < 0) return `<span class="pill warn">Hostile</span>`;
  if (!r.known) return `<span class="pill">Not met yet</span>`;
  return "";
}

function bar(r) {
  if (!hasBar(r)) return "";
  const share = r.title < 0 ? Math.min(1, -r.progress / r.size) : Math.max(0, Math.min(1, r.progress / r.size));
  return `<div class="bar syn ${r.title < 0 ? "neg" : ""}" style="${r.colour ? `--syn:${esc(r.colour)}` : ""}"><i style="width:${(share * 100).toFixed(1)}%"></i></div>`;
}

/** The next rank's price, with what is held of each item. */
function nextRank(r) {
  if (!r.next || r.steps) return "";
  const items = (r.next.items || []).map((i) => `<span class="chip res ${i.have >= i.count ? "owned" : ""}" title="You hold ${fmt(i.have)}">
      ${i.image ? `<img alt="" width="20" height="20" src="${esc(artUrl(i.image, 20))}" onerror="this.remove()">` : ""}${fmt(i.count)} × ${esc(i.name)}</span>`).join("");
  const credits = r.next.credits ? `<span class="chip res"><img alt="" width="20" height="20" src="icons/credits.png">${fmt(r.next.credits)}</span>` : "";
  return `<div class="next"><span class="sublabel">${r.atCap ? "Rank up to" : "Next"}: ${esc(r.next.name)}</span>
    <div class="chips">${credits}${items || (credits ? "" : `<span class="faint">no cost listed</span>`)}</div></div>`;
}

function medallions(r) {
  const m = r.medallions || {};
  if (!m.standing) return "";
  const what = m.items.map((i) => `${fmt(i.count)} ${i.name}`).join(", ");
  return `<div class="medals" title="${esc(what)}">${fmt(m.standing)} standing in hand-ins you hold${m.capped ? " · counts toward the daily limit" : " · not limited daily"}</div>`;
}

function poolLeft(r, pools) {
  const p = pools.find((x) => x.key === r.pool);
  if (!p || p.left == null) return "";
  return `<span class="${p.left === 0 ? "warn-text" : "muted"}" title="${esc(p.name)}: ${fmt(p.left)} of ${fmt(p.cap)} left today">${p.left === 0 ? "capped today" : `${fmt(p.left)} left today`}</span>`;
}

function factionRow(r, pools) {
  const allies = (r.allies || []).map((a) => `<span class="ally ${a.share > 0 ? "up" : "down"}" title="Standing gained here gives ${a.name} ${a.share > 0 ? "+" : ""}${Math.round(a.share * 100)}%">${a.share > 0 ? "+" : "−"}${esc(a.name)}</span>`).join("");
  return `<div class="list-row click syn-row" data-syn="${esc(r.tag)}">
      <span class="sigil">${sigil(r.rankIcon || r.icon, 40)}</span>
      <div class="main"><div class="t">${esc(r.name)} <span class="muted" style="font-weight:400">· ${esc(r.rankName)}</span></div>
        ${bar(r)}<div class="s">${amount(r)}${allies ? ` · <span class="allies">${allies}</span>` : ""}</div></div>
      <div class="side">${status(r)}</div></div>`;
}

function card(r, pools) {
  return `<div class="syn-card ${r.known ? "" : "unmet"}" data-syn="${esc(r.tag)}" style="${r.colour ? `--syn:${esc(r.colour)}` : ""}">
      <div class="syn-head"><span class="sigil">${sigil(r.rankIcon || r.icon, 44)}</span>
        <div class="grow"><div class="name">${esc(r.name)}</div><div class="rank">${esc(r.rankName)}${r.topRank && !r.steps ? ` <span class="faint">· rank ${r.title} of ${r.topRank}</span>` : ""}</div></div>
        ${status(r)}</div>
      ${bar(r)}
      <div class="row spread syn-nums"><span class="num">${amount(r)}</span>${poolLeft(r, pools)}</div>
      ${nextRank(r)}${medallions(r)}
    </div>`;
}

function pools(s) {
  const tiles = s.pools.map((p) => {
    const share = p.cap ? Math.max(0, Math.min(1, (p.left ?? p.cap) / p.cap)) : 0;
    return `<div class="pool ${p.left === 0 ? "empty" : ""}" title="${esc(p.name)}${p.read ? "" : " - not read since the daily reset"}">
        <div class="n ellipsis">${esc(p.name)}</div>
        <div class="v num">${p.left == null ? "?" : fmt(p.left)}<span class="faint"> / ${fmt(p.cap)}</span></div>
        <div class="bar thin lotus"><i style="width:${(share * 100).toFixed(1)}%"></i></div></div>`;
  }).join("");
  const f = s.focus || {};
  const focus = f.cap ? `<div class="pool"><div class="n">Focus</div><div class="v num">${fmt(f.left)}<span class="faint"> / ${fmt(f.cap)}</span></div>
      <div class="bar thin"><i style="width:${(Math.max(0, Math.min(1, (f.left ?? f.cap) / f.cap)) * 100).toFixed(1)}%"></i></div></div>` : "";
  return `<div class="pools">${tiles}${focus}</div>`;
}

function render(s) {
  if (!s.loaded) return `<div class="callout">The syndicate data has not been downloaded yet. It comes with the game data (Settings → Re-download game data).</div>`;
  if (!s.account) return `<div class="empty"><b>No account yet</b>Start the game once, or link your account in Settings.</div>`;
  const factions = s.syndicates.filter((r) => r.kind === "faction");
  const shared = s.pools.find((p) => p.key === "DailyAffiliation");
  const nightwave = s.syndicates.filter((r) => r.kind === "nightwave");
  const others = s.syndicates.filter((r) => r.kind === "other");
  const ready = s.syndicates.filter((r) => r.atCap);
  const full = s.syndicates.filter((r) => r.full);
  const tips = [
    ready.length ? `<b>Rank up ready:</b> ${ready.map((r) => esc(r.name)).join(", ")}.` : "",
    full.length ? `<b>Full at the top rank:</b> ${full.map((r) => esc(r.name)).join(", ")} - spend standing there before earning more.` : "",
    s.universal ? `You hold <b>${fmt(s.universal)} Universal Medallion${s.universal > 1 ? "s" : ""}</b> (1,000 standing each, for any of several syndicates).` : "",
  ].filter(Boolean);
  return `
    <div class="card"><div class="card-head"><h2>Left today</h2><span class="grow"></span>
        <span class="muted">resets in ${countdown(s.reset, "a moment")}</span></div>
      ${s.cap ? `<p class="muted" style="margin:-4px 0 12px;font-size:13.5px">${fmt(s.cap)} standing a day in each pool at MR ${s.masteryRank} (16,000 + 500 per rank).${s.asOf ? ` As of ${SOURCE[s.source] || esc(s.source)}, ${ago(s.asOf)}.` : ""}</p>` : `<p class="muted" style="margin:-4px 0 12px">Your mastery rank is not known yet, so the daily limits are not either.</p>`}
      ${pools(s)}
      ${tips.length ? `<div class="hint" style="margin-top:14px;display:block">${tips.join("<br>")}</div>` : ""}</div>
    <div class="grid cols-2" style="margin-top:16px">
      <div class="card"><div class="card-head"><h2>Faction syndicates</h2><span class="grow"></span>
          <span class="${shared?.left === 0 ? "warn-text" : "muted"}" style="font-size:13px">${shared?.left == null ? "one shared daily pool" : shared.left === 0 ? "shared pool capped today" : `${fmt(shared.left)} left today, shared`}</span></div>
        <div class="list">${factions.map((r) => factionRow(r, s.pools)).join("")}</div></div>
      <div class="stack">${nightwave.map((r) => `<div class="card"><div class="card-head"><h2>Nightwave</h2></div>${card(r, s.pools)}</div>`).join("")}
        <div class="card"><div class="card-head"><h2>How standing works here</h2></div>
          <p class="muted" style="margin:0;font-size:13.5px">Standing counts toward a rank until its bar is full; then the rank-up costs what is listed, and the bar starts on the next rank. At the top rank a full bar means standing is lost until you spend some. Medallions and other hand-ins are listed under each syndicate. Pick a syndicate to see what its standing buys and what that is worth on warframe.market.</p></div></div>
    </div>
    <h2 style="margin:22px 0 12px">Syndicates</h2>
    <div class="syn-grid">${others.map((r) => card(r, s.pools)).join("")}</div>`;
}

export default {
  id: "standing",
  title: "Standing",
  icon: "standing",
  async render(view, params, app) {
    const s = await api("standing");
    if (s.loading) { setTimeout(() => app.refresh(), 2500); throw new Error("Downloading game data…"); }
    view.innerHTML = `<div class="page"><div class="page-head"><div><h1>Standing</h1>
        <p>${s.account ? `${esc(s.account)} · ` : ""}ranks, rank-up costs, the daily limits and what standing buys</p></div></div>
      <div id="standing">${render(s)}</div></div>`;
    const wire = () => {
      for (const el of view.querySelectorAll("[data-syn]")) el.onclick = () => drawer.open("syndicate:" + el.dataset.syn);
    };
    wire();
    // The pools and ranks change with the next profile or memory read; check once a minute.
    let shown = JSON.stringify(s);
    app.every(async () => {
      const n = await api("standing").catch(() => null);
      const box = $("#standing");
      if (!n || !box || JSON.stringify(n) === shown) return;
      shown = JSON.stringify(n);
      box.innerHTML = render(n);
      wire();
    }, 60000);
  },
};
