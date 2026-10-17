#!/usr/bin/env python3
"""Collect exact locked dependency notices and Rust source archives for redistribution."""
import hashlib
import json
import pathlib
import shutil
import subprocess
import sys
import tomllib

ROOT = pathlib.Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def documents(root):
    for path in root.iterdir():
        if path.name.lower().startswith(("license", "copying", "notice", "copyright", "unlicense")):
            if path.is_file():
                yield path
            elif path.is_dir():
                yield from (child for child in path.rglob("*") if child.is_file())


def stage(resources):
    dest = resources / "licenses/dependencies"
    dest.mkdir(parents=True)
    metadata = json.loads(subprocess.check_output([
        "cargo", "metadata", "--locked", "--filter-platform", "aarch64-apple-darwin", "--format-version", "1"
    ], cwd=ROOT))
    active = {node["id"] for node in metadata["resolve"]["nodes"]}
    checksums = {(p["name"], p["version"]): p.get("checksum") for p in tomllib.loads((ROOT / "Cargo.lock").read_text())["package"]}
    index = []
    for package in metadata["packages"]:
        if package["id"] not in active or package["source"] is None:
            continue
        root = pathlib.Path(package["manifest_path"]).parent
        name = f"{package['name']}-{package['version']}"
        target = dest / "rust" / name
        target.mkdir(parents=True)
        for path in documents(root):
            relative = path.relative_to(root)
            output = target / relative
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, output)
        # Full unmodified crate sources accompany every linked crate, including MPL source obligations.
        # Verify Cargo.lock, not merely the name of an existing local Cargo cache archive.
        registry = root.parent.name
        archive = root.parents[2] / "cache" / registry / f"{name}.crate"
        assert archive.is_file() and digest(archive) == checksums[(package["name"], package["version"])], name
        shutil.copyfile(archive, target / archive.name)
        shutil.copyfile(root / "Cargo.toml", target / "Cargo.toml")
        readme = next((p for p in root.iterdir() if p.name.lower().startswith("readme") and p.is_file()), None)
        if readme:
            shutil.copyfile(readme, target / readme.name)
        index.append({"ecosystem": "Rust", "name": package["name"], "version": package["version"], "license": package["license"], "authors": package["authors"], "repository": package["repository"], "sourceArchiveSha256": digest(archive)})
    lock = json.loads((ROOT / "apps/desktop/package-lock.json").read_text())
    for location, pin in lock["packages"].items():
        if not location.startswith("node_modules/"):
            continue
        root = ROOT / "apps/desktop" / location
        if not root.exists():
            continue  # Optional binaries for other architectures are not installed or distributed.
        package = json.loads((root / "package.json").read_text())
        target = dest / "javascript" / location.removeprefix("node_modules/")
        target.mkdir(parents=True, exist_ok=True)
        for path in documents(root):
            output = target / path.relative_to(root)
            output.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, output)
        shutil.copyfile(root / "package.json", target / "package.json")
        index.append({"ecosystem": "JavaScript", "name": package["name"], "version": package["version"], "license": package.get("license"), "integrity": pin.get("integrity")})
    shutil.copyfile(ROOT / "Cargo.lock", dest / "Cargo.lock")
    shutil.copyfile(ROOT / "apps/desktop/package-lock.json", dest / "package-lock.json")
    (dest / "index.json").write_text(json.dumps(index, indent=2) + "\n")
    for path in dest.rglob("*"):
        assert not path.is_symlink()
        path.chmod(0o755 if path.is_dir() else 0o644)


if __name__ == "__main__":
    stage(pathlib.Path(sys.argv[1]))
