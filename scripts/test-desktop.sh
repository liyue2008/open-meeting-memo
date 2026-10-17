#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/desktop-common.sh"
foundation=false; wkwebview=false
for arg in "$@"; do
  case "$arg" in --foundation) foundation=true;; --wkwebview) wkwebview=true;; *) echo 'Usage: scripts/test-desktop.sh [--foundation] [--wkwebview]' >&2; exit 2;; esac
done
desktop_preflight
bash scripts/test-desktop-entrypoints.sh
desktop_dependencies
(cd apps/desktop && npm run contracts:check && npm run format:check && npm run typecheck && TZ=UTC npm test && TZ=America/Los_Angeles npm test && npm run build)
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
  cargo clippy --locked -p omm-desktop --all-targets --features custom-protocol,native-bridge-fixture -- -D warnings
  bash packaging/capture/build-dev.sh
  export OMM_CAPTURE_TEST_BINARY="$DESKTOP_ROOT/target/capture/OMMCapture.app/Contents/MacOS/omm-capture"
  cargo test --locked --test capture_adapter -- --ignored
  cargo test --locked --test core_integration review_r1_03_ -- --ignored
fi
if "$wkwebview"; then
  (
    unset OMM_WKWEBVIEW_SCENARIO OMM_WKWEBVIEW_DRIVER OMM_WKWEBVIEW_EVIDENCE OMM_WKWEBVIEW_STORE_MANIFEST
    desktop_component_script packaging/desktop/test-wkwebview.sh
    export OMM_WKWEBVIEW_SCENARIO=window-acl
    desktop_component_script packaging/desktop/test-wkwebview.sh
    desktop_component_script scripts/test-desktop-ui-wkwebview.sh
    desktop_component_script scripts/test-desktop-preferences-wkwebview.sh
  )
fi
