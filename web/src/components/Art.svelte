<!-- An item picture, from the engine's art cache at the size of its slot. A dropped request is
     tried twice more; a picture the source does not have leaves the slot empty. -->
<script lang="ts">
  import { artUrl } from "../lib/format";
  let { src, cls = "" }: { src: string | null | undefined; cls?: "" | "sm" | "lg" | "xl" | string } = $props();
  // The picture area of each .art size in app.css: the box minus its 1 px border.
  const PX: Record<string, number> = { sm: 30, lg: 70, xl: 102 };
  let attempt = $state(0);
  let hidden = $state(false);
  const url = $derived(src ? artUrl(src, PX[cls] || 42) + (attempt ? `&_r=${attempt}` : "") : "");
  function failed() {
    if (attempt >= 2) hidden = true;
    else setTimeout(() => attempt++, 400 * (attempt + 1));
  }
</script>

{#if src}
  <img
    class="art {cls}"
    loading="lazy"
    decoding="async"
    alt=""
    src={url}
    style:visibility={hidden ? "hidden" : null}
    onerror={failed}
    onload={(e) => { if (!(e.currentTarget as HTMLImageElement).naturalWidth) hidden = true; }}
  />
{:else}
  <div class="art {cls}"></div>
{/if}
