#!/usr/bin/env python3
"""Capture real GNOME Shell surfaces on an owned display with fictional mail."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import time

from mcp_harness import Desktop, ROOT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--label", default="after")
    parser.add_argument("--before", action="store_true")
    parser.add_argument("--baseline", default="16a52fd34544d05410767ec20e7e878e44f0c91d")
    parser.add_argument("--scale", type=int, choices=(1, 2), default=1)
    args = parser.parse_args()
    desktop = Desktop()
    shell = None
    neighbour = None
    try:
        desktop.start(width=1920, height=1080, tray="missing")
        directory = desktop.directory
        print(directory, flush=True)
        env = desktop.env.copy()
        runtime = directory / "runtime"
        runtime.mkdir(mode=0o700)
        env.update(XDG_RUNTIME_DIR=str(runtime), GSETTINGS_BACKEND="keyfile",
                   XDG_CURRENT_DESKTOP="ubuntu:GNOME", GNOME_SHELL_SESSION_MODE="ubuntu",
                   LIBGL_ALWAYS_SOFTWARE="1")
        data = Path(env["XDG_DATA_HOME"])
        applications = data / "applications"
        icons = data / "icons/hicolor/scalable/apps"
        applications.mkdir(parents=True, exist_ok=True)
        icons.mkdir(parents=True, exist_ok=True)
        for source, name in (("shepherd-light.svg", "so.shep.Shep.svg"),
                             ("shepherd-symbolic.svg", "so.shep.Shep-symbolic.svg"),
                             ("shepherd-tray.svg", "so.shep.Shep-tray.svg")):
            if args.before and source == "shepherd-symbolic.svg":
                content = subprocess.check_output(["git", "show", f"{args.baseline}:assets/shepherd-symbolic.svg"], cwd=ROOT)
                (icons / name).write_bytes(content)
            else:
                shutil.copyfile(ROOT / "assets" / source, icons / name)
        icon = "so.shep.Shep-symbolic" if args.before else "so.shep.Shep"
        (applications / "so.shep.Shep.desktop").write_text(
            "[Desktop Entry]\nType=Application\nName=Shep\nExec=true\n"
            f"Icon={icon}\nStartupWMClass=so.shep.Shep\nCategories=Network;Email;\n")

        def settings(schema, key, value):
            subprocess.run(["gsettings", "set", schema, key, value], env=env, check=True)

        settings("org.gnome.shell", "enabled-extensions",
                 "['ubuntu-appindicators@ubuntu.com', 'ubuntu-dock@ubuntu.com']")
        settings("org.gnome.shell", "disabled-extensions",
                 "['ding@rastersoft.com', 'tiling-assistant@ubuntu.com']")
        settings("org.gnome.shell", "favorite-apps", "['so.shep.Shep.desktop']")
        settings("org.gnome.shell", "app-picker-layout",
                 "[{'so.shep.Shep.desktop': <{'position': <0>}>}]")
        settings("org.gnome.desktop.interface", "scaling-factor", str(args.scale))
        settings("org.gnome.desktop.interface", "enable-animations", "false")
        settings("org.gnome.desktop.interface", "color-scheme", "prefer-dark")
        with (directory / "gnome-shell.log").open("w") as log:
            shell = subprocess.Popen(["gnome-shell", "--x11", "--sm-disable", "--mode=ubuntu"],
                                     env=env, stdout=log, stderr=log)
        time.sleep(8)
        if shell.poll() is not None:
            raise RuntimeError("Owned GNOME Shell exited; inspect gnome-shell.log")
        if not desktop.state().get("tray", {}).get("available"):
            raise RuntimeError("GNOME's StatusNotifier host did not register Shep")

        def key(value):
            subprocess.run(["xdotool", "key", "--clearmodifiers", value], env=env, check=True)
            time.sleep(.5)

        def capture(surface):
            subprocess.run(["import", "-window", "root", "-quality", "95",
                            str(directory / f"{args.label}-{args.scale}x-{surface}.webp")], env=env, check=True)

        key("Escape")
        key("Escape")
        capture("dark-panel-dash")
        settings("org.gnome.shell", "favorite-apps", "[]")
        key("super+a")
        time.sleep(2)
        capture("dark-grid")
        key("Escape")
        key("Escape")
        settings("org.gnome.shell", "favorite-apps", "['so.shep.Shep.desktop']")
        neighbour = subprocess.Popen(["zenity", "--info", "--title=Icon review", "--text=Owned GNOME icon review"],
                                     env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(1)
        subprocess.run(["xdotool", "keydown", "alt", "key", "Tab"], env=env, check=True)
        time.sleep(.5)
        capture("dark-switcher")
        subprocess.run(["xdotool", "keyup", "alt"], env=env, check=True)
        settings("org.gnome.desktop.interface", "color-scheme", "prefer-light")
        time.sleep(1)
        capture("light-panel-dash")
        settings("org.gnome.shell", "favorite-apps", "[]")
        key("super+a")
        time.sleep(2)
        capture("light-grid")
        key("Escape")
        key("Escape")
        (directory / "gnome-evidence.json").write_text(json.dumps({
            "shell": subprocess.check_output(["gnome-shell", "--version"], text=True).strip(),
            "scale": args.scale, "label": args.label, "tray": desktop.state().get("tray"),
            "display": env["DISPLAY"], "before": args.before}, indent=2))
    finally:
        if neighbour is not None and neighbour.poll() is None:
            neighbour.terminate()
            neighbour.wait(timeout=5)
        if shell is not None and shell.poll() is None:
            shell.terminate()
            try:
                shell.wait(timeout=10)
            except subprocess.TimeoutExpired:
                shell.kill()
                shell.wait()
        desktop.stop()


if __name__ == "__main__":
    main()
