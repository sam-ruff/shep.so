"""Check release integrity, native dependencies and the bundled Linux installer."""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

from release import CONTENTS, TARGETS, archive_name, sha256, validate_version

WINDOWS_SYSTEM_DLLS = frozenset({
    "advapi32.dll", "bcrypt.dll", "bcryptprimitives.dll", "comctl32.dll", "comdlg32.dll", "crypt32.dll",
    "d2d1.dll", "d3d9.dll", "d3d11.dll", "d3d12.dll", "dbghelp.dll", "dnsapi.dll", "dwmapi.dll", "dwrite.dll",
    "dxgi.dll", "gdi32.dll", "gdiplus.dll", "imm32.dll", "iphlpapi.dll", "kernel32.dll", "kernelbase.dll",
    "msimg32.dll", "ncrypt.dll", "normaliz.dll", "ntdll.dll", "ole32.dll", "oleaut32.dll", "powrprof.dll",
    "propsys.dll", "psapi.dll", "rpcrt4.dll", "secur32.dll", "setupapi.dll", "shell32.dll", "shlwapi.dll",
    "user32.dll", "userenv.dll", "uxtheme.dll", "version.dll", "winhttp.dll", "wininet.dll", "winmm.dll",
    "winspool.drv", "wintrust.dll", "ws2_32.dll", "wtsapi32.dll",
})


def find_dumpbin(run=subprocess.run):
    found = shutil.which("dumpbin.exe")
    if found:
        return Path(found)
    found = shutil.which("vswhere.exe")
    locator = Path(found) if found else Path(os.environ.get("ProgramFiles(x86)", "C:/Program Files (x86)")) / "Microsoft Visual Studio/Installer/vswhere.exe"
    if not locator.is_file():
        raise ValueError("Windows dependency verification needs Visual Studio dumpbin or vswhere")
    result = run([str(locator), "-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                  "-property", "installationPath"], check=True, capture_output=True, text=True)
    installations = result.stdout.strip().splitlines()
    if len(installations) != 1:
        raise ValueError("vswhere did not identify one Visual Studio C++ installation")
    installation = Path(installations[0])
    version = (installation / "VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt").read_text().strip()
    if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)+", version):
        raise ValueError("Visual Studio reported an invalid default C++ toolset")
    dumpbin = installation / "VC/Tools/MSVC" / version / "bin/Hostx64/x64/dumpbin.exe"
    if not dumpbin.is_file():
        raise ValueError("Visual Studio default x64 dumpbin is missing")
    return dumpbin


def windows_dependencies(output):
    dependencies = set()
    section = False
    summary = False
    for line in output.splitlines():
        line = line.strip()
        if re.fullmatch(r"Image has the following(?: delay load)? dependencies:", line):
            section = True
            continue
        if section and line == "Summary":
            summary = True
            break
        if not section or not line:
            continue
        if not re.fullmatch(r"[A-Za-z0-9_.-]+\.(?:dll|drv)", line, re.IGNORECASE):
            raise ValueError("Unrecognised dumpbin dependency output")
        dependencies.add(line.lower())
    if not dependencies or not summary:
        raise ValueError("dumpbin did not report complete executable dependencies")
    # Windows API sets name OS contracts; CRT contracts still require redistribution.
    unsupported = sorted(name for name in dependencies if name not in WINDOWS_SYSTEM_DLLS
                         and not re.fullmatch(r"api-ms-win-core-[a-z0-9-]+-l\d+-\d+-\d+\.dll", name))
    if unsupported:
        raise ValueError("Windows release requires unbundled or unapproved DLLs: " + ", ".join(unsupported))
    return sorted(dependencies)


def verify_windows(binary, run=subprocess.run):
    dumpbin = find_dumpbin(run)
    result = run([str(dumpbin), "/NOLOGO", "/DEPENDENTS", str(binary)], check=True,
                 capture_output=True, text=True, env={**os.environ, "VSLANG": "1033"})
    dependencies = windows_dependencies(result.stdout)
    print(f"Windows executable SHA256 {sha256(binary)} imports: {', '.join(dependencies)}")
    print("Windows import verification passed; installation and GUI startup were not exercised.")


def checksum(archive, path):
    entries = {}
    for line in path.read_text(encoding="ascii").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([A-Za-z0-9._-]+\.tar\.gz)", line)
        if not match or match[2] in entries:
            raise ValueError("Malformed or duplicate release checksum")
        entries[match[2]] = match[1]
    if sha256(archive) != entries.get(archive.name):
        raise ValueError("Release archive checksum mismatch")


def executable_target(source, target):
    header = source.read(64)
    if target == "x86_64-unknown-linux-gnu":
        if len(header) != 64 or header[:6] != b"\x7fELF\x02\x01" or header[18:20] != b"\x3e\x00":
            raise ValueError("Release executable is not x86_64 Linux ELF")
        return
    if len(header) != 64 or header[:2] != b"MZ":
        raise ValueError("Release executable is not Windows PE")
    source.seek(int.from_bytes(header[60:64], "little"))
    if source.read(6) != b"PE\0\0\x64\x86":
        raise ValueError("Release executable is not x86_64 Windows PE")


def inspect(archive, version=None, source=None, target=None):
    with tarfile.open(archive, "r:gz") as package:
        members = package.getmembers()
        names = [member.name for member in members]
        if len(names) != len(set(names)):
            raise ValueError("Release archive contains duplicate members")
        if any(not member.isreg() for member in members):
            raise ValueError("Release archive must contain only regular files")
        info = package.getmember("release.json")
        if not 0 < info.size <= 4096:
            raise ValueError("Release provenance is missing or oversized")
        metadata = json.load(package.extractfile(info))
        fields = {"schema", "version", "source", "target", "platform", "architecture", "binary", "binary_sha256"}
        if not isinstance(metadata, dict) or set(metadata) != fields or metadata["schema"] != 1:
            raise ValueError("Unsupported release provenance")
        validate_version(metadata["version"])
        if not re.fullmatch(r"[0-9a-f]{40}", metadata["source"]):
            raise ValueError("Release source must be a full Git commit")
        if metadata["target"] not in TARGETS:
            raise ValueError("Unsupported release target")
        system, architecture, binary = TARGETS[metadata["target"]]
        if (metadata["platform"], metadata["architecture"], metadata["binary"]) != (system, architecture, binary):
            raise ValueError("Conflicting release target metadata")
        for key, expected in (("version", version), ("source", source), ("target", target)):
            if expected is not None and metadata[key] != expected:
                raise ValueError(f"Release {key} does not match the expected build")
        if archive.name != archive_name(metadata["version"], metadata["target"]):
            raise ValueError("Release filename does not match its provenance")
        if set(names) != {*CONTENTS, binary, "release.json"}:
            raise ValueError("Release archive is missing required files or contains unexpected members")
        for member in members:
            mode = 0o755 if member.name in (binary, "scripts/install-linux.sh") else 0o644
            if member.mode != mode:
                raise ValueError("Release archive has unexpected file permissions")
        with package.extractfile(binary) as contents:
            executable_target(contents, metadata["target"])
        with package.extractfile(binary) as contents:
            if hashlib.file_digest(contents, "sha256").hexdigest() != metadata["binary_sha256"]:
                raise ValueError("Release executable checksum does not match its provenance")
    return metadata


def verify(archive, *, version=None, source=None, target=None, checksums=None, skip_install=False, run=subprocess.run):
    archive = Path(archive).resolve()
    checksum(archive, Path(checksums) if checksums else archive.parent / "SHA256SUMS")
    metadata = inspect(archive, version, source, target)
    if skip_install:
        print("Release checksum, target, contents and source identity passed; native checks were not exercised.")
        return metadata
    if metadata["platform"] == "windows":
        if sys.platform != "win32":
            raise ValueError("Windows dependency verification requires a Windows host")
        with tempfile.TemporaryDirectory(prefix="shep-release-check-") as directory:
            with tarfile.open(archive, "r:gz") as package:
                package.extract(metadata["binary"], directory, filter="data")
            verify_windows(Path(directory) / metadata["binary"], run)
        return metadata
    if sys.platform != "linux":
        raise ValueError("Linux installer verification requires a Linux host")
    with tempfile.TemporaryDirectory(prefix="shep-release-check-") as directory:
        with tarfile.open(archive, "r:gz") as package:
            package.extractall(directory, filter="data")
        root = Path(directory)
        run(["bash", str(root / "scripts/install-linux.sh"), "--prefix", str(root / "prefix"),
             "--data-dir", str(root / "data")], capture_output=True, text=True, check=True)
        installed = root / "prefix/bin/shep"
        launcher = root / "data/applications/so.shep.Shep.desktop"
        icon = root / "data/icons/hicolor/128x128/apps/so.shep.Shep.png"
        if not installed.is_file() or sha256(installed) != metadata["binary_sha256"]:
            raise ValueError("Bundled installer did not install the exact release executable")
        if not launcher.is_file() or "StartupWMClass=so.shep.Shep" not in launcher.read_text():
            raise ValueError("Bundled installer did not register the launcher")
        if not icon.is_file() or sha256(icon) != sha256(root / "assets/launcher.png"):
            raise ValueError("Bundled installer did not install the launcher icon")
    print("Release integrity and isolated bundled Linux installation passed.")
    return metadata


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("archive", type=Path)
    parser.add_argument("--version")
    parser.add_argument("--source")
    parser.add_argument("--target", choices=TARGETS)
    parser.add_argument("--checksums", type=Path)
    parser.add_argument("--skip-install", action="store_true", help="Skip native dependency and installer checks during cross-platform assembly")
    args = parser.parse_args()
    verify(args.archive, version=args.version, source=args.source, target=args.target,
           checksums=args.checksums, skip_install=args.skip_install)
