// The app shell: navigation, routing, state polling and keyboard shortcuts.

import { $, api, esc, hydrateIcons, icon, ago } from "./lib.js";
import * as drawer from "./drawer.js";
import * as palette from "./palette.js";
import home from "./pages/home.js";
import mastery from "./pages/mastery.js";
import primes from "./pages/primes.js";
import relics from "./pages/relics.js";
import fissures from "./pages/fissures.js";
import world from "./pages/world.js";
import foundry from "./pages/foundry.js";
import market from "./pages/market.js";
import settings from "./pages/settings.js";

export const PAGES = [home, mastery, primes, relics, fissures, world, foundry, market, settings];

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
    `<a href="#${p.id}" class="${p === page ? "on" : ""}" title="${esc(p.title)} (${i + 1})">${icon(p.icon)}<span>${esc(p.title)}</span></a>`).join("");
}

function renderRail() {
  const S = app.state;
  if (!S) {
    $("#railStatus").innerHTML = `<div><span class="dot warn"></span>Engine not responding</div>`;
    return;
  }
  const src = S.sources, ms = src.memscan || {};
  const inv = src.inventory.source ? `Inventory ${ago(src.inventory.imported)}` : "No inventory";
  $("#railStatus").innerHTML = `
    <div><span class="dot ${src.eeLog.active ? "on" : ""}"></span>${src.eeLog.active ? "Game running" : "Game not running"}</div>
    <div><span class="dot ${S.account.id ? "on" : "warn"}"></span>${S.account.id ? esc(S.account.name || "Account linked") : "Account not linked"}</div>
    <div><span class="dot ${src.inventory.source ? "on" : ""}"></span>${inv}</div>
    ${ms.state === "scanning" ? `<div><span class="dot gold"></span>Reading inventory ${Math.round((ms.progress || 0) * 100)}%</div>` : ""}`;
}

async function poll() {
  try {
    app.state = await api("state");
  } catch {
    app.state = null;
  }
  renderRail();
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
    if (palette.isOpen()) palette.close();
    else if (drawer.isOpen()) drawer.close();
  } else if (!typing && !e.ctrlKey && !e.altKey && /^[1-9]$/.test(e.key) && !palette.isOpen()) {
    const p = PAGES[+e.key - 1];
    if (p) location.hash = "#" + p.id;
  }
});

$("#openPalette").onclick = () => palette.open();
palette.setPages(PAGES);
// A change made in the drawer (a count, "mark as mastered") shows on the page behind it.
window.addEventListener("wfp:changed", () => app.refresh());
hydrateIcons(document.body);
poll().then(() => go());
setInterval(poll, 5000);
