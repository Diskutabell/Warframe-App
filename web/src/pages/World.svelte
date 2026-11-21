<!-- World: the timers and rotations worth knowing before logging in, with the game's own art. -->
<script lang="ts">
  import type { Snippet } from "svelte";
  import ArbTier from "../components/ArbTier.svelte";
  import Art from "../components/Art.svelte";
  import Countdown from "../components/Countdown.svelte";
  import CycleTile from "../components/CycleTile.svelte";
  import Duc from "../components/Duc.svelte";
  import Emblem from "../components/Emblem.svelte";
  import FactionTag from "../components/FactionTag.svelte";
  import GameImg from "../components/GameImg.svelte";
  import Page from "../components/Page.svelte";
  import Plat from "../components/Plat.svelte";
  import { fmt } from "../lib/format";
  import { ICON, PORTRAIT, archonPortrait, factionColor, factionIcon, foeName, syndicateIcon } from "../lib/game";
  import { every, loader } from "../lib/loader.svelte";
  import type { Challenge } from "../types/Challenge";
  import type { Slot } from "../types/Slot";
  import type { WorldView } from "../types/WorldView";

  const page = loader<WorldView>("world", ["catalog", "world"]);
  every(() => page.refresh(), 60000);

  /** When an hour of the schedule starts, in local time: "14:00", or "Tue 14:00" on another day. */
  function at(t: number) {
    const d = new Date(t * 1000);
    const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    return d.toDateString() === new Date().toDateString() ? time : `${d.toLocaleDateString([], { weekday: "short" })} ${time}`;
  }
  const where = (x: Slot) => `${x.node}${x.planet ? `, ${x.planet}` : ""}`;
  const kind = (c: Challenge) => (c.daily ? ["daily", "Daily"] : c.elite ? ["elite", "Elite weekly"] : ["weekly", "Weekly"]) as ["daily" | "elite" | "weekly", string];
  const rolledAt = (t: number | null) => (t ? new Date(t * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : "");
</script>

{#snippet lead(pic: Snippet, title: Snippet | string, sub: string, label: string, until: string | number | null | undefined)}
  <div class="lead">{@render pic()}
    <div class="grow"><div class="big">{#if typeof title === "string"}{title}{:else}{@render title()}{/if}</div>{#if sub}<div class="sub">{sub}</div>{/if}</div>
    {#if label}<div class="when">{label}<b><Countdown {until} /></b></div>{/if}
  </div>
{/snippet}

{#snippet portrait(path: string)}<GameImg {path} px={64} cls="portrait" />{/snippet}

{#snippet missions(list: { type: string | null; node: string | null; modifier?: string | null }[])}
  {#each list as m, i}
    <div class="list-row"><span class="step">{i + 1}</span>
      <div class="main"><div class="t">{m.type}</div><div class="s">{m.node || ""}{#if m.modifier}{" "}· {m.modifier}{/if}</div></div></div>
  {/each}
{/snippet}

{#snippet arbRow(x: Slot)}
  <div class="list-row arb"><span class="arb-at num">{at(x.start)}</span>
    <GameImg path={factionIcon(x.faction)} px={20} title={foeName(x.faction)} />
    <div class="main"><div class="t">{x.mission}</div><div class="s">{where(x)}{#if x.bonus}{" "}· +{x.bonus}% resources{/if}</div></div>
    <ArbTier tier={x.tier} /></div>
{/snippet}

<Page title="World" {page}>
  {#snippet children(data)}
    <div class="page">
      <div class="page-head"><div><h1>World</h1><p>Cycles, vendors and rotations. Times update live.</p></div></div>
      {#if !("cycles" in data)}
        <div class="callout">{data.error || "World state unavailable."}</div>
      {:else}
        {@const w = data}
        <div class="cycles scenic" style="margin-bottom:16px">{#each w.cycles as c (c.name)}<CycleTile {c} />{/each}</div>

        <div class="grid cols-3">
          <div class="card"><div class="card-head"><h2>Baro Ki'Teer</h2></div>
            {#snippet baroPic()}{@render portrait(PORTRAIT.baro)}{/snippet}
            {@render lead(baroPic, w.baro.active ? "Here now" : "Away", w.baro.location || "", w.baro.active ? "leaves in" : "arrives in", w.baro.active ? w.baro.expiry : w.baro.activation)}
            {#if w.baro.active && w.baro.inventory.length}
              <div class="list scroll">
                {#each w.baro.inventory as i}<div class="list-row"><Art src={i.image} cls="sm" /><span class="grow ellipsis">{i.item}</span><Duc n={i.ducats} /><span class="muted num">{fmt(i.credits)} cr</span></div>{/each}
              </div>
            {:else}
              <p class="muted" style="margin:0">{w.baro.active ? "No stock listed." : "The Void Trader comes every two weeks and takes ducats and credits."}</p>
            {/if}
          </div>

          <div class="card"><div class="card-head"><h2>Sortie</h2></div>
            {#if !w.sortie.boss}<div class="muted">Not listed.</div>
            {:else}
              {#snippet sortiePic()}<Emblem path={factionIcon(w.sortie.faction)} />{/snippet}
              {@render lead(sortiePic, w.sortie.boss, w.sortie.faction || "", "ends in", w.sortie.expiry)}
              <div class="list">{@render missions(w.sortie.missions)}</div>
            {/if}
          </div>

          <div class="card"><div class="card-head"><h2>Archon hunt</h2></div>
            {#if !w.archon.boss}<div class="muted">Not listed.</div>
            {:else}
              {@const pic = archonPortrait(w.archon.boss)}
              {#snippet archonPic()}{#if pic}{@render portrait(pic)}{:else}<Emblem path={ICON.archon} />{/if}{/snippet}
              {@render lead(archonPic, w.archon.boss, "Narmer", "ends in", w.archon.expiry)}
              <div class="list">{@render missions(w.archon.missions)}</div>
            {/if}
          </div>
        </div>

        <div class="grid cols-even" style="margin-top:16px">
          <div class="card"><div class="card-head"><h2>Arbitration</h2></div>
            {#if !w.arbitration.current}<div class="muted">The schedule is not loaded yet.</div>
            {:else}
              {@const c = w.arbitration.current}
              {@const soon = w.arbitration.upcoming.slice(0, 5)}
              {#snippet arbPic()}<Emblem path={factionIcon(c.faction)} />{/snippet}
              {#snippet arbTitle()}{c.mission} <ArbTier tier={c.tier} />{/snippet}
              {@render lead(arbPic, arbTitle, `${where(c)} · ${foeName(c.faction)}${c.bonus ? ` · +${c.bonus}% resources` : ""}`, "ends in", c.end)}
              {#if soon.length}<div class="sublabel">Next hours</div><div class="list">{#each soon as x}{@render arbRow(x)}{/each}</div>{/if}
              {#if w.arbitration.next.length}
                <div class="sublabel" style="margin-top:12px">Next by tier</div>
                <div class="list">
                  {#each w.arbitration.next as x}
                    <div class="list-row"><ArbTier tier={x.tier} /><div class="main"><div class="t">{x.mission} · {where(x)}</div><div class="s">{at(x.start)}</div></div>
                      <span class="muted">in <Countdown until={x.start} /></span></div>
                  {/each}
                </div>
              {/if}
              {#if w.arbitration.upcoming.length > soon.length}
                <details class="more"><summary>The next {w.arbitration.upcoming.length} hours</summary>
                  <div class="list">{#each w.arbitration.upcoming as x}{@render arbRow(x)}{/each}</div></details>
              {/if}
            {/if}
          </div>

          <div class="card"><div class="card-head"><h2>Bounties and events</h2></div>
            <div class="list">
              {#each w.bounties as b}
                <div class="list-row"><Emblem path={syndicateIcon(b.syndicate)} cls="sm" />
                  <div class="main"><div class="t">{b.syndicate}</div><div class="s">{b.jobs} bounties</div></div>
                  <span class="muted">new in <Countdown until={b.expiry} /></span></div>
              {/each}
              {#if w.varzia.expiry}
                <div class="list-row"><Emblem path={ICON.vault} cls="sm" />
                  <div class="main"><div class="t">Prime Resurgence</div><div class="s">{w.varzia.location || ""}</div></div>
                  <span class="muted">rotates in <Countdown until={w.varzia.expiry} /></span></div>
              {/if}
              {#each w.events as e}
                {@const share = e.goal ? Math.min(100, (100 * (e.score ?? 0)) / e.goal) : null}
                <div class="list-row"><Emblem path={ICON.alert} cls="sm" />
                  <div class="main"><div class="t">{e.name}</div><div class="s">{e.node || ""}{#if e.goal}{" "}· {fmt(e.score)} / {fmt(e.goal)}{/if}</div>
                    {#if share != null}<div class="bar thin"><i style:width="{share.toFixed(1)}%"></i></div>{/if}</div>
                  <span class="muted"><Countdown until={e.expiry} done="ending" /></span></div>
              {/each}
              {#if !w.bounties.length && !w.varzia.expiry && !w.events.length}<div class="muted">None listed.</div>{/if}
            </div>
          </div>
        </div>

        <div class="grid cols-3" style="margin-top:16px">
          <div class="card"><div class="card-head"><h2>Duviri</h2></div>
            <div class="duviri-head">Your picks in Teshin's Cave</div>
            {#if w.duviri?.groups.length}
              {#each w.duviri.groups as g}
                <div class="sublabel">{g.slot}</div>
                <div class="picks compact">
                  {#each g.picks as x}
                    <div class="pick" class:unowned={!x.owned}><Art src={x.image} /><span>{x.name}</span>
                      {#if !x.owned}<em class="warn-text">not owned</em>{:else if x.mastered}<em class="good">mastered</em>{/if}</div>
                  {/each}
                </div>
              {/each}
              {#if w.duviri.stale}
                <p class="callout" style="margin-top:0">A new spiral has begun since the game listed these, so it offers new ones now. Open the star chart in game to see them here.</p>
              {:else}
                <p class="faint arch-note" style="margin-top:0">What the game listed{w.duviri.rolled ? ` at ${rolledAt(w.duviri.rolled)}` : ""}. New picks come with every spiral and after each extraction.</p>
              {/if}
            {:else}
              <p class="muted">Open the star chart or Duviri in game once: the game then writes the Warframes and weapons it offers you
                into its log, and they show here. Seeing them in the star chart takes Opportunity rank 4.</p>
            {/if}
            {#if w.circuit.length}
              <div class="duviri-circuit"><div class="row duviri-head"><span class="grow">The Circuit's rewards this week</span>
                <span class="muted"><Countdown until={w.resets.weekly} /></span></div>
                {#each w.circuit as c}
                  <div class="sublabel">{c.steel ? "Steel Path · Incarnon adapters" : "Normal · Warframe parts"}</div>
                  <div class="picks compact">{#each c.choices as x}<div class="pick"><Art src={x.image} /><span>{x.name}</span></div>{/each}</div>
                {/each}
              </div>
            {/if}
          </div>

          {#each w.archimedea as a}
            {@const foes = [...new Set(a.missions.map((m) => m.faction).filter((f): f is string => !!f))]}
            <div class="card"><div class="card-head"><h2>{a.name}</h2></div>
              {#snippet archPic()}<Emblem path={factionIcon(foes[0])} />{/snippet}
              {@render lead(archPic, foes.map(foeName).join(" and ") || a.name, "Three missions in a row; Elite adds a second risk to each", "ends in", a.expiry)}
              {#each a.missions as m, i}
                <div class="arch-mission">
                  <div class="row"><span class="step">{i + 1}</span><b class="grow">{m.type}</b>{#if foes.length > 1}<FactionTag name={m.faction} px={16} />{/if}</div>
                  {#if m.deviation.name}<div class="arch-dev"><b>{m.deviation.name}</b>{#if m.deviation.description}<span>{m.deviation.description}</span>{/if}</div>{/if}
                  {#if m.risks.length}<div class="chips">{#each m.risks as r}<span class="pill" class:bad={r.elite} title={r.description || ""}>{r.elite ? "Elite · " : ""}{r.name}</span>{/each}</div>{/if}
                </div>
              {:else}<div class="muted">No missions listed.</div>{/each}
              {#if a.modifiers.length}
                <div class="sublabel" style="margin-top:14px">Personal modifiers</div>
                <div class="list">{#each a.modifiers as p}<div class="list-row"><div class="main"><div class="t">{p.name}</div>{#if p.description}<div class="s wrap">{p.description}</div>{/if}</div></div>{/each}</div>
              {/if}
              <p class="faint arch-note">Loadout picks differ for each player: the game draws them from your own gear and shows them only in game.</p>
            </div>
          {:else}
            <div class="card"><div class="card-head"><h2>Archimedea</h2></div><div class="muted">Not listed.</div></div>
          {/each}
        </div>

        <div class="grid cols-even" style="margin-top:16px">
          <div class="card"><div class="card-head"><h2>Alerts <span class="count">{w.alerts.length}</span></h2></div>
            <div class="list">
              {#each w.alerts as a}
                {@const rewards = a.rewards.length ? a.rewards : [{ name: a.reward || a.what || "", count: 1, image: null }]}
                {@const pic = rewards.find((r) => r.image)?.image}
                <div class="list-row">{#if pic}<Art src={pic} />{:else}<Emblem path={ICON.alert} />{/if}
                  <div class="main"><div class="t wrap">{rewards.map((r) => `${r.count > 1 ? `${r.count}× ` : ""}${r.name}`).join(" · ")}</div>
                    <div class="s">{a.type} · {a.node}</div></div>
                  <div class="side"><FactionTag name={a.faction} /><span class="muted"><Countdown until={a.expiry} done="ending" /></span></div></div>
              {:else}<div class="muted">No alerts.</div>{/each}
            </div>
          </div>

          <div class="card"><div class="card-head"><h2>Invasions <span class="count">{w.invasions.length}</span></h2></div>
            {#each w.invasions as i}
              {@const completion = Number(i.completion ?? 0)}
              <div class="inv">
                <div class="row"><span class="t grow">{i.node}</span><span class="muted">{i.what || ""}</span></div>
                <div class="inv-bar" style="--a:{factionColor(i.attacker)};--d:{factionColor(i.defender)}" title="{Math.round(completion)}% to the attackers">
                  <i style:width="{Math.max(0, Math.min(100, completion)).toFixed(1)}%"></i></div>
                <div class="inv-sides">
                  {#each [[i.attacker, i.attackerReward, i.attackerImage, false], [i.defender, i.defenderReward, i.defenderImage, true]] as [who, reward, image, right]}
                    <div class="inv-side" class:right><GameImg path={factionIcon(who as string)} px={22} />
                      <div><b>{who}</b><span>{reward || "no reward"}</span></div>{#if image}<Art src={image as string} cls="sm" />{/if}</div>
                  {/each}
                </div>
              </div>
            {:else}<div class="muted">No invasions.</div>{/each}
          </div>
        </div>

        <div class="grid cols-2" style="margin-top:16px">
          {#if w.nightwave}
            {@const nw = w.nightwave}
            {@const challenges = [...nw.challenges].sort((a, b) => Number(b.daily) - Number(a.daily) || Number(a.elite) - Number(b.elite))}
            <div class="card"><div class="card-head"><h2>Nightwave</h2></div>
              {#snippet noraPic()}{@render portrait(PORTRAIT.nora)}{/snippet}
              {@render lead(noraPic, nw.season != null ? `Season ${nw.season}` : "Nora Night", `${challenges.length} acts`, nw.expiry ? "ends in" : "", nw.expiry)}
              {#if nw.mine}
                {@const mine = nw.mine}
                <div class="nw-me"><div class="row"><span class="grow"><b>{mine.rankName}</b><span class="muted"> · rank {fmt(mine.title)}</span></span>
                  <span class="num muted">{fmt(mine.progress)} / {fmt(mine.size)}</span></div>
                  <div class="bar thin"><i style:width="{mine.size ? Math.max(0, Math.min(100, (100 * mine.progress) / mine.size)).toFixed(1) : 0}%"></i></div></div>
              {/if}
              <div class="list">
                {#each challenges as c}
                  {@const [k, label] = kind(c)}
                  <div class="list-row"><Emblem path={ICON[k]} cls="sm {k}" title={label} />
                    <div class="main"><div class="t">{c.title}</div><div class="s">{c.desc}</div></div>
                    <div class="side"><span class="num">{fmt(c.reputation)}</span><span class="faint"><Countdown until={c.expiry} done="ending" /></span></div></div>
                {:else}<div class="muted">No acts listed.</div>{/each}
              </div>
            </div>
          {/if}
          <div class="stack">
            <div class="card"><div class="card-head"><h2>Steel Path</h2></div>
              {#snippet teshinPic()}{@render portrait(PORTRAIT.teshin)}{/snippet}
              {@render lead(teshinPic, "Teshin's offer", "Steel Path honors", "changes in", w.steelPath.expiry)}
              <div class="row"><Art src={w.steelPath.image} /><span class="grow" style="font:700 19px/1.2 var(--display)">{w.steelPath.reward || ""}</span>
                <span class="ftag"><GameImg path={ICON.steel} px={18} />{fmt(w.steelPath.cost)}</span></div>
            </div>
            {#if w.darvo}
              {@const d = w.darvo}
              {@const remaining = Number(d.stock ?? 0) - Number(d.sold ?? 0)}
              <div class="card"><div class="card-head"><h2>Darvo's deal</h2></div>
                {#snippet darvoPic()}{#if d.image}<Art src={d.image} cls="lg" />{:else}<Emblem path={ICON.darvo} />{/if}{/snippet}
                <div class="lead">{@render darvoPic()}
                  <div class="grow"><div class="big">{d.item}</div><div class="sub">{#if remaining > 0}{fmt(remaining)} of {fmt(d.stock)} left{:else}<span class="pill warn">Sold out</span>{/if}</div></div>
                  <div class="when">ends in<b><Countdown until={d.expiry} /></b></div></div>
                <div class="row"><GameImg path={ICON.darvo} px={20} /><span class="muted grow">Darvo, daily</span><Plat n={d.price} />
                  <span class="faint" style="text-decoration:line-through">{fmt(d.originalPrice)}</span><span class="pill gold">−{d.discount}%</span></div>
              </div>
            {/if}
            <div class="card"><div class="card-head"><h2>Resets</h2></div><div class="list">
              <a class="list-row click plain" href="#standing"><Emblem path={ICON.daily} cls="sm" /><div class="main"><div class="t">Daily</div><div class="s">Standing limits, login reward</div></div><Countdown until={w.resets.daily} /></a>
              <div class="list-row"><Emblem path={ICON.weekly} cls="sm" /><div class="main"><div class="t">Weekly</div><div class="s">Archon hunt, Steel Path, Nightwave</div></div><Countdown until={w.resets.weekly} /></div>
            </div></div>
          </div>
        </div>
      {/if}
    </div>
  {/snippet}
</Page>
