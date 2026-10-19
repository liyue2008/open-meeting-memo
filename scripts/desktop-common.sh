#!/usr/bin/env bash
# Shared entrypoint preflight. No service, permissions, model, or global tool installation.
set -euo pipefail
DESKTOP_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$DESKTOP_ROOT"
export MACOSX_DEPLOYMENT_TARGET=15.5
desktop_preflight() {
  [[ "$(uname -s)" == Darwin && "$(uname -m)" == arm64 ]] || { echo 'Desktop supports macOS Apple Silicon only.' >&2; exit 2; }
  for tool in cargo rustc node npm xcrun; do command -v "$tool" >/dev/null || { echo "Required existing tool missing: $tool" >&2; exit 3; }; done
  node --input-type=module -e 'const [major, minor] = process.versions.node.split(".").map(Number); if (major !== 22 || minor < 19) throw Error("Use existing Node 22.19+ (<23)");'
  [[ "$(npm --version)" == 11.* ]] || { echo 'Use existing npm 11.x for the committed lock.' >&2; exit 3; }
  local major minor
  IFS=. read -r major minor _ <<<"$(sw_vers -productVersion)"
  (( major > 15 || major == 15 && minor >= 5 )) || { echo 'Desktop requires macOS 15.5+.' >&2; exit 3; }
}
desktop_dependencies() { (cd apps/desktop && npm ci --ignore-scripts --no-audit --no-fund); }
desktop_component_script() {
  local script="$1"; shift
  [[ -f "$script" && ! -L "$script" ]] || { echo "Required desktop component entrypoint not supplied: $script (see docs/desktop-lanes.md)" >&2; exit 3; }
  bash "$script" "$@"
}
