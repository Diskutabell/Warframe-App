<script lang="ts">
  import { PATHS, type IconName } from "../lib/icons";
  let { name, cls = "" }: { name: IconName; cls?: string } = $props();
</script>

<svg class="ic {cls}" viewBox="0 0 24 24" aria-hidden="true"><path d={PATHS[name]} /></svg>
