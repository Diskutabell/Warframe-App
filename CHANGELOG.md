# Changelog

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
- **Mastery progress:** item mastery points earned and left, per category, and what the levelled
  gear alone is worth in mastery rank.
- **Foundry: parts and gear.** Besides equipment it lists everything else with a recipe - amp, zaw,
  kitgun, K-drive and companion parts, ciphers, restores, specters, forma, lenses - with the
  blueprint and every resource checked against your inventory, and a "Buildable at MR n" filter.
- Foundry timers count down to the minute; the Market page sorts spare parts by platinum, ducats or
  ducats per platinum next to the price check.

### Fixed

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
