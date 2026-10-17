#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/desktop-common.sh"
[[ $# == 0 ]] || { echo 'Usage: scripts/dev-desktop.sh' >&2; exit 2; }
desktop_preflight
desktop_dependencies
cd apps/desktop
exec npm run tauri -- dev
