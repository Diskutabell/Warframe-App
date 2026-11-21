<!-- Market: what your spare parts are worth, and a price check for anything on warframe.market. -->
<script lang="ts" module>
  const st = $state({ sort: "plat" as "plat" | "ducats" | "ratio", q: "" });
</script>

<script lang="ts">
  import Art from "../components/Art.svelte";
  import Duc from "../components/Duc.svelte";
  import Page from "../components/Page.svelte";
  import Plat from "../components/Plat.svelte";
  import Seg from "../components/Seg.svelte";
  import { api } from "../lib/api";
  import { openDetails } from "../lib/app.svelte";
  import { fmt, wiki } from "../lib/format";
  import { loader } from "../lib/loader.svelte";
  import type { MarketHit } from "../types/MarketHit";
  import type { RivenResult } from "../types/RivenResult";
  import type { Trade } from "../types/Trade";
  import type { TradeRow } from "../types/TradeRow";

  const page = loader<Trade>("trade", ["catalog", "inventory", "prices"]);
  // The last riven, and its listings once they arrive.
  const riven = loader<RivenResult | null>("riven/result", ["riven"]);
  const SORTS = [["plat", "Platinum"], ["ducats", "Ducats"], ["ratio", "Ducats per plat"]] as const;
  const KEY: Record<typeof st.sort, (r: TradeRow) => number> = {
    plat: (r) => (r.price || 0) * r.spare,
    ducats: (r) => (r.ducats || 0) * r.spare,
    ratio: (r) => r.ducatsPerPlat || 0,
  };
  const gradeClass = (g: string | null | undefined) => (g?.[0] === "A" ? "a" : g?.[0] === "B" ? "b" : "c");

  // The price check: prices arrive in the background, so ask again until every row has one (or
  // a few tries passed). A newer query, or leaving the page, ends the older one's tries.
  let found = $state<MarketHit[] | null>(null);
  let checkError = $state<string | null>(null);
  $effect(() => {
    const q = st.q.trim();
    found = null;
    checkError = null;
    if (q.length < 2) return;
    const ctl = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const ask = async (tries: number) => {
      try {
        const res = await api<MarketHit[]>("search?q=" + encodeURIComponent(q), undefined, ctl.signal);
        if (ctl.signal.aborted) return;
        found = res;
        if (res.some((r) => r.sell == null) && tries < 6) timer = setTimeout(() => ask(tries + 1), 1500);
      } catch (e) {
        if (!ctl.signal.aborted) checkError = (e as Error).message;
      }
    };
    timer = setTimeout(() => ask(0), 250);
    return () => {
      ctl.abort();
      clearTimeout(timer);
    };
  });
</script>

<Page title="Market" {page}>
  {#snippet children(d)}
    {@const rows = [...d.rows].sort((a, b) => KEY[st.sort](b) - KEY[st.sort](a))}
    {@const r = riven.data}
    <div class="page">
      <div class="page-head"><div><h1>Market</h1><p>Prices are the lowest sell and highest buy orders from players online on warframe.market.</p></div></div>
      <div class="grid cols-2">
        <div class="card"><div class="card-head"><h2>Spare prime parts</h2><span class="grow"></span><Seg options={SORTS} bind:value={st.sort} label="Sort" /></div>
          <p class="muted" style="margin:-4px 0 12px;font-size:13.5px">Parts you own beyond what your unfinished sets still need{d.source === "manual" ? " - counted from what you entered by hand; an inventory gives complete numbers" : ""}.</p>
          {#if rows.length}
            <div class="tiles" style="margin-bottom:12px">
              <div class="tile"><b>{fmt(rows.reduce((a, x) => a + x.spare, 0))}</b><span>spare parts</span></div>
              <div class="tile"><b><Plat n={rows.reduce((a, x) => a + (x.price || 0) * x.spare, 0)} /></b><span>at the lowest sell price</span></div>
              <div class="tile"><b>{#if rows.some((x) => x.ducats)}<Duc n={rows.reduce((a, x) => a + (x.ducats || 0) * x.spare, 0)} />{:else}–{/if}</b><span>if all go to Baro</span></div>
            </div>
            <div class="table-wrap" style="max-height:calc(100vh - 330px)"><table class="table">
              <thead><tr><th>Part</th><th class="r">Spare</th><th class="r">Each</th><th class="r">Ducats</th><th class="r" title="Ducats per platinum: a high value is worth more to Baro than to other players">Ducats/p</th><th class="r">Total</th></tr></thead>
              <tbody>
                {#each rows as x (x.uid)}
                  <tr class="click" onclick={() => openDetails(x.uid)}>
                    <td><div class="row"><Art src={x.image} cls="sm" /><span class="ellipsis" style="font-weight:600">{x.name}</span>{#if x.vaulted}<span class="pill vaulted">vaulted</span>{/if}</div></td>
                    <td class="r num">{x.spare}<span class="faint"> / {x.have}</span></td>
                    <td class="r"><Plat n={x.price} /></td><td class="r">{#if x.ducats}<Duc n={x.ducats} />{:else}<span class="faint">–</span>{/if}</td>
                    <td class="r num">{#if x.ducatsPerPlat != null}{x.ducatsPerPlat}{:else}<span class="faint">–</span>{/if}</td>
                    <td class="r"><Plat n={x.price != null ? x.price * x.spare : null} /></td></tr>
                {/each}
              </tbody></table></div>
          {:else}
            <div class="empty"><b>No spare parts</b>This needs part counts: an inventory (Settings), or counts set by hand on an item's details.</div>
          {/if}
        </div>
        <div class="card"><div class="card-head"><h2>Price check</h2></div>
          <input type="search" placeholder="Any tradable item, e.g. Saryn Prime Set" aria-label="Item name" style="width:100%;margin-bottom:12px" bind:value={st.q} />
          <div class="table-wrap" style="max-height:calc(100vh - 290px)"><table class="table">
            <thead><tr><th>Item</th><th class="r">Sell</th><th class="r">Buy</th><th class="r">Ducats</th></tr></thead>
            <tbody>
              {#if st.q.trim().length < 2}<tr><td colspan="4" class="muted">Type two letters or more.</td></tr>
              {:else if checkError}<tr><td colspan="4" class="warn-text">{checkError}</td></tr>
              {:else if found}
                {#each found as h (h.slug)}
                  <tr>
                    <td><div class="row"><Art src={h.image} cls="sm" /><div><a href="https://warframe.market/items/{h.slug}" target="_blank" rel="noopener">{h.name}</a>
                      <a class="muted" href={wiki(h.name)} target="_blank" rel="noopener" title="Find {h.name} on the Warframe Wiki"> · sources</a></div></div></td>
                    <td class="r">{#if h.sell != null}<Plat n={h.sell} />{:else}<span class="faint">…</span>{/if}</td>
                    <td class="r">{#if h.buy != null}<Plat n={h.buy} />{:else}<span class="faint">…</span>{/if}</td>
                    <td class="r"><Duc n={h.ducats} /></td></tr>
                {:else}<tr><td colspan="4" class="muted">Nothing on warframe.market matches.</td></tr>{/each}
              {/if}
            </tbody></table></div>
        </div>
      </div>
      <div class="card" style="margin-top:16px"><div class="card-head"><h2>Last riven</h2></div>
        {#if !r || (!r.weapon && !r.error)}
          <div class="empty"><b>No riven read yet</b>Open a riven in the game so its card shows (cycling it, or its own screen) and press the riven hotkey (F10 unless you changed it in Settings).</div>
        {:else if r.error}
          <div class="callout">{r.error}</div>
        {:else}
          {@const m = r.market}
          {@const med = (k: "unrolled" | "rerolled") => { const v = r.traded?.[k]?.median; return v == null ? null : Math.round(Number(v)); }}
          <div class="riv-head"><Art src={r.image} cls="lg" /><div class="grow"><div class="riv-weapon">{r.weapon}</div>
            <div class="muted">{r.positives} positives{r.negatives ? " + 1 negative" : ""}{r.unread ? ` · ${r.unread} not readable` : ""} · {new Date(r.t * 1000).toLocaleString()}</div></div>
            {#if r.overall}<span class="riv-overall {gradeClass(r.overall)}" title="The average roll">{r.overall}</span>{/if}</div>
          <div class="riv-stats">
            {#each r.stats as g}
              <div class="riv-stat"><span class="riv-grade {gradeClass(g.grade)}">{g.grade}</span>
                <span class="grow"><b>{g.printed}</b> {g.stat}</span><span class="muted num">{g.deviation > 0 ? "+" : ""}{g.deviation.toFixed(1)}%</span>
                <div class="riv-roll {gradeClass(g.grade)}"><i style:width="{Math.max(0, Math.min(100, (g.deviation + 10) * 5))}%"></i><b></b></div></div>
            {/each}
          </div>
          <div class="tiles" style="margin-top:12px">
            <div class="tile"><b>{#if m?.count}<Plat n={m.lowest} />{:else}–{/if}</b><span>{m ? `lowest of ${fmt(m.count)} listed with these positives` : r.marketError ? "warframe.market did not answer" : "looking up listings…"}</span></div>
            <div class="tile"><b>{#if m?.median != null}<Plat n={m.median} />{:else}–{/if}</b><span>median listing</span></div>
            <div class="tile"><b>{#if med("rerolled") != null}<Plat n={med("rerolled")} />{:else}–{/if}</b><span>sold lately, rerolled</span></div>
            <div class="tile"><b>{#if med("unrolled") != null}<Plat n={med("unrolled")} />{:else}–{/if}</b><span>sold lately, unrolled</span></div>
          </div>
          {#if m?.url}<p style="margin:10px 0 0"><a href={m.url} target="_blank" rel="noopener">See these listings on warframe.market</a></p>{/if}
        {/if}
      </div>
    </div>
  {/snippet}
</Page>
