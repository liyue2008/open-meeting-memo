#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/desktop-common.sh"
[[ $# == 1 && "$1" == --check || $# == 2 && "$1" == --from ]] || { echo 'Usage: scripts/install-desktop.sh --check | --from <validated-package>' >&2; exit 2; }
desktop_preflight
# The install owner must inspect metadata and validate the entire package before publication.
desktop_component_script packaging/desktop/install.sh "$@"
