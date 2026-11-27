<!-- One item, part or relic, or the drop sources of anything else (`/api/item`). -->
<script lang="ts">
  import Art from "../components/Art.svelte";
  import Duc from "../components/Duc.svelte";
  import Era from "../components/Era.svelte";
  import GameImg from "../components/GameImg.svelte";
  import Icon from "../components/Icon.svelte";
  import Img from "../components/Img.svelte";
  import Plat from "../components/Plat.svelte";
  import StatusPill from "../components/StatusPill.svelte";
  import Stepper from "../components/Stepper.svelte";
  import { act, api } from "../lib/api";
  import { changed, openDetails } from "../lib/app.svelte";
  import { fmt, wiki } from "../lib/format";
  import { ICON } from "../lib/game";
  import { loader } from "../lib/loader.svelte";
  import type { DetailAnswer } from "../types/DetailAnswer";
  import Acquisition from "./Acquisition.svelte";
  import DropRows from "./DropRows.svelte";
  import Loading from "./Loading.svelte";
  import RecipeDetails from "./RecipeDetails.svelte";
  import RelicChips from "./RelicChips.svelte";
  import TaskButton from "./TaskButton.svelte";
  import Top from "./Top.svelte";

  let { key }: { key: string } = $props();
  const x = loader<DetailAnswer>((signal) => api("item?key=" + encodeURIComponent(key), undefined, signal),
    ["inventory", "catalog", "prices", "tasks"]);

  let building = $state(false);
  async function markBuilt(uid: string, name: string) {
    // One build per click: a second click while the first is on its way would count two.
    building = true;
    if (await act("foundry/built", { uid }, `${name} marked built`)) await changed();
    building = false;
  }
  async function pin(d: DetailAnswer) {
    if (await act("manual/pin", { uid: d.key, pinned: !d.pinned }, d.pinned ? "Unpinned" : "Pinned to the Overview")) await changed();
  }
  async function mastered(uid: string, on: boolean) {
    if (await act("manual/mastered", { uid, mastered: on }, "Saved")) await changed();
  }
  const pct = (v: number | null | undefined) => (v == null ? "?" : `${Math.round(v * 10) / 10}%`);
</script>

{#if !x.data}
  <Loading error={x.error ?? (x.downloading ? "Downloading game data…" : null)} />
{:else}
  {@const d = x.data}
  {#snippet pinButton()}
    {#if d.kind !== "drop"}<button class="ghost icon pin" class:on={d.pinned} title={d.pinned ? "Unpin from the Overview" : "Pin to the Overview"} onclick={() => pin(d)}><Icon name="star" /></button>{/if}
  {/snippet}
  <Top market={d.kind === "item" ? d.setSlug : d.kind === "part" || d.kind === "relic" ? d.slug : null} extra={pinButton} />

  {#if d.kind === "drop"}
    <div class="dhead"><Art src={d.image} cls="xl" /><div class="grow"><h2>{d.name}</h2>
      {#if d.task}<div class="row"><TaskButton uid={d.task.uid} onBoard={d.task.onBoard} /></div>{/if}</div></div>
    <Acquisition name={d.name} drops={d.drops} coverage={d.dropCoverage} />

  {:else if d.kind === "item"}
    <div class="dhead"><Art src={d.image} cls="xl" /><div class="grow">
      <h2>{d.name}</h2>
      <div class="row wrap"><StatusPill status={d.status} /><span class="pill">{d.category}</span>
        {#if d.masteryReq}<span class="pill" title="Mastery rank {d.masteryReq} to build or trade it"><GameImg path={ICON.mastery} px={14} />MR {d.masteryReq}</span>{/if}
        {#if d.vaulted}<span class="pill vaulted">Vaulted</span>{/if}{#if d.building}<span class="pill lotus">In the foundry</span>{/if}
        {#if d.owned}<span class="pill good">Owned</span>{/if}</div></div></div>
    <div class="kv">
      <div><span>Rank</span><b>{d.rank} / {d.maxRank}</b></div>
      <div><span>Mastery</span><b>{fmt(d.points)} / {fmt(d.maxPoints)}</b></div>
      <div><span>{d.setPrice != null ? "Set price" : "Affinity"}</span><b>{#if d.setPrice != null}<Plat n={d.setPrice} />{:else}{fmt(d.xp)}{/if}</b></div>
    </div>
    <div class="bar lotus" style="margin-top:10px"><i style:width="{d.maxRank ? Math.round((d.rank / d.maxRank) * 100) : 0}%"></i></div>
    <div class="row wrap" style="margin-top:12px">
      <button onclick={() => mastered(d.uid, !d.manual)}><Icon name="check" /> {d.manual ? "Unmark as mastered" : "Mark as mastered"}</button>
      {#if d.task}<TaskButton uid={d.task.uid} onBoard={d.task.onBoard} />{/if}
      {#if d.parts.length}<button disabled={building} title="{d.building ? 'Collect it from the foundry' : 'Takes its parts and resources from your counts'} until the next inventory read"
        onclick={() => markBuilt(d.uid, d.name)}><Icon name="foundry" /> Mark built</button>{/if}
      <span class="muted" style="font-size:var(--t-13)">{d.manual ? "Marked by hand." : "For when the profile is out of date."}</span>
    </div>
    <Acquisition name={d.name} drops={d.drops} coverage={d.dropCoverage} />
    {#if d.parts.length}
      <div class="dsec"><h2>Parts {#if d.buildable}<span class="pill good" title="Counts may include blueprints not built yet; check the foundry and resources">All collected</span>{/if}</h2>
        {#each d.parts as p (p.uid)}
          <div class="part">
            <div class="top"><Art src={p.image} cls="sm" />
              <div class="grow">
                <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                <div class="ellipsis" style="cursor:pointer;font-weight:600" onclick={() => openDetails(p.uid)}>{p.name}</div>
                <div class="meta"><Plat n={p.price} /><Duc n={p.ducats} />{#if p.vaulted}<span class="pill vaulted">Vaulted</span>{/if}</div></div>
              <div class="row"><span class="muted">{p.need} needed</span><Stepper uid={p.uid} n={p.have} /></div>
            </div>
            {#if p.relics.length}<div class="chips"><RelicChips relics={p.relics} /></div>{/if}
            {#if p.drops.length}<details class="more"><summary>Where to get this part ({p.drops.length})</summary><div class="list"><DropRows drops={p.drops} /></div></details>{/if}
          </div>
        {/each}
      </div>
    {/if}
    {#if d.requires.length}
      <div class="dsec"><h2>Also needs</h2><div class="list">
        {#each d.requires as r}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="list-row click" onclick={() => openDetails(r.uid)}><div class="main"><div class="t">{r.count > 1 ? `${r.count} × ` : ""}{r.name}</div></div>{#if r.status}<StatusPill status={r.status} />{/if}</div>
        {/each}
      </div></div>
    {/if}
    {#if d.resources.length}
      <div class="dsec"><h2>Resources</h2>
        <div class="chips">{#each d.resources as r}<a class="chip res" href={wiki(r.name)} target="_blank" rel="noopener" title="Find {r.name} on the Warframe Wiki"><Img src={r.image} px={22} />{fmt(r.count)} × {r.name}</a>{/each}</div>
        {#each d.resources.filter((r) => r.drops.length) as r}<details class="more"><summary>{r.name} drop sources ({r.drops.length})</summary><div class="list"><DropRows drops={r.drops} /></div></details>{/each}
      </div>
    {/if}

  {:else if d.kind === "part"}
    <div class="dhead"><Art src={d.image} cls="xl" /><div class="grow"><h2>{d.name}</h2>
      <div class="row wrap">{#if d.vaulted}<span class="pill vaulted">Vaulted</span>{:else if d.relics.length}<span class="pill good">Drops now</span>{/if}
        {#if d.needed > d.have}<span class="pill lotus">Needed ×{d.needed}</span>{/if}</div></div></div>
    <div class="kv">
      <div><span>Price</span><b><Plat n={d.price} /></b></div>
      <div><span>Ducats</span><b>{#if d.ducats}<Duc n={d.ducats} />{:else}–{/if}</b></div>
      <div><span>Owned</span><b><Stepper uid={d.uid} n={d.have} /></b></div>
    </div>
    <Acquisition name={d.name} drops={d.drops} coverage={d.dropCoverage} />
    {#if d.recipe}<RecipeDetails recipe={d.recipe} />{:else if !d.recipesAvailable}<p class="muted">Component recipes are unavailable; check the wiki.</p>{/if}
    {#if d.relics.length}
      <div class="dsec"><h2>Dropped by <span class="count">{d.relics.length}</span></h2><div class="list">
        {#each d.relics as r}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="list-row click" onclick={() => openDetails(r.relic)}><div class="main"><div class="t">{r.relic} Relic</div>
            <div class="s">{[r.rarity, r.chance != null ? `${r.chance}% intact` : "", r.owned ? `${r.owned} held` : ""].filter(Boolean).join(", ")}</div></div>
            {#if r.open}<span class="pill lotus">Fissure open</span>{/if}{#if r.vaulted}<span class="pill vaulted">Vaulted</span>{/if}</div>
        {/each}
      </div></div>
    {/if}
    {#if d.usedIn.length}
      <div class="dsec"><h2>Builds</h2><div class="list">
        {#each d.usedIn as i}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="list-row click" onclick={() => openDetails(i.uid)}><Art src={i.image} cls="sm" /><div class="main"><div class="t">{i.name}</div></div><StatusPill status={i.status} /></div>
        {/each}
      </div></div>
    {/if}

  {:else}
    <div class="dhead"><Art src={d.image} cls="xl" /><div class="grow"><h2>{d.name} Relic</h2>
      <div class="row wrap"><Era era={d.era} />{#if d.vaulted}<span class="pill vaulted">Vaulted</span>{/if}
        {#if d.open}<span class="pill lotus"><Icon name="fissures" /> Fissure open now</span>{/if}</div></div></div>
    <div class="kv">
      <div><span>Owned</span><b>{fmt(d.total)}</b></div>
      <div><span>Value radiant</span><b><Plat n={d.ev.Radiant} /></b></div>
      <div><span>Value intact</span><b><Plat n={d.ev.Intact} /></b></div>
    </div>
    <div class="dsec"><h2>Owned by refinement</h2><div class="kv" style="grid-template-columns:repeat(4,minmax(0,1fr))">
      {#each Object.entries(d.uids) as [ref, uid]}<div><span>{ref}</span><b><Stepper {uid} n={d.counts[ref] || 0} /></b></div>{/each}
    </div></div>
    {#snippet vaultedNote()}{#if d.kind === "relic" && d.vaulted}<p class="muted">This relic is vaulted; current sources may be time-limited.</p>{/if}{/snippet}
    <Acquisition name="{d.name} Relic" drops={d.drops} coverage={d.dropCoverage} extra={vaultedNote} />
    <div class="dsec"><h2>Drops</h2><div class="list">
      {#each d.rewards as r}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="list-row click" onclick={() => openDetails(r.uid)}><Art src={r.image} cls="sm" />
          <div class="main"><div class="t">{r.name}{#if r.needed > r.have}<span class="pill lotus" style="margin-left:6px">Needed</span>{/if}</div>
            <div class="s">{r.rarity ? `${r.rarity}: ` : ""}{pct(r.chance.Intact)} intact, {pct(r.chance.Radiant)} radiant{r.have ? `, ${r.have} held` : ""}</div></div>
          <div style="text-align:right"><Plat n={r.price} /><div><Duc n={r.ducats} /></div></div></div>
      {/each}
    </div></div>
  {/if}
{/if}
