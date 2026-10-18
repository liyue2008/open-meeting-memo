#!/usr/bin/env python3
"""Official synthetic-only App fixture runner. Never signals a timed-out process."""
import hashlib
import json
import os
import pathlib
import shutil
import stat
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parents[2]
CASES = {'native', 'window-acl', 'ui', 'lifecycle', 'recovery', 'store'}
FIXTURE_TARGET = 'aarch64-apple-darwin'
FIXTURE_DEPLOYMENT_TARGET = '15.5'

def fixture_build_spec(root):
    # Keep fixture features out of the production package's release host.
    target_dir = root / 'target/desktop-fixture'
    command = ['cargo', 'build', '--locked', '--release', '--target', FIXTURE_TARGET,
               '--target-dir', str(target_dir), '-p', 'omm-desktop',
               '--features', 'custom-protocol,native-bridge-fixture']
    return command, target_dir / FIXTURE_TARGET / 'release/omm-desktop'

def validate_request(args, env):
    if args:
        raise ValueError('No positional arguments; select OMM_WKWEBVIEW_SCENARIO and OMM_WKWEBVIEW_EVIDENCE')
    case = env.get('OMM_WKWEBVIEW_SCENARIO', 'native')
    if case not in CASES:
        raise ValueError('Unknown private fixture scenario: ' + case)
    return case

def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()

def write(root, name, value):
    path = root / name
    with path.open('w') as output:
        json.dump(value, output, indent=2)
        output.write('\n')
    path.chmod(0o600)

def checked_path(path, directory=False):
    assert path.is_absolute() and path.resolve(strict=True) == path, 'canonical path required'
    entries = [path, *path.rglob('*')] if directory else [path]
    for entry in entries:
        meta = entry.lstat()
        assert not entry.is_symlink() and meta.st_uid == os.getuid(), str(entry)
        assert not meta.st_mode & 0o022, 'writable resource: ' + str(entry)
        assert stat.S_ISREG(meta.st_mode) or stat.S_ISDIR(meta.st_mode), str(entry)
        if entry.is_file():
            assert meta.st_nlink == 1, 'hardlinked resource refused'
    return path

def source_snapshot():
    names = subprocess.check_output(['git', '--no-optional-locks', 'ls-files', '-z'], cwd=ROOT).decode().split('\0')
    return {name: sha(ROOT / name) for name in names if name and (ROOT / name).is_file()}

def await_owned(child, timeout, evidence, identity, binary):
    try:
        code = child.wait(timeout=timeout)
    except (subprocess.TimeoutExpired, KeyboardInterrupt) as error:
        cancelled = isinstance(error, KeyboardInterrupt)
        code = 130 if cancelled else 124
        write(evidence, 'owned-process.json', {'pid':child.pid,'exitCode':None,'waited':False,'timeout':not cancelled,'cancelled':cancelled,'terminatedOnTimeout':False,'forcedSignals':0,'workspaceRetained':True,'controlIdentity':identity,'executable':str(binary),'sha256':sha(binary)})
        (evidence/'runtime.exit').write_text(str(code)+'\n')
        print('BLOCKED: owned host retained; inspect host-start/retained-control; no signals sent', file=sys.stderr)
        return code, False
    write(evidence, 'owned-process.json', {'pid':child.pid,'exitCode':code,'waited':True,'timeout':False,'cancelled':False,'terminatedOnTimeout':False,'forcedSignals':0,'workspaceRetained':True,'controlIdentity':identity})
    return code, True

def main(args=None):
    args = sys.argv[1:] if args is None else args
    case = validate_request(args, os.environ)  # before build, mkdir, or launch
    head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True).strip()
    assert not subprocess.check_output(['git', '--no-optional-locks', 'status', '--porcelain'], cwd=ROOT).strip(), 'committed clean source required'
    requested = os.environ.get('OMM_WKWEBVIEW_EVIDENCE')
    evidence = pathlib.Path(requested) if requested else pathlib.Path(tempfile.mkdtemp(prefix='omm-wk-', dir='/private/tmp'))
    assert evidence.is_absolute() and evidence == evidence.resolve() and evidence.is_relative_to('/private/tmp')
    assert len(os.fsencode(evidence)) <= 55, 'use a short /private/tmp evidence root (<=55 bytes)'
    evidence.mkdir(mode=0o700, exist_ok=True)
    checked_path(evidence)
    assert not (evidence / 'owned-process.json').exists(), 'cannot reuse an executed fixture root'
    evidence.chmod(0o700)
    with (evidence / 'runner-admission.json').open('x') as admission:
        json.dump({'sourceRevision': head, 'scenario': case}, admission)
    (evidence / 'runner-admission.json').chmod(0o600)
    print('Evidence:', evidence, flush=True)
    before = source_snapshot()
    write(evidence, 'runner-source-before.json', {'head': head, 'files': before})
    source_app = ROOT / 'target/desktop-package' / head / 'Open Meeting Memo.app'
    override = os.environ.get('OMM_WKWEBVIEW_RESOURCE_APP', os.environ.get('OMM_LIFECYCLE_RESOURCE_APP'))
    if override:
        assert pathlib.Path(override) == source_app, 'resource App must be this checkout/current revision official package'
    build_env = dict(os.environ)
    for key in ['TAURI_CONFIG', 'OMM_SOURCE_REVISION', 'OMM_NATIVE_FIXTURE_ROOT']:
        build_env.pop(key, None)
    def run(command, name):
        with (evidence / (name + '.log')).open('w') as log:
            done = subprocess.run(command, cwd=ROOT, env=build_env, stdout=log, stderr=subprocess.STDOUT)
        (evidence / (name + '.exit')).write_text(str(done.returncode) + '\n')
        assert done.returncode == 0, name + ' failed; inspect ' + str(evidence)
    if not source_app.exists():
        run(['bash', 'packaging/desktop/package.sh'], 'resource-build')
    checked_path(source_app, directory=True)
    run([sys.executable, 'packaging/desktop/resources.py', 'verify', str(source_app)], 'source-resource-verification')
    source_resources = source_app / 'Contents/Resources'
    manifest = json.loads((source_resources / 'bundle-manifest.json').read_text())
    assert manifest['sourceRevision'] == head
    source_cli_hash = sha(source_resources / 'bin/omm')
    source_manifest_hash = sha(source_resources / 'bundle-manifest.json')
    run(['npm', 'run', 'build', '--prefix', 'apps/desktop'], 'fixture-frontend-build')
    build_env['OMM_SOURCE_REVISION'] = head
    build_env['MACOSX_DEPLOYMENT_TARGET'] = FIXTURE_DEPLOYMENT_TARGET
    fixture_command, fixture_binary = fixture_build_spec(ROOT)
    run(fixture_command, 'fixture-build')
    original_host = checked_path(fixture_binary)
    app = evidence / 'Fixture.app'
    assert not app.exists(), 'fixture App already exists'
    shutil.copytree(source_app, app)
    binary = app / 'Contents/MacOS/omm-desktop'
    shutil.copy2(original_host, binary)
    run(['/usr/bin/codesign', '--force', '--sign', '-', '--timestamp=none', str(app)], 'fixture-sign')
    run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)], 'fixture-signature-verification')
    resources = app / 'Contents/Resources'
    for name, digest in manifest['files'].items():
        path = resources / name
        assert path.resolve() == path and sha(path) == digest, name
    assert sha(resources / 'bin/omm') == source_cli_hash and sha(resources / 'bundle-manifest.json') == source_manifest_hash
    selected = pathlib.Path(os.environ.get('OMM_WKWEBVIEW_DRIVER', str(ROOT / 'apps/desktop/src-tauri/fixtures/bridge-probe.js')))
    checked_path(selected)
    assert selected.is_relative_to(ROOT) or selected.is_relative_to('/private/tmp'), 'owned repository/private driver only'
    assert selected.stat().st_size < 128*1024
    samples = {}
    for path in (ROOT / 'contracts/desktop/v1/examples').glob('*.json'):
        value = json.loads(path.read_text())
        if isinstance(value, dict) and 'request' in value:
            samples[path.stem] = value['request']['payload']
    driver = selected.read_text().replace('__OMM_SAMPLES__', json.dumps(samples))
    (evidence / 'driver.js').write_text(driver)
    (evidence / 'driver.js').chmod(0o600)
    for name in ['home', 'runtime']:
        (evidence / name).mkdir(mode=0o700)
    env = {'PATH':'/usr/bin:/bin:/usr/sbin:/sbin','LANG':'en_US.UTF-8','TMPDIR':'/private/tmp','HOME':str(evidence/'home'),'OMM_NATIVE_FIXTURE_ROOT':str(evidence),'OMM_WKWEBVIEW_SCENARIO':case,'XDG_RUNTIME_DIR':str(evidence/'runtime')}
    for name in ['CONFIG','DATA','STATE','CACHE']:
        env['XDG_'+name+'_HOME'] = str(evidence / ('xdg-'+name.lower()))
    for key in ['OMM_WKWEBVIEW_STORE_MANIFEST','OMM_WKWEBVIEW_STORE_ACTION','OMM_NATIVE_REVIEW_FIXTURE','OMM_NATIVE_ARCHIVE_MEDIA','OMM_NATIVE_ARCHIVE_NONCE']:
        if key in os.environ:
            env[key] = os.environ[key]
    # Optional persistent-store proof is allocated/validated by the existing store fixture.
    host_args = [str(binary), '--webview-store-diagnostics']
    if case != 'store' and 'OMM_WKWEBVIEW_STORE_MANIFEST' not in env:
        host_args.append('--ephemeral-webview')
    provenance = {'sourceRevision':head,'sourceApp':str(source_app),'sourceSnapshot':before,'originalFixtureHostHash':sha(original_host),'actualHostHash':sha(binary),'sourceDriver':str(selected),'sourceDriverHash':sha(selected),'driverHash':sha(evidence/'driver.js'),'sourceCliHash':source_cli_hash,'cliHash':sha(resources/'bin/omm'),'sourceManifestHash':source_manifest_hash,'manifestHash':sha(resources/'bundle-manifest.json'),'scenario':case,'argv':host_args,'privateHome':env['HOME'],'environmentKeys':sorted(env)}
    provenance.update({'fixtureProfile':'release','fixtureTarget':FIXTURE_TARGET,'fixtureDeploymentTarget':FIXTURE_DEPLOYMENT_TARGET,'fixtureBuildCommand':fixture_command,'fixtureSourceHost':str(original_host)})
    write(evidence, 'runner-provenance.json', provenance)
    timeout = 420 if case == 'lifecycle' else 240 if case == 'ui' else 120
    with (evidence / 'runtime.log').open('w') as log:
        child = subprocess.Popen(provenance['argv'], env=env, cwd=evidence, stdout=log, stderr=subprocess.STDOUT)
        identity = subprocess.run(['/bin/ps','-p',str(child.pid),'-o','pid=,ppid=,uid=,lstart=,comm='], text=True, capture_output=True, check=False).stdout.strip()
        write(evidence, 'host-start.json', {'pid':child.pid,'identity':identity,'executable':str(binary),'sha256':sha(binary),'sourceRevision':head})
        code, waited = await_owned(child, timeout, evidence, identity, binary)
        if not waited:
            return code
    (evidence/'runtime.exit').write_text(str(code)+'\n')
    after = source_snapshot()
    write(evidence, 'runner-source-after.json', {'head':head,'files':after})
    assert before == after and head == subprocess.check_output(['git','rev-parse','HEAD'],cwd=ROOT,text=True).strip(), 'source changed during fixture'
    assert sha(source_resources/'bin/omm') == sha(resources/'bin/omm') == source_cli_hash
    assert sha(source_resources/'bundle-manifest.json') == sha(resources/'bundle-manifest.json') == source_manifest_hash
    if code != 0:
        return code if code > 0 else 1
    result = json.loads((evidence / 'bridge-result.json').read_text())['result']
    assert result['ok'] is True and result['playCalls'] == 0, result
    print('Actual fixture passed:',case,flush=True)
    return 0

if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (AssertionError, ValueError) as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(2) from None
