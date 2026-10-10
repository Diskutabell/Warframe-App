// The command palette (Ctrl+K or /): pages, items, parts and relics by name.

import { $, api, art, debounce, esc, icon, plat } from "./lib.js";
import * as drawer from "./drawer.js";

let pages = [];
let results = [];
let sel = 0;

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
    || `<li class="muted">Nothing matches.</li>`;
  for (const li of document.querySelectorAll("#paletteResults li[data-i]")) {
    li.onclick = () => choose(results[+li.dataset.i]);
  }
  document.querySelector("#paletteResults li.sel")?.scrollIntoView({ block: "nearest" });
}

async function show(q) {
  const ql = q.trim().toLowerCase();
  const pageHits = pages.filter((p) => !ql || p.title.toLowerCase().includes(ql))
    .map((p) => ({ kind: "page", key: p.id, name: p.title, icon: p.icon, sub: "Go to page" }));
  results = pageHits;
  sel = 0;
  render();
  if (ql.length < 2) return;
  try {
    const found = await api("find?q=" + encodeURIComponent(q));
    if ($("#paletteInput").value !== q) return; // a newer query is on its way
    results = [...found, ...pageHits];
    sel = 0;
    render();
  } catch {
    /* keep the page hits */
  }
}

const search = debounce(show, 120);
$("#paletteInput").addEventListener("input", (e) => search(e.target.value));
$("#paletteInput").addEventListener("keydown", (e) => {
  if (e.key === "ArrowDown") { e.preventDefault(); sel = Math.min(sel + 1, results.length - 1); render(); }
  else if (e.key === "ArrowUp") { e.preventDefault(); sel = Math.max(sel - 1, 0); render(); }
  else if (e.key === "Enter" && results[sel]) { e.preventDefault(); choose(results[sel]); }
});
$("#palette").addEventListener("click", (e) => { if (e.target.id === "palette") close(); });
