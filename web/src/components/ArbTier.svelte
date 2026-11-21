<script lang="ts">
  import { TIER_CLASS } from "../lib/game";
  let { tier }: { tier: string | null | undefined } = $props();
</script>

{#if tier}
  <span class="pill tier {TIER_CLASS[tier] ?? ''}" title="Community tier from arbi.guide: S is the best for farming">{tier} tier</span>
{/if}
