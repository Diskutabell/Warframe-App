<!-- The sidebar: the account shown (and the others to switch to), the pages, and what the
     engine is doing. -->
<script lang="ts">
  import { act } from "../lib/api";
  import { app, changed, route, ui } from "../lib/app.svelte";
  import { ago, rankText } from "../lib/format";
  import { ICON, mrBadge } from "../lib/game";
  import { PAGES, pageKey } from "../lib/pages";
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

<aside class="rail">
  <div class="acct" id="acct">
    {#if !current}
      <a class="acct-btn empty" href="#settings"><Icon name="mastery" /><div class="acct-txt"><b>No account yet</b><span>Start the game once</span></div></a>
    {:else}
      {@const list = S?.accounts ?? []}
      <button class="acct-btn" type="button" aria-haspopup="true" aria-expanded={ui.accountMenu} title={list.length > 1 ? "Switch account" : "Account"} onclick={(e) => { e.stopPropagation(); ui.accountMenu = !ui.accountMenu; }}>
        <GameImg path={badge(current)} px={30} />
        <div class="acct-txt"><b class="ellipsis">{current.name}</b><span>{rankText(current)}{#if current.playing}{" "}· <span class="lotus">in game</span>{/if}</span></div>
        {#if list.length > 1}<Icon name="down" />{/if}
      </button>
      {#if ui.accountMenu}
        <div class="acct-menu" role="menu">
          {#each list as a (a.key)}
            <button type="button" role="menuitem" class:on={a.active} onclick={() => choose(a)}><GameImg path={badge(a)} px={24} />
              <div class="acct-txt"><b class="ellipsis">{a.name}</b><span>{rankText(a)}{#if a.playing}{" "}· <span class="lotus">in game</span>{/if}</span></div></button>
          {/each}
          <a href="#settings" class="acct-manage" onclick={() => (ui.accountMenu = false)}>Manage accounts</a>
        </div>
      {/if}
    {/if}
  </div>
  <button class="search-trigger" type="button" onclick={() => (ui.palette = true)}><Icon name="search" /><span>Quick jump…</span><kbd>Ctrl K</kbd></button>
  <nav id="nav" aria-label="Sections">
    {#each PAGES as p, i (p.id)}
      {@const key = pageKey(p, i)}
      {#if p.id === "settings"}<div class="sep"></div>{:else if p.group}<div class="nav-group">{p.group}</div>{/if}
      <a href="#{p.id}" class:on={route.page === p.id} title="{p.title}{key ? ` (${key})` : ''}">
        <Icon name={p.icon} /><span>{p.title}</span>
        {#if p.badge}
          {@const n = S ? p.badge(S) : 0}
          {#if n}<b class="nav-badge" title={p.badgeTitle?.(n) ?? ""}>{n > 99 ? "99+" : n}</b>{/if}
        {:else if key}<span class="nav-k">{key}</span>{/if}
      </a>
    {/each}
  </nav>
  <div class="rail-status">
    {#if !S}
      {#if app.connected}<div class="rail-stat"><span class="dot"></span><span>Connecting…</span></div>
      {:else}<div class="rail-stat"><span class="dot warn"></span><span>Engine not responding</span></div>{/if}
    {:else}
      {@const src = S.sources}
      <div class="rail-stat"><span class="dot" class:on={src.eeLog.active}></span><span>{src.eeLog.active ? "Game running" : "Game offline"}{#if src.eeLog.active && S.account.logName && !S.account.playing}<span class="faint"> ({S.account.logName})</span>{/if}</span></div>
      <div class="rail-stat"><span class="dot {S.account.id ? 'on' : 'warn'}"></span><span>{S.account.id ? "Profile linked" : "Account not linked"}</span></div>
      <div class="rail-stat"><span class="dot" class:on={!!src.inventory.source}></span><span>{src.inventory.source ? `Inventory ${ago(src.inventory.imported)}` : "No inventory"}</span></div>
      {#if src.memscan.state === "scanning"}<div class="rail-stat"><span class="dot gold"></span><span>Reading inventory {Math.round(src.memscan.progress * 100)}%</span></div>{/if}
      {#if S.desktop && S.minimizeToTray}
        <div class="rail-actions"><button type="button" class="ghost" title="Hide window to system tray" onclick={() => act("window/hide")}><Icon name="tray" /><span>Hide to tray</span></button>
          <button type="button" class="ghost icon" title="Quit warframeproject" aria-label="Quit" onclick={() => act("quit")}><Icon name="power" /></button></div>
      {/if}
    {/if}
  </div>
</aside>
