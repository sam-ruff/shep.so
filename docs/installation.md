# Install Shep

Linux is the currently verified platform. Windows and macOS still need testing.

## Run from source

Install stable Rust (1.88 or newer) and Python 3. On Debian or Ubuntu, add these build dependencies:

```sh
sudo apt install build-essential pkg-config libssl-dev libdbus-1-dev \
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
