"""Owned native StatusNotifier fixture. Never connects to the user's session bus."""
import html
import json
import os
from pathlib import Path
import subprocess
import time
import tempfile
import shutil


class TrayFixture:
    def __init__(self, desktop, present):
        self.desktop = desktop
        self.host = None
        directory = desktop.directory
        self.log = (directory / "tray-host.log").open("w")
        icon_directory = directory / "icon-theme" / "hicolor" / "scalable" / "apps"
        icon_directory.mkdir(parents=True)
        shutil.copyfile(Path(__file__).resolve().parents[1] / "assets/shepherd-symbolic.svg",
                        icon_directory / "so.shep.Shep-symbolic.svg")
        # AF_UNIX names have a small platform limit; worktree artifact paths can
        # exceed it. This owned alias still points to the exact fixture directory.
        self.alias = tempfile.TemporaryDirectory(prefix="shep-tray-")
        self.owned_dir = Path(self.alias.name) / "owned"
        self.owned_dir.symlink_to(directory, target_is_directory=True)
        address = f"unix:path={self.owned_dir}/tray-bus"
        desktop.env["DBUS_SESSION_BUS_ADDRESS"] = address
        config = directory / "tray-bus.conf"
        config.write_text(f'''<busconfig><type>session</type><listen>{html.escape(address)}</listen>
<auth>EXTERNAL</auth><policy user="{os.getuid()}"><allow own="*"/>
<allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>''')
        self.bus = subprocess.Popen(["dbus-daemon", f"--config-file={config}", "--nofork", "--nopidfile"],
            env=desktop.env, stdout=subprocess.DEVNULL, stderr=self.log)
        deadline = time.monotonic() + 5
        while not (directory / "tray-bus").exists():
            if self.bus.poll() is not None or time.monotonic() > deadline:
                self.close()
                raise RuntimeError("The owned tray bus did not start")
            time.sleep(.02)
        if present:
            self.start_host()

    def start_host(self):
        if self.host and self.host.poll() is None:
            raise RuntimeError("The fixture tray host is already running")
        self.host = subprocess.Popen(["/usr/bin/python3", str(Path(__file__).parent / "fixtures/tray_host.py"),
            "--state", str(self.owned_dir / "tray-host.json")], env=self.desktop.env,
            stdout=self.log, stderr=self.log)
        deadline = time.monotonic() + 5
        while True:
            try:
                self.host_window = self.desktop.command("xdotool", "search", "--onlyvisible", "--name", "^Shep tray test host$").splitlines()[-1]
                self.desktop.command("xdotool", "windowmove", self.host_window, "20", "20")
                return
            except (subprocess.SubprocessError, IndexError):
                if self.host.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("The native fixture tray host did not open; inspect tray-host.log")
                time.sleep(.02)

    def stop_host(self):
        if self.host and self.host.poll() is None:
            self.host.terminate()
            self.host.wait(timeout=3)
        self.host = None

    def menu(self):
        if not self.host or self.host.poll() is not None:
            raise RuntimeError("The owned native tray host is unavailable")
        self.desktop.command("xdotool", "windowraise", self.host_window)
        self.desktop.command("xdotool", "windowfocus", self.host_window)
        self.desktop.command("xdotool", "mousemove", "--window", self.host_window, "80", "23", "click", "1")

    def theme(self):
        if not self.host or self.host.poll() is not None:
            raise RuntimeError("The owned native tray host is unavailable")
        self.desktop.command("xdotool", "windowraise", self.host_window)
        self.desktop.command("xdotool", "windowfocus", self.host_window)
        self.desktop.command("xdotool", "mousemove", "--window", self.host_window, "80", "77", "click", "1")

    def state(self):
        path = self.desktop.directory / "tray-host.json"
        return json.loads(path.read_text()) if path.exists() else {}

    def close(self):
        self.stop_host()
        if self.bus.poll() is None:
            self.bus.terminate()
            self.bus.wait(timeout=3)
        self.log.close()
        self.alias.cleanup()
