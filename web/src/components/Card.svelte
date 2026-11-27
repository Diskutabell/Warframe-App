<!-- A section of a page: its heading, anything beside it, and the page it leads to. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import { navTitle } from "../lib/nav";
  import Icon from "./Icon.svelte";
  let { title, link = null, head, children }: { title: string; link?: string | null; head?: Snippet; children: Snippet } = $props();
</script>

<section class="card">
  <div class="card-head"><h2>{title}</h2><span class="grow"></span>
    {@render head?.()}
    {#if link}<a class="card-link" href="#{link}">{navTitle(link)}<Icon name="next" /></a>{/if}
  </div>
  <div>{@render children()}</div>
</section>
