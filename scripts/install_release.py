#!/usr/bin/env python3
"""Download a published Shep release, verify it, and install it for this user.

The raw OS entry points fetch this standard-library-only helper. No Rust or
checkout is required. Release files are staged before any installed file changes.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
import urllib.error
import urllib.parse
import urllib.request

REPOSITORY = "sam-ruff/shep.so"
API = f"https://api.github.com/repos/{REPOSITORY}/releases"
DOWNLOADS = f"https://github.com/{REPOSITORY}/releases/download/"
VERSION = re.compile(r"v?(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)\Z")


class InstallError(RuntimeError):
    pass


class SecureRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        if urllib.parse.urlsplit(new_url).scheme != "https":
            raise InstallError("The download redirected to an insecure address")
        return super().redirect_request(request, fp, code, message, headers, new_url)


def open_url(url):
    request = urllib.request.Request(url, headers={"User-Agent": "Shep-release-installer",
        "Accept": "application/vnd.github+json", "X-GitHub-Api-Version": "2026-03-10"})
    return urllib.request.build_opener(SecureRedirect()).open(request, timeout=30)


def release_asset(version, system, machine, fetch=open_url):
    suffix = "/latest"
    if version:
        match = VERSION.fullmatch(version)
        if not match:
            raise InstallError("Use a version such as 1.2.3 or v1.2.3")
        suffix = "/tags/v" + match[1]
    try:
        with fetch(API + suffix) as response:
            payload = response.read(2 * 1024 * 1024 + 1)
        if len(payload) > 2 * 1024 * 1024:
            raise InstallError("GitHub returned an unexpectedly large release description")
        release = json.loads(payload)
    except urllib.error.HTTPError as error:
        if error.code == 404:
            raise InstallError("No published Shep release was found for this request. Check https://github.com/sam-ruff/shep.so/releases or use the source-install instructions at https://sam-ruff.github.io/shep.so/installation/.") from error
        raise InstallError(f"GitHub could not provide the release (HTTP {error.code}); try again later") from error
    if not isinstance(release, dict):
        raise InstallError("GitHub returned an invalid release description")
    match = VERSION.fullmatch(str(release.get("tag_name", "")))
    if not match or release.get("draft"):
        raise InstallError("GitHub did not return a usable published release")
    architecture = {"amd64": "x86_64", "x86_64": "x86_64", "arm64": "aarch64", "aarch64": "aarch64"}.get(machine.lower())
    assets = release.get("assets", [])
    if not isinstance(assets, list):
        raise InstallError("GitHub returned invalid release assets")
    name = f"shep-{match[1]}-{system}-{architecture}.tar.gz"
    selected = {}
    for asset in assets:
        if isinstance(asset, dict) and asset.get("name") in (name, "SHA256SUMS"):
            asset_name = asset["name"]
            url = asset.get("browser_download_url", "")
            if asset_name in selected or not isinstance(url, str) or not url.startswith(DOWNLOADS):
                raise InstallError("The release has duplicate or unexpected download URLs")
            selected[asset_name] = url
    if not architecture or name not in selected:
        raise InstallError(f"Release {release['tag_name']} has no {system}/{machine} installer archive. Nothing was installed; use a supported release asset or build from source.")
    if "SHA256SUMS" not in selected:
        raise InstallError("This release has no SHA256SUMS file; nothing was installed")
    return match[1], name, selected


def download(url, path, fetch=open_url):
    digest = hashlib.sha256()
    with fetch(url) as response, path.open("wb") as output:
        while block := response.read(1024 * 1024):
            digest.update(block)
            output.write(block)
    return digest.hexdigest()


def expected_digest(path, name):
    found = None
    for line in path.read_text().splitlines():
        match = re.fullmatch(r"([0-9a-fA-F]{64})\s+\*?(.+)", line)
        if match and match[2] == name:
            if found:
                raise InstallError("The archive has duplicate checksum entries")
            found = match[1].lower()
    if not found:
        raise InstallError("The release checksum does not include the selected archive")
    return found


def extract_archive(archive, destination):
    # Only regular files/directories from the native release are needed. Reject
    # links and special files before extracting anything, including link pivots.
    with tarfile.open(archive, "r:gz") as package:
        members = package.getmembers()
        seen = set()
        for member in members:
            path = PurePosixPath(member.name)
            if (path.is_absolute() or ".." in path.parts or "\\" in member.name or ":" in member.name
                    or not path.parts or str(path) in seen or not (member.isfile() or member.isdir())):
                raise InstallError("The release archive contains unsafe or duplicate paths")
            seen.add(str(path))
        for member in members:
            path = destination.joinpath(*PurePosixPath(member.name).parts)
            if member.isdir():
                path.mkdir(parents=True, exist_ok=True)
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                with package.extractfile(member) as source, path.open("xb") as output:
                    shutil.copyfileobj(source, output, 1024 * 1024)


def installation_scope(args, choose=None):
    if args.system:
        return True
    if args.user or args.yes:
        return False
    if choose is None:
        if not sys.stdout.isatty():
            return False
        try:
            with open("/dev/tty", "r+") as terminal:
                terminal.write("Install for [u]ser (default), [a]ll users, or [c]ancel? [u/a/c]: ")
                terminal.flush()
                answer = terminal.readline().strip().lower()
        except OSError:
            return False
    else:
        answer = choose().strip().lower()
    if answer in ("c", "cancel", "n", "no"):
        raise InstallError("Installation cancelled; nothing was changed")
    if answer not in ("", "u", "user", "a", "all"):
        raise InstallError("Choose user, all users or cancel; nothing was changed")
    return answer in ("a", "all")


def install_linux(root, args, system, run=subprocess.run):
    for required in ("shep", "scripts/install_linux.py", "assets/launcher.png"):
        if not (root / required).is_file():
            raise InstallError(f"This archive is missing {required}; nothing was installed")
    if system and (args.prefix or args.data_dir or args.pin):
        raise InstallError("All-user installation cannot combine --prefix, --data-dir or --pin")
    command = [sys.executable, str(root / "scripts/install_linux.py"), "--binary", str(root / "shep")]
    if system:
        command += ["--prefix", "/usr/local", "--data-dir", "/usr/local/share"]
        if os.geteuid() != 0:
            sudo = shutil.which("sudo")
            if not sudo:
                raise InstallError("All-user installation needs administrator access. Install sudo or use --user.")
            command = [sudo, "--", *command]
    else:
        if args.prefix:
            command += ["--prefix", str(args.prefix)]
        if args.data_dir:
            command += ["--data-dir", str(args.data_dir)]
        if args.pin:
            command.append("--pin")
    run(command, check=True)


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument("--platform", choices=["linux"], default="linux", help=argparse.SUPPRESS)
    result.add_argument("--version", help="Published version (default latest stable release)")
    scope = result.add_mutually_exclusive_group()
    scope.add_argument("--user", action="store_true", help="Install for this user without a scope prompt")
    scope.add_argument("--system", action="store_true", help="Install for all users; sudo may ask for permission")
    result.add_argument("--yes", action="store_true", help="Use the per-user default without prompting")
    result.add_argument("--prefix", type=Path, help="Custom per-user binary prefix")
    result.add_argument("--data-dir", type=Path, help="Custom per-user application-menu directory")
    result.add_argument("--pin", action="store_true", help="Also pin Shep to the current user's GNOME dash")
    return result


def install(args, fetch=open_url, run=subprocess.run, choose=None, machine=None):
    if platform.system().lower() != args.platform:
        raise InstallError("Use the installer for this operating system")
    system = installation_scope(args, choose)
    version, name, assets = release_asset(args.version, args.platform, machine or platform.machine(), fetch)
    print(f"Downloading Shep {version} ({args.platform})…", flush=True)
    with tempfile.TemporaryDirectory(prefix="shep-release-") as directory:
        stage = Path(directory)
        archive, checksums = stage / name, stage / "SHA256SUMS"
        actual = download(assets[name], archive, fetch)
        download(assets["SHA256SUMS"], checksums, fetch)
        if actual != expected_digest(checksums, name):
            raise InstallError("Release checksum mismatch; nothing was installed. Try downloading again.")
        unpacked = stage / "unpacked"
        unpacked.mkdir()
        extract_archive(archive, unpacked)
        install_linux(unpacked, args, system, run)
    print("Installation complete. Reopen Shep to use this version.")


def main():
    try:
        install(parser().parse_args())
    except KeyboardInterrupt:
        sys.exit("Installation cancelled")
    except (InstallError, OSError, ValueError, tarfile.TarError, subprocess.SubprocessError) as error:
        sys.exit(f"Install failed: {error}")


if __name__ == "__main__":
    main()
