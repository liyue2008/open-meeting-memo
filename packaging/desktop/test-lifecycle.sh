#!/bin/bash
# Explicit fresh-profile proof using real bundled CLI processes, never LaunchAgent registration.
set -euo pipefail
umask 077
unset TAURI_CONFIG
ROOT=$(cd "$(dirname "$0")/../.." && pwd -P)
cd "$ROOT"
: "${OMM_LIFECYCLE_RESOURCE_APP:?set to an existing verified component release app}"
EVIDENCE=$(mktemp -d /private/tmp/omm-lifecycle-proof-XXXXXX)
export EVIDENCE
python3 packaging/desktop/resources.py verify "$OMM_LIFECYCLE_RESOURCE_APP" >"$EVIDENCE/resource-verification.json"
cargo build --release --locked --bin omm >"$EVIDENCE/cli-build.log" 2>&1
cargo build --release --locked -p omm-desktop --features custom-protocol,native-bridge-fixture >"$EVIDENCE/fixture-build.log" 2>&1
python3 - "$ROOT" "$OMM_LIFECYCLE_RESOURCE_APP" "$EVIDENCE" <<'PY'
import hashlib,json,os,pathlib,shutil,subprocess,sys
root,source,evidence=map(pathlib.Path,sys.argv[1:]);revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip()
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
app=evidence/'Lifecycle.app';shutil.copytree(source,app,symlinks=True)
resources=app/'Contents/Resources';cli=resources/'bin/omm';binary=app/'Contents/MacOS/omm-desktop'
shutil.copy2(root/'target/release/omm',cli);shutil.copy2(root/'target/release/omm-desktop',binary)
manifest=resources/'bundle-manifest.json';value=json.loads(manifest.read_text());value['sourceRevision']=revision;value['files']['bin/omm']=sha(cli)
manifest.write_text(json.dumps(value,indent=2)+'\n');manifest.chmod(0o600)
for name,digest in value['files'].items():
 path=resources/name
 assert path.resolve()==path and sha(path)==digest,name
profile=evidence/'profile';profile.mkdir(mode=0o700)
for name in ['home','runtime']:(profile/name).mkdir(mode=0o700)
assert not (profile/'home/Library/LaunchAgents').exists()
shutil.copyfile(root/'apps/desktop/src-tauri/fixtures/lifecycle-probe.js',profile/'driver.js');(profile/'driver.js').chmod(0o600)
env=os.environ.copy();env.update(HOME=str(profile/'home'),OMM_NATIVE_FIXTURE_ROOT=str(profile),OMM_WKWEBVIEW_SCENARIO='lifecycle',XDG_RUNTIME_DIR=str(profile/'runtime'))
for key,name in [('CONFIG','config'),('DATA','data'),('STATE','state'),('CACHE','cache')]:env['XDG_'+key+'_HOME']=str(profile/('xdg-'+name))
(evidence/'provenance.json').write_text(json.dumps({'sourceRevision':revision,'sourceResourceApp':str(source),'resourceManifestHash':sha(manifest),'cliHash':sha(cli),'fixtureHash':sha(binary),'resourceFiles':len(value['files']),'privateHome':str(profile/'home')},indent=2))
with (profile/'runtime.log').open('w') as log:
 child=subprocess.Popen([str(binary)],env=env,stdout=log,stderr=log);expired=False
 try:code=child.wait(timeout=420)
 except subprocess.TimeoutExpired:
  expired=True;child.terminate()
  try:code=child.wait(timeout=15)
  except subprocess.TimeoutExpired:child.kill();code=child.wait()
(profile/'owned-process.json').write_text(json.dumps({'pid':child.pid,'exitCode':code,'waited':True,'timeout':expired},indent=2))
(evidence/'run.exit').write_text(str(code)+'\n')
assert not expired and code==0,'inspect private lifecycle runtime.log and owned process receipt'
result=json.loads((profile/'bridge-result.json').read_text())
assert result['result']['ok'] is True and result['result']['playCalls']==0,result
print(json.dumps(result,indent=2));print('Evidence:',evidence)
PY
