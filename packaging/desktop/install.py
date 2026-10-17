#!/usr/bin/env python3
"""Metadata-only preflight, then explicit side-by-side copy; never reads config/authentication content."""

import json
import os
import pathlib
import subprocess
import sys
import uuid

from resources import sha, verify

source = pathlib.Path(sys.argv[1]).resolve(strict=True)
check = sys.argv[2] == "true"
assert source.name == "Open Meeting Memo.app"
verify(source)
manifest = json.loads((source / "Contents/Resources/bundle-manifest.json").read_text())
home = pathlib.Path.home().resolve(strict=True)
applications = home / "Applications"
if applications.exists():
    assert applications.resolve() == applications and applications.is_dir()
    assert applications.stat().st_uid == os.geteuid()
    assert applications.stat().st_mode & 0o022 == 0
name = f"Open Meeting Memo {manifest['version']}-{manifest['sourceRevision'][:12]}.app"
target = applications / name
receipt = (
    home / ".local/state/omm-desktop-installs" / f"{manifest['sourceRevision']}.json"
)
# Inspect metadata only. No reading or backing up config.json, Pi profiles, models or recordings.
observed = []
for path in [
    home / ".local/bin/omm",
    home / "Library/LaunchAgents/org.openmeetingmemo.service.plist",
    target,
]:
    if path.exists() or path.is_symlink():
        stat = path.lstat()
        observed.append(
            {
                "path": str(path),
                "uid": stat.st_uid,
                "mode": oct(stat.st_mode & 0o777),
                "symlink": path.is_symlink(),
            }
        )
result = {
    "preflight": True,
    "sourceRevision": manifest["sourceRevision"],
    "source": str(source),
    "target": str(target),
    "existingMetadata": observed,
    "serviceChanged": False,
    "personalDataRead": False,
    "checkOnly": check,
}
if not check:
    applications.mkdir(mode=0o700, exist_ok=True)
    if target.exists():
        verify(target)
        assert sha(target / "Contents/Resources/bundle-manifest.json") == sha(
            source / "Contents/Resources/bundle-manifest.json"
        )
        assert sha(target / "Contents/MacOS/omm-desktop") == sha(
            source / "Contents/MacOS/omm-desktop"
        )
    else:
        stage = applications / f".omm-stage-{uuid.uuid4()}.app"
        assert not stage.exists()
        subprocess.run(["ditto", str(source), str(stage)], check=True)
        verify(stage)
        assert sha(stage / "Contents/MacOS/omm-desktop") == sha(
            source / "Contents/MacOS/omm-desktop"
        )
        stage.rename(target)
    # Receipt is non-secret package metadata, never a duplicate of any user configuration.
    for directory in [home / ".local", home / ".local/state", receipt.parent]:
        if directory.exists():
            assert directory.resolve() == directory and directory.is_dir()
            assert (
                directory.stat().st_uid == os.geteuid()
                and directory.stat().st_mode & 0o022 == 0
            )
        else:
            directory.mkdir(mode=0o700)
    if not receipt.exists():
        with receipt.open("x") as output:
            os.chmod(receipt, 0o600)
            json.dump(
                {
                    **result,
                    "manifestSha256": sha(
                        target / "Contents/Resources/bundle-manifest.json"
                    ),
                    "executableSha256": sha(target / "Contents/MacOS/omm-desktop"),
                },
                output,
                indent=2,
            )
    result["installed"] = True
    result["receipt"] = str(receipt)
print(json.dumps(result))
