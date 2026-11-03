# Warframe App

A Warframe companion for Linux. It tracks what you still need for mastery and prime sets, follows
your syndicate standing and what is left of today's limits, values your relics and names the best
one to crack in the fissures open right now, shows the world timers and warframe.market prices, and
puts overlays over the game: the fissure reward screen priced the moment it opens, and riven cards
graded on a key press. Several accounts on one machine are kept apart.

It is one native program written in Rust: a GTK window with the app's interface, and in-game
overlays drawn by the desktop compositor. No Overwolf, no Wine, no browser tab. Game memory is
only read if you switch that on.

> Fan project. Not affiliated with or endorsed by Digital Extremes.

## Features

- **Overview** - your mastery rank and mastery XP (the same total the in-game profile shows),
  currencies, the relics worth cracking in open fissures, world timers, the foundry, pinned items
  and the last reward screens with what they were worth.
- **Reward screen overlay** - when the relic reward screen opens, a strip under the cards shows
  each item's platinum and ducat value and whether you own or still need it; the most valuable
  pick is outlined. Reads the screen in about 1.3 s.
- **Riven grades** - press a key with a riven card on screen and a panel beside it grades every
  stat against the weapon's disposition, and says so when a value cannot be right.
- **Standing** - every syndicate's rank and how far into it you are, what the next rank-up costs
  (with the items you hold), how much standing each daily pool still allows today and when it
  resets, medallions and other hand-ins you hold, a warning when a maxed syndicate is full, your
  Nightwave rank, and what each syndicate's standing buys ranked by platinum per 1,000 standing.
- **Several accounts** - every account that logs in on the machine gets its own profile,
  inventory, hand-set counts, pins and reward history. The window follows the game when it logs
  in to another account; the switcher at the top of the sidebar shows any of them.
- **Mastery** - everything that gives mastery, filtered by status, category, rank, primes, vaulted
  or ready to build, with the parts you hold and where to farm the rest.
- **Prime sets and relics** - each set's parts and price; every relic's chance-weighted platinum
  value, the drops you still need, and which relics can be cracked now.
- **Fissures** - live fissures (normal, Steel Path, Void Storms) with the best relic you own for
  each, and a bell to be notified when a tier and mission you watch opens.
- **World** - open-world cycles, Baro Ki'Teer and his stock, sortie, archon hunt, bounties, the
  Circuit, Archimedea, alerts, invasions, Steel Path, Nightwave, Darvo and the reset timers, with
  the game's own art.
- **Foundry** - what is building and when it is done; what you could build from what you hold.
- **Market** - your spare prime parts in platinum and ducats, and a price check for any item.
- **Search and details** - Ctrl+K finds any item, part, relic or page; any of them opens in a side
  panel with its parts, the relics that drop them (open fissures first) and prices.
- **Notifications** (all off by default) - watched fissures, Baro's arrival, finished builds.

## Requirements

- Linux on Wayland (Hyprland, Sway, KDE, GNOME) or X11.
- Warframe through Steam/Proton, or a Lutris, Heroic, umu or plain Wine prefix. The app reads the
  game's own log (`EE.log`) from the prefix.
- GTK 3, WebKitGTK 4.1 and gtk-layer-shell.
- `tesseract` with English data, and a screenshot tool for the reward and riven scans:

  | Desktop | Screenshot tool |
  |---|---|
  | Hyprland, Sway | `grim` |
  | KDE | `spectacle` |
  | GNOME | `gnome-screenshot` |
  | X11 | `maim`, `scrot` or ImageMagick `import` |

- Optional: `notify-send` for desktop notifications.

## Installation

### NixOS / Nix

```sh
git clone git@github.com:Diskutabell/Warframe-App.git
cd Warframe-App
nix-env -f . -i          # installs `warframeproject` with a menu entry
```

To try it without installing, run `nix-build && ./result/bin/warframeproject`. On NixOS you can
also add `(pkgs.callPackage /path/to/Warframe-App { })` to `environment.systemPackages`. The
package brings tesseract and grim with it.

### Other distributions

Install Rust (cargo), the development packages for GTK 3, WebKitGTK 4.1 (`webkit2gtk-4.1`) and
gtk-layer-shell, tesseract with English data and a screenshot tool, then:

```sh
git clone git@github.com:Diskutabell/Warframe-App.git
cd Warframe-App
./run.sh                 # builds on first start (a few minutes), then opens the app
```

`./run.sh --detach` gives the terminal back (log in `~/.cache/warframeproject/app.log`) and
`./run.sh quit` closes the app. `run.sh` rebuilds only when the sources changed. To put it in your
app launcher:

```sh
sed "s|/path/to/warframeproject|$PWD|" warframeproject.desktop > ~/.local/share/applications/warframeproject.desktop
```

## Getting started

1. **Start the app, then Warframe.** The app finds `EE.log` by itself and reads your name and mastery
   rank from it. Launching the app again while it runs brings its window forward.
2. **Link your account** for mastery data (see [Linking your account](#linking-your-account)).
3. **Install the hotkeys** under Settings → Hotkeys (Hyprland and Sway; elsewhere bind the
   commands below yourself).
4. Optional: give the app your **inventory** - parts, relics, currencies and the foundry - by
   importing an `inventory.json` or switching on the [memory scan](#memory-scan-optional).

## Using it

| Page | What it is for |
|---|---|
| Overview | Your account at a glance |
| Mastery | Everything that gives mastery, with filters |
| Prime sets | Each set's parts, what you hold and the set price |
| Relics | Your relics with their value, needed drops and which can be cracked now |
| Fissures | Live fissures with the best relic you own for each; the bell watches a tier and mission |
| World | Cycles, vendors, rotations and reset timers |
| Standing | Syndicate ranks, rank-up costs, today's limits, hand-ins; a syndicate opens with its offerings |
| Foundry | What is building, and everything you could build from what you hold |
| Market | Spare prime parts in platinum and ducats; a price check |
| Settings | Sources, memory scan, notifications, overlays, hotkeys, files |

**Ctrl+K** (or `/`) searches items, parts, relics, syndicates and pages; **1-9** and **0** switch
pages; **Esc** closes.
The side panel of an item is also where you correct a count, mark an item as mastered by hand, or
pin it to the Overview with the star.

### Hotkeys

| Key (default) | Does | Command it runs |
|---|---|---|
| F9 | Scan the relic reward screen | `curl -s -X POST http://127.0.0.1:8642/api/rewards/scan` |
| F10 | Grade the riven card on screen | `curl -s -X POST http://127.0.0.1:8642/api/riven/scan` |

The reward screen is also scanned automatically when the game logs that it opened; the game writes
its log in bursts, so that can arrive a second late, while F9 is instant. Pressing F9 during the
automatic scan does not scan twice. `warframeproject trigger` and `warframeproject riven` do the
same from a terminal and print the result.

### Overlays

On Hyprland, Sway and KDE the overlays use the compositor's overlay layer (wlr-layer-shell), which
stays above fullscreen games; on GNOME and X11 they are X11 popup windows. They never take keyboard
focus, hide themselves after a while, and stay open while hovered. Settings → *Show a test overlay*
shows sample rewards where the strip will appear.

## Linking your account

Mastery comes from DE's public profile, which needs your account id: a 24-character code like
`505875fb1a4d80894a000000`. The app takes it from `EE.log` and **never** sends test requests to DE
to find it: DE rate-limits that service, and too many requests (especially with wrong ids) can get
your IP temporarily blocked from logging into the game.

- If a log line names you and holds exactly one id, or your client sends its own fissure reward to
  the host (play a relic mission in a squad, not as host), that id is linked automatically.
- Otherwise the ids that might be yours are listed in Settings to pick from, or paste yours there.

Profile requests are limited to one every 5 minutes, plus one when the app starts and the saved
profile is older than the refresh interval.

### Several accounts

Each name the game logs in with (`Logged in <name>` in `EE.log`) is an account of its own, with
its own id, profile, inventory, counts, pins and reward history. Link each account's id once, in
Settings → Accounts. When the game logs in to an account, the window shows it; the switcher at the
top of the sidebar shows any other. What the game produces - the inventory read from memory, reward
screens, the rank badge, ids in the log - always goes to the account that is logged in.

The game can hold the previous account's inventory in memory after you log out and in with
another one. A memory read is therefore filed by its mastery XP, which differs between any two
accounts: a copy that belongs to another account is never filed under the one logged in. An
imported inventory file is checked the same way. A profile is filed under the account whose name
it carries, so an id linked to the wrong account moves to the right one.

## Data sources

| Source | What it gives | How |
|---|---|---|
| [WFCD warframe-items](https://github.com/WFCD/warframe-items) | Items, prime parts, relics, drop tables | Downloaded and cached; rechecked daily |
| `EE.log` | Your name and mastery rank badge, when the reward screen opens, when a mission ends | A plain read of the game's own log |
| DE public profile | Mastery XP per item, completed star chart nodes, intrinsics, mastery rank, syndicate standing and today's limits | What the in-game profile screen shows other players |
| [warframe-public-export-plus](https://github.com/calamity-inc/warframe-public-export-plus) | The mastery XP of each star chart node; every syndicate's ranks, rank-up costs, daily pools, offerings and medallions, with their English names | Downloaded and cached; rechecked daily |
| [browse.wf](https://browse.wf) | The game's own interface art | Fetched once per picture, scaled and cached |
| [warframe.market](https://warframe.market) | Prices, ducat values, item art | Public API, at most 3 requests a second |
| [api.warframestat.us](https://api.warframestat.us) | Fissures, world timers, traded riven prices | Public API, one fetch a minute |
| Reward screen OCR | The items on the fissure reward screen | Screenshot, tesseract, fuzzy matching |
| Manual tracking | Part and relic counts, "mark as mastered" | The +/- buttons and the button in an item's panel |
| `inventory.json` | The exact inventory: parts, relics, currencies, foundry, medallions, standing | A file you import, or one the app watches |
| Memory scan (off) | The same, read from the running game | See below |

Your **mastery rank** comes from your profile, or from `EE.log`, where the squad overlay loads your
rank badge (one badge per three ranks above rank 0, so the log alone gives a band such as 16-18).
Your **mastery XP** is the total the in-game profile shows: gear, star chart, Steel Path and
intrinsics. The Overview shows it with the rank that much XP is enough for, which can be above the
rank you hold, since every rank needs its test.

## Syndicate standing

Standing comes from your profile and, when the memory scan is on, from the inventory read after
each mission; the newer of the two wins. A syndicate's standing counts up over all its ranks, and
each rank has a range: the Standing page shows how far into its rank you are, the way the game does.
Every limit pool (the six faction syndicates share one; the open-world and other syndicates have
their own) allows 16,000 + 500 × your mastery rank per day, and refills at 00:00 UTC. Focus has its
own limit, 250,000 + 5,000 × your mastery rank.

## Memory scan (optional)

No public endpoint returns your inventory; the tools that keep an exact inventory automatically
(Overwolf's AlecaFrame, and on Linux Merframe, FrameForge-Linux, WFHelper, warframe-lite) read it
from the game's memory. Here it is a last resort:

- **Off by default.** Switch it on under Settings → Memory scan, which shows what it last read and
  when it reads next.
- **Read-only**, through `/proc/<pid>/mem`. It never writes to or injects into the game.
- **Automatic, never on a timer:** shortly after you log in and after each mission, when the game
  holds the whole inventory. There is no manual scan.
- Only known inventory fields are kept. The same memory holds your session token; it is discarded
  unread.
- A capture that lost sections your inventory normally has is refused, so a half-read never makes
  items vanish.
- Whether Linux allows the read is the kernel's decision. With `kernel.yama.ptrace_scope` 1 (the
  usual default) it works when the game runs in a user namespace you own, as it does in Steam's
  sandbox on NixOS. If Linux refuses, Settings says so; the app never changes system settings.

Digital Extremes' published position on third-party software is that it is used *at your own
risk*.

## Riven grades

A grade compares the roll with an average roll for the weapon's disposition (2.5-point bands
across the ±10% window: A+ ... C-). It says nothing about whether the stats are useful. All 32
regular riven attributes and the 18 the Riven Splicer can add are known, with base values from DE's
public export. If a value falls outside ±10%, the panel says the stat count or weapon was misread
instead of printing a confident wrong grade.

## Files

| What | Where |
|---|---|
| Settings | `~/.config/warframeproject/settings.json` |
| Accounts (`accounts.json`), each account's profile, inventory and counts (`accounts/<name>/`), reward history | `~/.local/share/warframeproject/` |
| Game data, prices, art, fonts | `~/.cache/warframeproject/` |
| The last 12 reward and riven scans | `~/.cache/warframeproject/frames/` (screenshot, OCR text, result) |

Nothing is sent anywhere except the requests listed under [Data sources](#data-sources).

## Troubleshooting

| Problem | Fix |
|---|---|
| A scan read the wrong items | Replay its frame: `warframeproject replay ~/.cache/warframeproject/frames/reward-....png` (`--riven` for a riven). Attach that PNG to a bug report; a pasted screenshot is cropped and rescaled. |
| A dock or launcher starts an old version after an update | It stored the full `/nix/store/...` path of the old build. Point it at `warframeproject`, or pin the app again. |
| The window stays empty | Start it from a terminal and read the output; `WARFRAMEPROJECT_DEBUG=1` enables the web inspector. |
| The overlay appears behind the game | Settings → *Keep the overlays above the game with*: try the other option, then restart. Run Warframe fullscreen or borderless through the compositor, not exclusive fullscreen. |
| "Port 8642 is in use" | Change `"port"` in settings.json or pass `--port` (and install the hotkeys again). |
| No account detected | Play a relic mission in a squad (not as host), or paste your id in Settings. |

## Development

```sh
nix-shell                                        # toolchain, libraries, tesseract, grim
cargo test
cargo run -- --headless --offline --port 8650    # the engine alone, without network
```

`--browser` serves the interface to your web browser instead of opening a window (for debugging).
The interface in `web/` is plain HTML, CSS and ES modules without a build step: release builds
embed it, debug builds read it from disk, so a reload shows an edit.

| Path | Role |
|---|---|
| `src/main.rs` | Command line, startup, single instance |
| `src/state.rs` | Merges the sources, follows the logged-in account, scans, notifications |
| `src/accounts.rs` | The accounts on the machine, their folders, telling captures apart by mastery XP |
| `src/standing.rs` | Syndicates from the game's export, rank progress, daily limits, offerings |
| `src/views.rs` | Everything the interface shows, computed from a snapshot |
| `src/server.rs` | The loopback HTTP API and the embedded interface |
| `src/eelog.rs` | Finding and following `EE.log` |
| `src/sources.rs`, `src/memscan.rs` | Profile and inventory parsing, the memory scan |
| `src/gamedata.rs`, `src/market.rs`, `src/world.rs` | Game data, prices, world state |
| `src/ocr/` | Screenshots, image preprocessing, tesseract, the name matcher, riven parsing |
| `src/rivens.rs` | Riven stats and grading |
| `src/desktop/` | The GTK window and the overlays |
| `web/` | The interface: `index.html`, `css/`, `js/` (one module per page) |
| `default.nix`, `shell.nix`, `nix/` | Nix package and development shell |

## Limitations

- DE's profile says what you have levelled, not what you own: part and relic counts need manual
  tracking, an import or the memory scan.
- The reward-screen reader is tuned for the default interface theme (names in light blue); other
  themes fall back to plain contrast and read less reliably.
- The X11 overlay path has not been tested on an X11 session yet.
