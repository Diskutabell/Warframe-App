<!-- The details drawer: one item, part, relic, task or syndicate, from anywhere in the app. Each
     key opens on top of the last; Back goes down the stack. -->
<script lang="ts">
  import { closeDetails, ui } from "../lib/app.svelte";
  import Details from "./Details.svelte";
  import Syndicate from "./Syndicate.svelte";
  import Task from "./Task.svelte";
  const key = $derived(ui.drawer.at(-1) ?? null);
</script>

{#if key}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="drawer-scrim" onclick={closeDetails}></div>
  <aside class="drawer" aria-label="Details">
    {#key key}
      {#if key.startsWith("syndicate:")}<Syndicate tag={key.slice("syndicate:".length)} />
      {:else if key.startsWith("task:")}<Task uid={key.slice("task:".length)} />
      {:else}<Details {key} />{/if}
    {/key}
  </aside>
{/if}
