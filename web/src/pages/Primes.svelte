<!-- Prime sets: each set, its parts, and how close it is. -->
<script lang="ts" module>
  const st = $state({ q: "", show: "started" as "started" | "complete" | "missing" | "", unvaulted: false, unmastered: false });
</script>

<script lang="ts">
  import Art from "../components/Art.svelte";
  import Page from "../components/Page.svelte";
  import PartPips from "../components/PartPips.svelte";
  import Plat from "../components/Plat.svelte";
  import Seg from "../components/Seg.svelte";
  import StatusPill from "../components/StatusPill.svelte";
  import { openDetails } from "../lib/app.svelte";
  import { fmt } from "../lib/format";
  import { loader } from "../lib/loader.svelte";
  import type { PrimeRow } from "../types/PrimeRow";

  const page = loader<PrimeRow[]>("primes", ["inventory", "catalog", "prices"]);
  const SHOW = [["started", "In progress"], ["complete", "Complete"], ["missing", "Not started"], ["", "All"]] as const;
  const rows = $derived.by(() => {
    const q = st.q.toLowerCase();
    return (page.data ?? []).filter((r) => (!q || r.name.toLowerCase().includes(q)) && (!st.unvaulted || !r.vaulted) &&
      (!st.unmastered || r.status !== "mastered") &&
      (st.show === "" || (st.show === "complete" && r.complete) || (st.show === "started" && !r.complete && r.have > 0) || (st.show === "missing" && r.have === 0)));
  });
</script>

<Page title="Prime sets" {page}>
  {#snippet children(data)}
    <div class="page">
      <div class="page-head"><div><h1>Prime sets</h1>
        <p>{data.filter((r) => r.complete).length} complete · {data.filter((r) => !r.complete && r.have > 0).length} in progress · {data.length} in total</p></div></div>
      <div class="toolbar">
        <input type="search" placeholder="Search sets" aria-label="Search sets" bind:value={st.q} />
        <Seg options={SHOW} bind:value={st.show} label="Show" />
        <label class="check"><input type="checkbox" bind:checked={st.unvaulted} />Not vaulted</label>
        <label class="check"><input type="checkbox" bind:checked={st.unmastered} />Not mastered</label>
        <span class="count">{fmt(rows.length)} sets</span>
      </div>
      <div class="cards">
        {#each rows as r (r.uid)}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="icard" class:done={r.complete} onclick={() => openDetails(r.uid)}>
            <Art src={r.image} cls="lg" />
            <div class="body">
              <div class="row"><span class="name grow">{r.name}</span>{#if r.setPrice != null}<Plat n={r.setPrice} />{/if}</div>
              <div class="pips"><PartPips parts={r.parts} /></div>
              <div class="meta"><span>{r.have}/{r.need} parts</span><StatusPill status={r.status} />
                {#if r.owned}<span class="pill good">owned</span>{/if}{#if r.vaulted}<span class="pill vaulted">vaulted</span>{/if}</div>
            </div>
          </div>
        {:else}
          <div class="empty"><b>No sets match</b>Try another filter.</div>
        {/each}
      </div>
    </div>
  {/snippet}
</Page>
