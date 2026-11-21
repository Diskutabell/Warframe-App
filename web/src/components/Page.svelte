<!-- A page's frame: grey bars until its data is there, "downloading" while the game data
     downloads, the error when it failed, else the page itself (with the error above it when a
     refresh failed). -->
<script lang="ts" generics="T">
  import type { Snippet } from "svelte";
  import type { Loaded } from "../lib/loader.svelte";
  import Skeleton from "./Skeleton.svelte";
  let { title, page, children }: { title: string; page: Loaded<T>; children: Snippet<[T]> } = $props();
</script>

{#if page.data != null}
  <!-- A refresh that failed: what is shown is older than it looks. -->
  {#if page.error}<div class="page"><div class="callout">Couldn't refresh: {page.error}</div></div>{/if}
  {@render children(page.data)}
{:else if page.downloading}
  <div class="page"><div class="page-head"><div><h1>{title}</h1></div></div>
    <div class="empty"><b>Downloading game data…</b>{page.downloading === true ? "" : page.downloading}</div></div>
{:else if page.error}
  <div class="page"><div class="page-head"><div><h1>{title}</h1></div></div><div class="callout">{page.error}</div></div>
{:else}
  <Skeleton {title} />
{/if}
