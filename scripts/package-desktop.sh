#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/desktop-common.sh"
[[ $# == 0 ]] || { echo 'Usage: scripts/package-desktop.sh' >&2; exit 2; }
desktop_preflight
# Committed clean source only: native packaging verifies resource closure and inside-out signatures.
desktop_component_script packaging/desktop/package.sh
