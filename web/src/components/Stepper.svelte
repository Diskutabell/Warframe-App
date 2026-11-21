<!-- A count set by hand, one up or down. -->
<script lang="ts">
  import { act, changed } from "../lib/stepper";
  let { uid, n, onchange }: { uid: string; n: number; onchange?: () => void } = $props();
  let busy = $state(false);
  // The count saved last: the page's own copy of `n` is reloaded only a moment later, and a quick
  // second click must step on from what the first one saved.
  let shown = $derived(n);
  async function set(count: number) {
    busy = true;
    count = Math.max(0, count);
    if (await act("manual/count", { uid, count })) {
      shown = count;
      await changed();
      // A reload that left before the save may have landed meanwhile.
      shown = count;
      onchange?.();
    }
    busy = false;
  }
</script>

<span class="stepper">
  <button type="button" aria-label="One fewer" disabled={busy} onclick={(e) => { e.stopPropagation(); set(shown - 1); }}>−</button><b>{shown}</b><button
    type="button" aria-label="One more" disabled={busy} onclick={(e) => { e.stopPropagation(); set(shown + 1); }}>+</button>
</span>
