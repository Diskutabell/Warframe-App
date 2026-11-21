<!-- A newer release is out: its notes, once per window. -->
<script lang="ts">
  import Icon from "./Icon.svelte";
  import Modal from "./Modal.svelte";
  import type { Release } from "../types/Release";
  let { release, onclose }: { release: Release; onclose: () => void } = $props();

  /** The notes are the changelog's Markdown: headings, bullets (their wrapped lines joined back),
   *  bold and code; everything else plain text. Parsed into blocks, never into HTML. */
  type Block = { kind: "h" | "li" | "p"; text: string };
  const blocks = $derived.by(() => {
    const out: (Block | null)[] = [];
    for (const line of String(release.notes || "").split(/\r?\n/)) {
      const last = out[out.length - 1];
      if (!line.trim()) out.push(null);
      else if (/^#{1,6}\s/.test(line)) out.push({ kind: "h", text: line.replace(/^#+\s*/, "") });
      else if (/^\s*[-*]\s/.test(line)) out.push({ kind: "li", text: line.replace(/^\s*[-*]\s+/, "") });
      else if (last) last.text += " " + line.trim();
      else out.push({ kind: "p", text: line.trim() });
    }
    // Consecutive bullets form one list.
    const groups: (Block | Block[])[] = [];
    for (const b of out.filter((b): b is Block => !!b)) {
      const prev = groups[groups.length - 1];
      if (b.kind === "li" && Array.isArray(prev)) prev.push(b);
      else groups.push(b.kind === "li" ? [b] : b);
    }
    return groups;
  });
  /** **bold** and `code` as pieces. */
  const inline = (t: string) => t.split(/(\*\*.+?\*\*|`[^`]+`)/).filter(Boolean).map((s) =>
    s.startsWith("**") ? { b: s.slice(2, -2) } : s.startsWith("`") ? { code: s.slice(1, -1) } : { text: s });
  const date = $derived(release.published ? new Date(release.published).toLocaleDateString([], { day: "numeric", month: "long", year: "numeric" }) : "");
</script>

{#snippet rich(t: string)}{#each inline(t) as p}{#if "b" in p}<b>{p.b}</b>{:else if "code" in p}<code>{p.code}</code>{:else}{p.text}{/if}{/each}{/snippet}

<Modal label="A new version is out" {onclose}>
  <div class="modal-head"><div class="grow"><h2>A new version is out</h2>
    <p class="muted">warframeproject {release.version}{date ? `, released ${date}` : ""}. You have {release.current}.</p></div>
    <button class="ghost icon" type="button" title="Close" aria-label="Close" onclick={onclose}><Icon name="x" /></button></div>
  {#if release.name}<div class="modal-title">{release.name}</div>{/if}
  {#if blocks.length}
    <div class="modal-notes">
      {#each blocks as b}
        {#if Array.isArray(b)}<ul>{#each b as li}<li>{@render rich(li.text)}</li>{/each}</ul>
        {:else if b.kind === "h"}<h3>{@render rich(b.text)}</h3>
        {:else}<p>{@render rich(b.text)}</p>{/if}
      {/each}
    </div>
  {/if}
  <div class="modal-foot"><button type="button" onclick={onclose}>Later</button>
    {#if release.url}<a class="btn primary" href={release.url} target="_blank" rel="noopener" onclick={onclose}><Icon name="link" /> Open the release page</a>{/if}</div>
</Modal>
