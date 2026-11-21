<!-- Farming tasks: a board of what the account shown is after - equipment, mods, arcanes and
     resources - each card coloured by how far along it is (src/tasks.rs). A task's details open
     in the drawer. -->
<script lang="ts" module>
  const st = $state({ query: "" });
</script>

<script lang="ts">
  import Art from "../components/Art.svelte";
  import Page from "../components/Page.svelte";
  import { act, api, isLoading } from "../lib/api";
  import { app, changed, openDetails } from "../lib/app.svelte";
  import { TASK_STATE, artUrl } from "../lib/format";
  import { loader } from "../lib/loader.svelte";
  import { toast } from "../lib/toasts.svelte";
  import type { Board } from "../types/Board";
  import type { Removed } from "../types/Removed";
  import type { TaskHit } from "../types/TaskHit";

  const page = loader<Board>("tasks", ["catalog", "tasks", "inventory"]);
  // The summary carries the board's signature: a change from elsewhere (a mission) redraws it.
  $effect(() => {
    const sig = app.state?.tasks.sig;
    if (sig && page.data && sig !== page.data.sig) page.refresh();
  });

  let found = $state<TaskHit[] | null>(null);
  let searchError = $state<string | null>(null);
  $effect(() => {
    const q = st.query.trim();
    found = null;
    searchError = null;
    if (q.length < 2) return;
    const ctl = new AbortController();
    const timer = setTimeout(async () => {
      try {
        const r = await api<TaskHit[] | { loading: true; error: string | null }>("find-task?q=" + encodeURIComponent(q), undefined, ctl.signal);
        if (isLoading(r)) throw new Error(r.error || "Game data is still loading.");
        found = r;
      } catch (e) {
        if (!ctl.signal.aborted) searchError = (e as Error).message;
      }
    }, 150);
    return () => {
      ctl.abort();
      clearTimeout(timer);
    };
  });

  let adding = $state(new Set<string>());
  async function add(hit: TaskHit) {
    adding = new Set([...adding, hit.key]);
    const ok = await act("manual/task", { uid: hit.key, onBoard: true }, "Added to the farming board");
    if (ok) {
      hit.added = true;
      await changed();
      // A resource is farmed to an amount: ask for it right away.
      if (hit.kind === "resource") openDetails("task:" + hit.key, { focusTarget: true });
    }
    adding.delete(hit.key);
    adding = new Set(adding);
  }

  async function clearDone() {
    const r = await act<Removed>("tasks/clear", {});
    if (r) {
      toast(`Took ${r.removed} finished task${r.removed === 1 ? "" : "s"} off the board`);
      await changed();
    }
  }
</script>

<Page title="Farming" {page}>
  {#snippet children(data)}
    {@const n = (state: string) => data.tasks.filter((t) => t.state === state).length}
    <div class="page">
      <div class="page-head"><div><h1>Farming</h1>
        <p>What you are farming, on one board and saved per account. Click a task for what it needs and where to get it.</p></div>
        <span class="grow"></span>
        {#if data.done}<button title="Take every finished task off the board" onclick={clearDone}>Clear done ({data.done})</button>{/if}
      </div>
      <div class="toolbar"><input id="taskSearch" type="search" aria-label="Add a task" placeholder="Add a task: equipment, mod, arcane or resource" bind:value={st.query} /></div>
      {#if searchError}<div class="callout">{searchError}</div>
      {:else if found}
        {#if found.length}
          <div class="list task-results">
            {#each found as r (r.key)}
              <div class="list-row"><Art src={r.image} cls="sm" /><div class="main"><div class="t">{r.name}</div><div class="s">{r.sub}</div></div>
                <button disabled={r.added || adding.has(r.key)} onclick={() => add(r)}>{r.added ? "On the board" : "Add"}</button></div>
            {/each}
          </div>
        {:else}
          <div class="muted">Nothing to farm by that name. Parts are farmed for their equipment, and Ctrl+K finds everything else.</div>
        {/if}
      {/if}
      {#if !data.tasks.length}
        <div class="empty"><b>No tasks yet</b>Search above for equipment, a mod, an arcane or a resource, or choose “Add to farming” in any item’s details.</div>
      {:else}
        <div class="task-legend">
          <span class="missing"><i></i>Missing parts or copies <b>{n("missing")}</b></span>
          <span class="resources"><i></i>Parts there, resources short <b>{n("resources")}</b></span>
          <span class="done"><i></i>Done <b>{n("done")}</b></span></div>
        {#if !data.inventoryKnown}<p class="callout">No inventory loaded: the colours only use counts you set by hand.</p>{/if}
        {#if data.inventoryKnown && !data.ranksKnown && data.tasks.some((t) => t.kind === "mod" || t.kind === "arcane")}
          <p class="callout info">Ranked mods and arcanes are counted from the next inventory read; until then only unranked copies count.</p>
        {/if}
        <div class="task-board">
          {#each data.tasks as t (t.uid)}
            <button type="button" class="task-card {t.state}" onclick={() => openDetails("task:" + t.uid)}>
              <span class="task-pic">{#if t.image}<img alt="" loading="lazy" decoding="async" src={artUrl(t.image, 60)} />{/if}</span>
              <span class="task-body">
                <span class="task-top"><span class="task-kind">{t.sub}</span><span class="task-state">{TASK_STATE[t.state] || t.state}</span></span>
                <span class="task-name">{t.name}</span>
                <span class="task-note">{t.note}</span>
                <span class="bar thin"><i style:width="{t.need ? Math.min(100, Math.round((100 * t.have) / t.need)) : 0}%"></i></span>
              </span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  {/snippet}
</Page>
