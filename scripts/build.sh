#!/usr/bin/env bash
# Build the whole record-maker workspace (engine + server). Extra args are
# forwarded to `cargo build` (e.g. `scripts/build.sh --release`).
set -euo pipefail

# Make cargo available even in a non-login shell; harmless if already on PATH.
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

# Run from the repo root regardless of where this is invoked from.
cd "$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

exec cargo build --workspace "$@"
