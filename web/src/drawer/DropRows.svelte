<script lang="ts">
  import type { Drop } from "../types/Drop";
  let { drops }: { drops: Drop[] } = $props();
</script>

{#each drops as d}
  <div class="list-row">
    <div class="main"><div class="t">{d.location}</div>
      <div class="s">{d.item}{#if d.mode}{" "}· {d.mode}{/if}{#if d.rotation}{" "}· rotation {d.rotation}{/if}{#if d.stage}{" "}· {d.stage}{/if}{#if d.rarity}{" "}· {d.rarity}{/if}</div></div>
    <span class="num" style="text-align:right">{d.chance}%{#if d.enemyDropChance != null}<div class="faint" style="font-size:12px">of {d.enemyDropChance}% enemy {d.enemyDropKind || "blueprint"} drop</div>{/if}</span>
  </div>
{/each}
