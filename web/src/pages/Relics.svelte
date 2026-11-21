<!-- Relics: what you hold, what each is worth, and which can be cracked right now. -->
<script lang="ts" module>
  const st = $state({ owned: true, era: "", q: "", sort: "ev" as "ev" | "need" | "name", unvaulted: false });
</script>

<script lang="ts">
  import Art from "../components/Art.svelte";
  import Era from "../components/Era.svelte";
  import Icon from "../components/Icon.svelte";
  import Img from "../components/Img.svelte";
  import Page from "../components/Page.svelte";
  import Plat from "../components/Plat.svelte";
  import Seg from "../components/Seg.svelte";
  import { api } from "../lib/api";
  import { openDetails } from "../lib/app.svelte";
  import { fmt } from "../lib/format";
  import { loader } from "../lib/loader.svelte";
  import type { Fissures } from "../types/Fissures";
  import type { RelicView } from "../types/RelicView";

  // "Only mine" asks the engine for the relics held only.
  const page = loader<RelicView[]>((signal) => api("relics" + (st.owned ? "?owned=1" : ""), undefined, signal), ["inventory", "catalog", "prices"]);
  const fissures = loader<Fissures>("fissures", ["world"]);
  const ERAS = [["", "All"], ["Lith", "Lith"], ["Meso", "Meso"], ["Neo", "Neo"], ["Axi", "Axi"], ["Requiem", "Requiem"]] as const;
  const SORTS = [["ev", "Value"], ["need", "Needed drops"], ["name", "Name"]] as const;

  const open = $derived(new Set((fissures.data?.fissures ?? []).filter((f) => !f.storm).map((f) => f.tier)));
  // Vaulted relics you hold crack like any other.
  const isOpen = (r: RelicView) => (!r.vaulted || r.total > 0) && (open.has(r.era) || open.has("Omnia"));
  const rows = $derived.by(() => {
    const q = st.q.toLowerCase();
    let out = (page.data ?? []).filter((r) => (!st.era || r.era === st.era) && (!st.unvaulted || !r.vaulted) &&
      (!q || (r.name + " " + r.rewards.map((x) => x.name).join(" ")).toLowerCase().includes(q)));
    if (st.sort === "ev") out = [...out].sort((a, b) => (b.ev.Radiant ?? 0) - (a.ev.Radiant ?? 0));
    if (st.sort === "need") out = [...out].sort((a, b) => b.needsCount - a.needsCount || (b.ev.Radiant ?? 0) - (a.ev.Radiant ?? 0));
    return out;
  });
</script>

<Page title="Relics" {page}>
  {#snippet children(data)}
    <div class="page">
      <div class="page-head"><div><h1>Relics</h1>
        <p>{st.owned ? `${fmt(data.reduce((a, r) => a + r.total, 0))} relics of ${fmt(data.length)} kinds` : `${fmt(data.length)} relics`} · value is the chance-weighted platinum of the drops</p></div></div>
      <div class="toolbar">
        <input type="search" placeholder="Relic or drop" aria-label="Search relics" bind:value={st.q} />
        <Seg options={ERAS} bind:value={st.era} label="Era" />
        <Seg options={SORTS} bind:value={st.sort} label="Sort" />
        <label class="check"><input type="checkbox" bind:checked={st.owned} onchange={() => page.refresh()} />Only mine</label>
        <label class="check"><input type="checkbox" bind:checked={st.unvaulted} />Not vaulted</label>
        <span class="count">{fmt(rows.length)} relics</span>
      </div>
      <div class="cards">
        {#each rows.slice(0, 400) as r (r.name)}
          {@const owned = Object.entries(r.counts).filter(([, n]) => n).map(([k, n]) => `${n}× ${k}`).join(", ")}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="icard" onclick={() => openDetails(r.name)}>
            <div class="relic-art"><Art src={r.image} cls="lg" />{#if r.total}<span class="relic-qty num">{r.total}</span>{/if}</div>
            <div class="body">
              <div class="row"><span class="name grow">{r.name}</span><Era era={r.era} /></div>
              <div class="pips">
                {#each r.rewards as x}
                  <span class="pip" class:ok={x.have > 0} class:want={x.needed > x.have}
                    title="{x.name} - {x.rarity || ''} · {x.price != null ? x.price + 'p' : 'no price'} · own {x.have}{x.needed > x.have ? ' · needed' : ''}"><Img src={x.image} px={24} /></span>
                {/each}
              </div>
              <div class="meta"><span><Plat n={r.ev.Radiant} /> radiant · <Plat n={r.ev.Intact} /> intact</span>
                {#if r.needsCount}<span class="pill lotus">{r.needsCount} needed</span>{/if}
                {#if isOpen(r)}<span class="pill lotus"><Icon name="fissures" /> open now</span>{/if}
                {#if r.vaulted}<span class="pill vaulted">vaulted</span>{/if}</div>
              {#if owned}<div class="meta">{owned}</div>{/if}
            </div>
          </div>
        {:else}
          <div class="empty"><b>{st.owned ? "No relics tracked" : "No relics match"}</b>{st.owned ? "Import an inventory or set counts in a relic's details, or untick “Only mine”." : "Try another filter."}</div>
        {/each}
      </div>
    </div>
  {/snippet}
</Page>
