<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon from "./Icon.svelte";
  let { title, link = null, head, children }: { title: string; link?: string | null; head?: Snippet; children: Snippet } = $props();
</script>

<div class="card">
  <div class="card-head"><h2>{title}</h2><span class="grow"></span>
    {@render head?.()}
    {#if link}<a class="card-link" href="#{link}">Open <Icon name="next" /></a>{/if}
  </div>
  <div>{@render children()}</div>
</div>
