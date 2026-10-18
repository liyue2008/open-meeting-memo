#!/usr/bin/env bash
# Two actual React processes, one proven private UUID store; never use the default store.
set -euo pipefail
umask 077
ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$ROOT"
export OMM_DESKTOP_PREFERENCES_EVIDENCE=${OMM_DESKTOP_PREFERENCES_EVIDENCE:-$(mktemp -d /private/tmp/omm-pref-XXXXXX)}
python3 - <<'PY'
import hashlib, json, os, pathlib, re, subprocess, sys
sys.dont_write_bytecode = True
root = pathlib.Path.cwd()
sys.path.insert(0, str(root / 'packaging/desktop'))
from fixture_artifacts import host_artifacts
evidence = pathlib.Path(os.environ['OMM_DESKTOP_PREFERENCES_EVIDENCE'])
assert evidence.is_absolute() and evidence == evidence.resolve(), 'Preference evidence requires a canonical absolute path without symlinks'
assert evidence.is_relative_to('/private/tmp') and len(os.fsencode(evidence)) <= 55, 'Preference evidence requires a short private /private/tmp root'
if not evidence.exists():
    evidence.mkdir(mode=0o700)
assert evidence.is_dir() and evidence.stat().st_uid == os.getuid() and not list(evidence.iterdir()), 'Preference evidence requires a new empty owned directory'
evidence.chmod(0o700)
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
def write(name, value):
    path = evidence / name
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    path.chmod(0o600)
def sources():
    paths = set(filter(None, subprocess.check_output(['git', 'ls-files', '-z', '--cached', '--others', '--exclude-standard']).decode().split('\0')))
    return {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD']).decode().strip(), 'files': {p: digest(root / p) for p in sorted(paths) if (root / p).is_file()}}
def run(script, arguments, env, name):
    with (evidence / (name + '.log')).open('w') as log:
        result = subprocess.run(['bash', script, *arguments], env=env, stdout=log, stderr=subprocess.STDOUT)
    (evidence / (name + '.exit')).write_text(str(result.returncode) + '\n')
    assert result.returncode == 0, f'{name} failed ({result.returncode}); inspect evidence'
base = dict(os.environ)
base.pop('OMM_WKWEBVIEW_STORE_MANIFEST', None)
before = sources()
write('source-before.json', before)
manifest = None
try:
    run('packaging/desktop/test-stores.sh', ['proof'], base, 'store-proof')
    run('packaging/desktop/test-stores.sh', ['allocate'], base, 'store-allocate')
    match = re.search(r'^Evidence: (.+)$', (evidence / 'store-allocate.log').read_text(), re.M)
    assert match, 'Allocation must return its owned root'
    manifest = pathlib.Path(match[1]) / 'owned-wk-store.json'
    assert manifest == manifest.resolve() and manifest.is_relative_to('/private/tmp')
    assert manifest.stat().st_uid == os.getuid() and not json.loads(manifest.read_text())['removed']
    (evidence / 'store-manifest-path').write_text(str(manifest) + '\n')
    env = dict(base, OMM_WKWEBVIEW_STORE_MANIFEST=str(manifest), OMM_WKWEBVIEW_SCENARIO='ui')
    values, receipts, attached = [], [], []
    for phase in ['write', 'read']:
        phase_root = evidence / phase
        phase_root.mkdir(mode=0o700)
        driver = phase_root / 'prepared-driver.js'
        driver.write_text('window.__ommPreferencePhase=' + json.dumps(phase) + ';\n' + (root / 'scripts/fixtures/desktop-preferences-probe.js').read_text())
        driver.chmod(0o600)
        run('packaging/desktop/test-wkwebview.sh', [], dict(env, OMM_WKWEBVIEW_DRIVER=str(driver), OMM_WKWEBVIEW_EVIDENCE=str(phase_root)), phase)
        # Capture the executable before cleanup rebuilds with different fixture configuration.
        artifacts = [driver, phase_root / 'driver.js', phase_root / 'bridge-result.json', phase_root / 'owned-process.json', phase_root / 'owned-store-attached.json', phase_root / 'close-before-destroy.json']
        artifacts += sorted((root / 'apps/desktop/dist').rglob('*'))
        artifact_hashes = {str(p): digest(p) for p in artifacts if p.is_file()}
        artifact_hashes.update(host_artifacts(root, phase_root, before['head'], 'ui'))
        write(phase + '-artifact-sha256.json', artifact_hashes)
        result = json.loads((phase_root / 'bridge-result.json').read_text())['result']
        receipt = json.loads((phase_root / 'owned-process.json').read_text())
        store = json.loads((phase_root / 'owned-store-attached.json').read_text())
        assert result['ok'] and result['playCalls'] == 0 and result['phase'] == phase
        for key in ['destroyedEvent', 'mainAbsent', 'readersReleased', 'shutdownAcknowledged', 'captureExitSuccess', 'captureChildWaited']:
            assert result['nativeClose'][key] is True, (key, result)
        assert receipt['waited'] and receipt['exitCode'] == 0 and not receipt['terminatedOnTimeout'], receipt
        assert not store['defaultStoreUsed'] and store['actualPersistent'] and store['ownedMarkerVerified']
        assert result['store']['actualIdentifier'] == store['actualIdentifier']
        values.append(result)
        receipts.append(receipt)
        attached.append(store)
    assert receipts[0]['pid'] != receipts[1]['pid'], 'Two independent App processes are required'
    assert values[0]['store'] == values[1]['store']
    ids = json.loads(manifest.read_text())['ids']
    assert len(ids) == 2 and ids[0] != ids[1]
    assert attached[0]['actualIdentifier'] == attached[1]['actualIdentifier'] == ids[0]
    assert values[1]['initial']['theme'] == 'dark' and values[1]['initial']['sidebar'] == 'collapsed'
    write('preferences-result.json', {'ok': True, 'playCalls': 0, 'receipts': receipts, 'initialRead': values[1]['initial'], 'actualStore': values[1]['store']})
finally:
    if manifest is not None:
        run('packaging/desktop/test-stores.sh', ['cleanup'], dict(base, OMM_WKWEBVIEW_STORE_MANIFEST=str(manifest)), 'store-cleanup')
        assert json.loads(manifest.read_text())['removed'] is True, 'Exact owned UUIDs must be removed'
    after = sources()
    write('source-after.json', after)
    assert before == after, 'Source or HEAD changed during preference validation'
print('Preference evidence:', evidence, flush=True)
print('Actual React: two processes restored dark/collapsed; private UUID isolation and exact cleanup passed; playCalls=0')
PY
