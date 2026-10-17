#!/bin/bash
# Feature-only UUID WK store proof. Does not open or remove the default store.
set -euo pipefail
umask 077
unset TAURI_CONFIG
ACTION=${1:-proof}
case "$ACTION" in proof|allocate|write|read|cleanup) ;; *) echo 'Expected proof|allocate|write|read|cleanup' >&2; exit 2 ;; esac
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
cd "$ROOT"
EVIDENCE=$(mktemp -d /private/tmp/omm-wk-owned-XXXXXX)
export EVIDENCE OMM_WKWEBVIEW_STORE_ACTION="$ACTION"
# Reuses the existing frontend distribution. No npm reinstall or unrelated release rebuild.
test -f apps/desktop/dist/index.html
cargo build --locked -p omm-desktop --features custom-protocol,native-bridge-fixture >"$EVIDENCE/build.log" 2>&1
python3 - "$ROOT" "$EVIDENCE" <<'PY'
import hashlib,json,os,pathlib,subprocess,sys
root,evidence=map(pathlib.Path,sys.argv[1:]);binary=root/'target/debug/omm-desktop'
(evidence/'runtime').mkdir(mode=0o700);(evidence/'driver.js').write_text('');(evidence/'driver.js').chmod(0o600)
env=os.environ.copy();env.update(OMM_NATIVE_FIXTURE_ROOT=str(evidence),OMM_WKWEBVIEW_SCENARIO='store',XDG_RUNTIME_DIR=str(evidence/'runtime'))
for key,name in [('CONFIG','config'),('DATA','data'),('STATE','state'),('CACHE','cache')]:env['XDG_'+key+'_HOME']=str(evidence/('xdg-'+name))
(evidence/'provenance.json').write_text(json.dumps({'sourceRevision':subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),'binaryHash':hashlib.sha256(binary.read_bytes()).hexdigest(),'action':env['OMM_WKWEBVIEW_STORE_ACTION'],'manifest':env.get('OMM_WKWEBVIEW_STORE_MANIFEST'),'frontendRebuilt':False},indent=2))
with (evidence/'runtime.log').open('w') as log:
 child=subprocess.Popen([str(binary)],env=env,stdout=log,stderr=log);expired=False
 try:code=child.wait(timeout=90)
 except subprocess.TimeoutExpired:
  expired=True;child.terminate()
  try:code=child.wait(timeout=10)
  except subprocess.TimeoutExpired:child.kill();code=child.wait()
(evidence/'owned-process.json').write_text(json.dumps({'pid':child.pid,'exitCode':code,'waited':True,'timeout':expired},indent=2));(evidence/'run.exit').write_text(str(code)+'\n')
print('Evidence:',evidence,flush=True)
assert not expired and code==0,'owned app waited; inspect runtime.log; retained UUID manifest must be cleaned with this exact entry'
result=json.loads((evidence/'bridge-result.json').read_text());assert result['result']['ok'] is True and result['result']['playCalls']==0,result
print(json.dumps(result,indent=2))
PY
