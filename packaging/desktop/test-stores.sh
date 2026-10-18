#!/bin/bash
# Existing UUID-store actions through the same verified App runner; never the default WK store.
set -euo pipefail
umask 077
[[ $# -le 1 ]] || { echo 'Expected proof|allocate|write|read|cleanup' >&2; exit 2; }
ACTION=${1:-proof}
case "$ACTION" in proof|allocate|write|read|cleanup) ;; *) echo 'Expected proof|allocate|write|read|cleanup' >&2; exit 2 ;; esac
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
cd "$ROOT"
export OMM_WKWEBVIEW_STORE_ACTION="$ACTION"
export OMM_WKWEBVIEW_SCENARIO=store
export OMM_WKWEBVIEW_EVIDENCE=$(mktemp -d /private/tmp/omm-wk-owned-XXXXXX)
exec bash packaging/desktop/test-wkwebview.sh
