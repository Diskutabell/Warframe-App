// What every part of the UI shares: the engine's summary (pushed over /api/events), the topics
// it announces, the page shown, the details drawer and the command palette.

import { api } from "./api";
import type { Summary } from "../types/Summary";

export const app = $state<{ state: Summary | null; connected: boolean }>({ state: null, connected: false });

// ------------------------------------------------------------------ topics
// The engine names what changed (`bus.rs`): pages subscribe to the topics they show.

export type Topic =
  | "account" | "inventory" | "catalog" | "tasks" | "world" | "prices" | "history" | "rewards" | "riven" | "settings" | "resync";

const listeners = new Map<Topic, Set<(data: unknown) => void>>();

/** Calls `fn` on every `topic` until the returned function is called (or the component that
 *  called it is destroyed, when used in an effect). */
export function on(topics: Topic | Topic[], fn: (data: unknown) => void): () => void {
  const list = Array.isArray(topics) ? topics : [topics];
  for (const t of list) {
    if (!listeners.has(t)) listeners.set(t, new Set());
    listeners.get(t)!.add(fn);
  }
  return () => list.forEach((t) => listeners.get(t)?.delete(fn));
}

function emit(topic: Topic, data: unknown) {
  for (const fn of listeners.get(topic) ?? []) fn(data);
}

/** Something was changed from here (a count, a pin): the summary and the page show it at once. */
export async function changed() {
  await poll();
  emit("resync", {});
}

export async function poll() {
  try {
    app.state = await api<Summary>("state");
    app.connected = true;
  } catch {
    app.state = null;
    app.connected = false;
  }
}

// ------------------------------------------------------------------ the event stream

let downTimer: ReturnType<typeof setTimeout> | null = null;

export function connect() {
  const es = new EventSource("/api/events");
  es.addEventListener("state", (e) => {
    if (downTimer) clearTimeout(downTimer);
    downTimer = null;
    const wasDown = !app.connected;
    app.state = JSON.parse((e as MessageEvent).data);
    app.connected = true;
    // Back after the engine was gone: what the page shows may be old.
    if (wasDown) emit("resync", {});
  });
  const topics: Topic[] = ["account", "inventory", "catalog", "tasks", "world", "prices", "history", "rewards", "riven", "settings", "resync"];
  for (const topic of topics) {
    es.addEventListener(topic, async (e) => {
      const data = JSON.parse((e as MessageEvent).data || "{}");
      // Another account is shown, or events were missed: the summary first, then the page.
      if (topic === "account" || topic === "resync") await poll();
      emit(topic, data);
    });
  }
  // The browser reconnects by itself; a few seconds without the engine are worth saying.
  es.onerror = () => {
    if (downTimer) return;
    downTimer = setTimeout(() => {
      if (es.readyState !== EventSource.OPEN) {
        app.connected = false;
        app.state = null;
      }
    }, 4000);
  };
}

// ------------------------------------------------------------------ the page shown

export const route = $state<{ page: string; params: URLSearchParams }>(parse());

function parse() {
  const [page, query] = (location.hash.slice(1) || "home").split("?");
  return { page: page || "home", params: new URLSearchParams(query || "") };
}

window.addEventListener("hashchange", () => Object.assign(route, parse()));

// ------------------------------------------------------------------ the details drawer and the palette

export const ui = $state<{ drawer: string[]; drawerFocusTarget: boolean; palette: boolean; modal: boolean }>({
  drawer: [],
  drawerFocusTarget: false,
  palette: false,
  modal: false,
});

/** Opens the details of `key` (an item, part, relic, `task:`, `syndicate:`, `lookup:` or
 *  `drop:` key) on top of what the drawer shows. */
export function openDetails(key: string, opts: { focusTarget?: boolean } = {}) {
  ui.drawer.push(key);
  ui.drawerFocusTarget = !!opts.focusTarget;
}

export function closeDetails() {
  ui.drawer.length = 0;
  if (route.params.has("item")) {
    const hash = location.hash.replace(/[?&]item=[^&]*/, "");
    history.replaceState(null, "", hash);
    route.params.delete("item");
  }
  document.getElementById("view")?.focus();
}

// ------------------------------------------------------------------ the clock
// One tick a second for every countdown.

export const clock = $state({ now: Date.now() / 1000 });
setInterval(() => (clock.now = Date.now() / 1000), 1000);
