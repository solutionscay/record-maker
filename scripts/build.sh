#!/usr/bin/env bash
# Build the whole record-maker app: the Layout Mode frontend (ui/ -> ui/dist)
# then the Rust workspace (engine + server). Extra args are forwarded to
# `cargo build` (e.g. `scripts/build.sh --release`). Pass `--skip-ui` to build
# only the Rust side (e.g. a Rust-only iteration).
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

# Frontend: install deps on first run, then build the static bundle into ui/dist
# (served by the server at /ui/...). The server runs without it, just with no
# editor island, so this is skippable.
if [ -z "$skip_ui" ]; then
  if command -v npm >/dev/null 2>&1; then
    echo "→ building Layout Mode frontend (ui/ -> ui/dist)"
    ( cd ui && { [ -d node_modules ] || npm install; } && npm run build )
  else
    echo "⚠ npm not found — skipping the ui/ frontend (install Node.js to build the editor island, or pass --skip-ui to silence this)" >&2
  fi
fi

echo "→ building Rust workspace"
exec cargo build --workspace "${cargo_args[@]}"
