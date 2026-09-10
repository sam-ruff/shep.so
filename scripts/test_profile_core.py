#!/usr/bin/env python3
"""Run the shared profile crate's tests from this workspace under the root lock."""
import argparse
import json
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CRATE = "shep-profile-core"


def workspace_member(manifest, metadata, root=ROOT):
    """The monorepo ships the shared crate as a workspace member, never a Git pin or sibling checkout."""
    dependency = manifest["dependencies"][CRATE]
    if "path" not in dependency or any(key in dependency for key in ("git", "rev")):
        raise ValueError("Declare the shared crate as a workspace path dependency before testing.")
    expected = (root / dependency["path"]).resolve()
    packages = [p for p in metadata["packages"]
                if p["name"] == CRATE and p.get("source") is None
                and Path(p["manifest_path"]).parent.resolve() == expected]
    if len(packages) != 1 or packages[0]["id"] not in metadata.get("workspace_members", []):
        raise ValueError("The shared crate path must be a member of this workspace.")
    return expected


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT))
    workspace_member(manifest, metadata)
    subprocess.run(["cargo", "test", "-p", CRATE, "--locked", "--all-features"],
                   cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
