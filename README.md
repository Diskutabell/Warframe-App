# Warframe App

A Warframe companion app for Linux. It keeps track of what you still need for mastery and prime
sets, your syndicate standing and daily limits, values your relics, shows world timers and
warframe.market prices, and draws overlays over the game: prices on the fissure reward screen and
grades for riven cards.

It's a single Rust program: a GTK window for the interface plus overlays drawn by the compositor.
No Overwolf, no Wine, no browser tab.

> [!CAUTION]
> ## Disclaimer: use at your own risk
>
> **This code is mostly vibe coded.** Large parts of it were written with AI tools and have only
> been tested on my own machine. Expect bugs.
>
> **Using it may be against Warframe's Terms of Service / EULA.** The app reads the game's log
> file, takes screenshots of the game for OCR and can (if you turn it on) read the game's memory.
> Digital Extremes' stance on third-party tools is "use at your own risk", and in theory any of
> this could get your account warned, suspended or banned.
>
> **I take no liability whatsoever.** The software is provided as is, without warranty of any
> kind (see [LICENSE](LICENSE)). I am not responsible for banned or suspended accounts, lost
> data, damage to your system or anything else that happens because you used this. If you are
> not okay with that, don't use it.
>
> This is a fan project. It is not affiliated with or endorsed by Digital Extremes.

## Features

- **Overview:** mastery rank and mastery XP (same total as the in-game profile), currencies,
  relics worth cracking right now, world timers, foundry, pinned items, recent reward screens.
- **Reward screen overlay:** when the relic reward screen opens, a strip under the cards shows
  each item's platinum and ducat value and whether you still need it.
- **Riven grades:** press a key with a riven card on screen and every stat gets graded against
  the weapon's disposition.
- **Standing:** rank progress for every syndicate, rank-up costs, what's left of today's daily
  limits, medallions you hold, and what each syndicate's offerings are worth in platinum.
- **Mastery, prime sets, relics:** filterable lists with the parts you have and where to farm the
  rest; relic values and which ones you can crack in open fissures.
- **Fissures and world:** live fissures with the best relic you own for each, cycles, Baro,
  sortie, archon hunt, bounties, the Circuit, Archimedea, alerts, invasions, Nightwave, Darvo.
- **Foundry and market:** what's building, what you could build, spare parts in platinum and
  ducats, price checks.
- **Several accounts:** every account that logs in on the machine gets its own data, and the
  window follows the game when you switch.
- **Search:** Ctrl+K finds any item, part, relic, syndicate or page.
- **Notifications** (off by default): watched fissures, Baro, finished builds.

## Requirements

- Linux on Wayland (Hyprland, Sway, KDE, GNOME) or X11
- Warframe through Steam/Proton, Lutris, Heroic, umu or a plain Wine prefix
- GTK 3, WebKitGTK 4.1, gtk-layer-shell
- `tesseract` with English data, and a screenshot tool:

  | Desktop | Screenshot tool |
  |---|---|
  | Hyprland, Sway | `grim` |
  | KDE | `spectacle` |
  | GNOME | `gnome-screenshot` |
  | X11 | `maim`, `scrot` or ImageMagick `import` |

- Optional: `notify-send` for notifications

## Installation

### NixOS / Nix

```sh
git clone https://github.com/Diskutabell/Warframe-App.git
cd Warframe-App
nix-env -f . -i
```

Or try it without installing: `nix-build && ./result/bin/warframeproject`. On NixOS you can also
add `(pkgs.callPackage /path/to/Warframe-App { })` to `environment.systemPackages`.

### Other distributions

Install Rust, the development packages for GTK 3, WebKitGTK 4.1 and gtk-layer-shell, tesseract
and a screenshot tool, then:

```sh
git clone https://github.com/Diskutabell/Warframe-App.git
cd Warframe-App
./run.sh
```

The first start builds the app, which takes a few minutes. `./run.sh --detach` gives you the
terminal back and `./run.sh quit` closes the app. To add it to your app launcher:

```sh
sed "s|/path/to/warframeproject|$PWD|" warframeproject.desktop > ~/.local/share/applications/warframeproject.desktop
```

## Getting started

1. Start the app, then Warframe. The app finds `EE.log` on its own.
2. Link your account for mastery data (see below).
3. Install the hotkeys under Settings → Hotkeys (Hyprland and Sway; on other desktops bind the
   commands below yourself).
4. Optional: import an `inventory.json` or turn on the memory scan to get part, relic and
   currency counts.

Keys: **Ctrl+K** or **/** to search, **1-9** and **0** to switch pages, **Esc** to close.

### Hotkeys

| Key | Does | Command |
|---|---|---|
| F9 | Scan the relic reward screen | `curl -s -X POST http://127.0.0.1:8642/api/rewards/scan` |
| F10 | Grade the riven card on screen | `curl -s -X POST http://127.0.0.1:8642/api/riven/scan` |

The reward screen is also scanned automatically when the game logs that it opened, which can be
a second late; F9 is instant. `warframeproject trigger` and `warframeproject riven` do the same
from a terminal.

On Hyprland, Sway and KDE the overlays sit on the compositor's overlay layer, above fullscreen
games. On GNOME and X11 they are popup windows. Settings has a button to show a test overlay.

## Linking your account

Mastery data comes from DE's public profile, which needs your 24-character account id (something
like `5f1e2d3c4b5a69788796a5b4`). The app picks it up from `EE.log` when it can, for example after
a relic mission in a squad where you are not the host. Otherwise you can choose from the ids it
found or paste yours in Settings.

The app never guesses ids against DE's servers and asks for a profile at most once every 5
minutes. DE rate-limits that service, and too many requests can get your IP blocked from logging
in for a while.

## Memory scan

There's no public API for your inventory, so tools that keep it up to date automatically read
it from the game's memory. Here that is optional and **off by default**:

- Read-only, through `/proc/<pid>/mem`. Nothing is written to or injected into the game.
- Runs by itself after login and after each mission, never on a timer.
- Only known inventory fields are kept. Your session token is in the same memory and is thrown
  away without being read.
- If Linux doesn't allow the read, Settings tells you. The app never changes system settings.

This is the part most likely to be against the ToS. See the disclaimer above.

## Data sources

- [WFCD warframe-items](https://github.com/WFCD/warframe-items) and its
  [drop data](https://drops.warframestat.us): items, parts, relics, drop tables
- [warframe-public-export-plus](https://github.com/calamity-inc/warframe-public-export-plus):
  star chart mastery XP, syndicates, relic drops
- [api.warframestat.us](https://api.warframestat.us): fissures, world timers, riven prices
- [warframe.market](https://warframe.market): prices and item art (at most 3 requests a second)
- [browse.wf](https://browse.wf): the game's own interface art
- DE's public profile: mastery XP, star chart, intrinsics, standing
- `EE.log`, reward screen OCR, your own counts, an imported inventory, or the memory scan

Nothing else is sent anywhere.

## Files

| What | Where |
|---|---|
| Settings | `~/.config/warframeproject/settings.json` |
| Accounts, profiles, inventories, reward history | `~/.local/share/warframeproject/` |
| Game data, prices, art | `~/.cache/warframeproject/` |
| The last 12 scans (screenshot, OCR text, result) | `~/.cache/warframeproject/frames/` |

## Troubleshooting

| Problem | Fix |
|---|---|
| A scan read the wrong items | Replay the frame: `warframeproject replay ~/.cache/warframeproject/frames/reward-....png` (`--riven` for rivens) and attach that PNG to the bug report. |
| A dock starts an old version after updating | It saved the full `/nix/store/...` path. Point it at `warframeproject` or pin the app again. |
| The window stays empty | Start it from a terminal and read the output. `WARFRAMEPROJECT_DEBUG=1` turns on the web inspector. |
| The overlay is behind the game | Settings → *Keep overlays above the game with*: try the other option, then restart. Run the game fullscreen or borderless through the compositor. |
| "Port 8642 is in use" | Change `"port"` in settings.json or pass `--port`, then install the hotkeys again. |
| No account detected | Play a relic mission in a squad (not as host), or paste your id in Settings. |

## Development

```sh
nix-shell                                        # toolchain, libraries, tesseract, grim
cargo test
cargo run -- --headless --offline --port 8650    # engine only, no network
```

`--browser` opens the interface in your web browser instead of a window. The interface in `web/`
is plain HTML, CSS and JavaScript modules with no build step; debug builds read it from disk, so a
reload picks up changes.

Known limitations: part and relic counts need manual tracking, an import or the memory scan; the
OCR is tuned for the default UI theme; the X11 overlay path hasn't been tested on a real X11
session.

## License

[MIT](LICENSE). Warframe and its art belong to Digital Extremes.
