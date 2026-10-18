#!/bin/bash
# Compatibility entry: the same verified App/no-signal runner as the independent UI gate.
set -euo pipefail
umask 077
[[ $# == 0 ]] || { echo 'Usage: test-lifecycle.sh (OMM_LIFECYCLE_RESOURCE_APP required)' >&2; exit 2; }
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
cd "$ROOT"
: "${OMM_LIFECYCLE_RESOURCE_APP:?set to the current verified component App}"
EVIDENCE=$(mktemp -d /private/tmp/omm-lifecycle-proof-XXXXXX)
export EVIDENCE
printf 'Evidence: %s\n' "$EVIDENCE"
export OMM_WKWEBVIEW_EVIDENCE="$EVIDENCE/profile"
export OMM_WKWEBVIEW_SCENARIO=lifecycle
export OMM_WKWEBVIEW_RESOURCE_APP="$OMM_LIFECYCLE_RESOURCE_APP"
export OMM_WKWEBVIEW_DRIVER="${OMM_LIFECYCLE_DRIVER:-$ROOT/apps/desktop/src-tauri/fixtures/lifecycle-probe.js}"
set +e
bash packaging/desktop/test-wkwebview.sh
code=$?
set -e
printf '%s\n' "$code" >"$EVIDENCE/run.exit"
[[ $code == 0 ]] || exit "$code"
python3 - "$EVIDENCE" <<'PY'
import json,pathlib,sys
root=pathlib.Path(sys.argv[1]);p=json.loads((root/'profile/runner-provenance.json').read_text());r=json.loads((root/'profile/bridge-result.json').read_text())['result']
provenance={'fixtureSourceRevision':p['sourceRevision'],'resourceSourceRevision':p['sourceRevision'],'driverHash':p['driverHash'],'sourceCliHash':p['sourceCliHash'],'cliHash':p['cliHash'],'sourceManifestHash':p['sourceManifestHash'],'resourceManifestHash':p['manifestHash'],'sourceCLIEqualsStagedCLI':p['sourceCliHash']==p['cliHash'],'manifestUnchanged':p['sourceManifestHash']==p['manifestHash'],'actualFixtureApp':str(root/'profile/Fixture.app'),'coreWaitResultObserved':False,'fixtureChildWaitedForCoreCli':False}
(root/'provenance.json').write_text(json.dumps(provenance,indent=2)+'\n')
(root/'cli-byte-preservation.json').write_text(json.dumps({'sourceCliHash':p['sourceCliHash'],'stagedCliHash':p['cliHash'],'sourceManifestHash':p['sourceManifestHash'],'stagedManifestHash':p['manifestHash'],'verifiedRunningCases':[c for c in r['nativeCases'] if c['case'] in ['running','switched']]},indent=2)+'\n')
PY
