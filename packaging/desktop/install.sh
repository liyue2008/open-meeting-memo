#!/bin/bash
# User-only stable App name; revision receipts retained. Does not select/start/stop personal services.
set -euo pipefail
umask 077
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
CHECK=false
if [[ ${1:-} == --check ]]; then CHECK=true; shift; fi
APP=${1:-"$ROOT/target/desktop-package/$(git -C "$ROOT" rev-parse HEAD)/Open Meeting Memo.app"}
python3 "$ROOT/packaging/desktop/install.py" "$APP" "$CHECK"
