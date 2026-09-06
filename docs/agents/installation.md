# Install and run

Use current stable Rust with `rustfmt` and `clippy`, and Python 3 for development scripts. On Debian/Ubuntu, build dependencies include:

```sh
sudo apt install build-essential pkg-config libssl-dev libdbus-1-dev \
  libx11-dev libxkbcommon-dev libwayland-dev
cargo run --release
```

Linux credentials require a working Secret Service, such as GNOME Keyring or KeePassXC with Secret Service enabled. Windows uses Credential Manager; macOS uses Keychain. Linux file dialogs use the desktop portal, so install your desktop's `xdg-desktop-portal` implementation.

Install the optimized production build for your Linux user:

```sh
bash scripts/install-linux.sh
# Optionally pin it to the GNOME dash as well:
bash scripts/install-linux.sh --pin
```

The installer builds with `--release --locked --no-default-features`, installs `~/.local/bin/shep`, and registers **Shep** in the applications menu. Right-click it and choose **Add to Favorites** or your panel's pin action. `--pin` supports GNOME and preserves existing favorites. KDE and other panels can pin the launcher through their normal menus. The desktop entry and window use `so.shep.Shep`, following the [freedesktop launcher specification](https://specifications.freedesktop.org/desktop-entry/latest-single/) and [iced's application ID guidance](https://docs.rs/iced/0.14.0/iced/window/settings/struct.PlatformSpecific.html).

An extracted release archive includes the installer and can be installed without Rust. You can also supply an existing binary:

```sh
bash scripts/install-linux.sh --binary /path/to/shep
bash scripts/install-linux.sh --uninstall    # keeps accounts, mail, drafts and backups
```

`--prefix` changes the binary prefix; `--data-dir` changes launcher/icon placement. Launcher files otherwise follow `XDG_DATA_HOME`. Do not run the user installer with sudo. App images are cached WebP; the desktop launcher has a small PNG compatibility icon for Linux icon themes.
