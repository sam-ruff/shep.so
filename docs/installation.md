# Install Shep

Linux is the currently verified platform. Windows and macOS still need testing.

## Download a release

```sh
curl -fsSL https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install-release-linux.sh | bash
```

The Linux installer requires curl and Python 3. It verifies `SHA256SUMS` before
installing to `~/.local/bin` and adding the native applications-menu entry.
On an interactive terminal, choose your user (default), all users, or cancel.
All-user installation uses `/usr/local` and asks through sudo when needed.
The installer never stops an open Shep window; reopen it after an update.

Release CI is currently paused and no binary releases are published yet. The
installer reports missing releases or platform assets without changing installed
files. Use the source instructions below until an archive is available;
Windows download installation and macOS/Windows release packaging remain in development.

For a particular release, a prompt-free user install or optional GNOME pinning:

```sh
curl -fsSL https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install-release-linux.sh | bash -s -- --version 1.2.3 --user --pin
```

Replace `1.2.3` with an actually published version. `--system` explicitly selects
all users; `--yes` uses the user default without prompting. Custom user locations
use `--prefix PATH` and `--data-dir PATH`. No Rust checkout is required for a
published archive.

### macOS

```sh
curl -fsSL https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install-release-macos.sh | bash
```

Uses built-in macOS tools; no Python or Homebrew is needed. The default is
`~/Applications/Shep.app`, including its native icon and application identity.
The terminal prompt offers your user, all users, or cancel. For explicit all-user
installation, append `bash -s -- --system` instead of `bash`; only the final staged
copy to `/Applications` requests administrator access. `--version`, `--user`,
`--yes` and `--prefix APPLICATIONS_FOLDER` are also available.

Checksums and exact regular archive members are validated before an installed app
changes. Failed replacement restores the previous app. Open Shep from Applications
and reopen after updates. No running process is stopped, user data is kept, and
Gatekeeper settings are preserved. This does not create a signed or notarized app.
The shell and isolated native-tool contracts are tested on Linux; actual macOS
execution and published macOS assets remain pending.

## Run from source

Install stable Rust (1.88 or newer) and Python 3. On Debian or Ubuntu, add these build dependencies:

```sh
sudo apt install build-essential cmake pkg-config libssl-dev libdbus-1-dev \
  libx11-dev libxkbcommon-dev libwayland-dev
```

From the repository checkout:

```sh
cargo run --release
```

Linux needs a Secret Service, such as GNOME Keyring, to save passwords. File dialogs need your desktop's `xdg-desktop-portal` implementation.

## Add it to your applications menu

```sh
bash scripts/install-linux.sh
```

This builds Shep and installs it for your user. Do not run the installer with sudo. Add `--pin` to pin it to the GNOME dash.

An extracted release archive uses the same installer and does not need Rust.

## Connect an account

Open **Preferences → Accounts** to add mail, or **Preferences → Calendars** to connect a calendar. Fastmail users can select the preset and enter an app password.

## Uninstall

```sh
bash scripts/install-linux.sh --uninstall
```

Your accounts, downloaded mail, drafts and backups are kept.

## Unread dock count

On Linux, **Preferences → General → Mail & performance** controls the unread
Inbox badge. It counts all connected accounts, even when viewing another folder.
The installed desktop entry must remain named `so.shep.Shep.desktop`.

The dock must support the [Unity Launcher API](https://wiki.ubuntu.com/Unity/LauncherAPI),
as [Dash to Dock does](https://github.com/micheleg/dash-to-dock/blob/master/launcherAPI.js).
Windows uses a native taskbar overlay (large taskbar icons), and macOS uses the native Dock badge. These adapters pass platform compilation checks; actual Windows/macOS desktop verification remains pending. Windows displays 99+ above 99 unread emails; its accessibility label retains the full count.
