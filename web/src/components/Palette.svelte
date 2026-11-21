<!-- The command palette (Ctrl+K or /): pages, items, parts, relics and syndicates by name. -->
<script lang="ts">
  import { api, isLoading } from "../lib/api";
  import { openDetails, ui } from "../lib/app.svelte";
  import type { IconName } from "../lib/icons";
  import { PAGES } from "../lib/pages";
  import type { FindHit } from "../types/FindHit";
  import Art from "./Art.svelte";
  import Icon from "./Icon.svelte";
  import Plat from "./Plat.svelte";

  type Hit = { kind: string; key: string; name: string; sub: string; image?: string | null; icon?: IconName; price?: number | null };
  let query = $state("");
  let sel = $state(0);
  let found = $state<Hit[]>([]);
  let searching = $state(false);
  let lookupError = $state("");
  let input: HTMLInputElement;
  let list: HTMLUListElement;

  const pageHits = $derived.by((): Hit[] => {
    const q = query.trim().toLowerCase();
    return PAGES.filter((p) => !q || p.title.toLowerCase().includes(q)).map((p) => ({ kind: "page", key: p.id, name: p.title, icon: p.icon, sub: "Go to page" }));
  });
  const results = $derived([...found, ...pageHits]);

  $effect(() => {
    input.focus();
  });
  // Searching as one types; an older search's answer never replaces a newer one's.
  $effect(() => {
    const q = query.trim();
    found = [];
    lookupError = "";
    sel = 0;
    // A local, not `searching`: an effect that read what it writes would start over each time.
    const wanted = q.length >= 2;
    searching = wanted;
    if (!wanted) return;
    const ctl = new AbortController();
    const timer = setTimeout(async () => {
      try {
        const r = await api<FindHit[]>("find?q=" + encodeURIComponent(q), undefined, ctl.signal);
        if (ctl.signal.aborted) return;
        if (isLoading(r)) throw new Error((r as { error?: string }).error || "Game data is still loading.");
        found = r;
      } catch (e) {
        if (!ctl.signal.aborted) lookupError = `Search unavailable: ${(e as Error).message}`;
      } finally {
        if (!ctl.signal.aborted) searching = false;
      }
    }, 120);
    return () => {
      ctl.abort();
      clearTimeout(timer);
    };
  });
  $effect(() => {
    void sel;
    list?.querySelector("li.sel")?.scrollIntoView({ block: "nearest" });
  });

  function choose(r: Hit) {
    ui.palette = false;
    if (r.kind === "page") location.hash = "#" + r.key;
    else openDetails(r.key);
  }
  function keydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") { e.preventDefault(); sel = Math.max(0, Math.min(sel + 1, results.length - 1)); }
    else if (e.key === "ArrowUp") { e.preventDefault(); sel = Math.max(sel - 1, 0); }
    else if (e.key === "Enter" && results[sel]) { e.preventDefault(); choose(results[sel]); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); ui.palette = false; }
  }
</script>

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="palette" onclick={(e) => { if (e.target === e.currentTarget) ui.palette = false; }}>
  <div class="palette-box" role="dialog" aria-label="Search">
    <div class="palette-input"><Icon name="search" />
      <input type="search" placeholder="Search items, parts, relics, syndicates, or pages…" autocomplete="off" spellcheck="false" bind:this={input} bind:value={query} onkeydown={keydown} />
      <kbd>Esc</kbd></div>
    <ul class="palette-results" role="listbox" bind:this={list}>
      {#each results as r, i (r.kind + r.key)}
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li class:sel={i === sel} role="option" aria-selected={i === sel} onclick={() => choose(r)}>
          {#if r.kind === "page"}<div class="art sm" style="display:grid;place-items:center"><Icon name={r.icon ?? "next"} /></div>{:else}<Art src={r.image} cls="sm" />{/if}
          <div class="main"><div class="t">{r.name}</div>{#if r.sub}<div class="s">{r.sub}</div>{/if}</div>
          {#if r.price != null}<Plat n={r.price} />{/if}<span class="kind">{r.kind}</span></li>
      {:else}
        <li class="muted">{searching ? "Searching…" : "Nothing matches in the loaded data."}</li>
      {/each}
      {#if lookupError}<li class="muted">{lookupError}</li>{/if}
      {#if query.trim().length >= 2}<li><a href="https://wiki.warframe.com/w/Special:Search?search={encodeURIComponent(query.trim())}" target="_blank" rel="noopener">Search the Warframe Wiki for {query.trim()}</a></li>{/if}
    </ul>
    <div class="palette-foot"><span><kbd>↑</kbd><kbd>↓</kbd> navigate</span><span><kbd>↵</kbd> open</span><span><kbd>/</kbd> anywhere to search</span></div>
  </div>
</div>
