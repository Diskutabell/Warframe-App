<!-- Settings: where the data comes from, and how the app behaves. Every control saves on change;
     a refused value is put back. -->
<script lang="ts">
  import Countdown from "../components/Countdown.svelte";
  import GameImg from "../components/GameImg.svelte";
  import Modal from "../components/Modal.svelte";
  import Page from "../components/Page.svelte";
  import { act, pickJson } from "../lib/api";
  import { changed, poll } from "../lib/app.svelte";
  import { ago, fmt, rankText } from "../lib/format";
  import { ICON, mrBadge } from "../lib/game";
  import { loader } from "../lib/loader.svelte";
  import { toast } from "../lib/toasts.svelte";
  import type { AccountRow } from "../types/AccountRow";
  import type { Hotkeys } from "../types/Hotkeys";
  import type { HotkeysInstalled } from "../types/HotkeysInstalled";
  import type { Settings } from "../types/Settings";
  import type { Summary } from "../types/Summary";

  let { S }: { S: Summary } = $props();
  const page = loader<Settings>("settings", ["settings"]);
  const keys = loader<Hotkeys>("hotkeys", ["settings"]);
  const linux = $derived(S.os !== "windows");
  const tools = $derived(S.sources.tools);
  const SOURCE: Record<string, string> = { saved: "linked earlier", "EE.log": "read from EE.log", "entered by you": "entered by you", Settings: "set in Settings", "DE profile": "from its profile" };

  type Key = keyof Settings;
  /** Saves one setting; the page's copy follows only once the engine took it. */
  async function save(k: Key, v: unknown, put: (old: unknown) => void) {
    const s = page.data!;
    if (k === "memory_scan_enabled" && v && !(await askRisk())) {
      put(s[k]);
      return;
    }
    if (!(await act("settings", { [k]: v }))) {
      put(s[k]);
      return;
    }
    (s as Record<Key, unknown>)[k] = v;
    toast("Saved");
    if (k === "memory_scan_enabled" || k === "minimize_to_tray") await poll();
  }
  function onToggle(k: Key) {
    return (e: Event) => {
      const el = e.currentTarget as HTMLInputElement;
      save(k, el.checked, (old) => (el.checked = !!old));
    };
  }
  function onValue(k: Key, numeric = false) {
    return (e: Event) => {
      const el = e.currentTarget as HTMLInputElement | HTMLSelectElement;
      const n = Number(el.value);
      if (numeric && (el.value === "" || !Number.isFinite(n))) {
        el.value = String(page.data![k]);
        return;
      }
      save(k, numeric ? n : el.value.trim(), (old) => (el.value = String(old)));
    };
  }

  // Switching the memory scan on goes through this warning first: it can cost the account.
  let risk = $state<{ resolve: (yes: boolean) => void; agreed: boolean } | null>(null);
  const askRisk = () => new Promise<boolean>((resolve) => (risk = { resolve, agreed: false }));
  function answerRisk(yes: boolean) {
    risk?.resolve(yes);
    risk = null;
  }

  async function installKeys() {
    const r = await act<HotkeysInstalled>("hotkeys/register");
    if (r) {
      toast(!r.file ? "Hotkeys registered" : r.sourced ? `Installed: ${r.file} is sourced and reloaded` : `Updated ${r.file} and reloaded`);
      keys.refresh();
    }
  }
  let reloading = $state(false);
  async function reload() {
    reloading = true;
    await act("data/reload", {}, "Game data downloaded again");
    reloading = false;
  }
  async function importFile(path: "profile/import" | "inventory/import", ok: string) {
    const j = await pickJson();
    if (j && (await act(path, j, ok))) await changed();
  }

  // Accounts: a second press of Forget within five seconds deletes the account's data.
  let forgetting = $state<{ key: string; t: number } | null>(null);
  const confirming = (key: string) => forgetting?.key === key && Date.now() - forgetting.t < 5000;
  async function forget(a: AccountRow) {
    if (!confirming(a.key)) {
      forgetting = { key: a.key, t: Date.now() };
      return;
    }
    forgetting = null;
    if (await act("account/forget", { key: a.key }, "Account forgotten")) await changed();
  }
  let linkIds = $state<Record<string, string>>({});
  async function link(id: string, key: string | null) {
    if (await act("account/set", key ? { id, key } : { id }, "Account linked")) await changed();
  }
  async function show(key: string) {
    if (await act("account/switch", { key })) await changed();
  }
  function scrollTo(id: string) {
    document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
  }
</script>

{#snippet toggle(s: Settings, k: Key, label: string, d: string)}
  <div class="setting"><div><div class="l">{label}</div>{#if d}<div class="d">{d}</div>{/if}</div>
    <div><span class="switch"><input type="checkbox" checked={!!s[k]} aria-label={label} onchange={onToggle(k)} /><span></span></span></div></div>
{/snippet}
{#snippet number(s: Settings, k: Key, label: string, d: string, min: number, max: number)}
  <div class="setting"><div><div class="l">{label}</div>{#if d}<div class="d">{d}</div>{/if}</div>
    <div><input type="number" value={s[k]} {min} {max} step="1" aria-label={label} onchange={onValue(k, true)} /></div></div>
{/snippet}
{#snippet text(s: Settings, k: Key, label: string, d: string, placeholder: string)}
  <div class="setting"><div><div class="l">{label}</div>{#if d}<div class="d">{d}</div>{/if}</div>
    <div><input type="text" value={s[k]} {placeholder} spellcheck="false" aria-label={label} onchange={onValue(k)} /></div></div>
{/snippet}
{#snippet select(s: Settings, k: Key, label: string, d: string, options: [string, string][])}
  <div class="setting"><div><div class="l">{label}</div>{#if d}<div class="d">{d}</div>{/if}</div>
    <div><select aria-label={label} onchange={onValue(k)}>{#each options as [v, l]}<option value={v} selected={s[k] === v}>{l}</option>{/each}</select></div></div>
{/snippet}
{#snippet dot(cls: string)}<span class="dot {cls}"></span>{/snippet}

<Page title="Settings" {page}>
  {#snippet children(s)}
    {@const src = S.sources}
    {@const m = src.memscan}
    <div class="page"><div class="page-head"><div><h1>Settings</h1><p>Changes save as you make them.</p></div></div>
      <div class="settings-nav">
        {#each [["sec-accounts", "Accounts"], ["sec-sources", "Sources"], ["sec-memscan", "Memory scan"], ["sec-notifications", "Notifications"], ["sec-overlays", "Overlays"], ["sec-hotkeys", "Tray & hotkeys"], ["sec-files", "Files & platform"]] as [id, label]}
          <a href="#settings" onclick={(e) => { e.preventDefault(); scrollTo(id); }}>{label}</a>
        {/each}
      </div>
      <div class="settings">
        <div class="card" id="sec-accounts"><div class="card-head"><h2>Accounts</h2></div>
          {#if !S.accounts.length}
            <div class="empty"><b>No account yet</b>One is added when the game logs in. You can also enter your account id.</div>
            <div class="row" style="margin-top:8px"><input type="text" class="acc-id" placeholder="Account id (24 characters, 0-9 a-f)" spellcheck="false" style="width:300px" bind:value={linkIds[""]} />
              <button onclick={() => link(linkIds[""] ?? "", null)}>Link</button></div>
          {:else}
            <p class="muted" style="margin:0 0 10px;font-size:13.5px">Every account that logs in on this machine gets its own profile, inventory, counts and pins. The window follows the game: when it logs in to an account, that account is shown. Switch with the account button at the top of the sidebar.</p>
            <div class="acct-list">
              {#each S.accounts as a (a.key)}
                <div class="acct-row" class:on={a.active}>
                  <div class="row"><GameImg path={a.masteryRank == null ? ICON.mastery : mrBadge(a.masteryRankMax ?? a.masteryRank)} px={36} />
                    <div class="grow"><div class="t"><b>{a.name}</b> <span class="muted">{a.masteryRank == null ? "MR unknown" : rankText(a)}</span>
                        {#if a.active}<span class="pill gold">shown</span>{/if}{#if a.playing}<span class="pill lotus">in game</span>{/if}</div>
                      <div class="s">{#if a.id}<code>{a.id}</code> · {SOURCE[a.source ?? ""] || a.source || "linked"}{:else}<span class="warn-text">No account id yet</span>{/if}
                        · profile {a.profile ? ago(a.profile) : "not loaded"} · inventory {a.inventory ? ago(a.inventory) : "none"}</div>
                      {#if a.profileError}<div class="s warn-text">{a.profileError}</div>{/if}</div>
                    <div class="btn-group">{#if !a.active}<button onclick={() => show(a.key)}>Show</button>{/if}
                      <button class={confirming(a.key) ? "danger" : "ghost"} onclick={() => forget(a)}>{confirming(a.key) ? "Delete its data?" : "Forget"}</button></div></div>
                  {#if !a.id}
                    <div class="row" style="margin-top:8px"><input type="text" class="acc-id" placeholder="Account id (24 characters, 0-9 a-f)" spellcheck="false" style="width:300px" bind:value={linkIds[a.key]} />
                      <button onclick={() => link(linkIds[a.key] ?? "", a.key)}>Link</button></div>
                  {/if}
                  {#if a.evidence.length}
                    <div class="muted" style="font-size:13px;margin-top:8px">Ids seen in EE.log while {a.name} played:</div>
                    {#each a.evidence as e}<div class="row" style="margin-top:6px" title={e.line}><code>{e.id}</code><button class="ghost" onclick={() => link(e.id, a.key)}>Link this id</button></div>{/each}
                  {/if}
                </div>
              {/each}
            </div>
          {/if}
        </div>

        <div class="card" id="sec-sources"><div class="card-head"><h2>Sources</h2></div>
          <div class="list">
            <div class="list-row">{@render dot(src.eeLog.path ? "on" : "warn")}<div class="main"><div class="t">Game log</div>
              <div class="s" style="white-space:normal">{#if src.eeLog.path}<code>{src.eeLog.path}</code>{src.eeLog.active ? " · game running" : ""}{:else}{src.eeLog.searching ? "Searching…" : "Not found - set its path under Files."}{/if}</div></div></div>
            <div class="list-row">{@render dot(src.profile.fetched ? "on" : src.profile.error ? "warn" : "")}<div class="main">
              <div class="t">Public profile{#if S.account.name}<span class="muted" style="font-weight:400"> · {S.account.name}</span>{/if}</div>
              <div class="s" style="white-space:normal">{src.profile.fetched ? `${fmt(src.profile.items)} items with mastery · ${ago(src.profile.fetched)}` : "Not loaded"}{#if src.profile.error}<br /><span class="warn-text">{src.profile.error}</span>{/if}</div></div>
              <div class="btn-group"><button disabled={!S.account.id} onclick={() => act("profile/refresh", {}, "Profile refreshed")}>Refresh</button>
                <button onclick={() => importFile("profile/import", "Profile imported")}>Import file</button></div></div>
            <div class="list-row">{@render dot(src.inventory.source ? "on" : "")}<div class="main"><div class="t">Inventory</div>
              <div class="s" style="white-space:normal">{src.inventory.source ? `${fmt(src.inventory.stacks)} stacks · ${src.inventory.source} · ${ago(src.inventory.imported)}` : "None - parts, relics, currencies and medallions need one (memory scan or an inventory.json)."}</div></div>
              <div class="btn-group"><button onclick={() => importFile("inventory/import", "Inventory imported")}>Import file</button>
                {#if src.inventory.source}<button class="ghost" onclick={async () => { if (await act("inventory/clear", {}, "Inventory forgotten")) await changed(); }}>Forget</button>{/if}</div></div>
            <div class="list-row">{@render dot(tools.tesseract && tools.screenshot ? "on" : "warn")}<div class="main"><div class="t">Screen reader</div>
              <div class="s" style="white-space:normal">{#if tools.tesseract}tesseract{:else}<span class="warn-text">tesseract missing{#if S.os === "windows"}{" "}- install it with <code>winget install UB-Mannheim.TesseractOCR</code>, then restart the app{/if}</span>{/if}
                · {#if tools.screenshot}{tools.screenshot}{:else}<span class="warn-text">no screenshot tool</span>{/if}</div></div></div>
            <div class="list-row">{@render dot(src.market.error ? "warn" : "on")}<div class="main"><div class="t">warframe.market</div>
              <div class="s" style="white-space:normal">{src.market.error || `Live${src.market.queued ? ` · ${src.market.queued} prices queued` : ""}`}</div></div></div>
          </div>
        </div>

        <div class="card" id="sec-memscan"><div class="card-head"><h2>Memory scan</h2></div>
          <p class="muted" style="margin:0 0 6px">Reads the inventory the game has already downloaded - parts, relics, currencies, the foundry - straight from its memory, read-only, and keeps nothing else. It runs by itself after you log in and after each mission, because that is when the game holds the whole inventory. Nothing is sent anywhere. Third-party tools are used at your own risk.</p>
          {@render toggle(s, "memory_scan_enabled", "Read the inventory from game memory", "Off by default.")}
          <div class="memscan-box">
            {#if m.state === "off"}{@render dot("")}<span>Off.</span>
            {:else if m.state === "unavailable"}{@render dot("warn")}<span>{m.detail || "Not possible on this system."}</span>
            {:else if m.state === "idle"}{@render dot("")}<span>{m.detail || "Waiting for the game."}{#if m.next}{" "}Next read in <Countdown until={m.next} done="a moment" />.{/if}</span>
            {:else if m.state === "scanning"}{@render dot("gold")}<span>Reading… {Math.round(m.progress * 100)}%</span>
            {:else if m.state === "ok"}{@render dot("on")}<span>{#if m.last}Last read {ago(m.last.t)}: {fmt(m.last.stacks)} stacks in {m.last.seconds}s.{/if}{m.detail ? ` ${m.detail}` : ""}{#if m.next}{" "}Next read in <Countdown until={m.next} done="a moment" />.{/if}</span>
            {:else if m.state === "error"}{@render dot("warn")}<span><span class="warn-text">{m.detail || "Failed."}</span>{#if m.next}{" "}Next read in <Countdown until={m.next} done="a moment" />.{/if}</span>
            {:else}{@render dot("")}<span>{m.state}</span>{/if}
          </div>
        </div>

        <div class="card" id="sec-notifications"><div class="card-head"><h2>Notifications</h2></div>
          {#if !tools["notify-send"]}<div class="callout" style="margin-bottom:8px">notify-send was not found, so no notification can be shown.</div>{/if}
          <div class="setting"><div><div class="l">Watched fissures</div><div class="d">Watch a tier and mission with the bell on the Fissures page. Watching {s.fissure_watch.length}: <a href="#fissures">manage</a>.</div></div>
            <div><span class="switch"><input type="checkbox" checked={s.notify_fissures} aria-label="Watched fissures" onchange={onToggle("notify_fissures")} /><span></span></span></div></div>
          {@render toggle(s, "notify_baro", "Baro Ki'Teer arrives", "")}
          {@render toggle(s, "notify_foundry", "Foundry builds finish", "Needs an inventory.")}
          {@render toggle(s, "notify", "Every reward screen", "The overlay already shows the prices; this adds a desktop notification.")}
        </div>

        <div class="card" id="sec-overlays"><div class="card-head"><h2>Reward screen and riven overlays</h2></div>
          {@render toggle(s, "ocr_enabled", "Read the reward screen when it opens", "Triggered by the game log; the hotkey below works any time.")}
          {@render toggle(s, "overlay_enabled", "Show prices over the game", "A panel under each reward card, gone when the reward screen closes. Clicks pass through to the game.")}
          {@render select(s, "overlay_position", "Price panels", "Above the cards leaves the row under them in view, where it shows what the squad picks.", [["below", "Under the cards"], ["above", "Above the cards"]])}
          {@render number(s, "overlay_seconds", "Take the prices down at the latest after", "Seconds. They normally go with the reward screen.", 3, 120)}
          {@render toggle(s, "riven_overlay_enabled", "Show riven grades over the game", "After a riven scan with the hotkey.")}
          {@render number(s, "riven_overlay_seconds", "Hide the riven panel after", "Seconds.", 3, 300)}
          {#if linux}
            {@render select(s, "overlay_backend", "Keep overlays above the game with", "Takes effect after a restart.", [["auto", "Automatic"], ["layer-shell", "Layer shell (Hyprland, Sway, KDE)"], ["x11", "X11 window (GNOME, X11 sessions)"]])}
            {@render select(s, "screenshot_tool", "Screenshot tool", "", [["auto", "Automatic" + (tools.screenshot ? ` (${tools.screenshot})` : "")], ["grim", "grim (Hyprland, Sway)"], ["spectacle", "Spectacle (KDE)"], ["gnome-screenshot", "GNOME Screenshot"], ["maim", "maim (X11)"], ["scrot", "scrot (X11)"], ["import", "ImageMagick import (X11)"]])}
          {:else}
            <div class="callout" style="margin-top:8px">The overlays only show over the game in <b>borderless</b> or windowed mode, not in exclusive fullscreen.</div>
          {/if}
          {#if S.desktop}<div class="row" style="margin-top:12px"><button onclick={() => act("overlay/test")}>Show a test overlay</button><span class="muted" style="font-size:13px">Sample rewards, where the panels will appear for four cards.</span></div>{/if}
        </div>

        <div class="card" id="sec-hotkeys"><div class="card-head"><h2>Tray and hotkeys</h2></div>
          {@render toggle(s, "minimize_to_tray", "Minimize to system tray", linux
            ? "Closing the window hides it to your system tray / status bar instead of quitting. Click the tray icon or press the window hotkey below to bring it back."
            : "Closing the window hides it to the system tray instead of quitting. Click the tray icon or press the window hotkey below to bring it back.")}
          {@render text(s, "hotkey_window", "Show or hide the window", "Toggles the window between the tray and your screen. Clear to disable.", "")}
          {@render text(s, "hotkey_rewards", "Read the reward screen", "", "")}
          {@render text(s, "hotkey_riven", "Grade the riven on screen", "", "")}
          <div class="row" style="margin-top:12px"><button onclick={installKeys}>{linux ? "Install hotkeys" : "Register hotkeys"}</button><span class="muted" style="font-size:13px">{keys.data?.note ?? ""}</span></div>
          <pre class="lines">{(keys.data?.lines ?? []).join("\n")}</pre>
        </div>

        <div class="card" id="sec-files"><div class="card-head"><h2>Account and files</h2></div>
          {@render select(s, "platform", "Platform", "", [["pc", "PC"], ["ps4", "PlayStation"], ["xb1", "Xbox"], ["swi", "Switch"]])}
          {@render toggle(s, "profile_auto_refresh", "Refresh the public profile while playing", "DE rate-limits this; keep the interval long.")}
          {@render number(s, "profile_refresh_minutes", "Minutes between refreshes", "At least 10.", 10, 1440)}
          {@render text(s, "ee_log_path", "EE.log path", linux ? "Empty: found automatically in Steam, Flatpak, Snap and ~/Games prefixes." : "Empty: %LOCALAPPDATA%\\Warframe\\EE.log.", src.eeLog.path || "")}
          {@render text(s, "inventory_json_path", "Watch an inventory.json", "Reloaded whenever the file changes.", "")}
          {@render number(s, "price_cache_minutes", "Keep bulk prices for", "Minutes. Reward-screen prices refresh after 15 minutes regardless.", 5, 1440)}
          {@render number(s, "port", "Engine port", "Takes effect after a restart.", 1024, 65535)}
        </div>

        <div class="row"><button disabled={reloading} onclick={reload}>Re-download game data</button>
          {#if S.desktop}<button class="ghost" onclick={() => act("quit")}>Quit app</button>{/if}
          <span class="faint" style="font-size:13px">warframeproject {S.version}</span></div>
      </div>
    </div>
  {/snippet}
</Page>

{#if risk}
  <Modal label="Read this before you turn it on" onclose={() => answerRisk(false)}>
    <div class="modal-head"><div class="grow"><h2>Read this before you turn it on</h2></div></div>
    <div class="modal-notes">
      <p><b>Memory scanning is a big risk.</b> Only turn it on if you know what you are doing: it may lead to a
        <b>permanent suspension of your Warframe account</b>, and of any account you make after it.</p>
      <p><b>This is a real warning.</b> It can happen to you.</p>
      {#if S.os === "windows"}<p>On Windows the read means opening the game with read rights, which the game can see.</p>{/if}
      <p>Digital Extremes' EULA forbids third-party programs that collect information about the game. You use this
        entirely at your own risk; nobody can get a suspended account back for you.</p>
      <label class="row" style="gap:8px;margin:12px 0 4px;cursor:pointer"><input type="checkbox" bind:checked={risk.agreed} />
        <span>I have read and understand the risks</span></label>
    </div>
    <div class="modal-foot"><button type="button" onclick={() => answerRisk(false)}>Cancel</button>
      <button type="button" class="primary" disabled={!risk.agreed} onclick={() => answerRisk(true)}>Turn it on</button></div>
  </Modal>
{/if}
