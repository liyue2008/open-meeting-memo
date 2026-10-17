#!/usr/bin/env bash
# Actual React/WK interaction, synthetic private profile, and immutable run evidence; never play audio.
set -euo pipefail
umask 077
ROOT=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$ROOT"
EVIDENCE=${OMM_DESKTOP_UI_EVIDENCE:-$(mktemp -d /private/tmp/omm-desktop-ui-XXXXXX)}
export OMM_DESKTOP_UI_EVIDENCE="$EVIDENCE"
python3 - <<'PY'
import hashlib, json, os, pathlib, subprocess
root = pathlib.Path.cwd()
requested = pathlib.Path(os.environ['OMM_DESKTOP_UI_EVIDENCE'])
evidence = requested.resolve()
assert requested.is_absolute() and requested == evidence, 'UI evidence requires a canonical absolute path without symlinks'
assert evidence.is_relative_to(pathlib.Path('/private/tmp')), 'UI evidence requires a private /private/tmp root'
if not evidence.exists():
    evidence.mkdir(mode=0o700)
assert evidence.is_dir() and evidence.stat().st_uid == os.getuid(), 'UI evidence must be a directory owned by the caller'
assert not list(evidence.iterdir()), 'UI evidence must be a new empty directory'
evidence.chmod(0o700)
paths = set(filter(None, subprocess.check_output(['git', 'ls-files', '-z']).decode().split('\0')))
paths.update(str(p) for p in pathlib.Path('scripts/fixtures').rglob('*') if p.is_file())
paths.add('scripts/test-desktop-ui-wkwebview.sh')
def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()
def sources():
    return {p: digest(root / p) for p in sorted(paths) if (root / p).is_file()}
def write(name, value):
    path = evidence / name
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + '\n')
    path.chmod(0o600)
head = subprocess.check_output(['git', 'rev-parse', 'HEAD']).decode().strip()
before = sources()
write('source-before.json', {'head': head, 'files': before})
env = dict(os.environ, OMM_WKWEBVIEW_SCENARIO='ui', OMM_WKWEBVIEW_DRIVER=str(root / 'scripts/fixtures/desktop-ui-probe.js'), OMM_WKWEBVIEW_EVIDENCE=str(evidence))
with (evidence / 'command.log').open('w') as log:
    result = subprocess.run(['bash', 'packaging/desktop/test-wkwebview.sh'], env=env, stdout=log, stderr=subprocess.STDOUT)
(evidence / 'command.exit').write_text(str(result.returncode) + '\n')
after = sources()
write('source-after.json', {'head': subprocess.check_output(['git', 'rev-parse', 'HEAD']).decode().strip(), 'files': after})
artifacts = [root / 'target/debug/omm-desktop', evidence / 'driver.js', evidence / 'bridge-result.json', evidence / 'owned-process.json']
artifacts += sorted((root / 'apps/desktop/dist').rglob('*'))
write('artifact-sha256.json', {str(p): digest(p) for p in artifacts if p.is_file()})
print('UI evidence:', evidence, flush=True)
assert before == after, 'Source changed during actual UI run'
assert head == subprocess.check_output(['git', 'rev-parse', 'HEAD']).decode().strip(), 'HEAD changed during actual UI run'
assert result.returncode == 0, f'Actual UI fixture failed ({result.returncode}); inspect command.log and bridge-result.json'
receipt = json.loads((evidence / 'owned-process.json').read_text())
assert receipt['waited'] and receipt['exitCode'] == 0 and not receipt['terminatedOnTimeout'], receipt
result = json.loads((evidence / 'bridge-result.json').read_text())['result']
assert result['ok'] and result['playCalls'] == 0 and not result.get('mediaNotTested'), result
print(f"Actual React/WK passed: {len(result['checks'])} checks, playCalls=0, owned process waited")
PY
