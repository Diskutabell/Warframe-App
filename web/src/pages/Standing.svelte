<!-- Standing: every syndicate's rank and what the next one costs, how much standing is left
     today, and the medallions waiting to be handed in. A syndicate opens in the side panel with
     what its standing buys. -->
<script lang="ts">
  import Countdown from "../components/Countdown.svelte";
  import Img from "../components/Img.svelte";
  import Page from "../components/Page.svelte";
  import { openDetails } from "../lib/app.svelte";
  import { ago, fmt } from "../lib/format";
  import { every, loader } from "../lib/loader.svelte";
  import type { Pool } from "../types/Pool";
  import type { StandingView } from "../types/StandingView";
  import type { SyndicateRow } from "../types/SyndicateRow";

  const page = loader<StandingView>("standing", ["inventory", "catalog"]);
  // The pools and ranks change with the next profile or memory read; check once a minute.
  every(() => page.refresh(), 60000);
  const SOURCE: Record<string, string> = { profile: "your profile", inventory: "the inventory read from the game" };

  /** A rank's range is drawn as a bar; the event syndicate's last rank spans 494,000 and is no cap. */
  const hasBar = (r: SyndicateRow) => !r.steps && r.size > 0 && r.size <= 200000;
  /** "18,143 / 22,000", or for a negative rank how deep into it the account is. */
  function amount(r: SyndicateRow) {
    if (r.steps) return `rank ${fmt(r.progress)} of ${fmt(r.size)}`;
    if (!r.size) return `${fmt(r.standing)} standing`;
    if (r.title < 0) return `${fmt(r.progress)} of −${fmt(r.size)}`;
    return `${fmt(r.progress)} / ${fmt(r.size)}`;
  }
  const share = (r: SyndicateRow) =>
    ((r.title < 0 ? Math.min(1, -r.progress / r.size) : Math.max(0, Math.min(1, r.progress / r.size))) * 100).toFixed(1) + "%";
  const poolOf = (r: SyndicateRow, pools: Pool[]) => pools.find((x) => x.key === r.pool && x.left != null);
</script>

{#snippet status(r: SyndicateRow)}
  {#if r.atCap}<span class="pill gold" title="This rank's standing is full: rank up to earn more">Rank up ready</span>
  {:else if r.full}<span class="pill warn" title="Top rank and full standing: anything earned now is lost unless you spend some">Full - spend it</span>
  {:else if r.title < 0}<span class="pill warn">Hostile</span>
  {:else if !r.known}<span class="pill">Not met yet</span>{/if}
{/snippet}

{#snippet bar(r: SyndicateRow)}
  {#if hasBar(r)}<div class="bar syn" class:neg={r.title < 0} style={r.colour ? `--syn:${r.colour}` : null}><i style:width={share(r)}></i></div>{/if}
{/snippet}

{#snippet card(r: SyndicateRow, pools: Pool[])}
  {@const p = poolOf(r, pools)}
  {@const m = r.medallions}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="syn-card" class:unmet={!r.known} style={r.colour ? `--syn:${r.colour}` : null} onclick={() => openDetails("syndicate:" + r.tag)}>
    <div class="syn-head"><span class="sigil"><Img src={r.rankIcon || r.icon} px={44} cls="gi" /></span>
      <div class="grow"><div class="name">{r.name}</div><div class="rank">{r.rankName}{#if r.topRank && !r.steps}<span class="faint"> · rank {r.title} of {r.topRank}</span>{/if}</div></div>
      {@render status(r)}</div>
    {@render bar(r)}
    <div class="row spread syn-nums"><span class="num">{amount(r)}</span>
      {#if p}<span class={p.left === 0 ? "warn-text" : "muted"} title="{p.name}: {fmt(p.left)} of {fmt(p.cap)} left today">{p.left === 0 ? "capped today" : `${fmt(p.left)} left today`}</span>{/if}</div>
    {#if r.next && !r.steps}
      <div class="next"><span class="sublabel">{r.atCap ? "Rank up to" : "Next"}: {r.next.name}</span>
        <div class="chips">
          {#if r.next.credits}<span class="chip res"><img alt="" width="20" height="20" src="icons/credits.png" />{fmt(r.next.credits)}</span>{/if}
          {#each r.next.items as i}<span class="chip res" class:owned={i.have >= i.count} title="You hold {fmt(i.have)}"><Img src={i.image} px={20} />{fmt(i.count)} × {i.name}</span>{/each}
          {#if !r.next.items.length && !r.next.credits}<span class="faint">no cost listed</span>{/if}
        </div></div>
    {/if}
    {#if m.standing}
      <div class="medals" title={m.items.map((i) => `${fmt(i.count)} ${i.name}`).join(", ")}>{fmt(m.standing)} standing in hand-ins you hold{m.capped ? " · counts toward the daily limit" : " · not limited daily"}</div>
    {/if}
  </div>
{/snippet}

<Page title="Standing" {page}>
  {#snippet children(s)}
    {@const factions = s.syndicates.filter((r) => r.kind === "faction")}
    {@const shared = s.pools.find((p) => p.key === "DailyAffiliation")}
    {@const ready = s.syndicates.filter((r) => r.atCap)}
    {@const full = s.syndicates.filter((r) => r.full)}
    <div class="page">
      <div class="page-head"><div><h1>Standing</h1><p>{s.account ? `${s.account} · ` : ""}ranks, rank-up costs, the daily limits and what standing buys</p></div></div>
      {#if !s.loaded}
        <div class="callout">The syndicate data has not been downloaded yet. It comes with the game data (Settings → Re-download game data).</div>
      {:else if !s.account}
        <div class="empty"><b>No account yet</b>Start the game once, or link your account in Settings.</div>
      {:else}
        <div class="card"><div class="card-head"><h2>Left today</h2><span class="grow"></span><span class="muted">resets in <Countdown until={s.reset} done="a moment" /></span></div>
          {#if s.cap}
            <p class="muted" style="margin:-4px 0 12px;font-size:13.5px">{fmt(s.cap)} standing a day in each pool at MR {s.masteryRank} (16,000 + 500 per rank).{#if s.asOf}{" "}As of {SOURCE[s.source ?? ""] || s.source}, {ago(s.asOf)}.{/if}</p>
          {:else}
            <p class="muted" style="margin:-4px 0 12px">Your mastery rank is not known yet, so the daily limits are not either.</p>
          {/if}
          <div class="pools">
            {#each s.pools as p}
              <div class="pool" class:empty={p.left === 0} title="{p.name}{p.read ? '' : ' - not read since the daily reset'}">
                <div class="n ellipsis">{p.name}</div>
                <div class="v num">{p.left == null ? "?" : fmt(p.left)}<span class="faint"> / {fmt(p.cap)}</span></div>
                <div class="bar thin lotus"><i style:width="{((p.cap ? Math.max(0, Math.min(1, (p.left ?? p.cap) / p.cap)) : 0) * 100).toFixed(1)}%"></i></div></div>
            {/each}
            {#if s.focus.cap}
              <div class="pool"><div class="n">Focus</div><div class="v num">{fmt(s.focus.left)}<span class="faint"> / {fmt(s.focus.cap)}</span></div>
                <div class="bar thin"><i style:width="{(Math.max(0, Math.min(1, (s.focus.left ?? s.focus.cap) / s.focus.cap)) * 100).toFixed(1)}%"></i></div></div>
            {/if}
          </div>
          {#if ready.length || full.length || s.universal}
            <div class="hint" style="margin-top:14px;display:block">
              {#if ready.length}<div><b>Rank up ready:</b> {ready.map((r) => r.name).join(", ")}.</div>{/if}
              {#if full.length}<div><b>Full at the top rank:</b> {full.map((r) => r.name).join(", ")} - spend standing there before earning more.</div>{/if}
              {#if s.universal}<div>You hold <b>{fmt(s.universal)} Universal Medallion{s.universal > 1 ? "s" : ""}</b> (1,000 standing each, for any of several syndicates).</div>{/if}
            </div>
          {/if}
        </div>
        <div class="grid cols-2" style="margin-top:16px">
          <div class="card"><div class="card-head"><h2>Faction syndicates</h2><span class="grow"></span>
              <span class={shared?.left === 0 ? "warn-text" : "muted"} style="font-size:13px">{shared?.left == null ? "one shared daily pool" : shared.left === 0 ? "shared pool capped today" : `${fmt(shared.left)} left today, shared`}</span></div>
            <div class="list">
              {#each factions as r (r.tag)}
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                <div class="list-row click syn-row" onclick={() => openDetails("syndicate:" + r.tag)}>
                  <span class="sigil"><Img src={r.rankIcon || r.icon} px={40} cls="gi" /></span>
                  <div class="main"><div class="t">{r.name} <span class="muted" style="font-weight:400">· {r.rankName}</span></div>
                    {@render bar(r)}
                    <div class="s">{amount(r)}{#if r.allies.length}{" "}· <span class="allies">{#each r.allies as a}<span class="ally {a.share > 0 ? 'up' : 'down'}" title="Standing gained here gives {a.name} {a.share > 0 ? '+' : ''}{Math.round(a.share * 100)}%">{a.share > 0 ? "+" : "−"}{a.name}</span>{/each}</span>{/if}</div></div>
                  <div class="side">{@render status(r)}</div></div>
              {/each}
            </div></div>
          <div class="stack">
            {#each s.syndicates.filter((r) => r.kind === "nightwave") as r (r.tag)}{@render card(r, s.pools)}{/each}
            <div class="card"><div class="card-head"><h2>How standing works here</h2></div>
              <p class="muted" style="margin:0;font-size:13.5px">Standing counts toward a rank until its bar is full; then the rank-up costs what is listed, and the bar starts on the next rank. At the top rank a full bar means standing is lost until you spend some. Medallions and other hand-ins are listed under each syndicate. Pick a syndicate to see what its standing buys and what that is worth on warframe.market.</p></div>
          </div>
        </div>
        <h2 style="margin:22px 0 12px">Syndicates</h2>
        <div class="syn-grid">{#each s.syndicates.filter((r) => r.kind === "other") as r (r.tag)}{@render card(r, s.pools)}{/each}</div>
      {/if}
    </div>
  {/snippet}
</Page>
