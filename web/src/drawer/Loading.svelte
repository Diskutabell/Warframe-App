<script lang="ts">
  import Top from "./Top.svelte";
  let { error = null }: { error?: string | null } = $props();
</script>

<Top />
<div class="stack">
  {#if error}<div class="callout">{error}</div>
  {:else}<div class="skeleton" style="width:60%"></div><div class="skeleton"></div><div class="skeleton" style="width:80%"></div>{/if}
</div>
