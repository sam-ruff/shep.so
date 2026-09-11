#!/usr/bin/env python3
"""Install Shep and its freedesktop launcher without root privileges."""
import argparse
import ast
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
APP_ID = "so.shep.Shep"


def exec_value(path):
    value = str(path)
    if any(c in value for c in "\r\n\x00"):
        raise ValueError("Installation path must not contain control characters")
    # Desktop Entry Exec quoting, then the general string escape layer.
    value = value.replace("%", "%%")
    value = "".join("\\" + c if c in '\\"`$' else c for c in value)
    return '"' + value.replace("\\", "\\\\") + '"'


def atomic_install(source, destination, mode):
    destination.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(prefix=".shep-install-", dir=destination.parent)
    try:
        with os.fdopen(fd, "wb") as output:
            if isinstance(source, bytes):
                output.write(source)
            else:
                with source.open("rb") as original:
                    shutil.copyfileobj(original, output)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(temporary, mode)
        os.replace(temporary, destination)
    finally:
        Path(temporary).unlink(missing_ok=True)


def pin_gnome(remove=False):
    if not shutil.which("gsettings"):
        raise RuntimeError("GNOME gsettings is unavailable. Pin Shep from your panel's application menu.")
    result = subprocess.run(["gsettings", "get", "org.gnome.shell", "favorite-apps"],
                            text=True, capture_output=True, check=True)
    value = result.stdout.strip().removeprefix("@as ")
    favorites = ast.literal_eval(value)
    if not isinstance(favorites, list) or not all(isinstance(v, str) for v in favorites):
        raise RuntimeError("Could not read GNOME favorites")
    desktop_id = APP_ID + ".desktop"
    if remove:
        favorites = [item for item in favorites if item != desktop_id]
    elif desktop_id not in favorites:
        favorites.append(desktop_id)
    subprocess.run(["gsettings", "set", "org.gnome.shell", "favorite-apps", repr(favorites)], check=True)


def install(binary, prefix, data, uninstall=False, pin=False):
    prefix, data = prefix.resolve(), data.resolve()
    destination = prefix / "bin" / "shep"
    desktop = data / "applications" / f"{APP_ID}.desktop"
    icon = data / "icons" / "hicolor" / "128x128" / "apps" / f"{APP_ID}.png"
    symbolic = data / "icons" / "hicolor" / "scalable" / "apps" / f"{APP_ID}-symbolic.svg"
    symbolic_source = ROOT / "assets" / "shepherd-symbolic.svg"
    scalable = data / "icons" / "hicolor" / "scalable" / "apps" / f"{APP_ID}.svg"
    tray = scalable.with_name(f"{APP_ID}-tray.svg")
    if uninstall:
        if pin:
            pin_gnome(remove=True)
        for path in (destination, desktop, icon, symbolic, scalable, tray):
            path.unlink(missing_ok=True)
        print("Removed Shep's binary, launcher and icon. Your accounts, mail and backups are preserved.")
    else:
        if not binary or not binary.is_file():
            raise ValueError("Supply --binary PATH or run scripts/install-linux.sh to build the release version")
        icon_name = APP_ID
        launcher = ("[Desktop Entry]\nVersion=1.0\nType=Application\nName=Shep\n"
                    "GenericName=Email and Calendar\nComment=A calm home for your mail and calendar\n"
                    f"Exec={exec_value(destination)}\nIcon={icon_name}\nTerminal=false\n"
                    "Categories=Network;Email;Office;Calendar;\nKeywords=mail;email;calendar;imap;pop3;\n"
                    f"StartupWMClass={APP_ID}\nStartupNotify=true\n")
        atomic_install(binary, destination, 0o755)
        atomic_install(ROOT / "assets" / "launcher.png", icon, 0o644)
        for source, target in (("shepherd-light.svg", scalable), ("shepherd-tray.svg", tray)):
            if (ROOT / "assets" / source).is_file():
                atomic_install(ROOT / "assets" / source, target, 0o644)
        if symbolic_source.is_file():
            atomic_install(symbolic_source, symbolic, 0o644)
        atomic_install(launcher.encode(), desktop, 0o644)
        print(f"Installed release binary: {destination}\nApplication launcher: {desktop}")
        if pin:
            pin_gnome()
        print("Open Shep from your applications menu, then choose Add to Favorites / Pin to panel.")
    if shutil.which("update-desktop-database"):
        subprocess.run(["update-desktop-database", str(desktop.parent)], check=False,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return destination, desktop, icon


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, help="Install an already built production release binary")
    parser.add_argument("--prefix", type=Path, default=Path.home() / ".local", help="Binary prefix (default ~/.local)")
    parser.add_argument("--data-dir", type=Path, default=Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")),
                        help="Desktop launcher/icon data directory (default XDG_DATA_HOME)")
    parser.add_argument("--pin", action="store_true", help="Also add to GNOME dash favorites; preserve existing favorites")
    parser.add_argument("--uninstall", action="store_true", help="Remove installed files, preserving user data; --pin also unpins")
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("This installer supports Linux only")
    try:
        install(args.binary, args.prefix, args.data_dir, args.uninstall, args.pin)
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        sys.exit(f"Install failed: {error}")


if __name__ == "__main__":
    main()
