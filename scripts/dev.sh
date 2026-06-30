#!/usr/bin/env bash
# Run the record-maker dev environment: the Layout Mode frontend in watch mode
# (rebuilds ui/dist on change) plus the Browse/Layout server on
# http://127.0.0.1:4317. Extra args are forwarded to `cargo run`
# (e.g. `scripts/dev.sh --release`). Pass `--skip-ui` to run only the server.
# Ctrl-C stops both.
set -euo pipefail

# Make cargo available even in a non-login shell; harmless if already on PATH.
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Run from the repo root regardless of where this is invoked from.
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Split out our own --skip-ui flag; everything else passes through to cargo.
skip_ui=
cargo_args=()
for arg in "$@"; do
  if [ "$arg" = "--skip-ui" ]; then skip_ui=1; else cargo_args+=("$arg"); fi
done

# Frontend: install on first run, then watch-build into ui/dist in the
# background. The server serves the static bundle from ui/dist, so editing a
# Svelte file rebuilds the bundle — refresh the page to see it.
if [ -z "$skip_ui" ] && command -v npm >/dev/null 2>&1; then
  ( cd ui && { [ -d node_modules ] || npm install; } )
  echo "→ Layout Mode frontend: watch-building ui/dist"
  ( cd ui && npm run watch ) &
  ui_pid=$!
  trap 'kill "$ui_pid" 2>/dev/null || true' EXIT INT TERM
elif [ -z "$skip_ui" ]; then
  echo "⚠ npm not found — running without the ui/ frontend watch (install Node.js, or pass --skip-ui)" >&2
fi

echo "→ record-maker dev server on http://127.0.0.1:4317  (Ctrl-C to stop)"
# No `exec`: keep this shell alive so the trap can stop the frontend watcher.
cargo run -p record-maker-server "${cargo_args[@]}"
