<!-- A row of buttons, one of them chosen. -->
<script lang="ts" generics="T extends string">
  let { options, value = $bindable(), label = "", onchange }: { options: readonly (readonly [T, string])[]; value: T; label?: string; onchange?: () => void } = $props();
</script>

<div class="seg" role="group" aria-label={label || null}>
  {#each options as [v, l]}
    <button type="button" class:on={value === v} aria-pressed={value === v} onclick={() => { value = v; onchange?.(); }}>{l}</button>
  {/each}
</div>
