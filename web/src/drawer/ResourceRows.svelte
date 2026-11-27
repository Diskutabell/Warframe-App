<script lang="ts">
  import { openDetails } from "../lib/app.svelte";
  import { artUrl, fmt, wiki } from "../lib/format";
  import type { Ingredient } from "../types/Ingredient";
  let { resources }: { resources: Ingredient[] } = $props();
  let broken = $state(new Set<string>());
</script>

{#each resources as r}
  <div class="list-row res-row">
    {#if r.image}<img class="res-pic" alt="" loading="lazy" decoding="async" src={artUrl(r.image, 34)} style:visibility={broken.has(r.uid) ? "hidden" : null} onerror={() => (broken = new Set([...broken, r.uid]))} />
    {:else}<span class="res-pic"></span>{/if}
    <div class="main"><div class="t">{r.name}</div>
      <div class="s">{r.have == null ? "Held: unknown" : `${fmt(r.have)} held`}, {fmt(r.count)} needed{#if r.have != null && r.have < r.count}, <span class="warn-text">{fmt(r.count - r.have)} short</span>{/if}</div>
      {#if r.have != null}<div class="bar thin {r.have >= r.count ? 'lotus' : 'warn'}"><i style:width="{Math.min(100, (100 * r.have) / Math.max(1, r.count)).toFixed(0)}%"></i></div>{/if}</div>
    {#if r.drops.length}<button onclick={() => openDetails(`drop:${r.name}`)}>Sources</button>
    {:else}<a href={wiki(r.name)} target="_blank" rel="noopener">Wiki</a>{/if}
  </div>
{/each}
