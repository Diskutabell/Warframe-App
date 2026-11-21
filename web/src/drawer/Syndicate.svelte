<!-- A syndicate: its rank, what its standing buys, and what that is worth. -->
<script lang="ts">
  import Art from "../components/Art.svelte";
  import Img from "../components/Img.svelte";
  import Plat from "../components/Plat.svelte";
  import { api } from "../lib/api";
  import { fmt } from "../lib/format";
  import { loader } from "../lib/loader.svelte";
  import type { OfferRow } from "../types/OfferRow";
  import type { SyndicateDetail } from "../types/SyndicateDetail";
  import Loading from "./Loading.svelte";
  import Top from "./Top.svelte";

  let { tag }: { tag: string } = $props();
  const x = loader<SyndicateDetail>((signal) => api("syndicate?tag=" + encodeURIComponent(tag), undefined, signal), ["inventory", "prices"]);
  // Prices arrive one by one from the market queue: look again while some are missing, for two
  // minutes at most.
  let tries = 0;
  $effect(() => {
    if (!x.data?.pricing || tries >= 30) return;
    const t = setTimeout(() => { tries++; x.refresh(); }, 4000);
    return () => clearTimeout(t);
  });
</script>

{#snippet row(o: OfferRow, title: number)}
  <div class="list-row" class:dim={!o.available}><Art src={o.image} cls="sm" />
    <div class="main"><div class="t">{o.name}</div><div class="s">{fmt(o.standing)} standing{o.level > title ? ` · needs rank ${o.level}` : ""}</div></div>
    <div style="text-align:right">{#if o.slug}<Plat n={o.price} />{:else}<span class="faint">not tradable</span>{/if}
      {#if o.perK != null}<div class="faint num" style="font-size:12.5px">{o.perK}p / 1k</div>{/if}</div></div>
{/snippet}

{#if !x.data}
  <Loading error={x.error ?? (x.downloading ? "Downloading game data…" : null)} />
{:else}
  {@const s = x.data}
  {@const priced = s.offers.filter((o) => o.available && o.perK != null)}
  {@const tradable = s.offers.filter((o) => o.slug)}
  {@const other = s.offers.filter((o) => !o.slug)}
  <Top />
  <div class="dhead"><Img src={s.rankIcon || s.icon} px={96} cls="gi" /><div class="grow">
    <h2>{s.name}</h2>
    <div class="row wrap"><span class="pill" style={s.colour ? `color:color-mix(in srgb, ${s.colour} 65%, #fff 35%);border-color:color-mix(in srgb, ${s.colour} 45%, transparent)` : null}>{s.rankName}</span>
      {#if s.atCap}<span class="pill gold">Rank up ready</span>{/if}{#if s.full}<span class="pill warn">Full - spend it</span>{/if}</div></div></div>
  <div class="kv">
    <div><span>Standing</span><b>{fmt(s.standing)}</b></div>
    <div><span>This rank</span><b>{s.steps ? `${s.progress} / ${s.size}` : s.size ? `${fmt(s.progress)} / ${fmt(s.size)}` : "–"}</b></div>
    <div><span>Best value</span><b>{priced.length ? `${priced[0].perK}p / 1k` : "–"}</b></div>
  </div>
  {#if s.allies.length}
    <div class="dsec"><h2>Standing earned here also changes</h2><div class="chips">
      {#each s.allies as a}<span class="chip res" class:owned={a.share > 0}>{a.share > 0 ? "+" : "−"}{Math.abs(Math.round(a.share * 100))}% {a.name}</span>{/each}
    </div></div>
  {/if}
  <div class="dsec"><h2>What standing buys <span class="count">{tradable.length} tradable</span>{#if s.pricing}<span class="muted" style="letter-spacing:0;text-transform:none;font-weight:400"> pricing {s.pricing}…</span>{/if}</h2>
    <p class="muted" style="margin:-4px 0 8px;font-size:13px">What you can buy at your rank first, by platinum per 1,000 standing (the lowest sell order on warframe.market).</p>
    <div class="list">{#each tradable as o}{@render row(o, s.title)}{:else}<div class="muted">Nothing this syndicate offers sells on warframe.market.</div>{/each}</div>
    {#if other.length}<details class="more"><summary>{other.length} more that cannot be traded</summary><div class="list">{#each other as o}{@render row(o, s.title)}{/each}</div></details>{/if}
  </div>
  {#if s.ranks.some((r) => r.level > 0)}
    <div class="dsec"><h2>Ranks</h2><div class="list">
      {#each s.ranks.filter((r) => r.level > 0) as r}<div class="list-row" class:here={r.level === s.title}><Img src={r.icon} px={28} cls="gi" /><div class="main"><div class="t">{r.name}</div></div><span class="faint num">rank {r.level}</span></div>{/each}
    </div></div>
  {/if}
{/if}
