#!/usr/bin/env python3
"""Verify a locally built Linux archive and its bundled, Rust-free installer."""
import hashlib
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile


def verify(archive):
    archive = Path(archive).resolve()
    checksums = {line.split()[1]: line.split()[0]
                 for line in (archive.parent / "SHA256SUMS").read_text().splitlines()}
    if hashlib.sha256(archive.read_bytes()).hexdigest() != checksums.get(archive.name):
        raise ValueError("Release archive checksum mismatch")
    with tempfile.TemporaryDirectory(prefix="shep-release-check-") as directory:
        with tarfile.open(archive) as package:
            package.extractall(directory, filter="data")
        root = Path(directory)
        subprocess.run(["bash", str(root / "scripts/install-linux.sh"),
                        "--prefix", str(root / "prefix"), "--data-dir", str(root / "data")],
                       capture_output=True, text=True, check=True)
        if not (root / "prefix/bin/shep").is_file():
            raise AssertionError("Bundled installer did not install the release binary")
        if not (root / "data/applications/so.shep.Shep.desktop").is_file():
            raise AssertionError("Bundled installer did not register the launcher")
    print("Release checksum, extraction and bundled installer passed.")


if __name__ == "__main__":
    verify(sys.argv[1])
