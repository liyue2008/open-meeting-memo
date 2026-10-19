#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/desktop-common.sh"
[[ $# == 0 || $# == 1 && "$1" == --release ]] || { echo 'Usage: scripts/build-desktop.sh [--release]' >&2; exit 2; }
desktop_preflight
desktop_dependencies
node scripts/generate-desktop-contracts.mjs --check
if [[ "${1:-}" == --release ]]; then
  (cd apps/desktop && npm run tauri -- build --no-bundle --target aarch64-apple-darwin -- --locked)
else
  (cd apps/desktop && npm run build)
  cargo build --locked --workspace --target aarch64-apple-darwin
fi
