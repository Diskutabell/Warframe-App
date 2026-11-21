<!-- A dialog over the page. Escape, a click beside it, or `onclose` close it; while it is open
     the app's own shortcuts stay quiet (ui.modal). -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { ui } from "../lib/app.svelte";
  let { label, onclose, children }: { label: string; onclose: () => void; children: Snippet } = $props();
  let box: HTMLDivElement;
  $effect(() => {
    ui.modal = true;
    const first = box.querySelector<HTMLElement>("input, button, a[href]");
    first?.focus();
    return () => (ui.modal = false);
  });
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="modal" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}
  onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); onclose(); } }}>
  <div class="modal-box" role="dialog" aria-modal="true" aria-label={label} bind:this={box}>{@render children()}</div>
</div>
