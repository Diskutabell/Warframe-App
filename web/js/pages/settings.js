// Settings: where the data comes from, and how the app behaves. Every control saves on change.

import { $, act, ago, api, countdown, esc, fmt, pickJson, toast } from "../lib.js";
import { gimg, ICON, mrBadge } from "../game.js";

let s = {};

const dot = (cls) => `<span class="dot ${cls}"></span>`;
const SOURCE = { saved: "linked earlier", "EE.log": "read from EE.log", "entered by you": "entered by you", Settings: "set in Settings", "DE profile": "from its profile" };
const desc = (d) => (d ? `<div class="d">${d}</div>` : "");
const setting = (label, d, control) => `<div class="setting"><div><div class="l">${label}</div>${desc(d)}</div><div>${control}</div></div>`;

const toggle = (k, label, d) => setting(label, d,
  `<span class="switch"><input type="checkbox" data-k="${k}" ${s[k] ? "checked" : ""} aria-label="${esc(label)}"><span></span></span>`);
const number = (k, label, d, { min = 0, max = "", step = 1 } = {}) => setting(label, d,
  `<input type="number" data-k="${k}" value="${esc(s[k])}" min="${min}" max="${max}" step="${step}" aria-label="${esc(label)}">`);
const text = (k, label, d, placeholder = "") => setting(label, d,
  `<input type="text" data-k="${k}" value="${esc(s[k])}" placeholder="${esc(placeholder)}" spellcheck="false" aria-label="${esc(label)}">`);
const select = (k, label, d, options) => setting(label, d,
  `<select data-k="${k}" aria-label="${esc(label)}">${options.map(([v, l]) => `<option value="${esc(v)}" ${s[k] === v ? "selected" : ""}>${esc(l)}</option>`).join("")}</select>`);

const section = (title, body, id = "") => `<div class="card" ${id ? `id="${id}"` : ""}><div class="card-head"><h2>${title}</h2></div>${body}</div>`;

// ------------------------------------------------------------------ live status blocks

function sources(S) {
  const src = S.sources;
  const acc = S.account;
  const tools = src.tools || {};
  const row = (cls, title, detail, actions = "") => `<div class="list-row">${dot(cls)}<div class="main"><div class="t">${title}</div><div class="s" style="white-space:normal">${detail}</div></div>${actions}</div>`;
  return `<div class="list">
    ${row(src.eeLog.path ? "on" : "warn", "Game log", src.eeLog.path ? `<code>${esc(src.eeLog.path)}</code>${src.eeLog.active ? " · game running" : ""}` : src.eeLog.searching ? "Searching…" : "Not found - set its path under Files.")}
    ${row(src.profile.fetched ? "on" : src.profile.error ? "warn" : "", `Public profile${acc.name ? ` <span class="muted" style="font-weight:400">· ${esc(acc.name)}</span>` : ""}`,
      (src.profile.fetched ? `${fmt(src.profile.items)} items with mastery · ${ago(src.profile.fetched)}` : "Not loaded") + (src.profile.error ? `<br><span class="warn-text">${esc(src.profile.error)}</span>` : ""),
      `<button data-act="profile" ${acc.id ? "" : "disabled"}>Refresh</button><button data-act="profileFile">Import file</button>`)}
    ${row(src.inventory.source ? "on" : "", "Inventory",
      src.inventory.source ? `${fmt(src.inventory.stacks)} stacks · ${esc(src.inventory.source)} · ${ago(src.inventory.imported)}` : "None - parts, relics, currencies and medallions need one (memory scan or an inventory.json).",
      `<button data-act="inventory">Import file</button>${src.inventory.source ? `<button data-act="forget" class="ghost">Forget</button>` : ""}`)}
    ${row(tools.tesseract && tools.screenshot ? "on" : "warn", "Screen reader",
      `${tools.tesseract ? "tesseract" : `<span class="warn-text">tesseract missing${S.os === "windows" ? " - install it with <code>winget install UB-Mannheim.TesseractOCR</code>, then restart the app" : ""}</span>`} · ${tools.screenshot ? esc(tools.screenshot) : `<span class="warn-text">no screenshot tool</span>`}`)}
    ${row(src.market.error ? "warn" : "on", "warframe.market", src.market.error ? esc(src.market.error) : `Live${src.market.queued ? ` · ${src.market.queued} prices queued` : ""}`)}
  </div>`;
}

// ------------------------------------------------------------------ accounts

/** The account whose Forget button was pressed once, and when: a second press deletes it. */
let forgetting = { key: null, t: 0 };

function accounts(S) {
  const list = S.accounts || [];
  const linkForm = (key) => `<div class="row" style="margin-top:8px"><input type="text" class="acc-id" data-for="${esc(key || "")}" placeholder="Account id (24 characters, 0-9 a-f)" spellcheck="false" style="width:300px">
      <button data-link="${esc(key || "")}">Link</button></div>`;
  if (!list.length) {
    return `<div class="empty"><b>No account yet</b>One is added when the game logs in. You can also enter your account id.</div>${linkForm(null)}`;
  }
  const confirming = (key) => forgetting.key === key && Date.now() - forgetting.t < 5000;
  const one = (a) => {
    const badge = gimg(a.masteryRank == null ? ICON.mastery : mrBadge(a.masteryRankMax ?? a.masteryRank), 36);
    const rank = a.masteryRank == null ? "MR unknown" : a.masteryRankMax != null && a.masteryRankMax !== a.masteryRank ? `MR ${a.masteryRank}-${a.masteryRankMax}` : `MR ${a.masteryRank}`;
    const ids = (a.evidence || []).map((e) => `<div class="row" style="margin-top:6px" title="${esc(e.line)}"><code>${esc(e.id)}</code><button class="ghost" data-use="${esc(e.id)}" data-for="${esc(a.key)}">Link this id</button></div>`).join("");
    return `<div class="acct-row ${a.active ? "on" : ""}">
      <div class="row">${badge}
        <div class="grow"><div class="t"><b>${esc(a.name)}</b> <span class="muted">${rank}</span>
            ${a.active ? `<span class="pill gold">shown</span>` : ""}${a.playing ? `<span class="pill lotus">in game</span>` : ""}</div>
          <div class="s">${a.id ? `<code>${esc(a.id)}</code> · ${esc(SOURCE[a.source] || a.source || "linked")}` : `<span class="warn-text">No account id yet</span>`}
            · profile ${a.profile ? ago(a.profile) : "not loaded"} · inventory ${a.inventory ? ago(a.inventory) : "none"}</div>
          ${a.profileError ? `<div class="s warn-text">${esc(a.profileError)}</div>` : ""}</div>
        ${a.active ? "" : `<button data-show="${esc(a.key)}">Show</button>`}
        <button class="${confirming(a.key) ? "danger" : "ghost"}" data-forget="${esc(a.key)}">${confirming(a.key) ? "Delete its data?" : "Forget"}</button></div>
      ${a.id ? "" : linkForm(a.key)}
      ${ids ? `<div class="muted" style="font-size:13px;margin-top:8px">Ids seen in EE.log while ${esc(a.name)} played:</div>${ids}` : ""}
    </div>`;
  };
  return `<p class="muted" style="margin:0 0 10px;font-size:13.5px">Every account that logs in on this machine gets its own profile, inventory, counts and pins. The window follows the game: when it logs in to an account, that account is shown. Switch with the account button at the top of the sidebar.</p>
    <div class="acct-list">${list.map(one).join("")}</div>`;
}

function memscan(S) {
  const m = S.sources.memscan || {};
  const last = m.last ? `Last read ${ago(m.last.t)}: ${fmt(m.last.stacks)} stacks in ${m.last.seconds}s.` : "";
  const next = m.next ? ` Next read in ${countdown(m.next, "a moment")}.` : "";
  const line = {
    off: [dot(""), "Off."],
    unavailable: [dot("warn"), esc(m.detail || "Not possible on this system.")],
    idle: [dot(""), (esc(m.detail || "Waiting for the game.")) + next],
    scanning: [dot("gold"), `Reading… ${Math.round((m.progress || 0) * 100)}%`],
    ok: [dot("on"), `${last}${m.detail ? ` ${esc(m.detail)}` : ""}${next}`],
    error: [dot("warn"), `<span class="warn-text">${esc(m.detail || "Failed.")}</span>${next}`],
  }[m.state] || [dot(""), esc(m.state || "")];
  return `<div class="row" style="align-items:flex-start;padding:10px 12px;border-radius:6px;background:var(--bg-2);border:1px solid var(--line)">
    <span style="margin-top:7px">${line[0]}</span><span>${line[1]}</span></div>`;
}

// ------------------------------------------------------------------ page

export default {
  id: "settings",
  title: "Settings",
  icon: "settings",
  async render(view, params, app) {
    s = await api("settings");
    const S = app.state;
    const tools = S.sources.tools || {};
    // Windows copies the screen itself and has one kind of overlay, so those choices are Linux's.
    const linux = S.os !== "windows";
    view.innerHTML = `<div class="page"><div class="page-head"><div><h1>Settings</h1><p>Changes save as you make them.</p></div></div>
    <div class="settings">
      ${section("Accounts", `<div id="accounts">${accounts(S)}</div>`)}

      ${section("Sources", `<div id="sources">${sources(S)}</div>`)}

      ${section("Memory scan", `
        <p class="muted" style="margin:0 0 6px">Reads the inventory the game has already downloaded - parts, relics, currencies, the foundry - straight from its memory, read-only, and keeps nothing else. It runs by itself after you log in and after each mission, because that is when the game holds the whole inventory. Nothing is sent anywhere. Third-party tools are used at your own risk.</p>
        ${toggle("memory_scan_enabled", "Read the inventory from game memory", "Off by default.")}
        <div id="memscan">${memscan(S)}</div>`)}

      ${section("Notifications", `
        ${tools["notify-send"] ? "" : `<div class="callout" style="margin-bottom:8px">notify-send was not found, so no notification can be shown.</div>`}
        ${toggle("notify_fissures", "Watched fissures", `Watch a tier and mission with the bell on the Fissures page. Watching ${s.fissure_watch.length}: <a href="#fissures">manage</a>.`)}
        ${toggle("notify_baro", "Baro Ki'Teer arrives", "")}
        ${toggle("notify_foundry", "Foundry builds finish", "Needs an inventory.")}
        ${toggle("notify", "Every reward screen", "The overlay already shows the prices; this adds a desktop notification.")}`)}

      ${section("Reward screen and riven overlays", `
        ${toggle("ocr_enabled", "Read the reward screen when it opens", "Triggered by the game log; the hotkey below works any time.")}
        ${toggle("overlay_enabled", "Show prices over the game", "A panel under each reward card, gone when the reward screen closes. Clicks pass through to the game.")}
        ${select("overlay_position", "Price panels", "Above the cards leaves the row under them in view, where it shows what the squad picks.", [["below", "Under the cards"], ["above", "Above the cards"]])}
        ${number("overlay_seconds", "Take the prices down at the latest after", "Seconds. They normally go with the reward screen.", { min: 3, max: 120 })}
        ${toggle("riven_overlay_enabled", "Show riven grades over the game", "After a riven scan with the hotkey.")}
        ${number("riven_overlay_seconds", "Hide the riven panel after", "Seconds.", { min: 3, max: 300 })}
        ${linux ? select("overlay_backend", "Keep overlays above the game with", "Takes effect after a restart.", [["auto", "Automatic"], ["layer-shell", "Layer shell (Hyprland, Sway, KDE)"], ["x11", "X11 window (GNOME, X11 sessions)"]]) : ""}
        ${linux ? "" : `<div class="callout" style="margin-top:8px">The overlays only show over the game in <b>borderless</b> or windowed mode, not in exclusive fullscreen.</div>`}
        ${!linux ? "" : select("screenshot_tool", "Screenshot tool", "", [["auto", "Automatic" + (tools.screenshot ? ` (${tools.screenshot})` : "")], ["grim", "grim (Hyprland, Sway)"], ["spectacle", "Spectacle (KDE)"], ["gnome-screenshot", "GNOME Screenshot"], ["maim", "maim (X11)"], ["scrot", "scrot (X11)"], ["import", "ImageMagick import (X11)"]])}
        ${S.desktop ? `<div class="row" style="margin-top:12px"><button id="testOverlay">Show a test overlay</button><span class="muted" style="font-size:13px">Sample rewards, where the panels will appear for four cards.</span></div>` : ""}`)}

      ${section("Hotkeys", `
        ${text("hotkey_rewards", "Read the reward screen", "")}
        ${text("hotkey_riven", "Grade the riven on screen", "")}
        <div class="row" style="margin-top:12px"><button id="installKeys">${linux ? "Install hotkeys" : "Register hotkeys"}</button><span class="muted" id="keysNote" style="font-size:13px"></span></div>
        <pre class="lines" id="keysLines"></pre>`)}

      ${section("Account and files", `
        ${select("platform", "Platform", "", [["pc", "PC"], ["ps4", "PlayStation"], ["xb1", "Xbox"], ["swi", "Switch"]])}
        ${toggle("profile_auto_refresh", "Refresh the public profile while playing", "DE rate-limits this; keep the interval long.")}
        ${number("profile_refresh_minutes", "Minutes between refreshes", "At least 10.", { min: 10, max: 1440 })}
        ${text("ee_log_path", "EE.log path", linux ? "Empty: found automatically in Steam, Flatpak, Snap and ~/Games prefixes." : "Empty: %LOCALAPPDATA%\\Warframe\\EE.log.", S.sources.eeLog.path || "")}
        ${text("inventory_json_path", "Watch an inventory.json", "Reloaded whenever the file changes.")}
        ${number("price_cache_minutes", "Keep bulk prices for", "Minutes. Reward-screen prices refresh after 15 minutes regardless.", { min: 5, max: 1440 })}
        ${number("port", "Engine port", "Takes effect after a restart.", { min: 1024, max: 65535 })}`)}

      <div class="row"><button id="reload">Re-download game data</button><span class="faint" style="font-size:13px">warframeproject ${esc(S.version || "")}</span></div>
    </div></div>`;

    // Each control saves itself; a refused value is put back.
    for (const el of view.querySelectorAll("[data-k]")) {
      el.addEventListener("change", async () => {
        const k = el.dataset.k;
        let v = el.type === "checkbox" ? el.checked : el.type === "number" ? Number(el.value) : el.value.trim();
        if (el.type === "number" && (el.value === "" || !Number.isFinite(v))) { el.value = s[k]; return; }
        const ok = await act("settings", { [k]: v });
        if (!ok) {
          if (el.type === "checkbox") el.checked = !!s[k]; else el.value = s[k];
          return;
        }
        s[k] = v;
        toast("Saved");
        if (k === "memory_scan_enabled") {
          const S2 = await api("state").catch(() => null);
          if (S2 && $("#memscan")) $("#memscan").innerHTML = memscan(S2);
        }
        if (k.startsWith("hotkey_")) showKeys();
      });
    }

    const showKeys = async () => {
      const h = await api("hotkeys").catch(() => null);
      if (!h || !$("#keysLines")) return;
      $("#keysNote").textContent = h.note || "";
      $("#keysLines").textContent = (h.lines || []).join("\n");
    };
    showKeys();
    $("#installKeys").onclick = async () => {
      const r = await act("hotkeys/register");
      if (r) { toast(!r.file ? "Hotkeys registered" : r.sourced ? `Installed: ${r.file} is sourced and reloaded` : `Updated ${r.file} and reloaded`); showKeys(); }
    };
    $("#reload").onclick = async (e) => {
      e.target.disabled = true;
      await act("data/reload", {}, "Game data downloaded again");
      e.target.disabled = false;
    };
    const test = $("#testOverlay");
    if (test) test.onclick = () => act("overlay/test");
    wireSources(view);
    wireAccounts($("#accounts"));
  },

  onState(S) {
    if (!$("#memscan")) return;
    $("#memscan").innerHTML = memscan(S);
    $("#sources").innerHTML = sources(S);
    wireSources($("#sources"));
    // Leave the account forms alone while someone is typing in one.
    if (!document.activeElement?.classList.contains("acc-id")) {
      $("#accounts").innerHTML = accounts(S);
      wireAccounts($("#accounts"));
    }
  },
};

async function redrawAccounts() {
  const S = await api("state").catch(() => null);
  if (S && $("#accounts")) {
    $("#accounts").innerHTML = accounts(S);
    wireAccounts($("#accounts"));
  }
}

function wireAccounts(root) {
  for (const b of root.querySelectorAll("[data-link]")) {
    b.onclick = async () => {
      const input = b.parentElement.querySelector(".acc-id");
      const key = b.dataset.link || undefined;
      if (await act("account/set", { id: input.value, key }, "Account linked")) redrawAccounts();
    };
  }
  for (const b of root.querySelectorAll("[data-use]")) {
    b.onclick = async () => { if (await act("account/set", { id: b.dataset.use, key: b.dataset.for }, "Account linked")) redrawAccounts(); };
  }
  for (const b of root.querySelectorAll("[data-show]")) {
    b.onclick = async () => { if (await act("account/switch", { key: b.dataset.show })) window.dispatchEvent(new CustomEvent("wfp:account")); };
  }
  for (const b of root.querySelectorAll("[data-forget]")) {
    b.onclick = async () => {
      const key = b.dataset.forget;
      if (forgetting.key !== key || Date.now() - forgetting.t > 5000) {
        forgetting = { key, t: Date.now() };
        b.textContent = "Delete its data?";
        b.className = "danger";
        return;
      }
      forgetting = { key: null, t: 0 };
      if (await act("account/forget", { key }, "Account forgotten")) window.dispatchEvent(new CustomEvent("wfp:account"));
    };
  }
}

function wireSources(root) {
  const on = (sel, fn) => { const el = root.querySelector(sel); if (el) el.onclick = fn; };
  on("[data-act=profile]", () => act("profile/refresh", {}, "Profile refreshed"));
  on("[data-act=profileFile]", async () => { const j = await pickJson(); if (j) act("profile/import", j, "Profile imported"); });
  on("[data-act=inventory]", async () => { const j = await pickJson(); if (j) act("inventory/import", j, "Inventory imported"); });
  on("[data-act=forget]", () => act("inventory/clear", {}, "Inventory forgotten"));
}
