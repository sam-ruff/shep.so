"""Prepare native desktop release archives without publishing them."""
import argparse
import gzip
import hashlib
import io
import json
import os
import re
import stat
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

import tomllib

ROOT = Path(__file__).resolve().parents[1]
MANIFESTS = {
    "Cargo.toml": "shep",
    "shared/mail-core/Cargo.toml": "shep-mail-core",
    "shared/mail-content/Cargo.toml": "shep-mail-content",
    "shared/profile-core/Cargo.toml": "shep-profile-core",
}
LOCKS = ("Cargo.lock", "backend/Cargo.lock", "flutter/rust/Cargo.lock")
VERSION_FILES = (*MANIFESTS, *LOCKS)
TARGETS = {
    "x86_64-unknown-linux-gnu": ("linux", "x86_64", "shep"),
    "x86_64-pc-windows-msvc": ("windows", "x86_64", "shep.exe"),
}
CONTENTS = (
    "README.md", "LICENSE", "licenses/libcurl.txt", "licenses/curl-rust.txt",
    "vendor/curl/LICENSE", "vendor/curl/README.shep.md",
    "vendor/libsqlite3-sys/LICENSE", "vendor/libsqlite3-sys/OpenSSL-LICENSE.txt",
    "vendor/libsqlite3-sys/sqlcipher/LICENSE", "vendor/libsqlite3-sys/README.shep.md",
    "vendor/libsqlite3-sys/shep-lifecycle.patch", "vendor/libsqlite3-sys/shep-temp-policy.patch",
    "vendor/libsqlite3-sys/shep-export.patch", "scripts/install-linux.sh", "scripts/install_linux.py",
    "assets/launcher.png", "assets/shepherd-symbolic.svg", "assets/shepherd-light.svg",
    "assets/shepherd-tray.svg", "vendor/shep-html-pixbuf/LICENSE", "vendor/shep-html-pixbuf/UPSTREAM.md",
    "vendor/iced_tiny_skia/LICENSE", "vendor/iced_tiny_skia/README.shep.md",
    "vendor/litehtml-sys/LICENSE", "vendor/litehtml-sys/README.shep.md",
    "vendor/litehtml-sys/vendor/litehtml/LICENSE", "vendor/litehtml-sys/vendor/litehtml/src/gumbo/LICENSE",
)


def validate_version(version):
    number = r"(?:0|[1-9][0-9]*)"
    if not re.fullmatch(rf"{number}\.{number}\.{number}(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?", version):
        raise ValueError("Expected a semantic version without build metadata")
    if "-" in version:
        for identifier in version.split("-", 1)[1].split("."):
            if identifier.isdigit() and len(identifier) > 1 and identifier.startswith("0"):
                raise ValueError("Numeric prerelease identifiers cannot have leading zeroes")


def sha256(path):
    with Path(path).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def version_updates(version, root):
    validate_version(version)
    updates = {}
    for path, name in MANIFESTS.items():
        text = (root / path).read_text(encoding="utf-8")
        if tomllib.loads(text).get("package", {}).get("name") != name:
            raise ValueError(f"Unexpected Cargo package in {path}")
        section = re.search(r"(?ms)^\[package\][ \t]*\n(.*?)(?=^\[|\Z)", text)
        if section is None:
            raise ValueError(f"Cargo package section is missing in {path}")
        updated, count = re.subn(r'(?m)^version[ \t]*=[ \t]*"[^"]+"[ \t]*$', f'version = "{version}"', section[1])
        if count != 1:
            raise ValueError(f"Expected one explicit package version in {path}")
        updates[path] = text[:section.start(1)] + updated + text[section.end(1):]
    names = set(MANIFESTS.values())
    for path in LOCKS:
        text = (root / path).read_text(encoding="utf-8")
        packages = tomllib.loads(text).get("package", [])
        expected = names if path == "Cargo.lock" else names - {"shep"}
        local = [package["name"] for package in packages if package["name"] in names and "source" not in package]
        if set(local) != expected or len(local) != len(expected):
            raise ValueError(f"Missing or repeated local packages in {path}")
        blocks = text.split("[[package]]")
        for index, block in enumerate(blocks[1:], 1):
            package = tomllib.loads(block)
            if package.get("name") in names and "source" not in package:
                updated, count = re.subn(r'(?m)^version = "[^"]+"$', f'version = "{version}"', block)
                if count != 1:
                    raise ValueError(f"Missing local package version in {path}")
                blocks[index] = updated
        updates[path] = "[[package]]".join(blocks)
    return updates


def stamp(version, root=ROOT):
    for path, text in version_updates(version, root).items():
        (root / path).write_text(text, encoding="utf-8")


def verify_versions(version, root=ROOT):
    for path, text in version_updates(version, root).items():
        if text != (root / path).read_text(encoding="utf-8"):
            raise ValueError(f"Release version was not stamped in {path}")


def git_identity(root, source=None, run=subprocess.run):
    actual = run(["git", "rev-parse", "HEAD"], cwd=root, check=True, text=True, stdout=subprocess.PIPE).stdout.strip()
    if not re.fullmatch(r"[0-9a-f]{40}", actual) or (source is not None and source != actual):
        raise ValueError("Release source must match the checked-out full Git commit")
    epoch = run(["git", "show", "-s", "--format=%ct", actual], cwd=root, check=True, text=True, stdout=subprocess.PIPE).stdout.strip()
    if not epoch.isdigit():
        raise ValueError("Git did not provide the source commit timestamp")
    return actual, int(epoch)


def verify_source_tree(root, run=subprocess.run):
    tracked = run(["git", "diff", "--name-only", "HEAD", "--"], cwd=root,
                  check=True, text=True, stdout=subprocess.PIPE).stdout.splitlines()
    untracked = run(["git", "ls-files", "--others", "--exclude-standard"], cwd=root,
                    check=True, text=True, stdout=subprocess.PIPE).stdout.splitlines()
    if set(tracked) - {*VERSION_FILES, "CHANGELOG.md"} or untracked:
        raise ValueError("Release source has changes outside the version/changelog preparation")
    for path in set(tracked) & set(VERSION_FILES):
        original = run(["git", "show", f"HEAD:{path}"], cwd=root,
                       check=True, text=True, stdout=subprocess.PIPE).stdout
        if without_release_versions(path, original) != without_release_versions(path, (root / path).read_text(encoding="utf-8")):
            raise ValueError("Release manifests or locks contain changes beyond the owned package versions")


def without_release_versions(path, text):
    data = tomllib.loads(text)
    if path in MANIFESTS:
        data["package"]["version"] = ""
    else:
        for package in data.get("package", []):
            if package["name"] in MANIFESTS.values() and "source" not in package:
                package["version"] = ""
    return data


def native_target(run=subprocess.run):
    output = run(["rustc", "-vV"], check=True, text=True, stdout=subprocess.PIPE).stdout
    targets = re.findall(r"(?m)^host: (\S+)$", output)
    if len(targets) != 1 or targets[0] not in TARGETS:
        raise ValueError("This release supports x86_64 GNU/Linux and Windows MSVC targets only")
    return targets[0]


def build(root, target, target_dir=None, run=subprocess.run):
    command = ["cargo", "build", "--manifest-path", str(root / "Cargo.toml"), "--locked", "--release",
               "--no-default-features", "--package", "shep", "--bin", "shep", "--target", target,
               "--jobs", "4", "--message-format=json-render-diagnostics"]
    if target == "x86_64-pc-windows-msvc":
        command.extend(["--config", 'target.x86_64-pc-windows-msvc.rustflags=["-C","target-feature=+crt-static"]'])
    if target_dir is not None:
        command.extend(["--target-dir", str(target_dir)])
    result = run(command, cwd=root, check=True, text=True, stdout=subprocess.PIPE)
    binaries = set()
    for line in result.stdout.splitlines():
        item = json.loads(line)
        if (item.get("reason") == "compiler-artifact" and item.get("target", {}).get("name") == "shep"
                and item.get("target", {}).get("kind") == ["bin"] and item.get("executable")):
            if item.get("profile", {}).get("test") or "test-support" in item.get("features", []):
                raise ValueError("Cargo reported a test executable for the release")
            binaries.add(Path(item["executable"]))
    if len(binaries) != 1:
        raise ValueError("Cargo did not report exactly one production Shep executable")
    binary = binaries.pop()
    regular_file(binary)
    return binary


def regular_file(path):
    if not stat.S_ISREG(path.lstat().st_mode):
        raise ValueError(f"Release input is not a regular file: {path.name}")


def archive_name(version, target):
    system, architecture, _ = TARGETS[target]
    return f"shep-{version}-{system}-{architecture}.tar.gz"


def check_dist(dist, version, source):
    allowed = {archive_name(version, target) for target in TARGETS}
    for path in dist.glob("*.tar.gz"):
        if path.name not in allowed:
            raise ValueError("dist contains another release; use a clean release workspace")
        regular_file(path)
        with tarfile.open(path, "r:gz") as archive:
            members = [member for member in archive if member.name == "release.json"]
            if len(members) != 1 or not members[0].isreg() or members[0].size > 4096:
                raise ValueError("An existing release archive has no unique provenance")
            metadata = json.load(archive.extractfile(members[0]))
        if metadata.get("version") != version or metadata.get("source") != source:
            raise ValueError("dist contains archives from a different source revision")


def checksums(dist):
    return "".join(f"{sha256(path)}  {path.name}\n" for path in sorted(dist.glob("*.tar.gz")))


def package(version, target, source, epoch, binary, root=ROOT):
    validate_version(version)
    system, architecture, binary_name = TARGETS[target]
    inputs = {name: root / name for name in CONTENTS}
    inputs[binary_name] = binary
    for path in inputs.values():
        regular_file(path)
    dist = root / "dist"
    dist.mkdir(exist_ok=True)
    check_dist(dist, version, source)
    metadata = json.dumps({"schema": 1, "version": version, "source": source, "target": target,
                           "platform": system, "architecture": architecture, "binary": binary_name,
                           "binary_sha256": sha256(binary)}, sort_keys=True, indent=2).encode() + b"\n"
    archive = dist / archive_name(version, target)
    with tempfile.TemporaryDirectory(prefix=".shep-package-", dir=dist) as temporary:
        prepared = Path(temporary) / archive.name
        with (prepared.open("wb") as destination,
              gzip.GzipFile(fileobj=destination, mode="wb", filename="", mtime=epoch) as compressed,
              tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as output):
            for name in sorted((*inputs, "release.json")):
                info = tarfile.TarInfo(name)
                info.mtime = epoch
                info.mode = 0o755 if name in (binary_name, "scripts/install-linux.sh") else 0o644
                if name == "release.json":
                    info.size = len(metadata)
                    output.addfile(info, io.BytesIO(metadata))
                else:
                    info.size = inputs[name].stat().st_size
                    with inputs[name].open("rb") as contents:
                        output.addfile(info, contents)
        os.replace(prepared, archive)
        sidecar = Path(temporary) / (archive.name + ".sha256")
        sidecar.write_text(f"{sha256(archive)}  {archive.name}\n", encoding="ascii")
        os.replace(sidecar, dist / sidecar.name)
        checksum_file = Path(temporary) / "SHA256SUMS"
        checksum_file.write_text(checksums(dist), encoding="ascii")
        os.replace(checksum_file, dist / "SHA256SUMS")
    return archive


def assemble(version, source, root=ROOT, run=subprocess.run):
    dist = root / "dist"
    check_dist(dist, version, source)
    for target in TARGETS:
        archive = dist / archive_name(version, target)
        if not archive.is_file():
            raise ValueError(f"Missing release archive: {archive.name}")
        run([sys.executable, str(root / "scripts/verify_release.py"), str(archive), "--version", version,
             "--source", source, "--target", target, "--checksums", str(archive) + ".sha256", "--skip-install"], check=True)
    (dist / "SHA256SUMS").write_text(checksums(dist), encoding="ascii")


def prepare(version, *, target=None, source=None, target_dir=None, stamp_only=False, no_stamp=False,
            assemble_only=False, root=ROOT, run=subprocess.run):
    validate_version(version)
    source, epoch = git_identity(root, source, run)
    verify_source_tree(root, run)
    if assemble_only:
        assemble(version, source, root, run)
        return None
    if no_stamp:
        verify_versions(version, root)
    else:
        stamp(version, root)
    if stamp_only:
        return None
    target = target or native_target(run)
    if target not in TARGETS:
        raise ValueError("Unsupported desktop release target")
    check_dist(root / "dist", version, source)
    if not (os.environ.get("SHEP_GOOGLE_CLIENT_ID") and os.environ.get("SHEP_GOOGLE_CLIENT_SECRET")):
        print("Google sign-in is unavailable without the build-time Google client configuration.", file=sys.stderr)
    binary = build(root, target, target_dir, run)
    git_identity(root, source, run)
    verify_source_tree(root, run)
    archive = package(version, target, source, epoch, binary, root)
    run([sys.executable, str(root / "scripts/verify_release.py"), str(archive),
         "--version", version, "--source", source, "--target", target], check=True)
    return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("version")
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--source", help="Require this exact checked-out Git commit")
    parser.add_argument("--target-dir", type=Path)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--stamp-only", action="store_true")
    mode.add_argument("--no-stamp", action="store_true")
    mode.add_argument("--assemble", action="store_true")
    args = parser.parse_args()
    prepare(args.version, target=args.target, source=args.source, target_dir=args.target_dir,
            stamp_only=args.stamp_only, no_stamp=args.no_stamp, assemble_only=args.assemble)
