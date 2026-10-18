#!/usr/bin/env bash
set -euo pipefail
source "$(dirname "$0")/desktop-common.sh"
foundation=false; wkwebview=false
for arg in "$@"; do
  case "$arg" in --foundation) foundation=true;; --wkwebview) wkwebview=true;; *) echo 'Usage: scripts/test-desktop.sh [--foundation] [--wkwebview]' >&2; exit 2;; esac
done
desktop_preflight
if "$wkwebview"; then
  : "${OMM_LIFECYCLE_RESOURCE_APP:?set to a verified release App from the current clean revision for the lifecycle gate}"
fi
bash scripts/test-desktop-entrypoints.sh
python3 packaging/desktop/test_fixture_runner.py
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
    unset OMM_WKWEBVIEW_SCENARIO OMM_WKWEBVIEW_DRIVER OMM_WKWEBVIEW_EVIDENCE OMM_WKWEBVIEW_STORE_MANIFEST OMM_NATIVE_REVIEW_FIXTURE OMM_NATIVE_REVIEW_CLEANUP
    record_native_source() {
      python3 - "$OMM_WKWEBVIEW_EVIDENCE/$1" "$DESKTOP_ROOT" <<'PY'
import hashlib, json, pathlib, subprocess, sys
path, root = map(pathlib.Path, sys.argv[1:])
files = subprocess.check_output(['git', 'ls-files'], cwd=root, text=True).splitlines()
value = {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=root, text=True).strip(), 'status': subprocess.check_output(['git', 'status', '--porcelain'], cwd=root, text=True), 'files': {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in files if (root / name).is_file()}}
assert not value['status'], 'WK acceptance requires clean committed source'
path.write_text(json.dumps(value, sort_keys=True, indent=2) + '\n')
path.chmod(0o600)
PY
    }
    record_native_artifacts() {
      record_native_source source-after.json
      cmp "$OMM_WKWEBVIEW_EVIDENCE/source-before.json" "$OMM_WKWEBVIEW_EVIDENCE/source-after.json"
      python3 - "$OMM_WKWEBVIEW_EVIDENCE" "$DESKTOP_ROOT" <<'PY'
import hashlib, json, pathlib, subprocess, sys
path, root = map(pathlib.Path, sys.argv[1:])
files = [root / 'target/debug/omm-desktop', path / 'driver.js', path / 'bridge-result.json', path / 'owned-process.json']
files += sorted((root / 'apps/desktop/dist').rglob('*'))
files += sorted(path.glob('*.png'))
files += [path / name for name in ['review-scenario.json', 'review-capture-pids', 'review-capture-events.jsonl']]
value = {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip(), 'files': {str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in files if p.is_file()}}
artifact = path / 'actual-artifact-sha256.json'
artifact.write_text(json.dumps(value, indent=2) + '\n')
artifact.chmod(0o600)
PY
    }
    export OMM_WKWEBVIEW_EVIDENCE
    OMM_WKWEBVIEW_EVIDENCE=$(mktemp -d /private/tmp/omm-wk-native-XXXXXX)
    record_native_source source-before.json
    desktop_component_script packaging/desktop/test-wkwebview.sh
    record_native_artifacts
    export OMM_WKWEBVIEW_SCENARIO=window-acl
    OMM_WKWEBVIEW_EVIDENCE=$(mktemp -d /private/tmp/omm-wk-acl-XXXXXX)
    record_native_source source-before.json
    desktop_component_script packaging/desktop/test-wkwebview.sh
    record_native_artifacts
    export OMM_WKWEBVIEW_SCENARIO=ui
    export OMM_WKWEBVIEW_DRIVER="$DESKTOP_ROOT/scripts/fixtures/desktop-review-guards-probe.js"
    OMM_WKWEBVIEW_EVIDENCE=$(mktemp -d /private/tmp/omm-review-wk-XXXXXX)
    record_native_source source-before.json
    desktop_component_script packaging/desktop/test-wkwebview.sh
    record_native_artifacts
    export OMM_WKWEBVIEW_DRIVER="$DESKTOP_ROOT/scripts/fixtures/desktop-service-player-probe.js"
    OMM_WKWEBVIEW_EVIDENCE=$(mktemp -d /private/tmp/omm-entry-wk-XXXXXX)
    record_native_source source-before.json
    desktop_component_script packaging/desktop/test-wkwebview.sh
    record_native_artifacts
    export OMM_WKWEBVIEW_DRIVER="$DESKTOP_ROOT/apps/desktop/src-tauri/fixtures/review-probe.js"
    for review_mode in cold-recording partial-transcript; do
      export OMM_NATIVE_REVIEW_FIXTURE="$review_mode"
      OMM_WKWEBVIEW_EVIDENCE=$(mktemp -d /private/tmp/omm-rvw-XXXXXX)
      record_native_source source-before.json
      desktop_component_script packaging/desktop/test-wkwebview.sh
      record_native_artifacts
    done
    unset OMM_WKWEBVIEW_SCENARIO OMM_WKWEBVIEW_EVIDENCE OMM_WKWEBVIEW_DRIVER OMM_NATIVE_REVIEW_FIXTURE
    desktop_component_script scripts/test-desktop-ui-wkwebview.sh
    desktop_component_script scripts/test-desktop-preferences-wkwebview.sh
    desktop_component_script scripts/test-desktop-lifecycle-wkwebview.sh
  )
fi
