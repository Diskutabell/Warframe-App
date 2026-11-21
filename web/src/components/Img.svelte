<!-- A picture from the art cache that removes itself when it cannot be loaded. -->
<script lang="ts">
  import { artUrl } from "../lib/format";
  let { src, px, cls = "", title = "", alt = "" }: { src: string | null | undefined; px: number; cls?: string; title?: string; alt?: string } = $props();
  let gone = $state(false);
</script>

{#if src && !gone}
  <img class={cls} {alt} title={title || null} width={px} height={px} loading="lazy" src={artUrl(src, px)} onerror={() => (gone = true)} />
{/if}
