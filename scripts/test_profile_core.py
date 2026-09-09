#!/usr/bin/env python3
"""Run the exact pinned shared crate's tests without editing its Cargo checkout."""
import argparse
import json
import shutil
import subprocess
import tempfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
FIXTURES = ("profile-operation.json", "profile-cases.json", "profile-drive-file.json")
LOCK = ROOT / "tests/support/profile-core.Cargo.lock"


def source_package(manifest, metadata):
    dependency = manifest["dependencies"]["shep-profile-core"]
    revision = dependency["rev"]
    if len(revision) != 40 or any(c not in "0123456789abcdef" for c in revision):
        raise ValueError("Pin the shared crate to a complete Git commit before testing.")
    expected = f"git+{dependency['git']}?rev={revision}#{revision}"
    packages = [p for p in metadata["packages"]
                if p["name"] == "shep-profile-core" and p.get("source") == expected]
    if len(packages) != 1:
        raise ValueError("Cargo did not resolve the exact pinned shared crate.")
    return Path(packages[0]["manifest_path"]).parent


def prepare(source, destination, lock):
    """Copy only the crate and its committed fixtures; leave source untouched."""
    destination.mkdir(parents=True, exist_ok=False)
    shared = destination / "shared"
    shared.mkdir()
    crate = shared / "profile-core"
    shutil.copytree(source, crate, ignore=shutil.ignore_patterns("target", "Cargo.lock"))
    for name in FIXTURES:
        shutil.copyfile(source.parent / name, shared / name)
    manifest = crate / "Cargo.toml"
    text = manifest.read_text(encoding="utf-8")
    if "workspace" in tomllib.loads(text):
        raise ValueError("Review the shared crate's workspace changes before testing.")
    # Cargo refuses dev-dependency tests for a package outside its workspace.
    # This declaration affects only the disposable copy, not the dependency.
    manifest.write_text(text + "\n[workspace]\n", encoding="utf-8", newline="\n")
    if lock.is_file():
        shutil.copyfile(lock, crate / "Cargo.lock")
    return manifest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--update-lock", action="store_true",
                        help="Refresh the isolated test lock after a reviewed shared dependency update")
    args = parser.parse_args()
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
    source = source_package(manifest, metadata)
    artifacts = ROOT / "artifacts/dependency-tests"
    artifacts.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="profile-core-", dir=artifacts) as temporary:
        copied = prepare(source, Path(temporary) / "workspace", LOCK)
        if args.update_lock:
            subprocess.run(["cargo", "generate-lockfile", "--manifest-path", str(copied)],
                           cwd=ROOT, check=True)
            shutil.copyfile(copied.parent / "Cargo.lock", LOCK)
        if not LOCK.is_file():
            raise ValueError("Missing shared test lock. Run with --update-lock after reviewing the pin.")
        subprocess.run(["cargo", "test", "--manifest-path", str(copied), "--locked",
                        "--all-features", "--target-dir", str(ROOT / "target/profile-core-tests")],
                       cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
