<script lang="ts">
  import { toasts } from "../lib/toasts.svelte";
</script>

<div id="toasts" role="status" aria-live="polite">
  {#each toasts as t (t.id)}<div class="toast" class:err={t.err}>{t.text}</div>{/each}
</div>
