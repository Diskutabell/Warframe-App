<!-- Mastery: everything that gives mastery, what is left, and what to do about it. -->
<script lang="ts" module>
  const st = $state({ q: "", status: "todo" as "todo" | "missing" | "leveling" | "mastered" | "", cat: "", prime: "", layout: "table" as "table" | "cards", unvaulted: false, buildable: false, maxMr: "", limit: 300 });
</script>

<script lang="ts">
  import Art from "../components/Art.svelte";
  import Page from "../components/Page.svelte";
  import PartPips from "../components/PartPips.svelte";
  import Seg from "../components/Seg.svelte";
  import StatusPill from "../components/StatusPill.svelte";
  import { openDetails } from "../lib/app.svelte";
  import { fmt } from "../lib/format";
  import { loader } from "../lib/loader.svelte";
  import type { MasteryRow } from "../types/MasteryRow";
  import type { Summary } from "../types/Summary";

  let { S }: { S: Summary } = $props();
  const page = loader<MasteryRow[]>("mastery", ["inventory", "catalog"]);
  const STATUS = [["todo", "To do"], ["missing", "Not levelled"], ["leveling", "Levelling"], ["mastered", "Mastered"], ["", "All"]] as const;

  const allCats = $derived([...new Set((page.data ?? []).map((r) => r.category))].sort());
  const rows = $derived.by(() => {
    const q = st.q.toLowerCase();
    return (page.data ?? []).filter((r) =>
      (st.status === "" || (st.status === "todo" ? r.status !== "mastered" : r.status === st.status)) &&
      (!st.cat || r.category === st.cat) && (!st.prime || (st.prime === "prime" ? r.isPrime : !r.isPrime)) &&
      (!st.unvaulted || !r.vaulted) && (!st.buildable || r.buildable) && (!q || r.name.toLowerCase().includes(q)) &&
      (st.maxMr === "" || (r.masteryReq || 0) <= Number(st.maxMr)));
  });
  const visible = $derived(rows.slice(0, st.limit));

  /** What to do next about an item, as plain text and a tone. */
  function next(r: MasteryRow): { text: string; cls: string } {
    if (r.status === "mastered") return { text: "", cls: "" };
    if (r.building) return { text: "In the foundry", cls: "lotus" };
    if (r.owned) return { text: `Owned: level it to ${r.maxRank}`, cls: "" };
    const parts = r.parts ?? [];
    if (r.status === "leveling") return { text: `Rank ${r.rank} of ${r.maxRank}`, cls: "" };
    if (!parts.length) return { text: r.requires?.length ? `Needs ${r.requires.join(", ")}` : "", cls: "" };
    if (r.buildable) return { text: "Parts collected", cls: "lotus" };
    const have = parts.filter((p) => p.have >= p.need).length;
    const farm = r.farm?.[0];
    return { text: `${have} of ${parts.length} parts${farm?.relics[0] ? `; crack ${farm.relics[0]}` : ""}`, cls: "" };
  }
  const pct = (r: MasteryRow) => `${((100 * r.rank) / Math.max(1, r.maxRank)).toFixed(0)}%`;
</script>

<Page title="Mastery" {page}>
  {#snippet children()}
    <div class="page">
      <div class="page-head"><div><h1>Mastery</h1>
        <p>{fmt(S.mastered)} of {fmt(S.total)} items mastered, {fmt(S.mastery.total)}{S.mastery.lacks ? "+" : ""} mastery XP in all. Choose an item for its parts and where to farm them.</p></div></div>
      <div class="tiles" style="margin-bottom:14px">
        {#each S.mastery.byCategory as c}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="tile click" class:on={st.cat === c.category} onclick={() => (st.cat = st.cat === c.category ? "" : c.category)}>
            <div class="row spread"><span class="muted" style="font-size:12.5px">{c.category}</span><span class="num" style="font-size:12.5px">{c.mastered}/{c.total}</span></div>
            <div class="bar thin" style="margin-top:8px"><i style:width="{((100 * c.mastered) / Math.max(1, c.total)).toFixed(1)}%"></i></div>
          </div>
        {/each}
      </div>
      <div class="toolbar">
        <input type="search" placeholder="Search" aria-label="Search" bind:value={st.q} oninput={() => (st.limit = 300)} />
        <Seg options={STATUS} bind:value={st.status} label="Status" />
        <select aria-label="Category" bind:value={st.cat}><option value="">All categories</option>{#each allCats as c}<option>{c}</option>{/each}</select>
        <select aria-label="Mastery rank needed" bind:value={st.maxMr}><option value="">Any MR</option>{#each Array.from({ length: 19 }, (_, i) => i) as i}<option value={String(i)}>MR {i} or lower</option>{/each}</select>
        <select aria-label="Prime filter" bind:value={st.prime}><option value="">All items</option><option value="prime">Primes only</option><option value="nonprime">Non-Primes only</option></select>
        <label class="check"><input type="checkbox" bind:checked={st.unvaulted} />Not vaulted</label>
        <label class="check" title="Parts may include unbuilt blueprints; resources and required equipment are not checked"><input type="checkbox" bind:checked={st.buildable} />Parts collected</label>
        <Seg options={[["table", "Table"], ["cards", "Cards"]] as const} bind:value={st.layout} label="Mastery display" />
        <span class="count">{fmt(rows.length)} items</span>
      </div>
      {#if st.layout === "table"}
        <div class="table-wrap"><table class="table">
          <thead><tr><th>Item</th><th>Category</th><th class="r">MR</th><th style="width:160px">Rank</th><th>Status</th><th>Next</th></tr></thead>
          <tbody>
            {#each visible as r (r.uid)}
              {@const n = next(r)}
              <tr class="click" onclick={() => openDetails(r.uid)}>
                <td><div class="row"><Art src={r.image} cls="sm" /><span class="ellipsis" style="font-weight:600">{r.name}</span>
                  {#if r.vaulted}<span class="pill vaulted">Vaulted</span>{/if}{#if r.manual}<span class="pill">Marked</span>{/if}</div></td>
                <td class="muted">{r.category}</td><td class="r num">{r.masteryReq || ""}</td>
                <td><div class="row"><div class="bar thin grow" class:lotus={r.status !== "mastered"}><i style:width={pct(r)}></i></div>
                  <span class="num muted" style="font-size:12.5px;width:44px;text-align:right">{r.rank}/{r.maxRank}</span></div></td>
                <td><StatusPill status={r.status} /></td><td><span class={n.cls}>{n.text}</span></td>
              </tr>
            {:else}
              <tr><td colspan="6"><div class="empty"><b>Nothing matches</b>Try another filter.</div></td></tr>
            {/each}
          </tbody>
        </table></div>
      {:else}
        <div class="cards">
          {#each visible as r (r.uid)}
            {@const n = next(r)}
            <button type="button" class="icard mastery-card" class:done={r.status === "mastered"} onclick={() => openDetails(r.uid)}>
              <Art src={r.image} cls="lg" />
              <span class="body">
                <span class="row"><span class="name grow">{r.name}</span>{#if r.masteryReq}<span class="pill">MR {r.masteryReq}</span>{/if}</span>
                <span class="muted">{r.category}</span>
                {#if r.parts?.length}<span class="pips"><PartPips parts={r.parts} /></span>{/if}
                <span class="row"><span class="bar thin grow" class:lotus={r.status !== "mastered"}><i style:width={pct(r)}></i></span><span class="num muted">{r.rank}/{r.maxRank}</span></span>
                <span class="meta"><StatusPill status={r.status} />{#if r.owned}<span class="pill good">Owned</span>{/if}{#if r.vaulted}<span class="pill vaulted">Vaulted</span>{/if}{#if r.manual}<span class="pill">Marked</span>{/if}</span>
                {#if n.text}<span class="muted"><span class={n.cls}>{n.text}</span></span>{/if}
              </span>
            </button>
          {:else}
            <div class="empty"><b>Nothing matches</b>Try another filter.</div>
          {/each}
        </div>
      {/if}
      {#if rows.length > st.limit}<div style="margin-top:12px"><button onclick={() => (st.limit += 300)}>Show {fmt(Math.min(300, rows.length - st.limit))} more</button></div>{/if}
    </div>
  {/snippet}
</Page>
