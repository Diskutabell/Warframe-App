<script lang="ts">
  import { fmt } from "../lib/format";
  import type { Recipe } from "../types/Recipe";
  import ResourceRows from "./ResourceRows.svelte";
  let { recipe }: { recipe: Recipe } = $props();
  const costs = $derived([
    recipe.credits != null ? `${fmt(recipe.credits)} credits` : "",
    recipe.seconds != null ? `${fmt(recipe.seconds / 3600)} hours` : "",
    recipe.quantity !== 1 ? `makes ${fmt(recipe.quantity)}` : "",
  ].filter(Boolean).join(", "));
</script>

<div class="dsec"><h3>Component recipe (per build)</h3>
  {#if costs}<p class="muted">{costs}</p>{/if}
  {#if recipe.ingredients.length}<div class="list"><ResourceRows resources={recipe.ingredients} /></div>{:else}<p class="muted">No ingredients listed.</p>{/if}
</div>
