#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/desktop-common.sh"
[[ $# == 0 ]] || { echo 'Usage: scripts/package-desktop.sh' >&2; exit 2; }
desktop_preflight
# Native owner provides resource closure, license and inside-out signing checks.
# A bare scaffold cannot be published as a complete product package.
desktop_component_script packaging/desktop/package.sh
