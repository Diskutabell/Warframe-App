<!-- A texture from the game's interface (lib/game.ts), `px` by `px`. -->
<script lang="ts">
  import { tex } from "../lib/game";
  let { path, px, cls = "", title = "" }: { path: string | null | undefined; px: number; cls?: string; title?: string } = $props();
  let gone = $state(false);
  $effect(() => {
    void path;
    gone = false;
  });
</script>

{#if path && !gone}
  <img class="gi {cls}" alt="" title={title || null} width={px} height={px} loading="lazy" src={tex(path, px)} onerror={() => (gone = true)} />
{/if}
