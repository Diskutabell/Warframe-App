<!-- A section of a page: its heading, anything beside it, and where it leads (the page's name
     and the key that opens it). -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { navKey, navTitle } from "../lib/nav";
  let { title, link = null, head, children }: { title: string; link?: string | null; head?: Snippet; children: Snippet } = $props();
  const key = $derived(link ? navKey(link) : "");
</script>

<section class="card">
  <div class="card-head"><h2>{title}</h2><span class="grow"></span>
    {@render head?.()}
    {#if link}<a class="card-link" href="#{link}">{navTitle(link)}{#if key}<kbd>{key}</kbd>{/if}</a>{/if}
  </div>
  <div>{@render children()}</div>
</section>
