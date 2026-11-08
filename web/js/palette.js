// The command palette (Ctrl+K or /): pages, items, parts and relics by name.

import { $, api, art, debounce, esc, icon, plat } from "./lib.js";
import * as drawer from "./drawer.js";

let pages = [];
let results = [];
let sel = 0;
let lookupError = "";
let query = "";
let searchVersion = 0;
let searching = false;

/** The pages the palette can jump to; set once by main.js. */
export function setPages(list) {
  pages = list;
}

export const isOpen = () => !$("#palette").hidden;

export function open() {
  $("#palette").hidden = false;
  const input = $("#paletteInput");
  input.value = "";
  input.focus();
  show("");
}

export function close() {
  searchVersion++;
  $("#palette").hidden = true;
}

function choose(r) {
  close();
  if (r.kind === "page") location.hash = "#" + r.key;
  else drawer.open(r.key);
}

function render() {
  $("#paletteResults").innerHTML = results.map((r, i) => `<li class="${i === sel ? "sel" : ""}" data-i="${i}" role="option">
      ${r.kind === "page" ? `<div class="art sm" style="display:grid;place-items:center">${icon(r.icon)}</div>` : art(r.image, "sm")}
      <div class="main"><div class="t">${esc(r.name)}</div>${r.sub ? `<div class="s">${esc(r.sub)}</div>` : ""}</div>
      ${r.price != null ? plat(r.price) : ""}<span class="kind">${esc(r.kind)}</span></li>`).join("")
    || `<li class="muted">${searching ? "Searching…" : "Nothing matches in the loaded data."}</li>`;
  if (lookupError) {
    $("#paletteResults").insertAdjacentHTML("beforeend", `<li class="muted">${esc(lookupError)}</li>`);
  }
  if (query.trim().length >= 2) {
    $("#paletteResults").insertAdjacentHTML("beforeend", `<li><a href="https://wiki.warframe.com/w/Special:Search?search=${encodeURIComponent(query.trim())}" target="_blank" rel="noopener">Search the Warframe Wiki for ${esc(query.trim())}</a></li>`);
  }
  for (const li of document.querySelectorAll("#paletteResults li[data-i]")) {
    li.onclick = () => choose(results[+li.dataset.i]);
  }
  document.querySelector("#paletteResults li.sel")?.scrollIntoView({ block: "nearest" });
}

async function show(q) {
  if (!isOpen() || $("#paletteInput").value !== q) return;
  const version = ++searchVersion;
  query = q;
  searching = q.trim().length >= 2;
  lookupError = "";
  const ql = q.trim().toLowerCase();
  const pageHits = pages.filter((p) => !ql || p.title.toLowerCase().includes(ql))
    .map((p) => ({ kind: "page", key: p.id, name: p.title, icon: p.icon, sub: "Go to page" }));
  results = pageHits;
  sel = 0;
  render();
  if (ql.length < 2) return;
  try {
    const found = await api("find?q=" + encodeURIComponent(q));
    if (version !== searchVersion || $("#paletteInput").value !== q || !isOpen()) return;
    if (found.loading) throw new Error(found.error || "Game data is still loading.");
    results = [...found, ...pageHits];
    searching = false;
    sel = 0;
    render();
  } catch (e) {
    if (version !== searchVersion || $("#paletteInput").value !== q || !isOpen()) return;
    searching = false;
    lookupError = `Search unavailable: ${e.message}`;
    render();
  }
}

const search = debounce(show, 120);
$("#paletteInput").addEventListener("input", (e) => {
  searchVersion++;
  query = e.target.value;
  results = [];
  lookupError = "";
  searching = query.trim().length >= 2;
  sel = 0;
  render();
  search(query);
});
$("#paletteInput").addEventListener("keydown", (e) => {
  if (e.key === "ArrowDown") { e.preventDefault(); sel = Math.min(sel + 1, results.length - 1); render(); }
  else if (e.key === "ArrowUp") { e.preventDefault(); sel = Math.max(sel - 1, 0); render(); }
  else if (e.key === "Enter" && results[sel]) { e.preventDefault(); choose(results[sel]); }
});
$("#palette").addEventListener("click", (e) => { if (e.target.id === "palette") close(); });
