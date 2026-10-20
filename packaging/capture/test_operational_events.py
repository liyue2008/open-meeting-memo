#!/usr/bin/env python3
"""Synthetic helper entry points only: no AppDelegate, device, permission or playback."""
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


class CaptureOperationalEventsTests(unittest.TestCase):
    def run_native(self, flag):
        binary = Path(os.environ["OMM_CAPTURE_TEST_BINARY"]).resolve(strict=True)
        with tempfile.TemporaryDirectory(prefix="omm-cap-") as temporary:
            root = Path(temporary)
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
            child = subprocess.Popen([str(binary), flag], env=environment,
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            stdout, stderr = child.communicate()
            receipt = {"flag": flag, "pid": child.pid, "exit": child.returncode,
                       "waited": True, "stdoutEof": True, "stderrEof": True}
            if "OMM_CAPTURE_TEST_RECEIPTS" in os.environ:
                with open(os.environ["OMM_CAPTURE_TEST_RECEIPTS"], "a", encoding="utf-8") as sink:
                    sink.write(json.dumps(receipt) + "\n")
            self.assertEqual(child.returncode, 0, stderr.decode(errors="replace"))
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


if __name__ == "__main__":
    unittest.main()
