// Farming tasks: a board of what the account shown is after - equipment, mods, arcanes and
// resources - each card coloured by how far along it is (src/tasks.rs). A task's details open in
// the drawer.

import { $, act, api, art, artUrl, debounce, esc, TASK_STATE, toast } from "../lib.js";
import * as drawer from "../drawer.js";

let query = "";
/** A task was added or taken off: main.js redraws the page and the sidebar's count. */
const changed = () => window.dispatchEvent(new CustomEvent("wfp:changed"));
/** The board's signature when it was drawn: /api/state carries the current one. */
let shown = "";

function card(t) {
  const pct = t.need ? Math.min(100, Math.round((100 * t.have) / t.need)) : 0;
  return `<button type="button" class="task-card ${esc(t.state)}" data-task="${esc(t.uid)}">
    <span class="task-pic">${t.image ? `<img alt="" loading="lazy" decoding="async" src="${esc(artUrl(t.image, 60))}" onerror="imgFail(this)" onload="imgDone(this)">` : ""}</span>
    <span class="task-body">
      <span class="task-top"><span class="task-kind">${esc(t.sub)}</span><span class="task-state">${esc(TASK_STATE[t.state] || t.state)}</span></span>
      <span class="task-name">${esc(t.name)}</span>
      <span class="task-note">${esc(t.note)}</span>
      <span class="bar thin"><i style="width:${pct}%"></i></span>
    </span></button>`;
}

function board(data) {
  const rows = data.tasks;
  if (!rows.length) {
    return `<div class="empty"><b>No tasks yet</b>Search above for equipment, a mod, an arcane or a resource,
      or choose “Add to farming” in any item’s details.</div>`;
  }
  const n = (state) => rows.filter((t) => t.state === state).length;
  const ranked = rows.some((t) => t.kind === "mod" || t.kind === "arcane");
  return `<div class="task-legend">
      <span class="missing"><i></i>Missing parts or copies <b>${n("missing")}</b></span>
      <span class="resources"><i></i>Parts there, resources short <b>${n("resources")}</b></span>
      <span class="done"><i></i>Done <b>${n("done")}</b></span></div>
    ${!data.inventoryKnown ? `<p class="callout">No inventory loaded: the colours only use counts you set by hand.</p>` : ""}
    ${data.inventoryKnown && !data.ranksKnown && ranked ? `<p class="callout info">Ranked mods and arcanes are counted from the next inventory read; until then only unranked copies count.</p>` : ""}
    <div class="task-board">${rows.map(card).join("")}</div>`;
}

const clearButton = (data) => (data.done ? `<button data-clear title="Take every finished task off the board">Clear done (${data.done})</button>` : "");

/** Draws the board again when a task changed (after a mission, say); the search stays. */
async function redraw() {
  const data = await api("tasks").catch(() => null);
  if (!data || data.loading || !$("#board")) return;
  shown = data.sig;
  $("#board").innerHTML = board(data);
  $("#clearSlot").innerHTML = clearButton(data);
}

export default {
  id: "farming",
  title: "Farming",
  icon: "tasks",
  /** The sidebar's count: the tasks that are done. */
  badge: (S) => S.tasks?.done || 0,
  badgeTitle: (n) => `${n} farming task${n === 1 ? "" : "s"} done`,
  async render(view, params, app) {
    const data = await api("tasks");
    if (data.loading) {
      view.innerHTML = `<div class="page"><div class="empty"><b>Loading game data…</b>${esc(data.error || "")}</div></div>`;
      setTimeout(() => app.refresh(), 2500);
      return;
    }
    shown = data.sig;
    view.innerHTML = `<div class="page"><div class="page-head"><div><h1>Farming</h1>
      <p>What you are farming, on one board and saved per account. Click a task for what it needs and where to get it.</p></div>
      <span class="grow"></span><span id="clearSlot">${clearButton(data)}</span></div>
      <div class="toolbar"><input id="taskSearch" type="search" aria-label="Add a task" placeholder="Add a task: equipment, mod, arcane or resource" value="${esc(query)}"></div>
      <div id="taskResults"></div>
      <div id="board">${board(data)}</div></div>`;
    const search = async () => {
      const current = query;
      if (current.trim().length < 2) { $("#taskResults").innerHTML = ""; return; }
      try {
        const found = await api("find-task?q=" + encodeURIComponent(current));
        if (!view.isConnected || $("#taskSearch")?.value !== current) return;
        if (found.loading) throw new Error(found.error || "Game data is still loading.");
        $("#taskResults").innerHTML = found.length ? `<div class="list task-results">${found.map((r) =>
          `<div class="list-row">${art(r.image, "sm")}<div class="main"><div class="t">${esc(r.name)}</div><div class="s">${esc(r.sub)}</div></div>
          <button data-add="${esc(r.key)}" data-kind="${esc(r.kind)}" ${r.added ? "disabled" : ""}>${r.added ? "On the board" : "Add"}</button></div>`).join("")}</div>`
          : `<div class="muted">Nothing to farm by that name. Parts are farmed for their equipment, and Ctrl+K finds everything else.</div>`;
      } catch (e) {
        if ($("#taskSearch")?.value === current) $("#taskResults").innerHTML = `<div class="callout">${esc(e.message)}</div>`;
      }
    };
    const searchLater = debounce(search, 150);
    $("#taskSearch").oninput = (e) => { query = e.target.value; searchLater(); };
    if (query) search();
    view.querySelector(".page").onclick = async (e) => {
      const task = e.target.closest("[data-task]");
      if (task) { drawer.open("task:" + task.dataset.task); return; }
      const add = e.target.closest("[data-add]");
      if (add) {
        add.disabled = true;
        if (!(await act("manual/task", { uid: add.dataset.add, onBoard: true }, "Added to the farming board"))) {
          add.disabled = false;
          return;
        }
        changed();
        // A resource is farmed to an amount: ask for it right away.
        if (add.dataset.kind === "resource") drawer.open("task:" + add.dataset.add, true, { focusTarget: true });
        return;
      }
      if (e.target.closest("[data-clear]")) {
        const r = await act("tasks/clear", {});
        if (r) {
          toast(`Took ${r.removed} finished task${r.removed === 1 ? "" : "s"} off the board`);
          changed();
        }
      }
    };
  },
  onState(S) {
    if (S.tasks && shown && S.tasks.sig !== shown) redraw();
  },
};
