# warframeproject

A Warframe companion for Linux. Shows what you still need for mastery, prime set progress, relic
values and which relic to crack in the fissures open right now, spare parts worth selling, the world
timers (cycles, Baro, sortie, archon hunt, Nightwave) and warframe.market prices. Over the game it
reads the fissure reward screen and prices the drops, and grades a riven card on a key press. No
Overwolf, no Wine; game memory is only read if you switch that on.

Written in Rust: one native binary, a GTK window that shows the web UI, and native in-game overlays.

## Run it

**From this checkout (NixOS):**

```
./run.sh              # builds on first start (a few minutes), then opens the app
./run.sh --detach     # same, but gives the terminal back; log in ~/.cache/warframeproject/app.log
./run.sh quit         # closes a running app
```

`run.sh` builds through `shell.nix` automatically and rebuilds only when the sources changed.
Launching it again while it runs brings the window forward.

**As a Nix package:**

```
nix-build && ./result/bin/warframeproject     # try it
nix-env -f . -i                               # install it, with a menu entry
```

or add `(pkgs.callPackage /path/to/this/checkout { })` to `environment.systemPackages`.

**Other distros:** install Rust (cargo) and the development packages for GTK 3, WebKitGTK 4.1
(`webkit2gtk-4.1`) and gtk-layer-shell, plus `tesseract` with English data and a screenshot tool,
then `./run.sh`. To put `./run.sh` in your app launcher:

```
sed "s|/path/to/warframeproject|$PWD|" warframeproject.desktop > ~/.local/share/applications/warframeproject.desktop
```

| Desktop | Screenshot tool |
|---|---|
| Hyprland, Sway | `grim` (in `shell.nix` and the Nix package) |
| KDE | `spectacle` |
| GNOME | `gnome-screenshot` |
| X11 | `maim`, `scrot` or ImageMagick `import` |

## Using it

| Page | What it is for |
|---|---|
| Overview | Your account at a glance: mastery, currencies, the relics worth cracking in open fissures, the world timers, the foundry, the last reward screens and what they were worth |
| Mastery | Everything that gives mastery, filtered by status, category, MR, primes, vaulted or "ready to build" |
| Prime sets | Each set's parts, what you hold and the set price |
| Relics | Your relics with their value (chance-weighted platinum), the drops you still need, and which can be cracked now |
| Fissures | Live fissures (normal, Steel Path, Void Storms) with the best relic you own for each; the bell watches a tier and mission |
| World | Cycles, Baro Ki'Teer, sortie, archon hunt, Steel Path, Nightwave, Darvo, reset timers |
| Foundry | What is building and when it is done; everything you could build from the parts you hold |
| Market | Your spare prime parts in platinum and ducats; a price check for anything on warframe.market |
| Settings | Sources, memory scan, notifications, overlays, hotkeys, files |

**Ctrl+K** (or `/`) searches items, parts, relics and pages; **1-9** switch pages; **Esc** closes.
Any item, part or relic opens in a side panel with its parts, the relics that drop them (open
fissures first), prices, and where to correct a count or mark an item as mastered by hand.

**Notifications** (Settings, all off by default): a watched fissure opening, Baro arriving, a
foundry build finishing, and every reward screen. They need `notify-send`.

## Hotkeys

| Key (default) | Does | Command it runs |
|---|---|---|
| F9 | Scan the relic reward screen | `curl -s -X POST http://127.0.0.1:8642/api/rewards/scan` |
| F10 | Grade the riven card on screen | `curl -s -X POST http://127.0.0.1:8642/api/riven/scan` |

Settings → **Install hotkeys** writes these for Hyprland (classic or Lua config) or Sway and reloads
the compositor. Elsewhere, bind the commands yourself. `warframeproject trigger` and
`warframeproject riven` do the same from a terminal and print the result.

The reward screen is also scanned automatically when the game logs that it opened. The game
writes its log in bursts, so the automatic trigger can arrive a second late; F9 is instant. Pressing
F9 while the automatic scan runs does not scan twice: both get the same result.

## The overlays

When the reward screen opens, a strip appears over the game with each item's price, ducats, and
whether you own or still need it; the most valuable pick is outlined in gold. After a riven scan, a
panel beside the card shows each stat's grade. Both hide themselves after a while (Settings);
hovering keeps them open and × closes the strip. They never take keyboard focus.

On Hyprland, Sway and KDE the overlays use the compositor's overlay layer (wlr-layer-shell), which
stays above fullscreen games. On GNOME and X11 sessions they are X11 popup windows instead.
Settings → **Show a test overlay** shows sample rewards where the strip will appear.

## Linking your account

Mastery comes from DE's public profile, which needs your account id: a 24-character code like
`505875fb1a4d80894a000000`. It is taken from `EE.log`; the app **never** sends test requests to DE
to find it. DE rate-limits that service aggressively, and too many requests (especially with wrong
ids) can get your IP temporarily blocked from logging into the game.

- If a log line names you and holds exactly one id, or your client sends its own fissure reward to
  the host (play a relic mission in a squad, not as host), that id is linked automatically.
- Otherwise the ids that might be yours are listed in Settings to pick from, or paste it there.

The id is saved, so this happens once. Profile requests are limited to one every 5 minutes; one is
made when the app starts and the saved profile is older than the refresh interval.

## Where the data comes from

| Source | What it gives | How |
|---|---|---|
| WFCD warframe-items | Every item, prime part, relic and drop table | GitHub, cached; rechecked daily (a no-change check costs a few bytes) |
| `EE.log` | Your account id, name and mastery rank badge; when the reward screen opens; when a mission ends | Plain file read of Warframe's own log in the Proton prefix |
| DE public profile | Mastery XP for every item you have levelled | `getProfileViewingData.php`, what the in-game profile screen shows |
| Reward-screen OCR | The 1-4 items on the fissure reward screen | Screenshot, tesseract, fuzzy matching (the WFInfo approach) |
| Manual tracking | Part and relic counts, "mark as mastered" | +/- and the button in an item's side panel |
| `inventory.json` import | The exact inventory: parts, relics, plat, ducats, endo, foundry | A file you import, or one that is watched (Settings) |
| Memory scan (off) | The same as above, read from the running game | See below |

Prices come from warframe.market's public API; fissures, the world timers and traded riven prices
from api.warframestat.us.

Your **mastery rank** comes from `EE.log`, where the squad overlay loads your rank badge: exact for
low ranks, a band such as 16-18 above. DE's profile leaves the rank out. The Overview also shows
what your levelled gear alone is worth.

### The memory scan

No public endpoint returns your inventory; every tool with an exact, automatic inventory (Overwolf's
AlecaFrame, and on Linux Merframe, FrameForge-Linux, WFHelper, warframe-lite) reads it from the game's
memory. Here it is a last resort:

- **Off by default.** Enable it under Settings → Memory scan; Settings shows what it last read and
  when it reads next.
- **Read-only**, through `/proc/<pid>/mem`. It never writes or injects anything.
- **Automatic, never on a timer:** shortly after you log in and after each mission, because the game
  only holds the whole inventory for a while after it syncs. There is no manual scan.
- Only known inventory fields are kept. The same memory holds your session token; it is discarded
  unread.
- Whether Linux allows it is up to the kernel. With `kernel.yama.ptrace_scope` 1 (the usual
  default) a program may read another's memory if it started it, or if that program runs in a user
  namespace you own - as Warframe does inside Steam's sandbox on NixOS, where the scan works at
  scope 1. If Linux refuses, Settings says so, and the app asks again only when the game restarts.
  It never changes system security settings.

DE's published third-party policy is that external software is used *at your own risk*.

## Riven grades

A grade compares the roll with an average roll for that weapon's disposition (2.5-point bands across
the ±10% window: A+ ... C-). It says nothing about whether the stats are useful. All 32 regular
riven attributes and the 18 the Riven Splicer can add are known, with base values from DE's public
export. If a value falls outside ±10%, the panel says the stat count or weapon was misread rather
than printing a confident wrong grade.

## Files

| What | Where |
|---|---|
| Settings | `~/.config/warframeproject/settings.json` |
| Account, profile, inventory, manual counts, reward history | `~/.local/share/warframeproject/` |
| Game data, prices, fonts | `~/.cache/warframeproject/` |
| The last 12 reward and riven scans | `~/.cache/warframeproject/frames/` (screenshot, OCR text, result) |

## When a scan reads the wrong items

Every scan keeps the exact frame it worked from in `~/.cache/warframeproject/frames/`. Replay it:

```
./run.sh replay ~/.cache/warframeproject/frames/reward-20261009-150231.png
./run.sh replay --riven ~/.cache/warframeproject/frames/riven-....png
```

Send that PNG with a bug report: a screenshot pasted into a chat is cropped and rescaled, which
changes what tesseract reads.

## Troubleshooting

| Problem | Fix |
|---|---|
| The window stays empty | Start it from a terminal (`./run.sh`) and read the output. `WARFRAMEPROJECT_DEBUG=1` enables the web inspector. |
| The overlay appears behind the game | Settings → *Keep the overlays above the game with*: try the other option, then restart. Run Warframe borderless or fullscreen through the compositor, not exclusive fullscreen. |
| "Port 8642 is in use" | Something else uses it. Change `"port"` in settings.json or pass `--port` (and re-install the hotkeys). |
| No account detected | Play a relic mission in a squad (not as host), or paste your id in Settings. |

## Development

```
nix-shell            # toolchain, libraries, tesseract, grim
cargo test
cargo run -- --headless --offline --port 8650   # the engine alone, without network
```

The UI in `web/` is plain HTML, CSS and ES modules with no build step. Release builds embed it;
debug builds (`cargo run`) read it from disk, so a reload shows an edit.

## Limitations

- The profile only says what you have levelled, not what you own. Part and relic counts need manual
  tracking, an import or a memory scan.
- Item mastery leaves out the star chart and intrinsics, so the totals shown are a floor.
- The reward-screen reader is tuned for the default UI theme (names in light blue); other themes fall
  back to plain contrast and read less reliably.
- Fan project, not affiliated with Digital Extremes.
