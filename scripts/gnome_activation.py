#!/usr/bin/env python3
"""Verify launcher activation with fictional mail on an owned GNOME desktop."""
import argparse
import hashlib
import json
import os
import signal
import subprocess
import tempfile
import time
from pathlib import Path

from e2e import check, click, key, type_text, wait
from install_linux import APP_ID, desktop_entry
from mcp_harness import Desktop, ROOT

DRAFT = "Keep this fictional draft through launcher activation."
OBSERVER = "shep-notification-observer@example.test"


def install_shell_observer(desktop):
    path = desktop.directory / "gnome-notifications.json"
    desktop.env["SHEP_NOTIFICATION_OBSERVATION"] = str(path)
    extension = Path(desktop.env["XDG_DATA_HOME"]) / "gnome-shell/extensions" / OBSERVER
    extension.mkdir(parents=True)
    version = desktop.command("gnome-shell", "--version").split()[-1].split(".")[0]
    (extension / "metadata.json").write_text(json.dumps({
        "uuid": OBSERVER, "name": "Owned desktop observer", "description": "Read-only fixture observations",
        "shell-version": [version],
    }))
    (extension / "extension.js").write_bytes((ROOT / "scripts/fixtures/gnome_notifications.js").read_bytes())
    return lambda: json.loads(path.read_text())


def prepare_window(desktop, width=1440, height=920):
    def maximised():
        state = desktop.command("xprop", "-id", desktop.window, "_NET_WM_STATE")
        return "_NET_WM_STATE_MAXIMIZED_" in state

    if maximised():
        desktop.command("xdotool", "key", "--clearmodifiers", "alt+F10")
        eventually(lambda: not maximised(), "owned window unmaximised")
    desktop.command("xdotool", "windowsize", "--sync", desktop.window, str(width), str(height))
    desktop.batch([check("window_size", [width, height])])
    return {"size": desktop.state()["window_size"], "maximised": maximised()}


def notifier_items(payload):
    value = json.loads(payload)
    if value.get("type") != "as" or not isinstance(value.get("data"), list):
        raise ValueError("Expected the watcher's full string-array registration list")
    items = value["data"]
    if any(not isinstance(item, str) or not item for item in items):
        raise ValueError("Invalid StatusNotifier registration")
    return items


def fixture_processes(binary, state_path, process_root=Path("/proc")):
    """Only identify this executable with this exact owned fixture argument."""
    matches = []
    for process in process_root.iterdir():
        if not process.name.isdigit():
            continue
        try:
            arguments = (process / "cmdline").read_bytes().split(b"\0")
            if arguments[0] != os.fsencode(binary) or b"--demo" not in arguments:
                continue
            index = arguments.index(b"--test-state")
            if arguments[index + 1] == os.fsencode(state_path):
                matches.append(int(process.name))
        except (OSError, ValueError, IndexError):
            continue
    return sorted(matches)


def eventually(probe, description, timeout=10):
    deadline = time.monotonic() + timeout
    error = None
    while time.monotonic() < deadline:
        try:
            result = probe()
            if result:
                return result
        except (subprocess.SubprocessError, FileNotFoundError, json.JSONDecodeError) as caught:
            error = caught
        time.sleep(.05)
    raise AssertionError(f"Timed out: {description}; last error: {error}")


def stop_process(process):
    if process is None or process.poll() is not None:
        return
    try:
        process.terminate()
        process.wait(timeout=3)
    except (OSError, subprocess.TimeoutExpired):
        try:
            process.kill()
        except ProcessLookupError:
            pass
        process.wait(timeout=3)


def cleanup_all(actions):
    errors = []
    for action in actions:
        try:
            action()
        except Exception as error:
            errors.append(error)
    if errors:
        raise ExceptionGroup("Owned GNOME fixture cleanup failed", errors)


def require_stable(probe, duration=.5):
    """Every observation must pass, including the final observation after the dwell."""
    deadline = time.monotonic() + duration
    while True:
        result = probe()
        if time.monotonic() >= deadline:
            return result
        time.sleep(.05)


def run(binary):
    binary = Path(binary).resolve(strict=True)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    desktop = Desktop(binary=binary)
    shell = None
    launches = []
    state_path = None
    receipt = {"binary": str(binary), "sha256": digest, "steps": []}
    runtime = tempfile.TemporaryDirectory(prefix="shep-activation-runtime-")
    desktop.env["XDG_RUNTIME_DIR"] = runtime.name
    try:
        desktop.start(width=1920, height=1080, tray="missing", persistent=True)
        directory = desktop.directory
        state_path = desktop.launch_args[desktop.launch_args.index("--test-state") + 1]
        print(f"GNOME activation evidence: {directory}", flush=True)
        env = desktop.env
        env.update(GSETTINGS_BACKEND="keyfile", XDG_CURRENT_DESKTOP="ubuntu:GNOME",
                   GNOME_SHELL_SESSION_MODE="ubuntu", LIBGL_ALWAYS_SOFTWARE="1")
        applications = Path(env["XDG_DATA_HOME"]) / "applications"
        applications.mkdir(parents=True, exist_ok=True)
        launcher = applications / f"{APP_ID}.desktop"
        launcher.write_text(desktop_entry(desktop.launch_args))
        receipt["desktop_entry"] = launcher.read_text()
        icons = Path(env["XDG_DATA_HOME"]) / "icons/hicolor/scalable/apps"
        icons.mkdir(parents=True, exist_ok=True)
        for source, target in (("shepherd-light.svg", APP_ID),
                               ("shepherd-tray.svg", f"{APP_ID}-tray")):
            (icons / f"{target}.svg").write_bytes((ROOT / "assets" / source).read_bytes())

        def setting(schema, name, value):
            desktop.command("gsettings", "set", schema, name, value)

        shell_observation = install_shell_observer(desktop)
        setting("org.gnome.shell", "enabled-extensions",
                f"['ubuntu-appindicators@ubuntu.com', 'ubuntu-dock@ubuntu.com', '{OBSERVER}']")
        setting("org.gnome.shell", "disabled-extensions",
                "['ding@rastersoft.com', 'tiling-assistant@ubuntu.com']")
        setting("org.gnome.shell", "favorite-apps", f"['{APP_ID}.desktop']")
        setting("org.gnome.shell.extensions.dash-to-dock", "dock-position", "LEFT")
        setting("org.gnome.shell.extensions.dash-to-dock", "dock-fixed", "true")
        setting("org.gnome.shell.extensions.dash-to-dock", "extend-height", "true")
        setting("org.gnome.shell.extensions.dash-to-dock", "dash-max-icon-size", "48")
        setting("org.gnome.desktop.interface", "enable-animations", "false")
        setting("org.gnome.desktop.interface", "scaling-factor", "1")
        setting("org.gnome.desktop.interface", "color-scheme", "prefer-dark")
        with (directory / "gnome-shell.log").open("w") as output:
            shell = subprocess.Popen(["gnome-shell", "--x11", "--sm-disable", "--mode=ubuntu"],
                                     env=env, stdout=output, stderr=output)

        def registrations():
            return notifier_items(desktop.command(
                "busctl", "--address=" + env["DBUS_SESSION_BUS_ADDRESS"], "--json=short",
                "get-property", "org.kde.StatusNotifierWatcher", "/StatusNotifierWatcher",
                "org.kde.StatusNotifierWatcher", "RegisteredStatusNotifierItems"))

        def registration_owners(items):
            owners = {}
            for item in items:
                service = item.split("/", 1)[0]
                value = json.loads(desktop.command(
                    "busctl", "--address=" + env["DBUS_SESSION_BUS_ADDRESS"], "--json=short",
                    "call", "org.freedesktop.DBus", "/org/freedesktop/DBus",
                    "org.freedesktop.DBus", "GetConnectionUnixProcessID", "s", service))
                assert value["type"] == "u" and len(value["data"]) == 1, value
                owners[item] = value["data"][0]
            return owners

        eventually(lambda: shell_observation()["shell_ready"] and
                   desktop.state()["tray"]["available"] and registrations(),
                   "GNOME startup complete and StatusNotifier registration", 20)
        primary = desktop.app.pid

        def visible_window():
            windows = desktop.command("xdotool", "search", "--all", "--onlyvisible", "--pid", str(primary),
                                      "--class", APP_ID).splitlines()
            if len(windows) != 1:
                return False
            window = windows[0]
            if "Iconic" in desktop.command("xprop", "-id", window, "WM_STATE"):
                return False
            if desktop.command("xdotool", "getactivewindow") != window:
                return False
            desktop.window = window
            return window

        def capture(name, draft=False):
            assert desktop.app.poll() is None, "The primary owner exited"
            assert desktop.app.pid == primary
            try:
                eventually(lambda: fixture_processes(binary, state_path) == [primary],
                           "activation helper exits without becoming another owner")
            except AssertionError:
                # Keep the actual process and watcher lists in the failed receipt.
                pass
            observation = {"name": name, "pid": primary, "passed": False}
            receipt["steps"].append(observation)

            def ownership():
                owners = fixture_processes(binary, state_path)
                items = registrations()
                notifier_pids = registration_owners(items)
                observation.update(owners=owners, notifier_items=items, notifier_pids=notifier_pids)
                assert desktop.app.poll() is None, "The primary owner exited"
                assert owners == [primary], f"Additional fixture processes: {owners}"
                assert len(items) == 1, f"Duplicate tray registrations: {items}"
                assert list(notifier_pids.values()) == [primary], notifier_pids

            try:
                ownership()
                window = eventually(visible_window, "one restored and active primary window")
                if draft:
                    desktop.batch([check("composer.visible", True),
                                   check("compose_fields.to", "friend@example.test"),
                                   check("compose_fields.subject", "Launcher activation draft"),
                                   check("editor", DRAFT, "contains")])
                require_stable(ownership)
                assert visible_window() == window, "Restored window lost native activation"
                desktop.command("import", "-window", "root", "-quality", "95",
                                str(directory / f"{name}.webp"))
                ownership()
                observation.update(window=window, draft_retained=draft, passed=True)
            except Exception:
                desktop.command("import", "-window", "root", "-quality", "95",
                                str(directory / f"{name}-failed.webp"))
                raise

        def launch(name):
            with (directory / f"{name}.log").open("w") as output:
                child = subprocess.Popen(["gio", "launch", str(launcher)], cwd=ROOT, env=env,
                                         stdout=output, stderr=output)
            launches.append(child)
            assert child.wait(timeout=10) == 0, "Desktop entry launch failed"

        desktop.command("xdotool", "key", "Escape", "Escape")
        eventually(lambda: not shell_observation()["overview_visible"], "GNOME overview dismissed")
        receipt["shell_startup"] = shell_observation()
        desktop.command("xdotool", "windowactivate", "--sync", desktop.window)
        receipt["setup_window"] = prepare_window(desktop)
        desktop.batch([wait(150), key("ctrl+comma"), check("tab", "Preferences"),
                       click(1150, 88), type_text("system tray"),
                       check("settings_matches", ["System tray"]), click(450, 289),
                       check("settings_group", "System tray")])
        if not desktop.state()["tray"]["enabled"]:
            desktop.batch([click(288, 342), check("tray.saved_enabled", True)])
        desktop.batch([key("ctrl+1"), check("tab", "Mail"), key("c"),
                       check("composer.visible", True), wait(80),
                       click(850, 230), type_text("friend@example.test"),
                       check("compose_fields.to", "friend@example.test"),
                       click(850, 279), type_text("Launcher activation draft"),
                       check("compose_fields.subject", "Launcher activation draft"),
                       click(850, 400), type_text(DRAFT), check("editor", DRAFT, "contains")])
        capture("before-launch", draft=True)
        for index in range(2):
            launch(f"launcher-{index}")
            # Allow a wrongly spawned secondary enough time to publish its tray.
            time.sleep(1)
            capture(f"launcher-{index}-same-owner", draft=True)
        desktop.batch([{"type": "close_request"}, check("tray.visible", False)])
        assert len(registrations()) == 1
        desktop.command("import", "-window", "root", str(directory / "hidden.webp"))
        desktop.command("xdotool", "key", "--clearmodifiers", "super+1")
        capture("favourite-restores-hidden", draft=True)
        desktop.batch([{"type": "close_request"}, check("tray.visible", False)])
        def native_window_gone():
            try:
                desktop.command("xdotool", "search", "--all", "--pid", str(primary), "--class", APP_ID)
            except subprocess.CalledProcessError as error:
                if error.returncode == 1:
                    return True
                raise
            return False
        eventually(native_window_gone, "previous native window destroyed before dock activation")
        receipt["steps"].append({"name": "before-mouse-dock", "pid": primary,
                                 "native_windows": [], "tray": desktop.state()["tray"]})
        # The owned shell pins only Shep, first on its fixed left dock at 1x.
        desktop.command("xdotool", "mousemove", "32", "66")
        desktop.command("import", "-window", "root", str(directory / "before-mouse-dock.webp"))
        desktop.command("xdotool", "click", "1")
        capture("mouse-dock-restores-hidden", draft=True)
        desktop.command("xdotool", "windowminimize", desktop.window)
        eventually(lambda: "Iconic" in desktop.command("xprop", "-id", desktop.window, "WM_STATE"),
                   "native minimised window")
        launch("launcher-minimised")
        capture("launcher-restores-minimised", draft=True)
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, "Binary changed during run"
        receipt.update(passed=True, shell=desktop.command("gnome-shell", "--version"))
        return directory
    except Exception as error:
        receipt.update(passed=False, error=str(error))
        raise
    finally:
        def save_receipt():
            if desktop.directory:
                (desktop.directory / "activation-evidence.json").write_text(json.dumps(receipt, indent=2))

        def stop_escaped():
            if state_path is None:
                return
            def kill_owned(pid):
                if desktop.app is not None and pid == desktop.app.pid:
                    return
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            cleanup_all([lambda pid=pid: kill_owned(pid)
                         for pid in fixture_processes(binary, state_path)])

        tray = desktop.tray_fixture
        processes = [*launches, desktop.app, shell, desktop.clipboard, desktop.badge_monitor,
                     desktop.badge_bus, desktop.xvfb]
        if tray:
            processes.extend([tray.host, tray.bus])
        actions = [save_receipt, stop_escaped, desktop.stop]
        actions.extend(lambda process=process: stop_process(process) for process in processes)
        if tray:
            actions.extend([tray.alias.cleanup, tray.log.close])
        actions.append(runtime.cleanup)
        cleanup_all(actions)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/test-ui/shep")
    run(parser.parse_args().binary)
