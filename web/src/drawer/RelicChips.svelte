<script lang="ts">
  import { openDetails } from "../lib/app.svelte";
  import type { RelicRef } from "../types/RelicRef";
  let { relics }: { relics: RelicRef[] } = $props();
</script>

{#each relics as r}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <span class="chip" class:vaulted={r.vaulted} class:cracks={r.open} class:owned={r.owned > 0} onclick={(e) => { e.stopPropagation(); openDetails(r.relic); }}
    title={[r.rarity, r.chance != null ? `${r.chance}% intact` : "", r.vaulted ? "vaulted" : "", r.open ? "a fissure is open now" : ""].filter(Boolean).join(", ")}>
    {r.relic}{#if r.owned}<b class="lotus">{" "}×{r.owned}</b>{/if}</span>
{/each}
