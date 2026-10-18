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

if __name__ == '__main__':
    unittest.main()
