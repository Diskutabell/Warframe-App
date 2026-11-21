<script lang="ts">
  import { fmt } from "../lib/format";
  let { n }: { n: number | null | undefined } = $props();
</script>

{#if n}<span class="duc">{fmt(n)}</span>{/if}
