<!-- One pip per part needed, filled when held, with the part's picture. -->
<script lang="ts">
  import Img from "./Img.svelte";
  let { parts }: { parts: { name: string; need: number; have: number; image: string | null }[] } = $props();
</script>

{#each parts as p}
  {#each Array.from({ length: p.need }, (_, i) => i) as i}
    <span class="pip" class:ok={i < p.have} title="{p.name} - own {p.have} of {p.need}"><Img src={p.image} px={24} />{#if i === 0 && p.have > p.need}<b>{p.have}</b>{/if}</span>
  {/each}
{/each}
