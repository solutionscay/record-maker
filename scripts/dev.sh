#!/usr/bin/env bash
# Run the record-maker dev server (Browse/Layout runtime) on
# http://127.0.0.1:4317. Extra args are forwarded to `cargo run`
# (e.g. `scripts/dev.sh --release`).
set -euo pipefail

# Make cargo available even in a non-login shell; harmless if already on PATH.
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Run from the repo root regardless of where this is invoked from.
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo "→ record-maker dev server on http://127.0.0.1:4317  (Ctrl-C to stop)"
exec cargo run -p record-maker-server "$@"
