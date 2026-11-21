<!-- A dialog over the page. Escape, a click beside it, or `onclose` close it; while one is open
     the app's own shortcuts stay quiet (ui.modals). Two at once (the update popup over a
     warning): Escape closes the one on top. -->
<script module lang="ts">
  const open: symbol[] = [];
</script>

<script lang="ts">
  import type { Snippet } from "svelte";
  import { ui } from "../lib/app.svelte";
  let { label, onclose, children }: { label: string; onclose: () => void; children: Snippet } = $props();
  let box: HTMLDivElement;
  const me = Symbol("modal");
  $effect(() => {
    const before = document.activeElement as HTMLElement | null;
    open.push(me);
    ui.modals++;
    const first = box.querySelector<HTMLElement>("input, button, a[href]");
    (first ?? box).focus();
    return () => {
      open.splice(open.indexOf(me), 1);
      ui.modals--;
      before?.focus?.({ preventScroll: true });
    };
  });
  // On the document: a click on the dialog's text leaves the keyboard on the page body.
  function keydown(e: KeyboardEvent) {
    if (e.key === "Escape" && open[open.length - 1] === me) {
      e.stopPropagation();
      onclose();
    }
  }
</script>

<svelte:document onkeydown={keydown} />
<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="modal" onclick={(e) => { if (e.target === e.currentTarget) onclose(); }}>
  <div class="modal-box" role="dialog" aria-modal="true" aria-label={label} tabindex="-1" bind:this={box}>{@render children()}</div>
</div>
