// Shared helpers: escaping, formatting, the engine API, toasts, icons, item art, countdowns.

export const $ = (s, el = document) => el.querySelector(s);
export const $$ = (s, el = document) => [...el.querySelectorAll(s)];

export const esc = (v) => String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
export const fmt = (n) => (n == null ? "–" : Number(n).toLocaleString());
export const plat = (n) => (n == null ? `<span class="faint">–</span>` : `<span class="plat">${fmt(n)}</span>`);
export const duc = (n) => (n ? `<span class="duc">${fmt(n)}</span>` : "");

/** "just now", "5 min ago", "3 h ago" for an epoch-seconds time in the past. */
export function ago(t) {
  if (!t) return "never";
  const s = Math.round(Date.now() / 1000 - t);
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
}

/** Seconds since the epoch from an ISO time or an epoch number - also one read back from a
 *  data- attribute as text, which Date.parse would not understand. */
export function epoch(t) {
  if (t == null || t === "") return NaN;
  if (typeof t === "number" || /^\d+(\.\d+)?$/.test(t)) return Number(t);
  return Date.parse(t) / 1000;
}

/** "2h 14m", "14m 05s", "now" until an epoch-seconds time. */
export function left(t) {
  const s = Math.round(epoch(t) - Date.now() / 1000);
  if (!Number.isFinite(s)) return "";
  if (s <= 0) return "now";
  const d = Math.floor(s / 86400), h = Math.floor((s % 86400) / 3600), m = Math.floor((s % 3600) / 60);
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${String(m).padStart(2, "0")}m`;
  return `${m}m ${String(s % 60).padStart(2, "0")}s`;
}

/** A countdown that the page ticker keeps current. */
export const countdown = (t, done = "now") => `<span data-until="${esc(t)}" data-done="${esc(done)}">${esc(left(t) === "now" ? done : left(t))}</span>`;

setInterval(() => {
  for (const el of $$("[data-until]")) {
    const l = left(el.dataset.until);
    el.textContent = l === "now" ? el.dataset.done : l;
  }
}, 1000);

// ------------------------------------------------------------------ engine API

export async function api(path, body) {
  const opt = body === undefined ? {} : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) };
  const ctl = new AbortController();
  const slow = /scan$/.test(path);
  const timer = setTimeout(() => ctl.abort(), slow ? 60000 : 20000);
  let r;
  try {
    r = await fetch("/api/" + path, { ...opt, signal: ctl.signal });
  } catch (e) {
    throw new Error(e.name === "AbortError" ? `No answer from /api/${path}.` : `Can't reach the engine: ${e.message}`);
  } finally {
    clearTimeout(timer);
  }
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || `HTTP ${r.status}`);
  return j;
}

export function toast(msg, err = false) {
  const t = document.createElement("div");
  t.className = "toast" + (err ? " err" : "");
  t.textContent = msg;
  $("#toasts").append(t);
  setTimeout(() => t.remove(), err ? 7000 : 3200);
}

/** Calls the API and reports failures as a toast; resolves to the result or null. */
export async function act(path, body, okMsg) {
  try {
    const r = await api(path, body ?? {});
    if (okMsg) toast(okMsg);
    return r;
  } catch (e) {
    toast(e.message, true);
    return null;
  }
}

export function debounce(fn, ms) {
  let t;
  return (...a) => { clearTimeout(t); t = setTimeout(() => fn(...a), ms); };
}

// ------------------------------------------------------------------ icons
// Simple line icons drawn for this app (24x24, stroked).
const PATHS = {
  home: "M3 11 12 4l9 7v9a1 1 0 0 1-1 1h-5v-6H9v6H4a1 1 0 0 1-1-1z",
  mastery: "M12 3 4.5 7v10L12 21l7.5-4V7z M12 8l3 2v4l-3 2-3-2v-4z",
  primes: "M4 7h16M4 12h16M4 17h10 M18 15l2 2-2 2",
  relics: "M12 2 20 7v10l-8 5-8-5V7z M12 7l4 2.5v5L12 17l-4-2.5v-5z",
  fissures: "M13 2 4 14h7l-1 8 9-12h-7z",
  world: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M3 12h18 M12 3c2.5 2.6 3.8 5.6 3.8 9s-1.3 6.4-3.8 9c-2.5-2.6-3.8-5.6-3.8-9S9.5 5.6 12 3z",
  foundry: "M4 20h16 M6 20V10l4-3v4l4-3v4l4-3v11 M14 4h4v4",
  market: "M4 8h16l-1.5 11a2 2 0 0 1-2 1.7h-9A2 2 0 0 1 5.5 19z M9 8V6a3 3 0 0 1 6 0v2",
  settings: "M12 8.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z M19.4 13.5l1.6 1.2-2 3.4-1.9-.7a7 7 0 0 1-2 1.1L14.8 21h-4l-.3-2a7 7 0 0 1-2-1.1l-1.9.7-2-3.4 1.6-1.2a7 7 0 0 1 0-2.4L4.6 10.4l2-3.4 1.9.7a7 7 0 0 1 2-1.1L10.8 4h4l.3 2.6a7 7 0 0 1 2 1.1l1.9-.7 2 3.4-1.6 1.2a7 7 0 0 1 0 2.4z",
  search: "M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14z M20 20l-4-4",
  bell: "M6 16V11a6 6 0 0 1 12 0v5l1.5 2h-15z M10 20a2 2 0 0 0 4 0",
  x: "M6 6l12 12M18 6 6 18",
  back: "M15 5l-7 7 7 7",
  next: "M9 5l7 7-7 7",
  star: "M12 3.5l2.6 5.4 5.9.8-4.3 4.1 1 5.8L12 16.9l-5.2 2.7 1-5.8L3.5 9.7l5.9-.8z",
  link: "M14 4h6v6 M20 4l-9 9 M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5",
  clock: "M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18z M12 7v5l3 2",
  check: "M5 12l4 4 10-10",
  plus: "M12 5v14M5 12h14",
  minus: "M5 12h14",
  refresh: "M20 11a8 8 0 1 0-2.3 5.7 M20 4v7h-7",
  standing: "M12 3l7 3v5c0 4.6-3 8.2-7 10-4-1.8-7-5.4-7-10V6z M8.5 12.5l2.4 2.4 4.6-4.8",
  down: "M6 9l6 6 6-6",
};
export const icon = (name, cls = "") => `<svg class="ic ${cls}" viewBox="0 0 24 24" aria-hidden="true"><path d="${PATHS[name] || ""}"/></svg>`;
export function hydrateIcons(root = document) {
  for (const el of $$("[data-icon]", root)) el.outerHTML = icon(el.dataset.icon);
}

// ------------------------------------------------------------------ item art
// Art comes from cdn.warframestat.us and warframe.market through the engine, which scales it to
// the slot (src/art.rs). A dropped request used to leave a permanent hole, so retry twice with a
// short backoff; art the source does not have is a 404, and the slot is hidden.
window.imgFail = (el) => {
  const n = (+el.dataset.try || 0) + 1;
  if (n > 2) { el.style.visibility = "hidden"; return; }
  el.dataset.try = n;
  const base = el.src.replace(/[?&]_r=\d+$/, "");
  setTimeout(() => { el.src = base + (base.includes("?") ? "&" : "?") + "_r=" + n; }, 400 * n);
};
window.imgDone = (el) => { if (!el.naturalWidth) el.style.visibility = "hidden"; };
/** The engine's copy of an image, scaled for a slot `px` CSS pixels wide (see src/art.rs). */
export const artUrl = (src, px) => `api/art?s=${Math.ceil(px * (window.devicePixelRatio || 1))}&u=${encodeURIComponent(src)}`;
// The picture area of each .art size in css/app.css: the box minus its 1 px border.
const ART_PX = { sm: 30, lg: 70, xl: 102 };
export const art = (src, cls = "") => (src
  ? `<img class="art ${cls}" loading="lazy" alt="" src="${esc(artUrl(src, ART_PX[cls] || 42))}" onerror="imgFail(this)" onload="imgDone(this)">`
  : `<div class="art ${cls}"></div>`);

export const STATUS = { mastered: "Mastered", leveling: "Leveling", missing: "Not levelled" };
export const statusPill = (s) => `<span class="pill ${esc(s)}">${STATUS[s] || esc(s)}</span>`;
export const era = (e) => `<span class="era ${esc(e)}">${esc(e)}</span>`;

/** Reads a JSON file the user picks; resolves to the parsed value or null. */
export function pickJson() {
  return new Promise((resolve) => {
    const inp = $("#filePick");
    inp.value = "";
    inp.onchange = async () => {
      const f = inp.files[0];
      if (!f) return resolve(null);
      try { resolve(JSON.parse(await f.text())); } catch { toast("That file is not valid JSON.", true); resolve(null); }
    };
    inp.click();
  });
}
