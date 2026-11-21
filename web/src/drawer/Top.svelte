<!-- The drawer's top row: back, links, pin, close. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "../components/Icon.svelte";
  import { closeDetails, ui } from "../lib/app.svelte";
  let { market = null, extra }: { market?: string | null; extra?: Snippet } = $props();
  let close: HTMLButtonElement;
  // Keyboard users land in the drawer, on its close button.
  $effect(() => {
    if (!document.activeElement || !close.closest(".drawer")?.contains(document.activeElement)) close.focus();
  });
</script>

<div class="drawer-top">
  {#if ui.drawer.length > 1}<button class="ghost" onclick={() => ui.drawer.pop()}><Icon name="back" /> Back</button>{/if}
  <span class="grow"></span>
  {#if market}<a class="btn ghost" href="https://warframe.market/items/{market}" target="_blank" rel="noopener"><Icon name="link" /> warframe.market</a>{/if}
  {@render extra?.()}
  <button class="ghost icon" title="Close (Esc)" aria-label="Close" bind:this={close} onclick={closeDetails}><Icon name="x" /></button>
</div>
