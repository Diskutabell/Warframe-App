<!-- An open-world cycle as a tile over a picture of the place. -->
<script lang="ts">
  import Countdown from "./Countdown.svelte";
  import GameImg from "./GameImg.svelte";
  import { DARK, ICON, factionIcon, placeScene, tex } from "../lib/game";
  import type { Cycle } from "../types/Cycle";
  let { c, withPlace = true }: { c: Cycle; withPlace?: boolean } = $props();
  const phase = $derived(String(c.state || "").toLowerCase());
  const scene = $derived(placeScene(c.name));
  let sceneGone = $state(false);
  $effect(() => {
    void scene;
    sceneGone = false;
  });
</script>

<div class="cycle scenic" class:dim={DARK.has(phase)}>
  {#if scene && !sceneGone}<img class="bg" alt="" loading="lazy" src={tex(scene, 256)} onerror={() => (sceneGone = true)} />{/if}
  <div class="shade"></div>
  {#if c.name === "Zariman"}<GameImg path={factionIcon(phase)} px={26} cls="state" />
  {:else if c.name !== "Duviri"}<GameImg path={DARK.has(phase) ? ICON.moon : ICON.sun} px={26} cls="state" />{/if}
  <div class="n">{c.name}</div>
  <div class="s">{c.state}</div>
  <div class="t"><Countdown until={c.expiry} done="changing" />{#if withPlace && c.place}<span class="faint"> · {c.place}</span>{/if}</div>
</div>
