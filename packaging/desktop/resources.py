#!/usr/bin/env python3
"""Build-time resource assembly. Not shipped as an application runtime dependency."""

import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
import tarfile
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[2]
PINS = json.loads((ROOT / "packaging/desktop/dependencies.json").read_text())


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def run(*args):
    return subprocess.check_output(args, text=True, stderr=subprocess.STDOUT)


def dependencies(dest):
    dest.mkdir(parents=True, exist_ok=True)
    archive = dest / "sherpa-onnx-1.13.2.tar.bz2"
    if not archive.exists():
        stage = archive.with_suffix(".download")
        with (
            urllib.request.urlopen(PINS["url"], timeout=180) as source,
            open(stage, "wb") as out,
        ):
            shutil.copyfileobj(source, out)
        assert sha(stage) == PINS["sha256"], "sherpa download checksum mismatch"
        stage.rename(archive)
    assert sha(archive) == PINS["sha256"], "sherpa archive checksum mismatch"
    prefix = "sherpa-onnx-v1.13.2-osx-arm64-shared-no-tts-lib/lib/"
    with tarfile.open(archive) as tar:
        for name in ["libsherpa-onnx-c-api.dylib", "libonnxruntime.1.24.4.dylib"]:
            member = tar.getmember(prefix + name)
            assert member.isfile() and 0 < member.size < 128 * 1024 * 1024
            src = tar.extractfile(member)
            assert src is not None
            with src, open(dest / name, "wb") as out:
                shutil.copyfileobj(src, out)
    for name, expected in PINS["licenses"].items():
        assert sha(ROOT / "packaging/desktop/licenses" / name) == expected
    for pin in json.loads((ROOT / "packaging/desktop/licenses/SOURCES.json").read_text()):
        assert sha(ROOT / "packaging/desktop/licenses" / pin["file"]) == pin["sha256"]


def stage(resources, deps):
    for name in ["libsherpa-onnx-c-api.dylib", "libonnxruntime.1.24.4.dylib"]:
        shutil.copyfile(deps / name, resources / "sherpa" / name)
    for path in (ROOT / "packaging/desktop/licenses").iterdir():
        shutil.copyfile(path, resources / "licenses" / path.name)
    shutil.copyfile(ROOT / "LICENSE", resources / "licenses/OMM-APACHE-2.0.txt")
    shutil.copyfile(
        ROOT / "native/yulu-capture/LICENSE", resources / "licenses/Yulu-MIT.txt"
    )
    shutil.copyfile(
        ROOT / "native/yulu-capture/NOTICE.md", resources / "licenses/Yulu-NOTICE.md"
    )
    shutil.copyfile(
        ROOT / "packaging/desktop/dependencies.json",
        resources / "licenses/dependencies.json",
    )
    from notices import stage as stage_notices
    stage_notices(resources)
    audio = ROOT / "target/audio-toolchain"
    names = [
        "ffmpeg",
        "ffprobe",
        "manifest.json",
        "sources.json",
        "build-dev.sh",
        "licenses/ffmpeg.txt",
        "licenses/opus.txt",
        "sources/ffmpeg-8.1.2.tar.xz",
        "sources/opus-1.5.2.tar.gz",
    ]
    for name in names:
        target = resources / "audio-toolchain" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(audio / name, target)
        target.chmod(0o755 if name in ["ffmpeg", "ffprobe", "build-dev.sh"] else 0o644)
    # Generated bundle resources are immutable user-owned inputs, not sensitive meeting artifacts.
    for path in resources.rglob("*"):
        assert not path.is_symlink(), "bundled resource symlink refused"
        if path.is_dir():
            path.chmod(0o755)
    # Dependencies are self-contained and use @loader_path. No personal absolute library dependency survives.
    for path in (resources / "sherpa").glob("*.dylib"):
        for line in run("otool", "-L", str(path)).splitlines()[1:]:
            dep = line.strip().split(" ")[0]
            assert dep.startswith(
                ("/System/Library/", "/usr/lib/", "@rpath/", "@loader_path/")
            ), dep


def manifest(resources, revision):
    files = {
        str(path.relative_to(resources)): sha(path)
        for path in sorted(resources.rglob("*"))
        if path.is_file() and path.name != "bundle-manifest.json"
    }
    version = json.loads((ROOT / "apps/desktop/src-tauri/tauri.conf.json").read_text())[
        "version"
    ]
    (resources / "bundle-manifest.json").write_text(
        json.dumps(
            {
                "schemaVersion": 1,
                "version": version,
                "sourceRevision": revision,
                "files": files,
            },
            indent=2,
        )
        + "\n"
    )
    (resources / "bundle-manifest.json").chmod(0o644)


def verify(app):
    resources = app / "Contents/Resources"
    value = json.loads((resources / "bundle-manifest.json").read_text())
    assert len(value["sourceRevision"]) == 40 and value["schemaVersion"] == 1
    for name, expected in value["files"].items():
        path = resources / name
        assert path.resolve() == path and path.is_file() and sha(path) == expected, name
    binaries = [
        app / "Contents/MacOS/omm-desktop",
        resources / "bin/omm",
        resources / "bin/omm-asr-worker",
        resources / "OMMCapture.app/Contents/MacOS/omm-capture",
        *list((resources / "sherpa").glob("*.dylib")),
        resources / "audio-toolchain/ffmpeg",
        resources / "audio-toolchain/ffprobe",
    ]
    for path in binaries:
        assert run("lipo", "-archs", str(path)).strip() == "arm64", str(path)
        run("codesign", "--verify", "--strict", str(path))
        for line in run("otool", "-L", str(path)).splitlines()[1:]:
            dep = line.strip().split(" ")[0]
            assert dep.startswith(
                ("/System/Library/", "/usr/lib/", "@rpath/", "@loader_path/")
            ), dep
            if dep.startswith("@rpath/"):
                assert (resources / "sherpa" / dep[7:]).exists(), dep
    run("codesign", "--verify", "--deep", "--strict", str(app))
    import plistlib

    assert (
        plistlib.loads((app / "Contents/Info.plist").read_bytes())[
            "LSMinimumSystemVersion"
        ]
        == "15.5"
    )
    helper = plistlib.loads(
        (resources / "OMMCapture.app/Contents/Info.plist").read_bytes()
    )
    assert helper["CFBundleIdentifier"] == "org.openmeetingmemo.capture"
    entitlements = plistlib.loads(subprocess.check_output(
        ["codesign", "-d", "--entitlements", ":-", str(resources / "OMMCapture.app")],
        stderr=subprocess.DEVNULL,
    ))
    assert entitlements == plistlib.loads((ROOT / "packaging/capture/OMMCapture.entitlements").read_bytes())
    for path in binaries[:3]:
        assert "minos 15.5" in run("xcrun", "vtool", "-show-build", str(path))
    assert (resources / "OMM.icns").stat().st_size > 0
    print(
        json.dumps(
            {
                "verified": True,
                "sourceRevision": value["sourceRevision"],
                "version": value["version"],
                "manifestSha256": sha(resources / "bundle-manifest.json"),
                "files": len(value["files"]),
                "architecture": "arm64",
                "signing": "ad-hoc-development",
            }
        )
    )


if __name__ == "__main__":
    command, *args = sys.argv[1:]
    if command == "dependencies":
        dependencies(pathlib.Path(args[0]))
    elif command == "stage":
        stage(pathlib.Path(args[0]), pathlib.Path(args[1]))
    elif command == "manifest":
        manifest(pathlib.Path(args[0]), args[1])
    elif command == "verify":
        verify(pathlib.Path(args[0]).resolve())
    else:
        raise SystemExit("unknown packaging operation")
