<!-- Overview: who you are, the next hours of the Origin System, and what to do about them. -->
<script lang="ts">
  import Art from "../components/Art.svelte";
  import Card from "../components/Card.svelte";
  import Countdown from "../components/Countdown.svelte";
  import Era from "../components/Era.svelte";
  import GameImg from "../components/GameImg.svelte";
  import Icon from "../components/Icon.svelte";
  import Img from "../components/Img.svelte";
  import Plat from "../components/Plat.svelte";
  import TimeStrip, { type Crack } from "../components/TimeStrip.svelte";
  import { act } from "../lib/api";
  import { changed, clock, openDetails } from "../lib/app.svelte";
  import { ago, epoch, fmt } from "../lib/format";
  import { ICON, mrBadge } from "../lib/game";
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
  const rankName = (r: number) => (r > 30 ? `Legendary ${r - 30}` : `MR ${r}`);
  // How far into the current rank: the bar fills toward the next one.
  const toNext = $derived(Math.max(0, m.nextAt - m.total));
  const within = $derived(m.nextAt > m.rankFloor ? Math.min(1, Math.max(0, (m.total - m.rankFloor) / (m.nextAt - m.rankFloor))) : 0);
  const purse = $derived(
    ([["platinum", "platinum", S.currencies?.platinum], ["ducats", "ducats", S.currencies?.ducats],
      ["endo", "endo", S.currencies?.endo], ["credits", "credits", S.currencies?.credits]] as const)
      .filter(([, , v]) => v != null),
  );
  const breakdown = $derived(
    ([["gear", m.sources.gear], ["star chart", m.sources.starChart], ["Steel Path", m.sources.steelPath],
      ["Railjack intrinsics", m.sources.railjack], ["Drifter intrinsics", m.sources.drifter]] as const)
      .filter(([, v]) => v).map(([label, v]) => `${fmt(v)} from ${label}`).join(", "),
  );
  const hint = $derived(
    m.lacks === "profile" ? "This counts gear only. The star chart and intrinsics are added once your profile has been read: link the account in Settings."
    : m.lacks === "starChart" ? "The star chart is not counted yet: its data could not be downloaded."
    : mr == null || (top != null && m.rank > top) ? `Your mastery XP is enough for ${rankName(m.rank)}${mr != null ? `; the game shows MR ${shown} until you take the test` : ""}.`
    : "",
  );

  // Crack now: per tier (and Steel Path), the best relic you hold for the fissures open.
  const crack = $derived.by((): (Crack & { best: NonNullable<NonNullable<Fissures["fissures"][number]["relics"]>["best"]>; owned: number })[] => {
    const groups = new Map<string, Crack & { best: NonNullable<NonNullable<Fissures["fissures"][number]["relics"]>["best"]>; owned: number }>();
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
  const arb = $derived(world.data?.arbitration ?? null);

  // What falls beyond the strip: the next S-tier arbitration, Baro, the resets.
  const STRIP_HOURS = 5;
  /** "15:00" today, else "Fri 15:00". */
  function day(t: number) {
    const d = new Date(t * 1000);
    const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
    return d.toDateString() === new Date().toDateString() ? time : `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
  }
  const later = $derived.by(() => {
    const beyond = clock.now + STRIP_HOURS * 3600;
    const out: { name: string; sub: string; t: number; href: string }[] = [];
    const s = arb?.next.find((x) => x.tier === "S");
    if (s && s.start > beyond) out.push({ name: "Next S-tier arbitration", sub: `${s.mission}, ${s.node}`, t: s.start, href: "#world" });
    if (w) {
      const baro = w.baro.active ? epoch(w.baro.expiry) : epoch(w.baro.activation);
      if (Number.isFinite(baro)) out.push({ name: w.baro.active ? "Baro Ki'Teer leaves" : "Baro Ki'Teer arrives", sub: w.baro.location ?? "", t: baro, href: "#world" });
      for (const [name, sub, iso] of [["Daily reset", "Standing limits, sortie", w.resets.daily], ["Weekly reset", "Archon hunt, Steel Path, Circuit", w.resets.weekly]] as const) {
        const t = epoch(iso);
        if (Number.isFinite(t) && t > beyond) out.push({ name, sub, t, href: "#world" });
      }
    }
    return out.sort((a, b) => a.t - b.t);
  });

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

<div class="page">
  {#if S.dataError}<div class="callout" style="margin-bottom:20px">{S.dataError}</div>{/if}

  {#if S.account.logName && !S.account.playing}
    <div class="hint warn"><Icon name="home" /><span><b>{S.account.logName}</b> is logged in to the game; this is {S.account.name}.</span>
      <button class="ghost" onclick={showGame}>Show {S.account.logName}</button></div>
  {/if}

  <header class="status">
    <span title={mr == null ? "Mastery rank: not known yet (it comes from the game log)" : `Mastery rank ${shown}`}>
      <GameImg path={mr == null ? ICON.mastery : mrBadge(top)} px={52} cls="badge" /></span>
    <div>
      <h1>{S.account.name || S.account.logName || "Tenno"}</h1>
      <div class="mr">
        <span><b>{mr == null ? "MR ?" : `MR ${shown}`}</b></span>
        <span title={breakdown}><b class="num">{fmt(toNext)}</b> mastery XP to {rankName(m.rank + 1)}</span>
        <span class="bar" title="{Math.round(within * 100)}% of the way from {rankName(m.rank)} to {rankName(m.rank + 1)}"><i style:width="{(within * 100).toFixed(1)}%"></i></span>
        <span><b class="num">{fmt(S.mastered)}</b> of {fmt(S.total)} items mastered</span>
      </div>
    </div>
    {#if purse.length}
      <div class="purse">
        {#each purse as [k, label, v] (k)}<div><img class="ci" alt="" src="icons/{k}.png" /><div><b>{fmt(v)}</b><span>{label}</span></div></div>{/each}
      </div>
    {/if}
  </header>
  {#if hint}<p class="overview-foot">{hint}</p>{/if}

  <div class="strip-wrap">
    <div>
      <TimeStrip arbitration={arb} cycles={w?.cycles ?? []} {crack} resets={w?.resets ?? null} />
      {#if !arb?.current && !w && !crack.length}
        <div class="empty"><b>The world state isn't loaded</b>{world.data?.error || world.error || "It comes from api.warframestat.us."} The Overview asks again every minute, and the strip fills in once it answers.</div>
      {/if}
    </div>
    {#if w || arb?.current}<aside class="later">
      <h2>Later</h2>
      <div class="list">
        {#each later as l (l.name)}
          <a class="list-row plain" href={l.href}><div class="main"><div class="t">{l.name}</div>{#if l.sub}<div class="s">{l.sub}</div>{/if}</div>
            <div class="when"><b>{day(l.t)}</b><span><Countdown until={l.t} /></span></div></a>
        {:else}<div class="muted">Nothing scheduled past the next {STRIP_HOURS} hours.</div>{/each}
      </div>
    </aside>{/if}
  </div>

  <div class="grid cols-2">
    <div class="stack">
      <Card title="Crack now" link="fissures">
        {#if !crack.length && fissures.data?.error}
          <div class="empty"><b>Fissures aren't loaded</b>{fissures.data.error}</div>
        {:else if !crack.length}
          <div class="empty"><b>Nothing to crack right now</b>No open fissure takes a relic you hold. The Fissures page has them all.</div>
        {:else}
          <div class="list crack">
            {#each crack as g (g.tier + g.hard)}
              <button type="button" class="list-row click" onclick={() => openDetails(g.best.name)}>
                <Art src={g.best.image} cls="sm" />
                <div class="main">
                  <div class="t">{g.best.name} Relic{#if !g.best.name.startsWith(g.tier)}<Era era={g.tier} />{/if}{#if g.hard}<span class="pill" style="margin-left:6px">Steel Path</span>{/if}</div>
                  <div class="s">{#if g.best.needs}<span class="lotus">{g.best.needs} {g.best.needs === 1 ? "drop" : "drops"} you still need</span>{", "}{/if}{fmt(g.owned)} {g.tier === "Omnia" ? "relics" : g.tier} held</div>
                </div>
                <div class="value"><Plat n={g.best.ev} /><span>radiant</span></div>
              </button>
            {/each}
          </div>
        {/if}
      </Card>

      <Card title="Reward screens">
        {@const sess = history.data?.session}
        {#if !history.data?.entries.length}
          <div class="empty"><b>No reward screen read yet</b>Crack a relic: the prices go over the game, and the screen is kept here.</div>
        {:else}
          {#if sess?.screens}<p class="muted" style="margin:0 0 10px"><b class="num" style="color:var(--text)">{fmt(sess.screens)}</b> this session. The best picks were worth <Plat n={sess.bestValue} /> at market price.</p>{/if}
          <div class="list">
            {#each history.data.entries.slice(0, 5) as e (e.id ?? e.t)}
              <div class="list-row">
                <span class="faint num" style="width:72px;flex:none;font-size:var(--t-13)">{ago(e.t)}</span>
                <div class="chips grow" style="margin:0">
                  {#each e.drops as d, i (i)}
                    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                    <span class="chip" class:open={d.name === e.best} class:picked={d.name === e.picked} title={d.name === e.best ? "The best pick" : ""} onclick={() => openDetails(d.uid)}>
                      {d.name}{#if d.price != null}<Plat n={d.price} />{/if}{#if d.needed}<span class="lotus" title="You still need this">needed</span>{/if}
                      {#if e.id != null}
                        <button class="pick" title={d.name === e.picked ? "Not picked after all" : "I picked this: add it to the inventory"}
                          aria-label={d.name === e.picked ? `Undo picking ${d.name}` : `I picked ${d.name}`}
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
        <div class="figures">
          <div><b>{fmt(S.collection.setsComplete)}</b><span>prime sets complete</span></div>
          <div><b>{fmt(S.collection.setsStarted)}</b><span>sets in progress</span></div>
          <div><b>{fmt(S.collection.primeParts)}</b><span>prime parts held</span></div>
          <div><b>{fmt(S.collection.relics)}</b><span>relics, {fmt(S.collection.relicKinds)} kinds</span></div>
          <div><b>{fmt(S.collection.mods)}</b><span>mods</span></div>
        </div>
      </Card>
    </div>

    <div class="stack">
      <Card title="Standing left today" link="standing">
        {#if !st || !st.loaded}
          <div class="empty"><b>No syndicate data yet</b>It comes with the game data.</div>
        {:else if !st.account}
          <div class="empty"><b>No account yet</b>Start the game once.</div>
        {:else}
          <p class="muted" style="margin:-4px 0 10px">{st.cap ? `${fmt(st.cap)} a day in each pool.` : "Mastery rank unknown, so the limit is too."}{usedPools.length ? "" : " Nothing earned today yet."} Resets in <Countdown until={st.reset} done="a moment" />.</p>
          {#if rankUps.length}
            <div class="chips" style="margin:0 0 10px">
              {#each rankUps as r (r.tag)}
                <button type="button" class="chip" class:open={r.atCap} onclick={() => openDetails("syndicate:" + r.tag)}><Img src={r.rankIcon || r.icon} px={18} cls="gi" />{r.name}: {r.atCap ? "rank up ready" : "full, spend some"}</button>
              {/each}
            </div>
          {/if}
          <div class="list">
            {#each poolRows.slice(0, 7) as p (p.key)}
              <div class="list-row"><span class="grow ellipsis">{p.name}</span>
                <div class="bar lotus" style="width:96px;flex:none"><i style:width="{((100 * (p.left ?? 0)) / (p.cap || 1)).toFixed(1)}%"></i></div>
                <span class="num {p.left === 0 ? 'warn-text' : ''}" style="width:96px;text-align:right">{p.left === 0 ? "capped" : `${fmt(p.left)} left`}</span></div>
            {/each}
          </div>
        {/if}
      </Card>

      <Card title="Foundry" link="foundry">
        {#if !S.foundry.length}
          <div class="empty"><b>Nothing building</b>{S.sources.inventory.source ? "The foundry is empty." : "This needs an inventory: see Settings."}</div>
        {:else}
          <div class="list">
            {#each S.foundry as x (x.uid)}
              {@const ready = x.done != null && x.done <= clock.now}
              {#snippet build()}
                <Art src={x.image} cls="sm" /><div class="main"><div class="t">{x.name}</div></div>
                {#if ready}<span class="pill gold">Ready to claim</span>
                {:else if x.done}<span class="muted num"><Countdown until={x.done} done="ready" /></span>
                {:else}<span class="muted">building</span>{/if}
              {/snippet}
              {#if x.known}<button type="button" class="list-row click" onclick={() => openDetails(x.uid)}>{@render build()}</button>
              {:else}<div class="list-row">{@render build()}</div>{/if}
            {/each}
          </div>
        {/if}
      </Card>

      {#if S.pinned.length}
        <Card title="Pinned">
          <div class="list">
            {#each S.pinned as p (p.key)}
              {#snippet pinned()}
                <Art src={p.image} cls="sm" />
                <div class="main"><div class="t">{p.name}</div>
                  {#if p.need > 0 && p.unit !== "held"}<div class="bar thin" style="margin-top:6px"><i style:width="{Math.min(100, (100 * p.have) / p.need).toFixed(0)}%"></i></div>{/if}
                </div>
                <span class="num {p.need > 0 && p.have >= p.need ? 'good' : 'muted'}">{p.unit === "held" ? `${fmt(p.have)} held${p.need ? `, need ${p.need}` : ""}` : `${p.have} of ${p.need} ${p.unit}`}</span>
              {/snippet}
              {#if p.open}<button type="button" class="list-row click" onclick={() => openDetails(p.key)}>{@render pinned()}</button>
              {:else}<div class="list-row">{@render pinned()}</div>{/if}
            {/each}
          </div>
        </Card>
      {/if}

      <Card title="Activity">
        <ul class="events">
          {#each S.events as e (e.t + e.text)}<li class={e.kind}><time>{new Date(e.t * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</time>{e.text}</li>
          {:else}<li>Nothing yet.</li>{/each}
        </ul>
      </Card>
    </div>
  </div>
</div>
