<!-- A farming task: what the builds still take, or what is held against the goal. -->
<script lang="ts">
  import Art from "../components/Art.svelte";
  import Stepper from "../components/Stepper.svelte";
  import { act, api } from "../lib/api";
  import { changed, closeDetails, openDetails, ui } from "../lib/app.svelte";
  import { TASK_PILL, TASK_STATE, fmt } from "../lib/format";
  import { loader } from "../lib/loader.svelte";
  import { toast } from "../lib/toasts.svelte";
  import type { DetailPart } from "../types/DetailPart";
  import type { TaskDetail } from "../types/TaskDetail";
  import Acquisition from "./Acquisition.svelte";
  import DropRows from "./DropRows.svelte";
  import Loading from "./Loading.svelte";
  import RecipeDetails from "./RecipeDetails.svelte";
  import RelicChips from "./RelicChips.svelte";
  import ResourceRows from "./ResourceRows.svelte";
  import TaskButton from "./TaskButton.svelte";
  import Top from "./Top.svelte";

  let { uid }: { uid: string } = $props();
  const x = loader<TaskDetail>((signal) => api("task?uid=" + encodeURIComponent(uid), undefined, signal), ["inventory", "catalog", "tasks"]);
  let goal = $state<HTMLInputElement | null>(null);
  // Opened right after adding a resource: start in its goal field.
  let focused = false;
  $effect(() => {
    if (goal && ui.drawerFocusTarget && !focused) {
      focused = true;
      goal.focus();
      goal.select();
    }
  });

  async function setGoal(t: TaskDetail, e: Event) {
    const text = (e.currentTarget as HTMLInputElement).value.trim();
    // Cleared: back to the default (one copy, or an arcane's top rank).
    const target = text === "" ? null : Math.max(1, Math.round(+text) || 1);
    if (!t.onBoard) {
      toast("Add it to the farming board first.", true);
      return;
    }
    if (await act("manual/task", { uid: t.uid, target })) await changed();
  }

  function partLine(p: DetailPart): { text: string; cls: string } {
    const held = `${fmt(p.have)} / ${fmt(p.need)}`;
    if (p.have < p.need) return { text: `${held} held, ${fmt(p.need - p.have)} missing`, cls: "warn-text" };
    if (p.crafting) return { text: `${held}, ${fmt(p.crafting)} crafting in the foundry`, cls: "lotus" };
    if (p.recipeUnknown) return { text: `${held} held, to craft; recipe unknown`, cls: "amber" };
    if (p.builds) return { text: `${held} held, ${fmt(p.builds)} to craft`, cls: "lotus" };
    return { text: `${held} held`, cls: "lotus" };
  }
</script>

{#if !x.data}
  <Loading error={x.error ?? (x.downloading ? "Downloading game data…" : null)} />
{:else}
  {@const t = x.data}
  {@const item = t.item && t.item.kind === "item" ? t.item : null}
  <Top market={item?.setSlug ?? null} />
  <div class="dhead"><Art src={t.image} cls="xl" /><div class="grow"><h2>{t.name}</h2>
    <div class="row wrap"><span class="pill {TASK_PILL[t.state] || ''}">{TASK_STATE[t.state] || t.state}</span><span class="pill">{t.sub}</span></div>
    <div class="task-line">{t.note}</div></div></div>
  <div class="row wrap">
    {#if t.kind === "item"}<button onclick={() => openDetails(t.uid)}>Item details</button>{/if}
    <TaskButton uid={t.uid} onBoard={t.onBoard} ondone={(on) => { if (!on) closeDetails(); }} />
  </div>

  {#if item}
    {@const parts = item.parts}
    {@const priority = (p: DetailPart) => (p.relics.some((r) => r.owned && r.open) ? 0 : p.relics.some((r) => !r.vaulted) ? 1 : p.drops.length ? 2 : 3)}
    {@const missing = parts.filter((p) => p.have < p.need).sort((a, b) => priority(a) - priority(b))}
    {@const materials = t.materials ?? []}
    {@const credits = t.credits}
    {@const short = materials.some((m) => m.have != null && m.have < m.count) || (credits?.have != null && credits.have < credits.need)}
    {@const unknown = parts.filter((p) => p.recipeUnknown && p.have >= p.need)}
    {@const next = t.state === "done" ? "" : missing.length ? `Consider starting with ${missing[0].name}.`
      : unknown.length && !short ? `The game data has no recipe for ${unknown.map((p) => p.name).join(", ")} yet: the foundry in game shows what crafting it takes.`
      : t.state === "resources" ? "Every part is there: farm the resources marked short below." : ""}
    {#if next}<p class="muted" style="margin:12px 0 0">{next}</p>{/if}
    {#if !t.inventoryKnown}<p class="callout" style="margin-top:12px">No inventory loaded: counts only include what you set by hand, and every part is taken to still need crafting.</p>{/if}
    {#if parts.length}
      <div class="dsec"><h2>Parts</h2>
        {#if !item.recipesAvailable}<p class="callout">Component recipes are unavailable. Connect to download the recipe data or check the wiki.</p>{/if}
        {#each parts as p (p.uid)}
          {@const line = partLine(p)}
          <div class="part">
            <div class="row"><button class="ghost grow" style="text-align:left" onclick={() => openDetails(p.uid)}><b>{p.name}</b></button><span class={line.cls}>{line.text}</span></div>
            {#if p.have < p.need}
              {#if !p.relics.length && !p.drops.length}<div class="muted">No source in the loaded drop tables. Open the part for the wiki link.</div>
              {:else}
                {#if p.relics.length}<div class="chips"><RelicChips relics={p.relics} /></div>{/if}
                {#if p.drops.length}<details class="more"><summary>Direct drops ({p.drops.length})</summary><div class="list"><DropRows drops={p.drops} /></div></details>{/if}
              {/if}
            {/if}
            {#if p.recipe && p.builds}<RecipeDetails recipe={p.recipe} />{/if}
          </div>
        {/each}
      </div>
    {:else}
      <p class="muted">No parts are known for this item: check where to get it in its details.</p>
    {/if}
    {#if item.requires.length}
      <div class="dsec"><h2>Equipment required</h2><div class="list">
        {#each item.requires as r}<div class="list-row"><button class="ghost grow" style="text-align:left" onclick={() => openDetails(r.uid)}>{r.name}</button>
          <span>{r.have == null ? "Owned: unknown" : `${fmt(r.have)} / ${fmt(r.count)} owned`}</span></div>{/each}
      </div></div>
    {/if}
    {#if materials.length || credits?.need}
      <div class="dsec"><h2>Still to spend</h2>
        <p class="muted" style="margin:-4px 0 8px;font-size:13px">The final build and the parts still to craft. Parts already crafted, or crafting in the foundry, are left out.</p>
        <div class="list"><ResourceRows resources={materials} />
          {#if credits?.need}
            <div class="list-row res-row"><img class="res-pic" alt="" src="icons/credits.png" />
              <div class="main"><div class="t">Credits</div>
                <div class="s">{credits.have == null ? "Held: unknown" : `${fmt(credits.have)} held`}, {fmt(credits.need)} needed{#if credits.have != null && credits.have < credits.need}, <span class="warn-text">{fmt(credits.need - credits.have)} short</span>{/if}</div></div></div>
          {/if}
        </div></div>
    {/if}
    <p class="muted" style="font-size:13px;margin-top:16px">Recipes and sources may be incomplete or cached. Quest, purchase and time-limited sources may be missing; check the item details or the wiki.</p>
  {:else}
    {@const h = t.held ?? { count: 0, ranks: null, byHand: false }}
    {@const ranks = (h.ranks ?? []).slice().sort((a, b) => b - a)}
    {@const unranked = h.count - (t.kind === "arcane" ? ranks.reduce((s, r) => s + ((r + 1) * (r + 2)) / 2, 0) : ranks.length)}
    {@const byRank = ranks.reduce((m, r) => m.set(r, (m.get(r) || 0) + 1), new Map<number, number>())}
    {@const heldAs = ranks.length ? `Held: ${[...[...byRank].map(([r, n]) => `${fmt(n)} at rank ${r}`), ...(unranked > 0 ? [`${fmt(unranked)} unranked`] : [])].join(", ")}.` : ""}
    {@const third = t.kind === "arcane" ? ["Rank reached", t.rank == null ? "–" : `${t.rank} / ${t.topRank}`]
      : t.kind === "mod" ? ["Top rank", t.topRank == null ? "–" : String(t.topRank)] : ["Still to farm", fmt(Math.max(0, (t.target ?? 0) - h.count))]}
    <div class="kv" style="margin-top:14px">
      <div><span>{t.kind === "resource" ? "Held" : "Copies held"}</span><b><Stepper uid={t.uid} n={h.count} /></b></div>
      <div><span>Goal</span><b><input type="number" class="goal" min="1" step="1" value={t.target} aria-label="How many to hold" bind:this={goal} onchange={(e) => setGoal(t, e)} /></b></div>
      <div><span>{third[0]}</span><b>{third[1]}</b></div>
    </div>
    {#if t.kind === "arcane" && t.topRank != null}<p class="muted" style="font-size:13px">{fmt(t.targetDefault)} copies fuse to rank {t.topRank}, its top rank. {heldAs}</p>
    {:else if heldAs}<p class="muted" style="font-size:13px">{heldAs}</p>{/if}
    {#if h.byHand}<p class="muted" style="font-size:13px">The count is the one you set; the next inventory read replaces it.</p>{/if}
    {#if t.kind !== "resource" && t.inventoryKnown && h.ranks == null && !h.byHand}<p class="callout info">Ranked copies are counted from the next inventory read; until then only unranked ones count.</p>{/if}
    <Acquisition name={t.name} drops={t.drops ?? []} coverage={t.dropCoverage} />
  {/if}
{/if}
