<!-- Where to get something: its drop sources, and a link to the wiki for the rest. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "../components/Icon.svelte";
  import { wiki } from "../lib/format";
  import type { Drop } from "../types/Drop";
  import type { DropCoverage } from "../types/DropCoverage";
  import DropRows from "./DropRows.svelte";
  let { name, drops, coverage, extra }: { name: string; drops: Drop[]; coverage: DropCoverage | null | undefined; extra?: Snippet } = $props();
</script>

<div class="dsec"><h2>Where to get it</h2>
  <p class="muted" style="font-size:13px">{coverage && coverage.available < coverage.total
    ? `Some drop tables are unavailable (${coverage.available}/${coverage.total} loaded).`
    : "Drop tables do not include every purchase, quest or time-limited source."} Chances are per listed reward roll; enemy chances are conditional on a mod or blueprint drop. Data from <a href="https://drops.warframestat.us/" target="_blank" rel="noopener">WFCD's drop tables</a> may be cached.</p>
  {@render extra?.()}
  {#if drops.length}
    <div class="list"><DropRows drops={drops.slice(0, 8)} /></div>
    {#if drops.length > 8}<details class="more"><summary>Show {drops.length - 8} more sources</summary><div class="list"><DropRows drops={drops.slice(8)} /></div></details>{/if}
  {:else}
    <div class="muted">No matching direct drops in the available tables. Check any parts and the wiki for other sources.</div>
  {/if}
  <a class="btn ghost" style="margin-top:8px" href={wiki(name)} target="_blank" rel="noopener"><Icon name="link" /> {name} on the Warframe Wiki</a>
</div>
