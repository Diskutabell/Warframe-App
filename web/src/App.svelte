<!-- The app: the sidebar, the page shown, the details drawer, the palette, toasts and popups. -->
<script lang="ts">
  import Palette from "./components/Palette.svelte";
  import Rail from "./components/Rail.svelte";
  import Skeleton from "./components/Skeleton.svelte";
  import Toasts from "./components/Toasts.svelte";
  import Update from "./components/Update.svelte";
  import Drawer from "./drawer/Drawer.svelte";
  import { app, closeDetails, openDetails, route, ui } from "./lib/app.svelte";
  import { PAGES, pageKey } from "./lib/pages";
  import { glide } from "./lib/wheel";

  const page = $derived(PAGES.find((p) => p.id === route.page) ?? PAGES[0]);
  let view: HTMLElement;
  let updateShown = $state(false);

  // A new page starts at its top, with the keyboard on it.
  $effect(() => {
    void page;
    view.scrollTop = 0;
    view.focus({ preventScroll: true });
  });
  // `#page?item=key` opens those details.
  $effect(() => {
    const item = route.params.get("item");
    if (item && !ui.drawer.length) openDetails(item);
  });

  // Not passive: a wheel click that glides must not also scroll natively.
  $effect(() => {
    document.addEventListener("wheel", glide, { passive: false });
    return () => document.removeEventListener("wheel", glide);
  });

  function keydown(e: KeyboardEvent) {
    // A dialog has the keyboard to itself.
    if (ui.modal) return;
    const el = document.activeElement as HTMLElement | null;
    const typing = !!el && /^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName);
    if ((e.ctrlKey && e.key.toLowerCase() === "k") || (e.key === "/" && !typing)) {
      e.preventDefault();
      ui.palette = true;
    } else if (e.key === "Escape") {
      if (ui.palette) ui.palette = false;
      else if (ui.drawer.length) closeDetails();
    } else if (!typing && !e.ctrlKey && !e.altKey && !e.metaKey && /^[0-9]$/.test(e.key) && !ui.palette) {
      const p = PAGES.find((pg, i) => pageKey(pg, i) === e.key);
      if (p) location.hash = "#" + p.id;
    }
  }
</script>

<svelte:document onkeydown={keydown} />

<Rail />
<main id="view" tabindex="-1" bind:this={view}>
  {#if !app.state}
    {#if app.connected}<Skeleton title={page.title} />
    {:else}
      <div class="page"><div class="page-head"><div><h1>The engine isn't responding</h1>
        <p>Reconnecting by itself. If it keeps failing, start the app again from a terminal and read its output.</p></div></div></div>
    {/if}
  {:else}
    {#key page.id}<page.component S={app.state} />{/key}
  {/if}
</main>
<Drawer />
{#if ui.palette}<Palette />{/if}
<Toasts />
{#if app.state?.update && !updateShown}<Update release={app.state.update} onclose={() => (updateShown = true)} />{/if}
