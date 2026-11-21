#!/usr/bin/env bash
# Start warframeproject from this checkout. Builds the release binary first when the sources
# changed (the first build takes a few minutes; after that it is a no-op), then runs it.
# On NixOS the toolchain and libraries come from shell.nix automatically.
#
#   ./run.sh                    open the app; the terminal has to stay open
#   ./run.sh --detach           hand it to init and give the terminal back (log: ~/.cache/warframeproject/app.log)
#   ./run.sh quit               close a running app
#   ./run.sh trigger | riven    scan the reward screen / grade the riven on screen, in the running app
#   ./run.sh replay FRAME       read a saved screenshot without the app (see README)
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"

if [ "${1:-}" = "--detach" ] || [ "${1:-}" = "-d" ]; then
  shift
  # A crash that only ever reached a terminal that is now gone is a crash nobody can read.
  log="${XDG_CACHE_HOME:-$HOME/.cache}/warframeproject"
  mkdir -p "$log"
  setsid nohup "$0" "$@" >>"$log/app.log" 2>&1 </dev/null &
  echo "warframeproject started in the background. Log: $log/app.log"
  echo "Close the window, or run: ./run.sh quit"
  exit 0
fi

# The interface (web/, Svelte) is built into web/dist, which the binary embeds: again whenever a
# source of it is newer than the last build.
build_ui() {
  if [ ! -d web/node_modules ]; then (cd web && npm ci --silent --no-audit --no-fund); fi
  if [ ! -f web/dist/index.html ] || [ -n "$(find web/src web/public web/index.html web/package.json -newer web/dist/index.html -print -quit)" ]; then
    (cd web && npm run --silent build >/dev/null)
  fi
}

if command -v cargo >/dev/null && command -v npm >/dev/null && pkg-config --exists webkit2gtk-4.1 gtk-layer-shell-0 2>/dev/null; then
  build_ui
  cargo build --release --quiet
  exec ./target/release/warframeproject "$@"
fi

if [ -e /etc/NIXOS ] && command -v nix-shell >/dev/null; then
  args=""
  for a in "$@"; do args+=" $(printf '%q' "$a")"; done
  exec nix-shell "$PWD/shell.nix" --run "$(declare -f build_ui); build_ui && cargo build --release --quiet && exec ./target/release/warframeproject$args"
fi

echo "Building needs cargo, npm (Node 22) and the development files of GTK 3, WebKitGTK 4.1 and gtk-layer-shell," >&2
echo "or Nix. See README.md." >&2
exit 1
