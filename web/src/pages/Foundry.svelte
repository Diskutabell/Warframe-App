<!-- Foundry: what is building, and everything you could build with the parts you hold. -->
<script lang="ts" module>
  const st = $state({ q: "", cat: "", show: "buildable" as "buildable" | "started" | "missing" | "", kind: "" as "" | "gear" | "craft", myMr: false });
</script>

<script lang="ts">
  import Art from "../components/Art.svelte";
  import Countdown from "../components/Countdown.svelte";
  import GameImg from "../components/GameImg.svelte";
  import Icon from "../components/Icon.svelte";
  import Img from "../components/Img.svelte";
  import Page from "../components/Page.svelte";
  import Seg from "../components/Seg.svelte";
  import StatusPill from "../components/StatusPill.svelte";
  import { act } from "../lib/api";
  import { clock, openDetails } from "../lib/app.svelte";
  import { fmt, wiki } from "../lib/format";
  import { ICON } from "../lib/game";
  import { loader } from "../lib/loader.svelte";
  import type { FoundryRow } from "../types/FoundryRow";
  import type { FoundryView } from "../types/FoundryView";
  import type { Summary } from "../types/Summary";

  let { S }: { S: Summary } = $props();
  const page = loader<FoundryView>("foundry", ["inventory", "catalog"]);
  const KINDS = [["", "All"], ["gear", "Equipment"], ["craft", "Parts and gear"]] as const;
  const SHOW = [["buildable", "Requirements collected"], ["started", "Started"], ["missing", "Not built yet"], ["", "Everything"]] as const;
  const isCraft = (r: FoundryRow): r is Extract<FoundryRow, { craft: boolean }> => "craft" in r;
  // The low end of a badge band ("16-18") is the rank that is certain.
  const mr = $derived(S.masteryRank);

  const rows = $derived.by(() => {
    const q = st.q.toLowerCase();
    return (page.data?.items ?? []).filter((r) => (!q || r.name.toLowerCase().includes(q)) && (!st.cat || r.category === st.cat) &&
      (st.kind === "" || (st.kind === "craft") === isCraft(r)) &&
      (!st.myMr || mr == null || (r.masteryReq || 0) <= mr) &&
      (st.show === "" || (st.show === "buildable" && r.buildable && (isCraft(r) || r.status !== "mastered")) ||
        (st.show === "started" && r.have > 0 && !r.buildable) ||
        (st.show === "missing" && (isCraft(r) ? !r.held : r.status !== "mastered"))));
  });

  async function pin(r: Extract<FoundryRow, { craft: boolean }>) {
    if (await act("manual/pin", { uid: r.uid, pinned: !r.pinned }, r.pinned ? "Unpinned" : "Pinned to the Overview")) r.pinned = !r.pinned;
  }
</script>

<Page title="Foundry" {page}>
  {#snippet children(d)}
    {@const cats = [...new Set(d.items.map((r) => r.category))].sort()}
    {@const ready = d.items.filter((r) => r.buildable && (isCraft(r) || r.status !== "mastered")).length}
    <div class="page">
      <div class="page-head"><div><h1>Foundry</h1><p>{d.pending.length} building, and {ready} with every listed part in hand. Part counts can include blueprints you have not built yet; the foundry in game shows the resources.</p></div></div>
      {#if d.pending.length}
        <div class="card" style="margin-bottom:16px"><div class="card-head"><h2>Building</h2></div>
          <div class="cards">
            {#each d.pending as f}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div class="icard" style={f.known ? null : "cursor:default"} onclick={() => f.known && openDetails(f.uid)}>
                <Art src={f.image} />
                <div class="body"><span class="name">{f.name}</span>
                  <div class="meta">{#if f.done}{#if f.done <= clock.now}<span class="pill gold">Ready to claim</span>{:else}Ready in <Countdown until={f.done} />{/if}{:else}Building{/if}</div></div>
              </div>
            {/each}
          </div>
        </div>
      {/if}
      <div class="toolbar">
        <input type="search" placeholder="Search" aria-label="Search" bind:value={st.q} />
        <Seg options={SHOW} bind:value={st.show} label="Show" />
        <Seg options={KINDS} bind:value={st.kind} label="Kind" />
        <select aria-label="Category" bind:value={st.cat}><option value="">All categories</option>{#each cats as c}<option>{c}</option>{/each}</select>
        {#if mr != null}<label class="check" title="Hide what needs a higher mastery rank than yours"><input type="checkbox" bind:checked={st.myMr} />Buildable at MR {mr}</label>{/if}
        <span class="count">{fmt(rows.length)} items</span>
      </div>
      <div class="cards">
        {#each rows.slice(0, 300) as r (r.uid)}
          {@const craft = isCraft(r) ? r : null}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="icard" class:done={r.buildable} style={craft ? "cursor:default" : null} onclick={() => !craft && openDetails(r.uid)}>
            <Art src={r.image} cls="lg" />
            <div class="body">
              <div class="row"><span class="name grow">{r.name}</span>
                {#if craft}<button class="ghost icon pin" class:on={craft.pinned} title={craft.pinned ? "Unpin from the Overview" : "Pin to the Overview"} style="padding:2px"
                  onclick={(e) => { e.stopPropagation(); pin(craft); }}><Icon name="star" /></button>{/if}
                {#if r.masteryReq}<span class="mrreq" title="Needs mastery rank {r.masteryReq}"><GameImg path={ICON.mastery} px={14} />{r.masteryReq}</span>{/if}</div>
              <div class="pips">
                {#each r.parts as p}
                  <span class="pip" class:ok={p.have >= p.need} title="{p.name} - own {fmt(p.have)} of {fmt(p.need)}">
                    {#if p.image}<Img src={p.image} px={24} />{:else if p.have >= p.need}<Icon name="check" />{/if}{#if !craft && p.have > 1}<b>{p.have}</b>{/if}</span>
                {/each}
              </div>
              <div class="meta"><span>{r.have} of {r.need} {craft ? "needs met" : "parts"}</span>
                {#if craft}
                  <span class="pill">{craft.category}</span>{#if craft.held}<span class="pill lotus">{fmt(craft.held)} held</span>{/if}{#if craft.quantity > 1}<span class="faint">makes {craft.quantity}</span>{/if}
                {:else if "status" in r}<StatusPill status={r.status} />{/if}
                {#if r.buildable && (craft || ("status" in r && r.status !== "mastered"))}<span class="pill good">{craft ? "Needs met" : "Parts collected"}</span>{/if}
                {#if "vaulted" in r && r.vaulted}<span class="pill vaulted">Vaulted</span>{/if}
                {#if craft}<a href={wiki(r.name)} target="_blank" rel="noopener" onclick={(e) => e.stopPropagation()}>Where to get</a>{/if}</div>
            </div>
          </div>
        {:else}
          <div class="empty"><b>Nothing here</b>{st.show === "buildable" ? "No item has all its parts yet." : "Try another filter."}</div>
        {/each}
      </div>
    </div>
  {/snippet}
</Page>
