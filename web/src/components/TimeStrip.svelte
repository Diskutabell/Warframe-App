<!-- The next hours of the Origin System on one scale, now at the left edge: the arbitration of each
     hour, the open-world cycles, and the resets. Only timers: everything here is drawn where it
     ends, so what overlaps is plain to see. -->
<script lang="ts">
  import { clock } from "../lib/app.svelte";
  import { epoch } from "../lib/format";
  import type { Arbitrations } from "../types/Arbitrations";
  import type { Cycle } from "../types/Cycle";
  import type { Resets } from "../types/Resets";
  import type { Slot } from "../types/Slot";

  let { arbitration = null, cycles = [], resets = null }: {
    arbitration?: Arbitrations | null;
    cycles?: Cycle[];
    resets?: Resets | null;
  } = $props();

  let width = $state(900);
  // The strip draws out from now once, when the page opens; later changes just appear.
  let entering = $state(true);
  $effect(() => {
    const t = setTimeout(() => (entering = false), 1500);
    return () => clearTimeout(t);
  });
  // Narrow, the lane names go above their lanes and the window shortens.
  const narrow = $derived(width <= 560);
  const hours = $derived(width < 520 ? 3 : width < 760 ? 4 : 5);
  // Half a minute is finer than a pixel at this scale.
  const now = $derived(Math.floor(clock.now / 30) * 30);
  const end = $derived(now + hours * 3600);
  const x = (t: number) => Math.max(0, Math.min(100, ((t - now) / (end - now)) * 100));
  // The locale decides 12 or 24 hours; no leading zero either way.
  const hhmm = (t: number) => new Date(t * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });

  /** Local whole hours inside the window. An hour's label is left out where it would run into the
   *  "Now" label or off the right edge; its line stays. */
  const ticks = $derived.by(() => {
    // The lanes are the strip less its label column, which moves above them in a narrow window.
    const lane = narrow ? width : width - 112;
    const d = new Date(now * 1000);
    d.setMinutes(0, 0, 0);
    const out: { t: number; label: boolean }[] = [];
    for (d.setHours(d.getHours() + 1); d.getTime() / 1000 < end; d.setHours(d.getHours() + 1)) {
      const t = d.getTime() / 1000;
      const px = (x(t) / 100) * lane;
      out.push({ t, label: px > 150 && lane - px > 34 });
    }
    return out;
  });

  // ---------------------------------------------------------------- the lanes
  // `key` stays the same while `from` follows now, so a segment is kept, not drawn anew.
  type Seg = { key: string; from: number; to: number; cls: string; label: string; strong?: string; title: string; href: string };
  type Lane = { label: string; thin?: boolean; segs: Seg[] };

  const LIGHT = new Set(["day", "warm", "fass"]);
  const DARK = new Set(["night", "cold", "vome"]);
  const NEXT: Record<string, string> = { day: "night", night: "day", warm: "cold", cold: "warm", fass: "vome", vome: "fass", corpus: "grineer", grineer: "corpus" };
  const cap = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);
  const tone = (s: string) => (LIGHT.has(s) ? "day" : DARK.has(s) ? "night" : "plain");

  const lanes = $derived.by((): Lane[] => {
    const out: Lane[] = [];
    // Arbitrations, an hour each.
    const slots: Slot[] = arbitration ? [arbitration.current, ...arbitration.upcoming].filter((s): s is Slot => !!s && s.end > now && s.start < end) : [];
    if (slots.length) {
      out.push({ label: "Arbitration", segs: slots.map((s) => ({
        key: `slot${s.start}`, from: Math.max(s.start, now), to: Math.min(s.end, end), cls: `slot${s.tier === "S" ? " now" : ""}`,
        strong: s.tier ?? "", label: `${s.mission}, ${s.node}`, href: "#world",
        title: `${s.tier ? `${s.tier} tier: ` : ""}${s.mission} on ${s.node}, ${s.planet}. ${hhmm(s.start)} to ${hhmm(s.end)}`,
      })) });
    }
    // The open worlds: this half until it turns, then the name of the next. How long that one lasts
    // is not in the world state, so no bar claims it.
    for (const c of cycles) {
      const state = String(c.state || "").toLowerCase();
      const turn = epoch(c.expiry);
      if (!state || !Number.isFinite(turn)) continue;
      const next = NEXT[state];
      const segs: Seg[] = [{
        key: "now", from: now, to: Math.min(turn, end), cls: tone(state), href: "#world",
        label: turn > end ? `${cap(state)} until ${hhmm(turn)}` : cap(state), title: `${c.name}: ${cap(state)} until ${hhmm(turn)}`,
      }];
      if (turn < end)
        segs.push({
          key: "next", from: turn, to: end, cls: `next ${next ? tone(next) : "plain"}`, href: "#world",
          label: next ? `${cap(next)} from ${hhmm(turn)}` : `Changes at ${hhmm(turn)}`,
          title: next ? `${c.name}: ${cap(next)} from ${hhmm(turn)}` : `${c.name} changes at ${hhmm(turn)}`,
        });
      out.push({ label: c.name, thin: true, segs });
    }
    return out;
  });

  const marks = $derived(
    ([["Daily reset", resets?.daily], ["Weekly reset", resets?.weekly]] as const)
      .map(([label, t]) => ({ label, t: epoch(t) }))
      .filter((m) => Number.isFinite(m.t) && m.t > now && m.t < end),
  );
</script>

{#if lanes.length}
  <div class="strip" class:drawn={entering} class:narrow class:marked={marks.length > 0} bind:clientWidth={width} role="group" aria-label="The next {hours} hours">
    <div></div>
    <div class="strip-hours" aria-hidden="true">
      <span class="now" style:left="0">Now {hhmm(now)}</span>
      {#each ticks as tk (tk.t)}{#if tk.label}<span style:left="{x(tk.t)}%">{hhmm(tk.t)}</span>{/if}{/each}
    </div>
    {#each lanes as lane, i (lane.label + i)}
      <div class="strip-label">{lane.label}</div>
      <div class="strip-lane" class:thin={lane.thin}>
        {#each lane.segs as s (s.key)}
          <a class="tseg {s.cls}" style="left:{x(s.from)}%;width:calc({x(s.to) - x(s.from)}% - 3px);--lane:{i}" href={s.href} title={s.title} aria-label={s.title}>
            {#if s.strong}<b>{s.strong}</b>{/if}<span>{s.label}</span></a>
        {/each}
      </div>
    {/each}
    <div class="strip-marks" aria-hidden="true">
      {#each ticks as tk (tk.t)}<i class="strip-grid" style:left="{x(tk.t)}%"></i>{/each}
      <i class="strip-now"></i>
      {#each marks as m (m.label)}<i class="strip-reset" style:left="{x(m.t)}%"><span>{m.label} {hhmm(m.t)}</span></i>{/each}
    </div>
  </div>
{/if}
