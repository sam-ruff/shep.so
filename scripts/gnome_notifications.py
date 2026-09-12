#!/usr/bin/env python3
"""Exercise new-mail policy and native delivery on an owned GNOME desktop."""
import argparse
import hashlib
import importlib.util
import json
import subprocess
import tempfile
import time
from pathlib import Path

from e2e import check, click, key, type_text, wait
from gnome_activation import OBSERVER, cleanup_all, eventually, install_shell_observer, prepare_window, require_stable, stop_process
from install_linux import APP_ID, desktop_entry
from mcp_harness import Desktop, ROOT

def validate_notification(items, mode):
    if mode == "muted":
        assert not items, f"Muted arrival produced a notification: {items}"
        return
    assert len(items) == 1, f"Expected one retained notification: {items}"
    item = items[0]
    assert item["app"] == f"{APP_ID}.desktop", item
    if mode == "private":
        assert item["title"] == "New email", item
        assert item["body"] == "You have a new message in your Inbox.", item
    else:
        assert "Morgan" in item["title"], item
        assert item["body"] == "New mail from the background", item


def run(binary, mode="details", desktop_type=Desktop):
    if mode not in ("details", "private", "muted"):
        raise ValueError("Choose details, private or muted")
    binary = Path(binary).resolve(strict=True)
    digest = hashlib.sha256(binary.read_bytes()).hexdigest()
    desktop = desktop_type(binary=binary)
    runtime = tempfile.TemporaryDirectory(prefix="shep-notification-runtime-")
    desktop.env["XDG_RUNTIME_DIR"] = runtime.name
    desktop.env["PULSE_SERVER"] = f"unix:{runtime.name}/no-audio-server"
    processes = []
    receipt = {"binary": str(binary), "sha256": digest, "mode": mode, "passed": False}
    try:
        desktop.start(width=1920, height=1080, tray="missing", persistent=True,
                      notification_delivery="native")
        directory = desktop.directory
        print(f"GNOME notification evidence: {directory}", flush=True)
        env = desktop.env
        env.update(GSETTINGS_BACKEND="keyfile", XDG_CURRENT_DESKTOP="ubuntu:GNOME",
                   GNOME_SHELL_SESSION_MODE="ubuntu", LIBGL_ALWAYS_SOFTWARE="1",
                   SHEP_NOTIFICATION_OBSERVATION=str(directory / "gnome-notifications.json"))
        data = Path(env["XDG_DATA_HOME"])
        applications = data / "applications"
        applications.mkdir(parents=True, exist_ok=True)
        (applications / f"{APP_ID}.desktop").write_text(desktop_entry(desktop.launch_args))
        icons = data / "icons/hicolor/scalable/apps"
        icons.mkdir(parents=True, exist_ok=True)
        (icons / f"{APP_ID}.svg").write_bytes((ROOT / "assets/shepherd-light.svg").read_bytes())
        observation = install_shell_observer(desktop)
        desktop.command("gsettings", "set", "org.gnome.shell", "enabled-extensions", f"['{OBSERVER}']")
        desktop.command("gsettings", "set", "org.gnome.shell", "disabled-extensions",
                        "['ding@rastersoft.com', 'tiling-assistant@ubuntu.com']")
        desktop.command("gsettings", "set", "org.gnome.desktop.interface", "enable-animations", "false")
        desktop.command("gsettings", "set", "org.gnome.desktop.interface", "scaling-factor", "1")
        desktop.command("gsettings", "set", "org.gnome.desktop.notifications", "show-banners", "true")
        for name, command in [
            ("gnome-shell", ["gnome-shell", "--x11", "--sm-disable", "--mode=ubuntu"]),
            ("gnome-notification-service", ["/usr/bin/gjs", "-m", "/usr/share/gnome-shell/org.gnome.Shell.Notifications"]),
        ]:
            with (directory / f"{name}.log").open("w") as output:
                processes.append(subprocess.Popen(command, env=env, stdout=output, stderr=output))
            if name == "gnome-shell":
                eventually(lambda: observation()["shell_ready"], "GNOME startup complete", 20)

        eventually(lambda: desktop.command("gdbus", "call", "--session", "--dest", "org.freedesktop.Notifications",
                                           "--object-path", "/org/freedesktop/Notifications", "--method",
                                           "org.freedesktop.Notifications.GetServerInformation"),
                   "actual GNOME notification service", 20)

        def bus_pid(name):
            value = json.loads(desktop.command(
                "busctl", "--address=" + env["DBUS_SESSION_BUS_ADDRESS"], "--json=short", "call",
                "org.freedesktop.DBus", "/org/freedesktop/DBus", "org.freedesktop.DBus",
                "GetConnectionUnixProcessID", "s", name))
            assert value["type"] == "u" and len(value["data"]) == 1, value
            return value["data"][0]

        receipt["shell_pid"] = bus_pid("org.gnome.Shell")
        receipt["notification_service_pid"] = bus_pid("org.freedesktop.Notifications")
        assert receipt["shell_pid"] == processes[0].pid
        assert receipt["notification_service_pid"] == processes[1].pid
        assert desktop.tray_fixture.host is None, "A fake notification host must not be running"
        desktop.command("xdotool", "key", "Escape", "Escape")
        eventually(lambda: not observation()["overview_visible"], "GNOME overview dismissed")
        receipt["shell_startup"] = observation()
        baseline = desktop.command(
            "gdbus", "call", "--session", "--dest", "org.freedesktop.Notifications", "--object-path",
            "/org/freedesktop/Notifications", "--method", "org.freedesktop.Notifications.Notify",
            "Shep", "0", APP_ID, "Fictional short-lived sender", "Disconnected sender baseline", "[]",
            "{'desktop-entry': <'so.shep.Shep'>, 'suppress-sound': <true>}", "--", "-1")
        time.sleep(.7)
        assert not observation()["notifications"], "Short-lived sender unexpectedly survived"
        receipt["short_lived_sender_acknowledgment"] = baseline
        receipt["short_lived_sender_removed"] = True
        desktop.batch([check("notifications.requested", 0), check("notifications.sent", 0)])

        desktop.launch_size = (1440, 920)
        desktop.batch([{"type": "restart"}])
        desktop.command("xdotool", "windowactivate", "--sync", desktop.window)
        receipt["setup_window"] = prepare_window(desktop)
        desktop.batch([click(100, 878), check("tab", "Preferences"), wait(150)])
        if mode != "details":
            desktop.batch([click(690, 366), check("dark", True)])
        desktop.batch([click(1150, 88), type_text("notifications"),
                       check("settings_matches", ["Notifications"]), click(450, 289),
                       check("settings_group", "Notifications"),
                       click(288, 413), check("notifications.settings.sound", False)])
        if mode == "private":
            desktop.batch([click(288, 452), check("notifications.settings.show_details", False)])
        if mode == "muted":
            desktop.batch([click(288, 374), check("notifications.settings.popups", False)])
        desktop.batch([key("ctrl+1"), check("tab", "Mail")])
        if mode == "private":
            desktop.launch_size = (900, 640)

        # Only the next fictional arrival uses the ordinary sync/cache claim path.
        desktop.launch_args.append("--background-sync")
        desktop.batch([{"type": "restart"}])
        desktop.command("xdotool", "windowactivate", "--sync", desktop.window)
        desktop.batch([check("total", 121), check("notifications.requested", 1),
                       check("notifications.sent", 0 if mode == "muted" else 1)])
        if mode != "muted":
            eventually(lambda: observation()["notifications"] and observation()["banner"],
                       "actual new-mail banner")
        # Delivery has returned. Keep observing across the old per-call sender lifetime.
        time.sleep(.7)
        validate_notification(observation()["notifications"], mode)
        desktop.command("import", "-window", "root", "-quality", "95", str(directory / "new-mail-banner.webp"))
        receipt["arrival"] = observation()
        desktop.batch([key("ctrl+r"), {**check("sync_round", 2), "timeout_ms": 5000},
                       check("refreshing", False), check("notifications.sent", 0 if mode == "muted" else 1)])
        validate_notification(observation()["notifications"], mode)
        desktop.command("xdotool", "key", "--clearmodifiers", "super+v")
        eventually(lambda: observation()["centre_open"] and observation()["centre_mapped"],
                   "native notification centre mapped")
        def centre_ready():
            state = observation()
            assert state["centre_open"] and state["centre_mapped"], state
            validate_notification(state["notifications"], mode)
        require_stable(centre_ready)
        validate_notification(observation()["notifications"], mode)
        desktop.command("import", "-window", "root", "-quality", "95", str(directory / "notification-centre.webp"))
        receipt["centre"] = observation()
        assert bus_pid("org.freedesktop.Notifications") == processes[1].pid
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == digest, "Binary changed during verification"
        receipt.update(passed=True, shell=desktop.command("gnome-shell", "--version"))
        return directory
    except Exception as error:
        receipt["error"] = str(error)
        raise
    finally:
        def save_receipt():
            if desktop.directory:
                (desktop.directory / "notification-evidence.json").write_text(json.dumps(receipt, indent=2))
        tray = desktop.tray_fixture
        processes.extend([desktop.app, desktop.clipboard, desktop.xvfb])
        if tray:
            processes.extend([tray.host, tray.bus])
        cleanup_all([save_receipt, desktop.stop,
                     *[lambda process=process: stop_process(process) for process in reversed(processes)],
                     *([tray.alias.cleanup, tray.log.close] if tray else []), runtime.cleanup])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/test-ui/shep")
    parser.add_argument("--mode", choices=("details", "private", "muted", "all"), default="all")
    parser.add_argument("--harness-root", type=Path, help="Use another worktree's matching native harness")
    arguments = parser.parse_args()
    desktop_type = Desktop
    if arguments.harness_root:
        spec = importlib.util.spec_from_file_location("notification_harness", arguments.harness_root / "scripts/mcp_harness.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        desktop_type = module.Desktop
    for mode in ("details", "private", "muted") if arguments.mode == "all" else (arguments.mode,):
        run(arguments.binary, mode=mode, desktop_type=desktop_type)
