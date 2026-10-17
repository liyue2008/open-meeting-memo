#!/usr/bin/env bash
# Real React first-use/service controls plus native CLI cases, using unchanged current-package resources.
set -euo pipefail
umask 077
ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$ROOT"
export OMM_DESKTOP_LIFECYCLE_EVIDENCE=${OMM_DESKTOP_LIFECYCLE_EVIDENCE:-$(mktemp -d /private/tmp/omm-lui-XXXXXX)}
python3 - <<'PY'
import hashlib, json, os, pathlib, re, subprocess
root = pathlib.Path.cwd()
evidence = pathlib.Path(os.environ['OMM_DESKTOP_LIFECYCLE_EVIDENCE'])
assert evidence.is_absolute() and evidence == evidence.resolve(), 'Lifecycle evidence requires a canonical absolute path without symlinks'
assert evidence.is_relative_to('/private/tmp'), 'Lifecycle evidence requires a private /private/tmp root'
if not evidence.exists():
    evidence.mkdir(mode=0o700)
assert evidence.is_dir() and evidence.stat().st_uid == os.getuid() and not list(evidence.iterdir()), 'Lifecycle evidence requires a new empty owned directory'
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
before = sources()
write('source-before.json', before)
app = pathlib.Path(os.environ['OMM_LIFECYCLE_RESOURCE_APP'])
assert app.is_absolute() and app.resolve(strict=True) == app, 'Current resource App requires a canonical absolute path'
manifest = app / 'Contents/Resources/bundle-manifest.json'
cli = app / 'Contents/Resources/bin/omm'
assert json.loads(manifest.read_text())['sourceRevision'] == before['head'], 'Lifecycle requires a package from the current clean revision'
assert not subprocess.check_output(['git', 'status', '--porcelain']).strip(), 'Lifecycle requires committed clean source'
resource = {'app': str(app), 'manifestHash': digest(manifest), 'cliHash': digest(cli)}
base = dict(os.environ)
base.pop('OMM_LIFECYCLE_DRIVER', None)
base.pop('OMM_WKWEBVIEW_STORE_MANIFEST', None)
results = []
try:
    for phase in ['react', 'native']:
        env = dict(base)
        if phase == 'react':
            driver = evidence / 'react-driver.js'
            driver.write_bytes((root / 'scripts/fixtures/desktop-lifecycle-probe.js').read_bytes())
            driver.chmod(0o600)
            env['OMM_LIFECYCLE_DRIVER'] = str(driver)
        log_path = evidence / (phase + '.log')
        with log_path.open('w') as log:
            # -x records the newly allocated native root even if a later stage fails.
            process = subprocess.run(['bash', '-x', 'packaging/desktop/test-lifecycle.sh'], env=env, stdout=log, stderr=subprocess.STDOUT)
        (evidence / (phase + '.exit')).write_text(str(process.returncode) + '\n')
        text = log_path.read_text()
        match = re.search(r'^Evidence: (.+)$', text, re.M) or re.search(r'^\+ EVIDENCE=(/private/tmp/omm-lifecycle-proof-[A-Za-z0-9]+)$', text, re.M)
        assert match, 'Native lifecycle must record its owned root'
        native_root = pathlib.Path(match[1])
        assert native_root.resolve(strict=True) == native_root and native_root.is_relative_to('/private/tmp')
        (evidence / (phase + '-root')).write_text(str(native_root) + '\n')
        assert process.returncode == 0, f'{phase} lifecycle failed; inspect {native_root} and {log_path}'
        result = json.loads((native_root / 'profile/bridge-result.json').read_text())['result']
        receipt = json.loads((native_root / 'profile/owned-process.json').read_text())
        provenance = json.loads((native_root / 'provenance.json').read_text())
        preservation = json.loads((native_root / 'cli-byte-preservation.json').read_text())
        assert result['ok'] and result['playCalls'] == 0
        assert receipt['waited'] and receipt['exitCode'] == 0 and not receipt['timeout'], receipt
        assert provenance['fixtureSourceRevision'] == provenance['resourceSourceRevision'] == before['head']
        assert provenance['sourceCLIEqualsStagedCLI'] and provenance['manifestUnchanged']
        assert provenance['sourceCliHash'] == provenance['cliHash'] == preservation['sourceCliHash'] == preservation['stagedCliHash'] == digest(cli) == resource['cliHash']
        assert provenance['sourceManifestHash'] == provenance['resourceManifestHash'] == preservation['sourceManifestHash'] == preservation['stagedManifestHash'] == digest(manifest) == resource['manifestHash']
        running = preservation['verifiedRunningCases']
        assert len(running) == 2 and running[0]['ownedPid'] != running[1]['ownedPid'] and all(c['actualExecutableVerified'] for c in running)
        lifecycle = result['nativeLifecycle']
        for key in ['allOwnedExited', 'dataPreserved', 'locksReleased', 'socketRemoved']:
            assert lifecycle[key] is True, (key, lifecycle)
        if phase == 'react':
            assert result['actualReactLifecycle'] is True
            for name in ['actual UI initialize yields safe config', 'expired real confirm disabled without implicit apply', 'actual unknown ownership refuses: 启动后台服务', 'actual unknown ownership refuses: 停止后台服务', 'actual unknown ownership refuses: 安全切换服务版本', 'rejected plan consumed in real UI: 确认启动', 'busy real confirm disabled', 'actual React action completed: 安全切换服务版本', 'actual React action completed: 停止后台服务', 'finish restores and waits listener plus capture peer']:
                assert name in result['checks'], name
            assert provenance['driverHash'] == digest(driver) == digest(root / 'scripts/fixtures/desktop-lifecycle-probe.js')
            assert [c['action'] for c in result['controls']] == ['stale-plan', 'restore', 'hold-unknown', 'restore', 'hold-busy', 'restore', 'hold-unknown', 'finish']
        artifacts = [native_root / 'provenance.json', native_root / 'cli-byte-preservation.json', native_root / 'profile/driver.js', native_root / 'profile/bridge-result.json', native_root / 'profile/owned-process.json', native_root / 'Lifecycle.app/Contents/MacOS/omm-desktop', native_root / 'Lifecycle.app/Contents/Resources/bin/omm', native_root / 'Lifecycle.app/Contents/Resources/bundle-manifest.json', cli, manifest]
        artifacts += sorted((root / 'apps/desktop/dist').rglob('*'))
        write(phase + '-artifact-sha256.json', {str(p): digest(p) for p in artifacts if p.is_file()})
        results.append({'phase': phase, 'root': str(native_root), 'checks': len(result['checks']), 'receipt': receipt, 'cliCases': running, 'nativeLifecycle': lifecycle})
    write('lifecycle-result.json', {'ok': True, 'playCalls': 0, 'resource': resource, 'phases': results})
finally:
    after = sources()
    write('source-after.json', after)
    assert before == after, 'Source or HEAD changed during lifecycle validation'
print('Lifecycle evidence:', evidence, flush=True)
print('Actual React initialization/expiry/stale/unknown/busy/start/switch/stop and original package CLI cases passed; playCalls=0')
PY
