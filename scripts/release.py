#!/usr/bin/env python3
"""Prepare a versioned native archive for semantic-release; never publish by itself."""
import hashlib
from pathlib import Path
import platform
import re
import subprocess
import sys
import tarfile


def prepare(version):
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError("Expected a semantic version")
    root = Path(__file__).resolve().parents[1]
    manifest = root / "Cargo.toml"
    original = manifest.read_text()
    manifest.write_text(re.sub(r'(?m)^version = "[^"]+"$', f'version = "{version}"', original, count=1))
    subprocess.run(["cargo", "build", "--release", "--no-default-features"], cwd=root, check=True)
    dist = root / "dist"
    dist.mkdir(exist_ok=True)
    system = platform.system().lower()
    binary = root / "target" / "release" / ("shep.exe" if system == "windows" else "shep")
    archive = dist / f"shep-{version}-{system}-{platform.machine().lower()}.tar.gz"
    with tarfile.open(archive, "w:gz") as package:
        package.add(binary, arcname=binary.name)
        for name in ("README.md", "LICENSE", "licenses/libcurl.txt", "licenses/curl-rust.txt", "vendor/curl/LICENSE", "vendor/curl/README.shep.md", "vendor/libsqlite3-sys/LICENSE", "vendor/libsqlite3-sys/OpenSSL-LICENSE.txt", "vendor/libsqlite3-sys/sqlcipher/LICENSE", "vendor/libsqlite3-sys/README.shep.md", "vendor/libsqlite3-sys/shep-lifecycle.patch", "vendor/libsqlite3-sys/shep-temp-policy.patch", "vendor/libsqlite3-sys/shep-export.patch", "scripts/install-linux.sh", "scripts/install_linux.py", "assets/launcher.png", "assets/shepherd-symbolic.svg", "vendor/shep-html-pixbuf/LICENSE", "vendor/shep-html-pixbuf/UPSTREAM.md", "vendor/iced_tiny_skia/LICENSE", "vendor/iced_tiny_skia/README.shep.md", "vendor/litehtml-sys/LICENSE", "vendor/litehtml-sys/README.shep.md", "vendor/litehtml-sys/vendor/litehtml/LICENSE", "vendor/litehtml-sys/vendor/litehtml/src/gumbo/LICENSE"):
            package.add(root / name, arcname=name)
    (dist / "SHA256SUMS").write_text(f"{hashlib.sha256(archive.read_bytes()).hexdigest()}  {archive.name}\n")
    if system == "linux":
        subprocess.run([sys.executable, str(root / "scripts/verify_release.py"), str(archive)], check=True)


if __name__ == "__main__":
    prepare(sys.argv[1])
