#!/bin/bash
# User-only, versioned side-by-side installation. Does not select/start/stop any personal service.
set -euo pipefail
umask 077
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
CHECK=false
if [[ ${1:-} == --check ]]; then CHECK=true; shift; fi
APP=${1:-"$ROOT/target/desktop-package/$(git -C "$ROOT" rev-parse HEAD)/Open Meeting Memo.app"}
python3 "$ROOT/packaging/desktop/install.py" "$APP" "$CHECK"
