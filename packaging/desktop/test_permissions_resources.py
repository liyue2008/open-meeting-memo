#!/usr/bin/env python3
"""Finite synthetic bundle metadata only; all native tools are injected."""
import contextlib
import io
import json
from pathlib import Path
import plistlib
import tempfile
import unittest
from unittest.mock import patch
import resources

class PermissionResourceTests(unittest.TestCase):
    def test_purpose_and_signing_contracts_without_real_tools(self):
        with tempfile.TemporaryDirectory(prefix="omm-purpose-") as root:
            app = Path(root).resolve() / "Synthetic.app"
            res = app / "Contents/Resources"
            info = {"LSMinimumSystemVersion":"15.5", **{key:"Explicit action" for key in ["NSMicrophoneUsageDescription", "NSAudioCaptureUsageDescription", "NSScreenCaptureUsageDescription"]}}
            (app / "Contents").mkdir(parents=True)
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
            helper = res / "OMMCapture.app/Contents/Info.plist"
            helper.parent.mkdir(parents=True)
            helper.write_bytes(plistlib.dumps({"CFBundleIdentifier":"org.openmeetingmemo.capture"}))
            for name in ["bin/omm", "bin/omm-asr-worker", "OMMCapture.app/Contents/MacOS/omm-capture", "audio-toolchain/ffmpeg", "audio-toolchain/ffprobe", "OMM.icns"]:
                p = res / name; p.parent.mkdir(parents=True,exist_ok=True); p.write_bytes(b"synthetic-not-executable")
            binary = app / "Contents/MacOS/omm-desktop"; binary.parent.mkdir(); binary.write_bytes(b"synthetic")
            manifest = {"sourceRevision":"3"*40,"schemaVersion":1,"version":"synthetic","files":{}}
            (res / "bundle-manifest.json").write_text(json.dumps(manifest))
            def run(*args):
                return "arm64" if args[0]=="lipo" else "minos 15.5" if args[0]=="xcrun" else ""
            def entitlements(args,**kwargs):
                path = Path(args[-1])
                return (resources.ROOT / ("packaging/capture/OMMCapture.entitlements" if path.name=="OMMCapture.app" else "packaging/desktop/OMMDesktop.entitlements")).read_bytes()
            with patch.object(resources,"run",run), patch.object(resources.subprocess,"check_output",entitlements), contextlib.redirect_stdout(io.StringIO()):
                resources.verify(app)
                for key in ["NSMicrophoneUsageDescription", "NSAudioCaptureUsageDescription", "NSScreenCaptureUsageDescription"]:
                    changed = dict(info); changed.pop(key)
                    (app / "Contents/Info.plist").write_bytes(plistlib.dumps(changed))
                    with self.assertRaises(AssertionError): resources.verify(app)
                (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))
                for subject in [app, res / "bin/omm", binary]:
                    def wrong(args, **kwargs):
                        return plistlib.dumps({}) if Path(args[-1])==subject else entitlements(args,**kwargs)
                    with patch.object(resources.subprocess,"check_output",wrong), self.assertRaises(AssertionError): resources.verify(app)

if __name__ == "__main__": unittest.main()
