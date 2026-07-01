#!/usr/bin/env bash
#
# run.sh — build the UI bundle and run record-maker.
#
#   scripts/run.sh              Desktop app (Tauri 2). Needs the Tauri v2 CLI +
#                               Linux system libs (see the ✗ hint below).
#   scripts/run.sh --server     Headless axum server on http://127.0.0.1:4317.
#                               No Tauri deps required — handy to see the app now.
#
# Both modes build ui/dist first and point the app at it via RM_UI_DIR, so the
# Layout-mode editor assets (/ui/*) always load from the fresh build.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

echo "▶ building UI bundle (ui/dist)…"
( cd ui && npm run build )
export RM_UI_DIR="$repo_root/ui/dist"

# Headless server mode — runs the standalone bin, no Tauri toolchain needed.
if [[ "${1:-}" == "--server" ]]; then
  echo "▶ cargo run -p record-maker-server  →  http://127.0.0.1:${RM_PORT:-4317}"
  exec cargo run -p record-maker-server
fi

# Desktop mode — preflight the Tauri CLI so the failure is actionable.
if ! cargo tauri --version >/dev/null 2>&1; then
  cat >&2 <<'EOF'
✗ Tauri v2 CLI not found. Install it + the Linux system libs, or run the
  headless server instead:  scripts/run.sh --server

  cargo install tauri-cli --version "^2.0.0" --locked
  sudo apt update && sudo apt install -y \
    libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev libssl-dev \
    libayatana-appindicator3-dev build-essential curl wget file pkg-config

  Then promote src-tauri to a workspace member in Cargo.toml
  (members = [..., "src-tauri"]; delete the exclude line) and re-run.
EOF
  exit 1
fi

echo "▶ cargo tauri dev"
exec cargo tauri dev
