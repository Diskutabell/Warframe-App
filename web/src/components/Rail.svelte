<!-- The sidebar: the account shown (and the others to switch to), the pages, and one pill that says
     what the engine is doing, with the details above it on hover. -->
<script lang="ts">
  import { act } from "../lib/api";
  import { app, changed, route, ui } from "../lib/app.svelte";
  import { ago, rankText } from "../lib/format";
  import { ICON, mrBadge } from "../lib/game";
  import { NAV, navKey } from "../lib/nav";
  import type { AccountRow } from "../types/AccountRow";
  import GameImg from "./GameImg.svelte";
  import Icon from "./Icon.svelte";

  const S = $derived(app.state);
  const current = $derived(S?.accounts.find((a) => a.active) ?? null);
  const badge = (a: AccountRow) => (a.masteryRank == null ? ICON.mastery : mrBadge(a.masteryRankMax ?? a.masteryRank));

  async function choose(a: AccountRow) {
    ui.accountMenu = false;
    if (!a.active && (await act("account/switch", { key: a.key }))) await changed();
  }
  // The menu closes on a click elsewhere; Escape is the app's (App.svelte), one layer at a time.
  $effect(() => {
    if (!ui.accountMenu) return;
    const close = (e: MouseEvent) => {
      if (!(e.target as HTMLElement).closest("#acct")) ui.accountMenu = false;
    };
    document.addEventListener("click", close);
    return () => document.removeEventListener("click", close);
  });

  /** What the engine is doing: the most pressing line in the pill, every line in its tip. */
  type Line = { dot: string; text: string };
  const status = $derived.by((): Line & { lines: Line[] } => {
    if (!S) return app.connected ? { dot: "", text: "Connecting to the engine", lines: [] } : { dot: "warn", text: "The engine isn't answering", lines: [] };
    const src = S.sources;
    const elsewhere = src.eeLog.active && S.account.logName && !S.account.playing ? ` as ${S.account.logName}` : "";
    const game: Line = { dot: src.eeLog.active ? "on" : "", text: src.eeLog.active ? `Game running${elsewhere}` : "Game not running" };
    const profile: Line = { dot: S.account.id ? "on" : "warn", text: S.account.id ? "Profile linked" : "Account not linked" };
    const inventory: Line = { dot: src.inventory.source ? "on" : "", text: src.inventory.source ? `Inventory read ${ago(src.inventory.imported)}` : "No inventory yet" };
    const lines = [game, profile, inventory];
    if (src.memscan.state === "scanning") {
      const scan = { dot: "gold", text: `Reading the inventory, ${Math.round(src.memscan.progress * 100)}%` };
      return { ...scan, lines: [...lines, scan] };
    }
    return { ...(S.account.id ? game : profile), lines };
  });
</script>

{#snippet who(a: AccountRow)}
  <div class="acct-txt"><b class="ellipsis">{a.name}</b><span>{rankText(a)}{#if a.playing}, <span class="lotus">in game</span>{/if}</span></div>
{/snippet}

<aside class="rail">
  <div class="acct" id="acct">
    {#if !current}
      <a class="acct-btn empty" href="#settings"><Icon name="mastery" /><div class="acct-txt"><b>No account yet</b><span>Start the game once</span></div></a>
    {:else}
      {@const list = S?.accounts ?? []}
      <button class="acct-btn" type="button" aria-haspopup="true" aria-expanded={ui.accountMenu} title={list.length > 1 ? "Switch account" : "Account"} onclick={(e) => { e.stopPropagation(); ui.accountMenu = !ui.accountMenu; }}>
        <GameImg path={badge(current)} px={32} />
        {@render who(current)}
        {#if list.length > 1}<Icon name="down" />{/if}
      </button>
      {#if ui.accountMenu}
        <div class="acct-menu" role="menu">
          {#each list as a (a.key)}
            <button type="button" role="menuitem" class:on={a.active} onclick={() => choose(a)}><GameImg path={badge(a)} px={24} />{@render who(a)}</button>
          {/each}
          <a href="#settings" class="acct-manage" onclick={() => (ui.accountMenu = false)}>Manage accounts</a>
        </div>
      {/if}
    {/if}
  </div>
  <button class="search-trigger" type="button" onclick={() => (ui.palette = true)}><Icon name="search" /><span>Search</span><kbd>Ctrl K</kbd></button>
  <nav id="nav" aria-label="Pages">
    {#each NAV as p (p.id)}
      {@const key = navKey(p.id)}
      {@const n = p.badge && S ? p.badge(S) : 0}
      <a href="#{p.id}" class:on={route.page === p.id} class:group={p.group} aria-current={route.page === p.id ? "page" : undefined} aria-keyshortcuts={key || undefined}>
        <Icon name={p.icon} /><span>{p.title}</span>
        {#if n}<b class="nav-badge" title={p.badgeTitle?.(n) ?? ""}>{n > 99 ? "99+" : n} done</b>
        {:else if key}<kbd class="key" aria-hidden="true">{key}</kbd>{/if}
      </a>
    {/each}
  </nav>
  <div class="rail-status">
    <a class="status-pill" href="#settings" aria-describedby="status-tip"><span class="dot {status.dot}"></span><span>{status.text}</span></a>
    {#if status.lines.length}
      <div class="status-tip" id="status-tip" role="tooltip">
        {#each status.lines as l (l.text)}<div><span class="dot {l.dot}"></span><span>{l.text}</span></div>{/each}
      </div>
    {/if}
  </div>
</aside>
