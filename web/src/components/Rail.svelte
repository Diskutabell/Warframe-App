<!-- The sidebar: the account shown (and the others to switch to), the pages with the key that
     opens each, and what the engine is doing. -->
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
      <a href="#{p.id}" class:on={route.page === p.id} class:group={p.group} aria-current={route.page === p.id ? "page" : undefined}
        title={key ? `${p.title} (key ${key})` : p.title}>
        <span class="key" aria-hidden="true">{key}</span><span>{p.title}</span>
        {#if n}<b class="nav-badge" title={p.badgeTitle?.(n) ?? ""}>{n > 99 ? "99+" : n} done</b>{/if}
      </a>
    {/each}
  </nav>
  <div class="rail-status">
    {#if !S}
      {#if app.connected}<div class="rail-stat"><span class="dot"></span><span>Connecting to the engine</span></div>
      {:else}<div class="rail-stat"><span class="dot warn"></span><span>The engine isn't answering</span></div>{/if}
    {:else}
      {@const src = S.sources}
      <div class="rail-stat"><span class="dot" class:on={src.eeLog.active}></span><span>{src.eeLog.active ? "Game running" : "Game not running"}{#if src.eeLog.active && S.account.logName && !S.account.playing}<span class="faint">{" "}as {S.account.logName}</span>{/if}</span></div>
      <div class="rail-stat"><span class="dot {S.account.id ? 'on' : 'warn'}"></span><span>{S.account.id ? "Profile linked" : "Account not linked"}</span></div>
      <div class="rail-stat"><span class="dot" class:on={!!src.inventory.source}></span><span>{src.inventory.source ? `Inventory read ${ago(src.inventory.imported)}` : "No inventory yet"}</span></div>
      {#if src.memscan.state === "scanning"}<div class="rail-stat"><span class="dot gold"></span><span>Reading the inventory, {Math.round(src.memscan.progress * 100)}%</span></div>{/if}
      {#if S.desktop && S.minimizeToTray}
        <div class="rail-actions"><button type="button" class="ghost" title="Hide the window to the system tray" onclick={() => act("window/hide")}><Icon name="tray" /><span>Hide to tray</span></button>
          <button type="button" class="ghost icon" title="Quit warframeproject" aria-label="Quit" onclick={() => act("quit")}><Icon name="power" /></button></div>
      {/if}
    {/if}
  </div>
</aside>
