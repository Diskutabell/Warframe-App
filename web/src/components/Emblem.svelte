<!-- A game texture in a framed square, for the head of a card or a list row. -->
<script lang="ts">
  import GameImg from "./GameImg.svelte";
  let { path, cls = "", title = "" }: { path: string | null | undefined; cls?: string; title?: string } = $props();
</script>

<span class="emblem {cls}" title={title || null}><GameImg {path} px={cls.includes("sm") ? 22 : 36} /></span>
