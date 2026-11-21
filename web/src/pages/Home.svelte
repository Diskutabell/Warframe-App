<!-- Overview: who you are, what to do next, and what is going on in the world. -->
<script lang="ts">
  import Art from "../components/Art.svelte";
  import ArbTier from "../components/ArbTier.svelte";
  import Card from "../components/Card.svelte";
  import Countdown from "../components/Countdown.svelte";
  import CycleTile from "../components/CycleTile.svelte";
  import Era from "../components/Era.svelte";
  import GameImg from "../components/GameImg.svelte";
  import Icon from "../components/Icon.svelte";
  import Img from "../components/Img.svelte";
  import Plat from "../components/Plat.svelte";
  import { act } from "../lib/api";
  import { changed, clock, openDetails } from "../lib/app.svelte";
  import { ago, fmt } from "../lib/format";
  import { ICON, PORTRAIT, factionIcon, foeName, mrBadge } from "../lib/game";
  import { every, loader } from "../lib/loader.svelte";
  import type { Fissures } from "../types/Fissures";
  import type { History } from "../types/History";
  import type { StandingView } from "../types/StandingView";
  import type { Summary } from "../types/Summary";
  import type { WorldView } from "../types/WorldView";

  let { S }: { S: Summary } = $props();

  const fissures = loader<Fissures>("fissures", ["catalog", "world", "inventory"]);
  const world = loader<WorldView>("world", ["catalog", "world"]);
  const standing = loader<StandingView>("standing", ["inventory"]);
  const history = loader<History>("history", ["history", "rewards"]);
  // Fissures open and close and the world moves on without telling: look again each minute.
  every(() => { fissures.refresh(); world.refresh(); standing.refresh(); }, 60000);
  // A new reward screen comes with the summary.
  let screens = $state<number | null>(null);
  $effect(() => {
    const n = S.session.screens;
    if (screens !== null && n !== screens) history.refresh();
    screens = n;
  });

  const m = $derived(S.mastery);
  const mr = $derived(S.masteryRank);
  const top = $derived(S.masteryRankMax ?? mr);
  const shown = $derived(mr == null ? "?" : top !== mr ? `${mr}-${top}` : `${mr}`);
  const share = $derived(m.possible ? Math.min(1, m.points / m.possible) : 0);
  const rankName = (r: number) => (r > 30 ? `Legendary ${r - 30}` : `MR ${r}`);
  const purse = $derived(
    ([["platinum", "Platinum", S.currencies?.platinum], ["ducats", "Ducats", S.currencies?.ducats],
      ["endo", "Endo", S.currencies?.endo], ["credits", "Credits", S.currencies?.credits]] as const)
      .filter(([, , v]) => v != null),
  );
  const breakdown = $derived(
    ([["Gear", m.sources.gear], ["Star chart", m.sources.starChart], ["Steel Path", m.sources.steelPath],
      ["Railjack intrinsics", m.sources.railjack], ["Drifter intrinsics", m.sources.drifter]] as const)
      .filter(([, v]) => v).map(([label, v]) => `${label} ${fmt(v)}`).join(" · "),
  );
  const toNext = $derived(`${fmt(m.nextAt - m.total)} more for ${rankName(m.rank + 1)}`);
  const hint = $derived(
    m.lacks === "profile" ? "This is gear only. The star chart and intrinsics are added once your profile has been read (link the account in Settings)."
    : m.lacks === "starChart" ? "The star chart is not counted yet: its data could not be downloaded."
    : mr == null || (top != null && m.rank > top) ? `Your mastery XP is enough for ${rankName(m.rank)}${mr != null ? `; you hold MR ${shown}` : ""}. ${toNext}.`
    : m.rank === top ? `${toNext}.` : "",
  );

  // Crack now: per tier (and Steel Path), the best relic you hold for the fissures open.
  type Group = { tier: string; hard: boolean; best: NonNullable<NonNullable<Fissures["fissures"][number]["relics"]>["best"]>; owned: number; missions: Fissures["fissures"] };
  const crack = $derived.by(() => {
    const groups = new Map<string, Group>();
    for (const f of fissures.data?.fissures ?? []) {
      if (f.storm || !f.relics || !f.relics.owned || !f.relics.best) continue;
      const key = `${f.tier}|${f.hard}`;
      if (!groups.has(key)) groups.set(key, { tier: f.tier ?? "", hard: f.hard, best: f.relics.best, owned: f.relics.owned, missions: [] });
      groups.get(key)!.missions.push(f);
    }
    return [...groups.values()].sort((a, b) => b.best.needs - a.best.needs || b.best.ev - a.best.ev).slice(0, 6);
  });

  const st = $derived(standing.data);
  const usedPools = $derived(st ? st.pools.filter((p) => p.left != null && p.cap && p.left < p.cap) : []);
  const poolRows = $derived(st ? (usedPools.length ? usedPools : st.pools.filter((p) => p.key === "DailyAffiliation")) : []);
  const rankUps = $derived(st ? st.syndicates.filter((r) => r.atCap || r.full) : []);

  const w = $derived(world.data && "cycles" in world.data ? world.data : null);

  async function pick(id: number, name: string, undo: boolean, btn: HTMLButtonElement) {
    btn.disabled = true;
    await act("history/pick", { id, name: undo ? null : name }, undo ? "Pick undone" : "Added to the inventory");
    btn.disabled = false;
  }

  async function showGame() {
    const game = S.accounts.find((a) => a.playing);
    if (game && (await act("account/switch", { key: game.key }))) await changed();
  }
</script>

<!-- The arbitration schedule needs no world state. -->
{#snippet arbitration()}
  {#if world.data?.arbitration.current}
    {@const arb = world.data.arbitration.current}
    <a class="list-row click plain" href="#world"><GameImg path={factionIcon(arb.faction)} px={22} title={foeName(arb.faction)} />
      <span class="grow">Arbitration <span class="muted">· {arb.mission}, {arb.node}</span></span>
      <ArbTier tier={arb.tier} /><span class="muted"><Countdown until={arb.end} /></span></a>
  {/if}
{/snippet}

<div class="page">
  {#if S.dataError}<div class="callout" style="margin-bottom:16px">{S.dataError}</div>{/if}

  {#if S.account.logName && !S.account.playing}
    <div class="hint warn"><Icon name="home" /><span><b>{S.account.logName}</b> is logged in to the game; this is {S.account.name}.</span>
      <button class="ghost" onclick={showGame}>Show {S.account.logName}</button></div>
  {/if}
  <div class="card hero">
    <div class="hero-main">
      <div class="mr-badge" title={mr == null ? "Mastery rank: not known yet (it comes from the game log)" : "Mastery rank"}>
        <GameImg path={mr == null ? ICON.mastery : mrBadge(top)} px={96} />
        <b>{mr == null ? "?" : `MR ${shown}`}</b><span>Mastery rank</span>
      </div>
      <div class="grow">
        <h1>{S.account.name || S.account.logName || "Tenno"}</h1>
        <div class="facts">
          <span><b>{fmt(S.mastered)}</b>of {fmt(S.total)} mastered</span>
          <span><b>{fmt(S.total - S.mastered)}</b>remaining</span>
          <span title={breakdown}><b>{fmt(m.total)}{m.lacks ? "+" : ""}</b>mastery XP</span>
        </div>
        <div class="hero-prog">
          <div class="bar"><i style:width="{(share * 100).toFixed(1)}%"></i></div>
          <span class="num">{(share * 100).toFixed(1)}% · {fmt(m.possible - m.points)} gear XP left</span>
        </div>
      </div>
      {#if purse.length}
        <div class="purse">
          {#each purse as [k, label, v]}<div class="cur"><img class="ci" alt="" src="icons/{k}.png" /><div><b>{fmt(v)}</b><span>{label}</span></div></div>{/each}
        </div>
      {/if}
    </div>
    {#if hint || (breakdown && !m.lacks)}
      <div class="hero-foot">
        {#if hint}<span class="hero-next"><Icon name="mastery" /><span>{hint}</span></span>{:else}<span></span>{/if}
        {#if breakdown && !m.lacks}<span class="hero-break num">{breakdown}</span>{/if}
      </div>
    {/if}
  </div>

  <div class="grid cols-2" style="margin-top:16px">
    <div class="stack">
      <Card title="Crack now" link="fissures">
        {#if !crack.length}
          <div class="empty"><b>Nothing to crack right now</b>{fissures.data?.error || "No open fissure matches a relic you own."}</div>
        {:else}
          <div class="list">
            {#each crack as g}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div class="list-row click" onclick={() => openDetails(g.best.name)}>
                <Art src={g.best.image} cls="sm" /><Era era={g.tier} />
                <div class="main">
                  <div class="t">{g.best.name} Relic{#if g.hard}<span class="pill warn">Steel Path</span>{/if}
                    <span class="muted" style="font-weight:400;font-size:13px"> · {g.best.needs ? `${g.best.needs} needed drops · ` : ""}<Plat n={g.best.ev} /> radiant</span></div>
                  <div class="s">{#each g.missions as f, i}{i ? " · " : ""}{f.mission} <span class="faint">({(f.node ?? "").replace(/ \(.*\)$/, "")}, <Countdown until={f.expiry} done="ending" />)</span>{/each}</div>
                </div>
                <span class="faint num" style="font-size:12.5px">{fmt(g.owned)} {g.tier === "Omnia" ? "relics" : g.tier}</span>
              </div>
            {/each}
          </div>
        {/if}
      </Card>

      <Card title="Relic rewards">
        <div class="tiles" style="margin-bottom:10px">
          <div class="tile"><b>{fmt(history.data?.session.screens ?? 0)}</b><span>reward screens this session</span></div>
          <div class="tile"><b><Plat n={history.data?.session.bestValue ?? 0} /></b><span>best picks, at market price</span></div>
        </div>
        {#if !history.data?.entries.length}
          <div class="empty"><b>No reward screens yet</b>Crack a relic; the reward strip and this list fill in by themselves.</div>
        {:else}
          <div class="list">
            {#each history.data.entries.slice(0, 5) as e (e.id ?? e.t)}
              <div class="list-row">
                <span class="faint num" style="width:72px;flex:none;font-size:12.5px">{ago(e.t)}</span>
                <div class="chips grow" style="margin:0">
                  {#each e.drops as d}
                    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                    <span class="chip" class:open={d.name === e.best} class:picked={d.name === e.picked} onclick={() => openDetails(d.uid)}>
                      {d.name} {#if d.price != null}<Plat n={d.price} />{/if}{#if d.needed}<span class="lotus"> ●</span>{/if}
                      {#if e.id != null}
                        <button class="ghost icon pick" title={d.name === e.picked ? "Not picked after all" : "I picked this: add it to the inventory"}
                          onclick={(ev) => { ev.stopPropagation(); pick(e.id!, d.name, d.name === e.picked, ev.currentTarget); }}><Icon name="check" /></button>
                      {/if}
                    </span>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </Card>

      <Card title="Collection" link="primes">
        <div class="tiles">
          <div class="tile"><b>{fmt(S.collection.setsComplete)}</b><span>prime sets complete</span></div>
          <div class="tile"><b>{fmt(S.collection.setsStarted)}</b><span>sets in progress</span></div>
          <div class="tile"><b>{fmt(S.collection.primeParts)}</b><span>prime parts held</span></div>
          <div class="tile"><b>{fmt(S.collection.relics)}</b><span>relics ({fmt(S.collection.relicKinds)} kinds)</span></div>
          <div class="tile"><b>{fmt(S.collection.mods)}</b><span>mods</span></div>
        </div>
      </Card>
    </div>

    <div class="stack">
      {#if S.pinned.length}
        <Card title="Pinned">
          <div class="list">
            {#each S.pinned as p (p.key)}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div class="list-row" class:click={p.open} onclick={() => p.open && openDetails(p.key)}>
                <Art src={p.image} cls="sm" />
                <div class="main"><div class="t">{p.name}</div>
                  {#if p.need > 0 && p.unit !== "held"}<div class="bar thin lotus" style="margin-top:5px"><i style:width="{Math.min(100, (100 * p.have) / p.need).toFixed(0)}%"></i></div>{/if}
                </div>
                <span class="num {p.need > 0 && p.have >= p.need ? 'lotus' : 'muted'}" style="font-size:13px">{p.unit === "held" ? `${fmt(p.have)} held${p.need ? ` · need ${p.need}` : ""}` : `${p.have}/${p.need} ${p.unit}`}</span>
              </div>
            {/each}
          </div>
        </Card>
      {/if}

      <Card title="Standing" link="standing">
        {#if !st || !st.loaded}
          <div class="empty"><b>No syndicate data yet</b>It comes with the game data.</div>
        {:else if !st.account}
          <div class="empty"><b>No account yet</b>Start the game once.</div>
        {:else}
          <div class="row" style="margin-bottom:6px">
            <span class="muted grow" style="font-size:13px">{st.cap ? `${fmt(st.cap)} a day per pool` : "Mastery rank unknown"}{usedPools.length ? "" : " · nothing earned today"}</span>
            <span class="muted" style="font-size:13px">resets in <Countdown until={st.reset} done="a moment" /></span>
          </div>
          {#if rankUps.length}
            <div class="chips" style="margin:0 0 8px">
              {#each rankUps as r}
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                <span class="chip" class:open={r.atCap} onclick={() => openDetails("syndicate:" + r.tag)}><Img src={r.rankIcon || r.icon} px={18} cls="gi" />{r.name}: {r.atCap ? "rank up" : "full"}</span>
              {/each}
            </div>
          {/if}
          <div class="list">
            {#each poolRows.slice(0, 7) as p}
              <div class="list-row"><span class="grow ellipsis">{p.name}</span>
                <div class="bar thin lotus" style="width:110px;margin:0"><i style:width="{((100 * (p.left ?? 0)) / (p.cap || 1)).toFixed(1)}%"></i></div>
                <span class="num {p.left === 0 ? 'warn-text' : 'muted'}" style="width:92px;text-align:right">{p.left === 0 ? "capped" : `${fmt(p.left)} left`}</span></div>
            {/each}
          </div>
        {/if}
      </Card>

      <Card title="World" link="world">
        {#if !w}
          <div class="empty"><b>World state unavailable</b>{world.data?.error || world.error || ""}</div>
          <div class="list">{@render arbitration()}</div>
        {:else}
          <div class="cycles scenic small">{#each w.cycles as c}<CycleTile {c} withPlace={false} />{/each}</div>
          <div class="list" style="margin-top:8px">
            <div class="list-row"><GameImg path={PORTRAIT.baro} px={30} cls="portrait mini" />
              <span class="grow">Baro Ki'Teer{#if w.baro.location}<span class="muted"> · {w.baro.location}</span>{/if}</span>
              <span class={w.baro.active ? "lotus" : "muted"}>{#if w.baro.active}here for <Countdown until={w.baro.expiry} done="leaving" />{:else}arrives in <Countdown until={w.baro.activation} />{/if}</span></div>
            {@render arbitration()}
            <div class="list-row"><span class="grow">Daily reset</span><span class="muted"><Countdown until={w.resets.daily} /></span></div>
            <div class="list-row"><span class="grow">Weekly reset</span><span class="muted"><Countdown until={w.resets.weekly} /></span></div>
          </div>
        {/if}
      </Card>

      <Card title="Foundry" link="foundry">
        {#if !S.foundry.length}
          <div class="empty"><b>Nothing building</b>{S.sources.inventory.source ? "The foundry is empty." : "Needs an inventory (Settings)."}</div>
        {:else}
          <div class="list">
            {#each S.foundry as x}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div class="list-row" class:click={x.known} onclick={() => x.known && openDetails(x.uid)}>
                <Art src={x.image} cls="sm" /><div class="main"><div class="t">{x.name}</div></div>
                {#if x.done}{#if x.done <= clock.now}<span class="pill good">Ready</span>{:else}<span class="muted"><Countdown until={x.done} done="ready" /></span>{/if}
                {:else}<span class="muted">building</span>{/if}
              </div>
            {/each}
          </div>
        {/if}
      </Card>

      <Card title="Activity">
        <ul class="events">
          {#each S.events as e}<li class={e.kind}><time>{new Date(e.t * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>{e.text}</li>
          {:else}<li>Nothing yet.</li>{/each}
        </ul>
      </Card>
    </div>
  </div>
</div>
