"""Regression checks for the official fixture entry, without launching any app."""
import importlib.util
import json
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[2]

class OfficialFixtureRunnerTests(unittest.TestCase):
    def test_public_entry_uses_verified_app_support(self):
        text = (ROOT / 'packaging/desktop/test-wkwebview.sh').read_text()
        self.assertIn('fixture_runner.py', text)
        self.assertNotIn('child.kill()', text)
        self.assertNotIn('child.terminate()', text)

    def test_fixture_build_matches_release_and_isolates_feature_artifact(self):
        spec = importlib.util.spec_from_file_location('fixture_runner', ROOT / 'packaging/desktop/fixture_runner.py')
        assert spec is not None and spec.loader is not None
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        command, binary = runner.fixture_build_spec(ROOT)
        self.assertEqual(command, [
            'cargo', 'build', '--locked', '--release', '--target', 'aarch64-apple-darwin',
            '--target-dir', str(ROOT / 'target/desktop-fixture'), '-p', 'omm-desktop',
            '--features', 'custom-protocol,native-bridge-fixture',
        ])
        self.assertEqual(binary, ROOT / 'target/desktop-fixture/aarch64-apple-darwin/release/omm-desktop')
        self.assertNotEqual(binary, ROOT / 'target/aarch64-apple-darwin/release/omm-desktop')
        self.assertNotEqual(binary, ROOT / 'target/debug/omm-desktop')
        self.assertEqual(runner.FIXTURE_DEPLOYMENT_TARGET, '15.5')
        package = (ROOT / 'packaging/desktop/package.sh').read_text()
        self.assertIn('MACOSX_DEPLOYMENT_TARGET=15.5 cargo build --locked --release --target ' + runner.FIXTURE_TARGET, package)
        self.assertIn("build_env['MACOSX_DEPLOYMENT_TARGET'] = FIXTURE_DEPLOYMENT_TARGET", (ROOT / 'packaging/desktop/fixture_runner.py').read_text())

    def test_no_fixture_force_cleanup(self):
        family = ROOT / 'apps/desktop/src-tauri/src/bridge_fixture'
        for path in family.rglob('*.rs'):
            text = path.read_text()
            self.assertNotIn('.kill()', text, str(path))
            self.assertNotIn('libc::SIGTERM', text, str(path))
            self.assertNotIn('libc::SIGKILL', text, str(path))

    def test_timeout_and_cancel_keep_control_without_signal(self):
        spec = importlib.util.spec_from_file_location('fixture_runner', ROOT / 'packaging/desktop/fixture_runner.py')
        assert spec is not None and spec.loader is not None
        runner = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(runner)
        class RetainedFake:
            pid = 123456
            def __init__(self, error):
                self.error = error
            def wait(self, timeout):
                raise self.error
            def kill(self):
                raise AssertionError('must not kill')
            def terminate(self):
                raise AssertionError('must not terminate')
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            binary = root / 'synthetic-host'
            binary.write_bytes(b'no executable launched')
            for error, expected in [(subprocess.TimeoutExpired('fake', 1),124),(KeyboardInterrupt(),130)]:
                self.assertEqual(runner.await_owned(RetainedFake(error),1,root,'fake owned handle',binary),(expected,False))
                receipt = json.loads((root/'owned-process.json').read_text())
                self.assertFalse(receipt['waited'])
                self.assertTrue(receipt['workspaceRetained'])
                self.assertEqual(receipt['forcedSignals'],0)
                self.assertEqual(int((root/'runtime.exit').read_text()),expected)

    def test_argv_and_case_rejected_before_build(self):
        path = ROOT / 'packaging/desktop/fixture_runner.py'
        self.assertTrue(path.is_file())
        spec = importlib.util.spec_from_file_location('fixture_runner', path)
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for args, env in [(['lifecycle'], {}), ([], {'OMM_WKWEBVIEW_SCENARIO':'typo'}), (['unexpected-evidence', 'ui'], {})]:
            with self.assertRaises(ValueError):
                module.validate_request(args, env)
        self.assertEqual(module.validate_request([], {}), 'native')
        self.assertEqual(module.validate_request([], {'OMM_WKWEBVIEW_SCENARIO':'ui'}), 'ui')

class FixtureArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='omm-artifacts-', dir='/private/tmp')
        self.addCleanup(self.temporary.cleanup)
        self.base = pathlib.Path(self.temporary.name)
        self.root = self.base / 'source'
        self.root.mkdir()
        self.head = 'a' * 40

    def module(self):
        spec = importlib.util.spec_from_file_location('fixture_artifacts', ROOT / 'packaging/desktop/fixture_artifacts.py')
        assert spec is not None and spec.loader is not None
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def stage(self, name):
        import hashlib
        stage = self.base / name
        actual = stage / 'Fixture.app/Contents/MacOS/omm-desktop'
        actual.parent.mkdir(parents=True)
        actual.write_bytes(('signed private ' + name).encode())
        source = self.root / 'target/desktop-fixture/aarch64-apple-darwin/release/omm-desktop'
        source.parent.mkdir(parents=True, exist_ok=True)
        source.write_bytes(b'unsigned release fixture')
        def digest(path):
            return hashlib.sha256(path.read_bytes()).hexdigest()
        provenance = {'sourceRevision': self.head, 'scenario': 'ui', 'argv': [str(actual), '--ephemeral-webview'], 'fixtureSourceHost': str(source), 'actualHostHash': digest(actual), 'originalFixtureHostHash': digest(source)}
        (stage / 'runner-provenance.json').write_text(json.dumps(provenance))
        (stage / 'host-start.json').write_text(json.dumps({'sourceRevision': self.head, 'executable': str(actual), 'sha256': digest(actual)}))
        return stage, actual, source, provenance

    def test_debug_present_and_absent_bind_both_actual_hosts(self):
        module = self.module()
        for debug_exists in [False, True]:
            with self.subTest(debug_exists=debug_exists):
                stage, actual, source, provenance = self.stage('debug-' + str(debug_exists))
                debug = self.root / 'target/debug/omm-desktop'
                if debug_exists:
                    debug.parent.mkdir(parents=True)
                    debug.write_bytes(b'unrelated old debug')
                result = module.host_artifacts(self.root, stage, self.head, 'ui')
                self.assertEqual(result[str(actual)], provenance['actualHostHash'])
                self.assertEqual(result[str(source)], provenance['originalFixtureHostHash'])
                self.assertNotEqual(result[str(actual)], result[str(source)])
                self.assertNotIn(str(debug), result)

    def test_required_hosts_missing_or_hash_mismatch_fail(self):
        module = self.module()
        for kind in ['actual-missing', 'source-missing', 'actual-mismatch', 'source-mismatch']:
            with self.subTest(kind=kind):
                stage, actual, source, _ = self.stage(kind)
                path = actual if kind.startswith('actual') else source
                if kind.endswith('missing'):
                    path.unlink()
                else:
                    path.write_bytes(b'changed after provenance')
                with self.assertRaises((AssertionError, FileNotFoundError)):
                    module.host_artifacts(self.root, stage, self.head, 'ui')

    def test_missing_malformed_unrelated_or_outside_provenance_fails(self):
        module = self.module()
        for kind in ['missing', 'malformed', 'revision', 'scenario', 'other-stage', 'outside-source', 'start-hash']:
            with self.subTest(kind=kind):
                stage, actual, source, value = self.stage(kind)
                path = stage / 'runner-provenance.json'
                if kind == 'missing':
                    path.unlink()
                elif kind == 'malformed':
                    path.write_text('[]')
                elif kind == 'start-hash':
                    (stage / 'host-start.json').write_text(json.dumps({'sourceRevision':self.head, 'executable':str(actual), 'sha256':'wrong'}))
                else:
                    if kind == 'revision':
                        value['sourceRevision'] = 'b' * 40
                    if kind == 'scenario':
                        value['scenario'] = 'native'
                    if kind == 'other-stage':
                        value['argv'][0] = str(self.base / 'another/Fixture.app/Contents/MacOS/omm-desktop')
                    if kind == 'outside-source':
                        value['fixtureSourceHost'] = str(self.base / 'outside-host')
                    path.write_text(json.dumps(value))
                with self.assertRaises((AssertionError, FileNotFoundError)):
                    module.host_artifacts(self.root, stage, self.head, 'ui')

    def test_multiple_stages_bind_independently_without_inventing_unrun_host(self):
        module = self.module()
        write, write_host, _, _ = self.stage('write')
        read, read_host, _, _ = self.stage('read')
        unrun = self.base / 'not-run'
        unrun.mkdir()
        write_files = module.host_artifacts(self.root, write, self.head, 'ui')
        read_files = module.host_artifacts(self.root, read, self.head, 'ui')
        self.assertIn(str(write_host), write_files)
        self.assertNotIn(str(read_host), write_files)
        self.assertIn(str(read_host), read_files)
        self.assertNotIn(str(write_host), read_files)
        self.assertFalse(any(p.is_relative_to(unrun) for p in map(pathlib.Path, [*write_files, *read_files])))
        with self.assertRaises(FileNotFoundError):
            module.host_artifacts(self.root, unrun, self.head, 'ui')

if __name__ == '__main__':
    unittest.main()
