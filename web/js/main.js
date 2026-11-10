// The app shell: navigation, routing, state polling and keyboard shortcuts.

import { $, act, api, esc, hydrateIcons, icon, ago } from "./lib.js";
import * as drawer from "./drawer.js";
import * as palette from "./palette.js";
import { gimg, ICON, mrBadge } from "./game.js";
import home from "./pages/home.js";
import mastery from "./pages/mastery.js";
import primes from "./pages/primes.js";
import relics from "./pages/relics.js";
import fissures from "./pages/fissures.js";
import world from "./pages/world.js";
import standing from "./pages/standing.js";
import foundry from "./pages/foundry.js";
import market from "./pages/market.js";
import farming from "./pages/farming.js";
import inventory from "./pages/inventory.js";
import settings from "./pages/settings.js";

export const PAGES = [home, mastery, primes, relics, fissures, world, standing, foundry, market, farming, inventory, settings];
/** Preserve 1-9 for the original pages and 0 for Settings. */
const pageKey = (page, i) => page === settings ? "0" : i < 9 ? String(i + 1) : "";

/** What every page can read: the latest /api/state, and a way to re-render. */
export const app = {
  state: null,
  timers: [],
  /** Re-renders the current page (after a change made in the drawer, say). */
  refresh: () => go(true),
  /** Runs `fn` every `ms` while the current page is shown. */
  every(fn, ms) { this.timers.push(setInterval(fn, ms)); },
};

function route() {
  const [id, query] = (location.hash.slice(1) || "home").split("?");
  return { page: PAGES.find((p) => p.id === id) || home, params: new URLSearchParams(query || "") };
}

function renderNav() {
  const { page } = route();
  $("#nav").innerHTML = PAGES.map((p, i) =>
    (p.id === "settings" ? `<div class="sep"></div>` : "") +
    `<a href="#${p.id}" class="${p === page ? "on" : ""}" data-page="${p.id}" title="${esc(p.title)}${pageKey(p, i) ? ` (${pageKey(p, i)})` : ""}">${icon(p.icon)}<span>${esc(p.title)}</span>${p.badge ? `<b class="nav-badge" hidden></b>` : ""}</a>`).join("");
  renderBadges();
}

/** The small counts beside a page's name: a page with a `badge(state)` says how many. */
function renderBadges() {
  for (const p of PAGES.filter((p) => p.badge)) {
    const el = $(`#nav a[data-page="${p.id}"] .nav-badge`);
    if (!el) continue;
    const n = app.state ? p.badge(app.state) : 0;
    el.hidden = !n;
    el.textContent = n > 99 ? "99+" : String(n);
    el.title = p.badgeTitle ? p.badgeTitle(n) : "";
  }
}

// ------------------------------------------------------------------ the account switcher

const rankText = (a) => (a.masteryRank == null ? "MR ?" : a.masteryRankMax != null && a.masteryRankMax !== a.masteryRank
  ? `MR ${a.masteryRank}-${a.masteryRankMax}` : a.masteryRank > 30 ? `Legendary ${a.masteryRank - 30}` : `MR ${a.masteryRank}`);
const badge = (a, px) => gimg(a.masteryRank == null ? ICON.mastery : mrBadge(a.masteryRankMax ?? a.masteryRank), px);
let acctShown = "";

/** The account shown, and the others to switch to. Redrawn only when something changed, so an
 *  open menu stays open between polls. */
function renderAccounts() {
  const list = app.state?.accounts || [];
  const sig = JSON.stringify(list.map((a) => [a.key, a.name, a.active, a.playing, a.masteryRank, a.masteryRankMax]));
  if (sig === acctShown) return;
  acctShown = sig;
  const box = $("#acct");
  const cur = list.find((a) => a.active);
  if (!cur) {
    box.innerHTML = `<a class="acct-btn empty" href="#settings">${icon("mastery")}<div class="acct-txt"><b>No account yet</b><span>Start the game once</span></div></a>`;
    return;
  }
  const sub = (a) => `${rankText(a)}${a.playing ? ` · <span class="lotus">in game</span>` : ""}`;
  box.innerHTML = `<button class="acct-btn" id="acctBtn" type="button" aria-haspopup="true" aria-expanded="false" title="${list.length > 1 ? "Switch account" : "Account"}">
      ${badge(cur, 30)}<div class="acct-txt"><b class="ellipsis">${esc(cur.name)}</b><span>${sub(cur)}</span></div>${list.length > 1 ? icon("down") : ""}</button>
    <div class="acct-menu" id="acctMenu" role="menu" hidden>
      ${list.map((a) => `<button type="button" role="menuitem" class="${a.active ? "on" : ""}" data-acct="${esc(a.key)}">${badge(a, 24)}
        <div class="acct-txt"><b class="ellipsis">${esc(a.name)}</b><span>${sub(a)}</span></div></button>`).join("")}
      <a href="#settings" class="acct-manage">Manage accounts</a></div>`;
  const menu = $("#acctMenu");
  const toggle = (open) => { menu.hidden = !open; $("#acctBtn").setAttribute("aria-expanded", String(open)); };
  $("#acctBtn").onclick = (e) => { e.stopPropagation(); toggle(menu.hidden); };
  menu.onclick = async (e) => {
    const b = e.target.closest("[data-acct]");
    toggle(false);
    if (!b || b.classList.contains("on")) return;
    if (await act("account/switch", { key: b.dataset.acct })) window.dispatchEvent(new CustomEvent("wfp:account"));
  };
}

document.addEventListener("click", (e) => {
  const menu = $("#acctMenu");
  if (menu && !menu.hidden && !e.target.closest("#acct")) menu.hidden = true;
});

function renderRail() {
  const S = app.state;
  if (!S) {
    $("#railStatus").innerHTML = `<div><span class="dot warn"></span>Engine not responding</div>`;
    return;
  }
  const src = S.sources, ms = src.memscan || {};
  const inv = src.inventory.source ? `Inventory ${ago(src.inventory.imported)}` : "No inventory";
  renderAccounts();
  const playing = S.account.logName && !S.account.playing ? ` <span class="faint">(${esc(S.account.logName)})</span>` : "";
  $("#railStatus").innerHTML = `
    <div><span class="dot ${src.eeLog.active ? "on" : ""}"></span>${src.eeLog.active ? "Game running" : "Game not running"}${src.eeLog.active ? playing : ""}</div>
    <div><span class="dot ${S.account.id ? "on" : "warn"}"></span>${S.account.id ? "Profile linked" : "Account not linked"}</div>
    <div><span class="dot ${src.inventory.source ? "on" : ""}"></span>${inv}</div>
    ${ms.state === "scanning" ? `<div><span class="dot gold"></span>Reading inventory ${Math.round((ms.progress || 0) * 100)}%</div>` : ""}`;
}

// ------------------------------------------------------------------ a newer version
// The engine asks GitHub once when it starts (update.rs); the popup shows once per window.

let updateShown = false;

/** A release's notes, which are the changelog's Markdown: headings, bullets (their wrapped lines
 *  joined back), bold and code. Everything else stays plain text. */
function notesHtml(md) {
  const blocks = [];
  for (const line of String(md || "").split(/\r?\n/)) {
    const last = blocks[blocks.length - 1];
    if (!line.trim()) blocks.push(null);
    else if (/^#{1,6}\s/.test(line)) blocks.push({ kind: "h", text: line.replace(/^#+\s*/, "") });
    else if (/^\s*[-*]\s/.test(line)) blocks.push({ kind: "li", text: line.replace(/^\s*[-*]\s+/, "") });
    else if (last) last.text += " " + line.trim();
    else blocks.push({ kind: "p", text: line.trim() });
  }
  const inline = (t) => esc(t).replace(/\*\*(.+?)\*\*/g, "<b>$1</b>").replace(/`([^`]+)`/g, "<code>$1</code>");
  let html = "", list = false;
  for (const b of blocks.filter(Boolean)) {
    if (b.kind !== "li" && list) { html += "</ul>"; list = false; }
    if (b.kind === "li" && !list) { html += "<ul>"; list = true; }
    html += b.kind === "h" ? `<h3>${inline(b.text)}</h3>` : b.kind === "li" ? `<li>${inline(b.text)}</li>` : `<p>${inline(b.text)}</p>`;
  }
  return list ? html + "</ul>" : html;
}

function closeUpdate() {
  $("#update").hidden = true;
}

function showUpdate(u) {
  updateShown = true;
  const box = $("#update .modal-box");
  const date = u.published ? new Date(u.published).toLocaleDateString([], { day: "numeric", month: "long", year: "numeric" }) : "";
  const notes = notesHtml(u.notes);
  box.innerHTML = `<div class="modal-head"><div class="grow"><h2 id="updateTitle">A new version is out</h2>
      <p class="muted">warframeproject ${esc(u.version)}${date ? `, released ${esc(date)}` : ""}. You have ${esc(u.current)}.</p></div>
      <button class="ghost icon" type="button" data-later title="Close" aria-label="Close">${icon("x")}</button></div>
    ${u.name ? `<div class="modal-title">${esc(u.name)}</div>` : ""}
    ${notes ? `<div class="modal-notes">${notes}</div>` : ""}
    <div class="modal-foot"><button type="button" data-later>Later</button>
      ${u.url ? `<a class="btn primary" href="${esc(u.url)}" target="_blank" rel="noopener" data-get>${icon("link")} Open the release page</a>` : ""}</div>`;
  for (const b of box.querySelectorAll("[data-later], [data-get]")) b.addEventListener("click", closeUpdate);
  $("#update").hidden = false;
  box.querySelector("[data-get]")?.focus();
}

// A click beside the box closes it too.
$("#update").addEventListener("click", (e) => { if (e.target.id === "update") closeUpdate(); });

async function poll() {
  try {
    app.state = await api("state");
  } catch {
    app.state = null;
  }
  if (app.state?.update && !updateShown) {
    // The popup is a nicety: whatever goes wrong in it must not stop the page.
    try { showUpdate(app.state.update); } catch (e) { console.error("update popup:", e); }
  }
  renderRail();
  renderBadges();
  const { page } = route();
  if (app.state && page.onState) page.onState(app.state);
}

let current = null;

async function go(keepScroll = false) {
  const { page, params } = route();
  renderNav();
  for (const t of app.timers) clearInterval(t);
  app.timers = [];
  const view = $("#view");
  const scroll = keepScroll && current === page ? view.scrollTop : 0;
  if (!app.state) await poll();
  if (!app.state) {
    view.innerHTML = `<div class="page"><div class="page-head"><div><h1>The engine isn't responding</h1>
      <p>Retrying every few seconds. If it keeps failing, start the app again from a terminal and read its output.</p></div></div></div>`;
    return;
  }
  if (!keepScroll || current !== page) {
    view.innerHTML = `<div class="page"><div class="page-head"><div><h1>${esc(page.title)}</h1></div></div>
      <div class="stack">${`<div class="skeleton" style="width:${40 + Math.random() * 40}%"></div>`.repeat(4)}</div></div>`;
  }
  current = page;
  try {
    await page.render(view, params, app);
  } catch (e) {
    view.innerHTML = `<div class="page"><div class="page-head"><div><h1>${esc(page.title)}</h1></div></div>
      <div class="callout">${esc(e.message)}</div></div>`;
  }
  hydrateIcons(view);
  if (keepScroll) view.scrollTop = scroll;
  else view.focus();
  const item = params.get("item");
  if (item && !drawer.isOpen()) drawer.open(item);
}

window.addEventListener("hashchange", () => go());

document.addEventListener("keydown", (e) => {
  const typing = /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement?.tagName);
  if ((e.ctrlKey && e.key.toLowerCase() === "k") || (e.key === "/" && !typing)) {
    e.preventDefault();
    palette.open();
  } else if (e.key === "Escape") {
    const menu = $("#acctMenu");
    if (!$("#update").hidden) closeUpdate();
    else if (menu && !menu.hidden) menu.hidden = true;
    else if (palette.isOpen()) palette.close();
    else if (drawer.isOpen()) drawer.close();
  } else if (!typing && !e.ctrlKey && !e.altKey && /^[0-9]$/.test(e.key) && !palette.isOpen()) {
    const p = PAGES.find((page, i) => pageKey(page, i) === e.key);
    if (p) location.hash = "#" + p.id;
  }
});

// ------------------------------------------------------------------ wheel scrolling
// WebKitGTK on Wayland moves a whole wheel click (about 50-100 px) in one frame, which reads as
// a stutter. Wheel clicks glide here instead: each adds to where the scroll is heading, and the
// view eases there over a few frames. Touchpads send many small steps and keep the native path.
const glide = new Map();
function step(el) {
  const g = glide.get(el);
  const left = g.target - el.scrollTop;
  if (Math.abs(left) < 0.5) { el.scrollTop = g.target; glide.delete(el); return; }
  const before = el.scrollTop;
  el.scrollTop += left * 0.28 + Math.sign(left) * 0.5;
  // An edge (or another script) stopped it: the target is out of reach.
  if (el.scrollTop === before) { glide.delete(el); return; }
  requestAnimationFrame(() => step(el));
}
document.addEventListener("wheel", (e) => {
  if (e.ctrlKey || e.shiftKey || Math.abs(e.deltaY) < 30 || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
  const el = e.target.closest("#view, .drawer, .palette-results, #nav, .acct-menu");
  if (!el || el.scrollHeight <= el.clientHeight) return;
  e.preventDefault();
  const delta = e.deltaMode === 1 ? e.deltaY * 18 : e.deltaY;
  const g = glide.get(el);
  const from = g ? g.target : el.scrollTop;
  const target = Math.max(0, Math.min(el.scrollHeight - el.clientHeight, from + delta));
  if (g) g.target = target;
  else { glide.set(el, { target }); requestAnimationFrame(() => step(el)); }
}, { passive: false });

$("#openPalette").onclick = () => palette.open();
palette.setPages(PAGES);
// A change made in the drawer (a count, "mark as mastered") shows on the page behind it, and in
// the sidebar's counts at once rather than at the next poll.
window.addEventListener("wfp:changed", () => { app.refresh(); poll(); });
// Another account is shown: everything on the page belongs to it.
window.addEventListener("wfp:account", async () => { await poll(); go(true); });
hydrateIcons(document.body);
poll().then(() => go());
setInterval(poll, 5000);
