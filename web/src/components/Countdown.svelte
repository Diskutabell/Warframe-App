<!-- "2h 14m" until `until` (ISO or epoch seconds), then `done`. -->
<script lang="ts">
  import { clock } from "../lib/app.svelte";
  import { left } from "../lib/format";
  let { until, done = "now" }: { until: string | number | null | undefined; done?: string } = $props();
  const text = $derived(left(until, clock.now));
</script>

<span>{text === "now" ? done : text}</span>
