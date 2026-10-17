#!/bin/bash
# Actual WKWebView + production command/ACL/media/core service, using a fresh synthetic-only profile.
set -euo pipefail
umask 077
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
cd "$ROOT"
EVIDENCE=${OMM_WKWEBVIEW_EVIDENCE:-$(mktemp -d /private/tmp/omm-native-bridge-XXXXXX)}
mkdir -p "$EVIDENCE"; chmod 700 "$EVIDENCE"
export OMM_NATIVE_FIXTURE_ROOT=$(cd "$EVIDENCE" && pwd -P)
export XDG_CONFIG_HOME="$OMM_NATIVE_FIXTURE_ROOT/xdg-config"
export XDG_DATA_HOME="$OMM_NATIVE_FIXTURE_ROOT/xdg-data"
export XDG_STATE_HOME="$OMM_NATIVE_FIXTURE_ROOT/xdg-state"
export XDG_CACHE_HOME="$OMM_NATIVE_FIXTURE_ROOT/xdg-cache"
export XDG_RUNTIME_DIR="$OMM_NATIVE_FIXTURE_ROOT/runtime"
mkdir -p "$XDG_RUNTIME_DIR";chmod 700 "$XDG_RUNTIME_DIR"
python3 - "$ROOT" "$OMM_NATIVE_FIXTURE_ROOT" <<'PY'
import json,pathlib,sys
root,evidence=map(pathlib.Path,sys.argv[1:])
samples={p.stem:json.loads(p.read_text())['request']['payload'] for p in (root/'contracts/desktop/v1/examples').glob('*.json') if isinstance(json.loads(p.read_text()),dict) and 'request' in json.loads(p.read_text())}
import os
script=pathlib.Path(os.environ.get('OMM_WKWEBVIEW_DRIVER',root/'apps/desktop/src-tauri/fixtures/bridge-probe.js')).read_text().replace('__OMM_SAMPLES__',json.dumps(samples))
(evidence/'driver.js').write_text(script);(evidence/'driver.js').chmod(0o600)
PY
# Project dependencies only. No system tool install or production bundle is involved.
npm ci --prefix apps/desktop >"$EVIDENCE/npm-ci.log" 2>&1
npm run build --prefix apps/desktop >"$EVIDENCE/frontend-build.log" 2>&1
TAURI_CONFIG='{"app":{"security":{"csp":"default-src '\''none'\''; script-src '\''self'\''; style-src '\''self'\''; img-src '\''self'\'' data:; connect-src ipc: http://ipc.localhost omm-media: omm-fixture-result:; media-src omm-media:; object-src '\''none'\''; frame-src '\''none'\''; base-uri '\''none'\''; form-action '\''none'\''"}}}' cargo build --locked -p omm-desktop --features custom-protocol,native-bridge-fixture >"$EVIDENCE/build.log" 2>&1
python3 - "$ROOT/target/debug/omm-desktop" "$EVIDENCE" <<'PY'
import json,pathlib,subprocess,sys
binary,evidence=sys.argv[1],pathlib.Path(sys.argv[2])
with open(evidence/'runtime.log','w') as log:
    child=subprocess.Popen([binary],stdout=log,stderr=log)
    try:code=child.wait(timeout=90)
    except subprocess.TimeoutExpired:
        child.terminate()
        try:child.wait(timeout=15)
        except subprocess.TimeoutExpired:child.kill();child.wait()
        raise SystemExit('private fixture timed out; owned app waited; inspect runtime log')
(evidence/'runtime.exit').write_text(str(code)+'\n')
assert code==0,f'fixture exit {code}'
result=json.loads((evidence/'bridge-result.json').read_text())
assert result['result']['ok'] is True,result
print(json.dumps(result,indent=2))
PY
printf 'Evidence: %s\n' "$EVIDENCE"
