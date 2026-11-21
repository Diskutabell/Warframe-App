<!-- A faction's emblem and name side by side. -->
<script lang="ts">
  import GameImg from "./GameImg.svelte";
  import { factionIcon, foeName } from "../lib/game";
  let { name, px = 18 }: { name: string | null | undefined; px?: number } = $props();
</script>

{#if name}<span class="ftag"><GameImg path={factionIcon(name)} {px} />{foeName(name)}</span>{/if}
