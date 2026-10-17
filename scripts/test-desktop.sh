#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/desktop-common.sh"
foundation=false; wkwebview=false
for arg in "$@"; do
  case "$arg" in --foundation) foundation=true;; --wkwebview) wkwebview=true;; *) echo 'Usage: scripts/test-desktop.sh [--foundation] [--wkwebview]' >&2; exit 2;; esac
done
desktop_preflight
desktop_dependencies
(cd apps/desktop && npm run contracts:check && npm run typecheck && npm test && npm run build)
cargo fmt --all --check
if "$foundation"; then
  cargo test --locked --test desktop_contracts
  cargo check --locked --workspace
  cargo build --locked --workspace
else
  cargo test --locked --all-targets
  cargo clippy --locked --all-targets -- -D warnings
  cargo build --locked
  cargo test --locked --workspace --all-targets
  cargo clippy --locked --workspace --all-targets -- -D warnings
  cargo build --locked --workspace
  bash packaging/capture/build-dev.sh
  export OMM_CAPTURE_TEST_BINARY="$DESKTOP_ROOT/target/capture/OMMCapture.app/Contents/MacOS/omm-capture"
  cargo test --locked --test capture_adapter -- --ignored
  cargo test --locked --test core_integration review_r1_03_ -- --ignored
fi
if "$wkwebview"; then desktop_component_script packaging/desktop/test-wkwebview.sh; fi
