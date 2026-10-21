#!/usr/bin/env python3
"""Synthetic helper entry points only: no AppDelegate, device, permission or playback."""
import json
import os
import shutil
import sys
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock


RETAINED_NATIVE_CHILDREN = {}


def run_owned_native(command, environment, root, timeout=30.0, drain_timeout=5.0):
    child = None
    receipt = {"flag": command[1], "waited": False, "stdoutEof": False, "stderrEof": False}

    def record():
        path = root / "receipt.json"
        with os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as sink:
            json.dump(receipt, sink)
        if "OMM_CAPTURE_TEST_RECEIPTS" in os.environ:
            with open(os.environ["OMM_CAPTURE_TEST_RECEIPTS"], "a", encoding="utf-8") as sink:
                sink.write(json.dumps(receipt) + "\n")

    def drain(limit):
        assert child is not None
        stdout, stderr = child.communicate(timeout=limit)
        receipt.update(exit=child.returncode, waited=True, stdoutEof=True, stderrEof=True)
        return stdout, stderr

    try:
        child = subprocess.Popen(command, env=environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        receipt["pid"] = child.pid
        stdout, stderr = drain(timeout)
        if child.returncode != 0:
            raise subprocess.CalledProcessError(child.returncode, command)
        record()
    except BaseException as error:
        receipt["errorType"] = type(error).__name__
        if child is not None and not receipt["waited"]:
            try:
                drain(drain_timeout)
            except BaseException as cleanup_error:
                receipt["drainErrorType"] = type(cleanup_error).__name__
                # Keep the actual handle, not merely a PID or disappearance check.
                RETAINED_NATIVE_CHILDREN[str(root)] = child
        record()
        print("fixture workspace retained: " + str(root), file=sys.stderr)
        raise
    # No automatic TemporaryDirectory finalizer can remove a live child's files.
    shutil.rmtree(root)
    return stdout, stderr


class CaptureOperationalEventsTests(unittest.TestCase):
    def run_native(self, flag):
        binary = Path(os.environ["OMM_CAPTURE_TEST_BINARY"]).resolve(strict=True)
        root = Path(tempfile.mkdtemp(prefix="omm-cap-"))
        names = {
                "HOME": "home", "TMPDIR": "tmp", "XDG_CONFIG_HOME": "config",
                "XDG_DATA_HOME": "data", "XDG_STATE_HOME": "state",
                "XDG_CACHE_HOME": "cache", "XDG_RUNTIME_DIR": "runtime",
                "YULU_APPLICATION_SUPPORT_DIR": "support", "YULU_LOG_DIR": "logs",
                "YULU_MEDIA_LIBRARY_DIR": "media", "YULU_IPC_DIR": "ipc",
                "OMM_TEST_ROOT": "test",
            }
        environment = {"PATH": "/usr/bin:/bin:/usr/sbin:/sbin", "YULU_SERVICE_OWNER": "synthetic"}
        for key, name in names.items():
            directory = root / name
            directory.mkdir(mode=0o700)
            environment[key] = str(directory)
        stdout, _ = run_owned_native([str(binary), flag], environment, root)
        return stdout.decode()

    def test_capture_operational_events_are_bounded_and_redacted(self):
        output = self.run_native("--capture-events-self-test")
        self.assertIn("transition_bounded=true; arbitrary_content_redacted=true", output)

    def test_nonzero_mic_conversion_including_16k_and_44100(self):
        self.assertIn("input-format-self-test passed", self.run_native("--input-format-self-test"))

    def test_actual_system_callback_and_route_recovery(self):
        self.assertIn("safe_recovery=true", self.run_native("--input-format-lifecycle-self-test"))

    def test_mic_gate_zero_recovery_policy_and_300_second_boundary(self):
        self.assertIn("audio_daemon self-test ok", self.run_native("--self-test"))


class CaptureFixtureOwnershipTests(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="omm-driver-test-"))
        self.children = []
        self.popen = subprocess.Popen

    def tearDown(self):
        # Finite synthetic children only; cleanup requires actual wait and both EOFs.
        for child in self.children:
            child.communicate(timeout=2)
            if "OMM_CAPTURE_TEST_RECEIPTS" in os.environ:
                with open(os.environ["OMM_CAPTURE_TEST_RECEIPTS"], "a", encoding="utf-8") as sink:
                    sink.write(json.dumps({"scope": "ownership-test-teardown", "pid": child.pid,
                        "exit": child.returncode, "waited": True,
                        "stdoutEof": True, "stderrEof": True}) + "\n")
        RETAINED_NATIVE_CHILDREN.pop(str(self.root), None)
        if self.root.exists():
            shutil.rmtree(self.root)

    def spawn(self, *args, **kwargs):
        child = self.popen(*args, **kwargs)
        self.children.append(child)
        return child

    def invoke(self, first_error=None, timeout=1.0, drain_timeout=1.0):
        def spawn(*args, **kwargs):
            child = self.spawn(*args, **kwargs)
            if first_error is not None:
                communicate = child.communicate
                calls = [0]
                def interrupted(*a, **kw):
                    calls[0] += 1
                    if calls[0] == 1:
                        self.assertTrue(self.root.exists())
                        assert isinstance(first_error, BaseException)
                        raise first_error
                    return communicate(*a, **kw)
                child.communicate = interrupted
            return child
        command = [sys.executable, "-c", "import time,sys; time.sleep(.15); print('out'); print('err',file=sys.stderr)"]
        with mock.patch.object(subprocess, "Popen", side_effect=spawn):
            environment = {"PATH": "/usr/bin:/bin", "HOME": str(self.root), "TMPDIR": str(self.root)}
            return run_owned_native(command, environment, self.root,
                                    timeout=timeout, drain_timeout=drain_timeout)

    def test_normal_wait_and_both_eofs_before_cleanup(self):
        stdout, stderr = self.invoke()
        self.assertEqual((stdout, stderr), (b"out\n", b"err\n"))
        self.assertFalse(self.root.exists())
        self.assertEqual(self.children[0].returncode, 0)
        # tearDown still owns/waits the child, but there is no remaining workspace.
        self.root.mkdir(mode=0o700)

    def assert_retained_receipt(self, waited):
        receipt = json.loads((self.root / "receipt.json").read_text())
        self.assertEqual(receipt["waited"], waited)
        self.assertEqual(receipt["stdoutEof"], waited)
        self.assertEqual(receipt["stderrEof"], waited)
        self.assertTrue(self.root.exists())

    def test_timeout_drains_but_retains_failed_workspace(self):
        with self.assertRaises(subprocess.TimeoutExpired):
            self.invoke(timeout=.01)
        self.assert_retained_receipt(True)

    def test_keyboard_interrupt_drains_but_retains_workspace(self):
        with self.assertRaises(KeyboardInterrupt):
            self.invoke(first_error=KeyboardInterrupt())
        self.assert_retained_receipt(True)

    def test_communication_error_retains_live_workspace_and_handle(self):
        with self.assertRaises(OSError):
            self.invoke(first_error=OSError("synthetic communication failure"), drain_timeout=.01)
        self.assert_retained_receipt(False)
        self.assertIs(RETAINED_NATIVE_CHILDREN[str(self.root)], self.children[0])
        self.assertIsNone(self.children[0].returncode)
        # tearDown performs the actual later wait/drain before any directory removal.


if __name__ == "__main__":
    unittest.main()
