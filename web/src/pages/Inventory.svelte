<!-- Inventory: everything the last memory scan (or imported file) holds - resources, mods,
     arcanes, relics, parts, fish, decorations, equipment - searchable, with the game's icons. -->
<script lang="ts" module>
  const st = $state({ q: "", kind: "", sort: "name" as "name" | "count" | "value", tradable: false, limit: 240 });
</script>

<script lang="ts">
  import Page from "../components/Page.svelte";
  import Seg from "../components/Seg.svelte";
  import { openDetails } from "../lib/app.svelte";
  import { ago, artUrl, fmt } from "../lib/format";
  import { loader } from "../lib/loader.svelte";
  import type { InventoryRow } from "../types/InventoryRow";
  import type { InventoryView } from "../types/InventoryView";
  import type { Summary } from "../types/Summary";

  let { S }: { S: Summary } = $props();
  const page = loader<InventoryView>("inventory", ["inventory", "catalog"]);
  // Equipment kinds from the catalog, gathered under one chip; the details panel knows them,
  // parts and relics too.
  const EQUIPMENT = new Set(["Warframes", "Primary", "Secondary", "Melee", "Sentinels", "SentinelWeapons", "Archwing", "Arch-Gun",
    "Arch-Melee", "Pets", "Misc", "Weapons", "Amps", "Companions"]);
  const DETAILS = new Set(["Warframes", "Primary", "Secondary", "Melee", "Sentinels", "SentinelWeapons", "Archwing", "Arch-Gun",
    "Arch-Melee", "Pets", "Misc", "Prime parts", "Parts", "Relics"]);
  const group = (kind: string) => (EQUIPMENT.has(kind) ? "Equipment" : kind);
  const SORTS = [["name", "Name"], ["count", "Most held"], ["value", "Value"]] as const;
  const value = (x: InventoryRow) => (x.price || 0) * x.count;

  let q = $state(st.q);
  let qTimer: ReturnType<typeof setTimeout>;
  function typed() {
    clearTimeout(qTimer);
    qTimer = setTimeout(() => { st.q = q; st.limit = 240; }, 120);
  }

  const rows = $derived.by(() => {
    const ql = st.q.toLowerCase();
    const out = (page.data?.items ?? []).filter((x) => (!st.kind || group(x.kind) === st.kind) && (!st.tradable || x.slug) &&
      (!ql || x.name.toLowerCase().includes(ql) || x.kind.toLowerCase().includes(ql)));
    out.sort(st.sort === "count" ? (a, b) => b.count - a.count
      : st.sort === "value" ? (a, b) => value(b) - value(a) || (b.ducats || 0) - (a.ducats || 0)
      : (a, b) => a.name.localeCompare(b.name));
    return out;
  });
  const chips = $derived.by(() => {
    const kinds = new Map<string, number>();
    for (const x of page.data?.items ?? []) kinds.set(group(x.kind), (kinds.get(group(x.kind)) || 0) + 1);
    return [...kinds].sort((a, b) => b[1] - a[1]);
  });
  let broken = $state(new Set<string>());
</script>

{#snippet tileBody(x: InventoryRow)}
  {#if x.image && !broken.has(x.uid)}<img class="inv-pic" loading="lazy" decoding="async" alt="" src={artUrl(x.image, 52)} onerror={() => (broken = new Set([...broken, x.uid]))} />
  {:else}<span class="inv-pic none"></span>{/if}
  <span class="inv-body"><span class="inv-name">{x.name}</span>
    <span class="inv-meta"><span class="inv-kind">{x.kind}</span>
      {#if x.price != null}<span class="plat" title="Lowest sell order, each">{fmt(x.price)}</span>{/if}
      {#if x.ducats}<span class="duc" title="Ducats each">{fmt(x.ducats)}</span>{/if}</span></span>
  <span class="inv-count">{x.count > 1 || !EQUIPMENT.has(x.kind) ? fmt(x.count) : ""}</span>
{/snippet}

<Page title="Inventory" {page}>
  {#snippet children(data)}
    <div class="page">
      {#if !data.items.length}
        <div class="page-head"><div><h1>Inventory</h1></div></div>
        {#if S.sources.memscan.state === "absent"}
          <div class="empty"><b>No inventory loaded yet</b>Import an exported <code>inventory.json</code> in Settings (this build was made without the memory scan).</div>
        {:else}
          <div class="empty"><b>No inventory loaded yet</b>Switch on the memory scan in Settings and it is read while the game runs,
            after login and after each mission. An exported inventory file can be imported there too.</div>
        {/if}
      {:else}
        {@const c = data.currencies}
        <div class="page-head"><div><h1>Inventory</h1><p>{fmt(data.items.length)} stacks{data.source ? `, from the ${data.source}` : ""}, read {ago(data.imported)}.</p></div></div>
        <div class="tiles cur-row">
          {#each [["platinum", c?.platinum, "Platinum"], ["credits", c?.credits, "Credits"], ["endo", c?.endo, "Endo"], ["ducats", c?.ducats, "Ducats"]] as [icon, n, label]}
            {#if n != null}<div class="tile cur"><img src="icons/{icon}.png" alt="" width="26" height="26" /><div><b>{fmt(n as number)}</b><span>{label}</span></div></div>{/if}
          {/each}
        </div>
        <div class="toolbar">
          <input type="search" placeholder="Search the inventory" aria-label="Search" bind:value={q} oninput={typed} />
          <Seg options={SORTS} bind:value={st.sort} label="Sort" />
          <label class="check"><input type="checkbox" bind:checked={st.tradable} />Tradable</label>
          <span class="count">{fmt(rows.length)} shown</span>
        </div>
        <div class="kind-chips">
          <button class="chip" class:on={!st.kind} onclick={() => { st.kind = ""; st.limit = 240; }}>All <span>{fmt(data.items.length)}</span></button>
          {#each chips as [k, n]}<button class="chip" class:on={st.kind === k} onclick={() => { st.kind = k; st.limit = 240; }}>{k} <span>{fmt(n)}</span></button>{/each}
        </div>
        <div class="inv-grid">
          {#each rows.slice(0, st.limit) as x (x.uid)}
            {#if DETAILS.has(x.kind)}
              <button type="button" class="inv-tile click" title={x.name} onclick={() => openDetails(x.uid)}>{@render tileBody(x)}</button>
            {:else}
              <div class="inv-tile" title={x.name}>{@render tileBody(x)}</div>
            {/if}
          {:else}
            <div class="empty" style="grid-column:1/-1"><b>Nothing matches</b>Try another word or kind.</div>
          {/each}
        </div>
        {#if rows.length > st.limit}<div style="margin-top:12px"><button onclick={() => (st.limit += 240)}>Show {fmt(Math.min(240, rows.length - st.limit))} more</button></div>{/if}
      {/if}
    </div>
  {/snippet}
</Page>
