<!-- A count set by hand, one up or down. -->
<script lang="ts">
  import { act, changed } from "../lib/stepper";
  let { uid, n, onchange }: { uid: string; n: number; onchange?: () => void } = $props();
  let busy = $state(false);
  async function set(count: number) {
    busy = true;
    if (await act("manual/count", { uid, count: Math.max(0, count) })) {
      await changed();
      onchange?.();
    }
    busy = false;
  }
</script>

<span class="stepper">
  <button type="button" aria-label="One fewer" disabled={busy} onclick={(e) => { e.stopPropagation(); set(n - 1); }}>−</button><b>{n}</b><button
    type="button" aria-label="One more" disabled={busy} onclick={(e) => { e.stopPropagation(); set(n + 1); }}>+</button>
</span>
