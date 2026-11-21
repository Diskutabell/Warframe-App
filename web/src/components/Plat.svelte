<script lang="ts">
  import { fmt } from "../lib/format";
  let { n }: { n: number | null | undefined } = $props();
</script>

{#if n == null}<span class="faint">–</span>{:else}<span class="plat">{fmt(n)}</span>{/if}
