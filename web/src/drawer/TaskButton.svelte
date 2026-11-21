<!-- Puts something on the farming board, or takes it off. -->
<script lang="ts">
  import Icon from "../components/Icon.svelte";
  import { act } from "../lib/api";
  import { changed } from "../lib/app.svelte";
  let { uid, onBoard, ondone }: { uid: string; onBoard: boolean; ondone?: (on: boolean) => void } = $props();
  let busy = $state(false);
  async function click() {
    busy = true;
    const on = !onBoard;
    if (await act("manual/task", { uid, onBoard: on }, on ? "Added to the farming board" : "Taken off the farming board")) {
      await changed();
      ondone?.(on);
    }
    busy = false;
  }
</script>

<button disabled={busy} onclick={click}><Icon name={onBoard ? "x" : "plus"} /> {onBoard ? "Remove from farming" : "Add to farming"}</button>
