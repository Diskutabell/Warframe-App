# Changelog

## Unreleased

### Added

- Ctrl+K searches mods, resources, gear and names in loaded drop tables. Item, part and relic
  details show known drop sources and chances, with wiki links for other sources.

### Fixed

- Game-data and market caches keep usable data when a response is invalid. Cache errors are
  reported rather than silently replacing good data.

## 0.4.1 - 2026-11-06

### Changed

- **Nothing to install on Windows.** The exe carries the OCR program that reads the reward screen
  and riven cards (Tesseract, with the same English model as the Linux version) and unpacks it on
  the first start. The C runtime is built in too, so the exe also runs on a Windows without the
  Visual C++ redistributable.
- Every release comes with the Windows exe attached.

## 0.4.0 - 2026-11-06

A Windows version, and the repository is public.

### Added

- **Windows.** `warframeproject-windows-x64.exe` on the release page: the same app in a WebView2
  window, with the reward strip and riven panel as always-on-top overlays that never take focus,
  F9/F10 registered by the app itself, screenshots without any extra tool, the game log found in
  `%LOCALAPPDATA%\Warframe`, Windows notifications, and the optional memory scan. It needs
  tesseract for the scans (`winget install UB-Mannheim.TesseractOCR`) and the game in borderless
  fullscreen. Built and tested on Windows by GitHub Actions; not yet used with the real game.
- A disclaimer in the README: the code is mostly vibe coded, using it may be against Warframe's
  terms of service, and you use it at your own risk.

### Changed

- Shorter, plainer README and code comments.

## 0.3.2 - 2026-11-03

### Fixed

- **An empty Standing page after the update.** A profile saved by an earlier version has no standing
  in it, and it was only loaded again once it was half an hour old. Such a profile now counts as out
  of date and is loaded again at the next chance: when the app starts with that account shown, or
  when you switch to it. Switching to an account also loads its profile when that is out of date, one
  request within DE's rate limit as always.

## 0.3.1 - 2026-11-03

### Fixed

- **Relics without drops.** Since WFCD's data update of 2026-10-23 its relic file lists every relic
  without its rewards and without the vaulted flag. Relic values read 0p, "Crack now" and the farm
  hints had nothing to go on, every relic counted as vaulted, and the reward-screen reader no longer
  knew the names of prime parts (a real screen from before read "Forma Blueprint" alone instead of
  "Forma Blueprint" and "Orthos Prime Blueprint"). The drops now come from the game's own export, with
  the names the game prints from DE's drop tables; a relic is vaulted when no drop table names it
  today. The app does this only while WFCD's file is incomplete. Real reward screens saved before the
  break read exactly as they did then.

### Added

- The project is licensed under the MIT license: use it, change it and share it, keeping the
  copyright notice.

## 0.3.0 - 2026-11-03

Syndicate standing, and two accounts on one machine kept apart.

### Added

- **Standing page.** Every syndicate with its rank and how far into that rank you are (the way the
  game shows it), what the next rank-up costs with the items you hold, a "rank up ready" mark when a
  rank's standing is full, and a warning when a syndicate at its top rank is full and standing would
  be lost. The six faction syndicates show which others gain or lose with them.
- **Daily limits.** How much standing every daily pool still allows today (16,000 + 500 per mastery
  rank; the faction syndicates share one pool), the focus left today, and when they refill. A pool
  read before the daily reset counts as full again.
- **What standing buys.** Any syndicate opens in the side panel with its offerings; the tradable ones
  are priced on warframe.market and ranked by platinum per 1,000 standing.
- **Hand-ins.** Medallions and other items a syndicate takes for standing, with the standing they are
  worth, and whether that counts toward the daily limit.
- **Your Nightwave rank** on the World page; the Overview has a Standing card with today's limits and
  waiting rank-ups. Syndicates can be found with Ctrl+K.
- **Several accounts.** Every account that logs in on the machine has its own profile, inventory,
  hand-set counts, pins and reward history (`accounts/<name>/` in the data folder). The window
  follows the game when it logs in to another account; the switcher at the top of the sidebar shows
  any account, and Settings → Accounts links, shows and forgets them.
- The memory scan now also keeps syndicate standing, the daily limits, the star chart and intrinsics,
  so standing and mastery XP stay current after each mission.

### Fixed

- **Two accounts mixed up.** With two accounts on one machine, the app kept one account file: the
  profile of one account was shown next to the inventory read from the other, and a login with the
  other account unlinked the id without saying whose data was on screen. Each account is now kept
  apart, and existing data is moved into an account of its own on the first start; an inventory that
  belongs to another account than the saved profile is set aside instead of adopted.
- A memory read is filed under the account its mastery XP belongs to. After logging out and in with
  another account the game can still hold the first account's inventory; that copy is no longer
  filed under the account now logged in. An imported inventory file of another account is refused.
- A profile is filed under the account whose name it carries: an id linked to the wrong account
  moves to the right one instead of being dropped.
- The mastery rank is read from the profile, which does carry it from rank 1 up.
- The Overview's reward history and session totals count only the account shown.

### Changed

- Pages are reached with 1-9 and 0 (Settings); Standing sits between World and Foundry.
- The `account_id` setting is gone; ids are linked per account in Settings → Accounts.
- Game data now includes the syndicate export and its English names from
  warframe-public-export-plus (about 6 MB, cached and rechecked daily).

## 0.2.0 - 2026-10-09

Rewritten in Rust, with a new interface and new companion features. The local API keeps every
endpoint the hotkeys use, so existing binds keep working; the OCR matcher reproduces the Python
version's results exactly, checked against its own output.

### Added

- **A new interface:** sidebar navigation, a search palette for items, parts, relics and pages
  (Ctrl+K or `/`), keys 1-9 for the pages, and one side panel for any item, part or relic - its
  parts, the relics that drop them (open fissures first), prices, hand-set counts and "mark as
  mastered". Settings save as you change them.
- **Crack now:** the Overview and the Fissures page name the best relic you own for every open
  fissure: the one with the most drops you still need, then the highest value.
- **World page:** open-world cycles, Baro Ki'Teer and his stock, sortie, archon hunt, Steel Path
  reward, Nightwave challenges, Darvo's deal, daily and weekly reset timers.
- **Notifications** (all off by default): watched fissures (the bell on the Fissures page watches a
  tier, mission and kind), Baro's arrival, finished foundry builds.
- **Reward history:** every reward screen read is kept with its prices; the Overview shows the last
  ones and the session's best picks in platinum.
- **Mastery progress:** item mastery points earned and left, per category.
- **Foundry: parts and gear.** Besides equipment it lists everything else with a recipe - amp, zaw,
  kitgun, K-drive and companion parts, ciphers, restores, specters, forma, lenses - with the
  blueprint and every resource checked against your inventory, and a "Buildable at MR n" filter.
- **Pins:** the star in any details panel (and on parts-and-gear cards in the Foundry) pins it to
  the Overview with what is still missing.
- **Full mastery XP.** The Overview shows the same total as the in-game profile - gear, star chart
  nodes and junctions, Steel Path, Railjack and Drifter intrinsics - with the rank that much XP is
  enough for and what the next rank takes, instead of an estimate from gear alone. The Plexus is
  now counted as the masterable item it is.
- **The game's own art.** The Overview shows your mastery rank as the game's badge. The World page
  is rebuilt around pictures: the open-world cycles over shots of each place, Baro, Nora and Teshin,
  the archon, faction emblems, syndicate sigils on the bounties, warframe and weapon art for the
  Circuit, reward art and coloured progress bars for invasions. Fissures show relic and faction
  icons; the details panel shows resource art.
- **More timers on the World page:** bounty rotations, Prime Resurgence, events, the Circuit's
  weekly choices, Deep and Temporal Archimedea, alerts and invasions with their rewards.
- **Cut-short memory captures are refused.** The game sometimes holds only the tail of the
  inventory; applying that made whole sections drop to zero until the next read. The app learns
  which sections your captures normally have and keeps the last complete inventory instead. A
  capture of a sync that is already applied is skipped.
- Foundry timers count down to the minute; the Market page sorts spare parts by platinum, ducats or
  ducats per platinum next to the price check.

### Fixed

- **Internal names on ordinary parts.** Parts that do not come from relics showed the game's
  internal names ("Ironframe Chassis" for Hildryn's chassis, a bare "Braton" for its blueprint).
  They are now named after their item: "Hildryn Chassis", "Braton Blueprint", "Brakk Barrel".
- **Thin, hard-to-read small labels** ("CETUS", "RANK", "PLATINUM"): now in the heavier display
  face and a brighter grey.
- **Parts that cannot be traded had no icon.** Part icons came only from warframe.market, which
  lists tradable parts, so every ordinary warframe, weapon, archwing and sentinel part drew as an
  empty slot. They now get the generic icon of their slot (chassis, neuroptics, barrel, blueprint...).
- **Pixelated item art.** The window shrank 512 px pictures into small slots itself and they came
  out pixelated. The app now scales each picture once to the size it is drawn at and keeps it in
  its cache, so art also loads instantly and works without a connection.
- **EE.log lines cut in two.** The game flushes its log mid-line (9 of 10 reads on a live log), and
  each half was handled as a line: triggers and account-id lines could be missed, and a cut inside
  `Logged in <name>` unlinked the account for the session. Partial lines now wait for their end.
- **Overlapping scans corrupted each other.** F9 plus the automatic trigger (the normal case) wrote the
  same temporary images. Images now go to tesseract through stdin, and a second trigger within 3 s
  shares the first scan's result.
- **Riven weapon detection matched inside words** ("Bo" in other text, "War" in "Warframe").
- **A riven stat line lost by OCR made every grade wrong.** On a real card tesseract read
  "+57.5% Magazine" and lost the wrapped "Capacity"; the stat was dropped, the riven counted as 2+0,
  and all grades came out ~18% low (C- instead of A+/A/A+). A first word that names only one stat
  now suffices, and the positive/negative count is inferred from the values when the read count does
  not fit (the panel says how many stats were not readable). "Punch" no longer passes for "Puncture".
- **Riven stats:** all 32 regular attributes (2 were missing) and the 18 spliced ones are known; the
  longest wording wins ("Critical Chance for Slide Attack" was read as "Critical Chance"); faction
  damage is read in `+45%` and `x1.45` form; base values updated to DE's public export.
- **Mastery:** Kitgun chambers level like weapons (they were given the warframe curve); Necramechs
  rank to 40.
- The best-pick highlight went to the first card even when no price was known.
- A re-shown reward overlay showed the previous mission's rewards for up to a second.
- The overlay could be read by the next scan; overlays are hidden before every capture.
- **The memory scan's permission handling.** The Python notes said `ptrace_scope` 1 blocks it, but
  Yama also allows a reader that holds ptrace rights in the game's user namespace - which the user
  owns when Steam runs in its sandbox, as on NixOS - and a scan did succeed at scope 1. Only the
  kernel decides now: a refusal is reported in Settings and remembered for that game process, and the
  next attempt waits for the game to restart instead of failing on every mission.
- A `--port` given on the command line was saved to settings.json by the next settings change.
- Hotkey names could inject lines into the compositor config, and the screenshot command setting
  ran any program set through the local API; keys are validated and the command became a fixed choice.
- The compositor was picked by which tools were installed, not which one was running.
- The account name taken from DE's profile could keep the platform glyph.

### Changed

- **Overlays are native and use the compositor's overlay layer** (Hyprland, Sway, KDE), verified above
  the fullscreen game; X11 popups elsewhere. Setting `overlay_force_x11` became `overlay_backend`.
- **Reward scans take ~1.3 s instead of ~4.4 s:** uncompressed screenshots, no PNG intermediates,
  the four tesseract passes in parallel.
- Items map to warframe.market by the game's own id (`gameRef`), which found prices and art for 126
  parts that name matching missed.
- Game data is rechecked daily with ETags instead of re-downloaded every three days.
- The last 12 reward and riven scans are kept in `~/.cache/warframeproject/frames/` (was: only the
  last one), and `warframeproject replay FILE` re-reads them offline.
- New commands: `warframeproject quit`, `replay`; `--offline` for tests.
- Bundled fonts are TrueType (GTK cannot use WOFF2).
- Packaged for Nix (`default.nix`); `run.sh` builds and runs the checkout.
- **The memory scan is automatic only:** after login and after each mission, never on a timer.
  Settings shows what it last read and when it reads next.

### Removed

- The pywebview/Qt WebEngine shell, the old web UI and the HTML overlay pages.
- The manual "Scan game memory now" button and its `POST /api/memscan` endpoint, and the
  `memory_scan_auto` and `memory_scan_min_gap` settings: the scan runs on login and after missions.
- The `screenshot_command` setting (see Fixed).
