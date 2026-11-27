<!-- Fissures: what is open, which of your relics fit, and the ones you watch. -->
<script lang="ts" module>
  const st = $state({ kind: "normal" as "normal" | "steel" | "storm", tier: "" });
</script>

<script lang="ts">
  import Countdown from "../components/Countdown.svelte";
  import Era from "../components/Era.svelte";
  import FactionTag from "../components/FactionTag.svelte";
  import GameImg from "../components/GameImg.svelte";
  import Icon from "../components/Icon.svelte";
  import Page from "../components/Page.svelte";
  import Plat from "../components/Plat.svelte";
  import Seg from "../components/Seg.svelte";
  import { act } from "../lib/api";
  import { clock, openDetails } from "../lib/app.svelte";
  import { epoch, fmt } from "../lib/format";
  import { missionScene, relicIcon, tex } from "../lib/game";
  import { every, loader } from "../lib/loader.svelte";
  import type { FissureRow } from "../types/FissureRow";
  import type { Fissures } from "../types/Fissures";
  import type { Settings } from "../types/Settings";

  const page = loader<Fissures>("fissures", ["inventory", "catalog", "world"]);
  const settings = loader<Settings>("settings", ["settings"]);
  every(() => page.refresh(), 60000);
  const KINDS = [["normal", "Normal"], ["steel", "Steel Path"], ["storm", "Void Storms"]] as const;
  const TIERS = [["", "All tiers"], ...["Lith", "Meso", "Neo", "Axi", "Requiem", "Omnia"].map((t) => [t, t] as const)] as const;
  const kindOf = (f: FissureRow) => (f.storm ? "storm" : f.hard ? "steel" : "normal");

  // The watch list as shown: the engine's, with a change on its way already in (and taken back
  // if the engine refuses it).
  // Quick clicks are saved one after another, each with every click before it.
  let pending = $state<string[] | null>(null);
  const watch = $derived(new Set(pending ?? page.data?.watch ?? []));
  let saving = 0;
  let saves: Promise<unknown> = Promise.resolve();
  function toggle(key: string) {
    const next = new Set(watch);
    const on = !next.has(key);
    if (on) next.add(key);
    else next.delete(key);
    const list = (pending = [...next]);
    saving++;
    saves = saves.then(async () => {
      await act("settings", { fissure_watch: list }, on ? "Watching" : "Stopped watching");
      if (--saving) return;
      // Either way the engine's list is what counts.
      await page.refresh();
      if (!saving) pending = null;
    });
  }
  let missing = $state(new Set<string>());
  // Every row's time left on one scale, three hours wide: a fissure rarely lasts longer.
  const SCALE = 3 * 3600;
  const share = (f: FissureRow) => Math.max(0, Math.min(100, ((epoch(f.expiry) - clock.now) / SCALE) * 100));
  const soon = (f: FissureRow) => epoch(f.expiry) - clock.now < 600;
</script>

<Page title="Fissures" {page}>
  {#snippet children(data)}
    {@const rows = data.fissures.filter((f) => kindOf(f) === st.kind && (!st.tier || f.tier === st.tier))}
    <div class="page">
      <div class="page-head"><div class="grow"><h1>Fissures</h1><p>Open void fissures, and the best relic you hold for each.</p></div>
        <label class="row" style="gap:10px;cursor:pointer"><span class="switch"><input type="checkbox" checked={settings.data?.notify_fissures ?? false}
          onchange={(e) => act("settings", { notify_fissures: e.currentTarget.checked }, e.currentTarget.checked ? "Notifications on" : "Notifications off")} /><span></span></span>
          <span>Notify me about watched fissures <Icon name="bell" /></span></label></div>
      {#if data.error}<div class="callout" style="margin-bottom:12px">{data.error}</div>{/if}
      <div class="toolbar">
        <Seg options={KINDS} bind:value={st.kind} label="Kind" />
        <Seg options={TIERS} bind:value={st.tier} label="Tier" />
        <span class="count">{rows.length} open</span>
      </div>
      <div class="card flush">
        {#if rows.length}
          <div class="fis-head"><span>Tier</span><span>Mission</span><span>Faction</span><span>Ends in</span><span>Your best relic</span><span></span></div>
          {#each rows as f, i (f.id ?? `row-${i}`)}
            {@const scene = missionScene(f.mission)}
            <div class="fis">
              <div class="tier"><GameImg path={relicIcon(f.tier)} px={30} /><Era era={f.tier ?? ""} /></div>
              <div class="what">
                {#if scene && !missing.has(scene)}<img class="scene" alt="" loading="lazy" decoding="async" src={tex(scene, 54)} onerror={() => (missing = new Set([...missing, scene]))} />{/if}
                <div><div class="mission">{f.mission}</div><div class="node">{f.node}</div></div>
              </div>
              <div class="faction"><FactionTag name={f.enemy} px={20} /></div>
              <div class="ends" class:soon={soon(f)} title="Closes at {new Date(epoch(f.expiry) * 1000).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' })}">
                <b><Countdown until={f.expiry} done="ending" /></b><div class="bar"><i style:width="{share(f).toFixed(1)}%"></i></div></div>
              {#if f.storm}<span class="mine none">Railjack</span>
              {:else if !f.relics || !f.relics.owned}<span class="mine none">No {f.tier} relics</span>
              {:else}
                {@const b = f.relics.best}
                <div class="mine">
                  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                  <span class="click-relic" onclick={() => b && openDetails(b.name)}>{b?.name ?? ""}</span> <Plat n={b?.ev} />
                  <span class="muted">{fmt(f.relics.owned)} {f.tier === "Omnia" ? "" : `${f.tier} `}relics held{#if b?.needs}, <span class="lotus">{b.needs} {b.needs === 1 ? "drop" : "drops"} needed</span>{/if}</span>
                </div>
              {/if}
              <button class="ghost icon watch" class:on={watch.has(f.watch)} title="{watch.has(f.watch) ? 'Stop watching' : 'Watch'} {f.tier} {f.mission}" onclick={() => toggle(f.watch)}><Icon name="bell" /></button>
            </div>
          {/each}
        {:else}
          <div class="empty"><b>None open</b>No fissure of this kind and tier is open right now. New ones open every few minutes.</div>
        {/if}
      </div>
      <p class="footnote"><Icon name="bell" /><span>The bell watches a tier, mission and kind (for example Axi Capture on the Steel Path), not one fissure, so you hear about the next one too.</span></p>
    </div>
  {/snippet}
</Page>
