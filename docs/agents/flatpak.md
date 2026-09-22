# Flatpak and Linux stores

Shep's Flathub packaging lives in `packaging/flatpak/`. It is prepared, not published: no Flatpak has been built with `flatpak-builder` yet, nothing has been submitted to Flathub, and store publication is tracked separately in [TODO.md](https://github.com/sam-ruff/shep.so/blob/main/TODO.md).

## Files

- `so.shep.Shep.json`: the manifest. It builds the release binary on the `org.freedesktop.Platform` runtime with the `rust-stable` SDK extension, offline and with `--locked`.
- `so.shep.Shep.metainfo.xml`: AppStream metadata with description, releases, OARS rating, links, branding colours and screenshots.
- `so.shep.Shep.desktop`: the launcher. It keeps the installer's identity (`so.shep.Shep`, the same `StartupWMClass`, icons and categories) with `Exec=shep`.
- `cargo_sources.py`: writes `cargo-sources.json` from `Cargo.lock`. The generated file is ignored in this repository.
- `capture_screenshots.py` and `screenshots/`: store screenshots from fictional fixture data.

The manifest installs the same icon set as `scripts/install_linux.py`, including the symbolic and tray icons, plus every licence notice shipped in release archives. `tests/test_flatpak_packaging.py` checks this against the installer and `scripts/release.py`, so identity changes must update both.

## Building locally

```sh
python3 packaging/flatpak/cargo_sources.py
flatpak-builder --user --install --force-clean build-dir packaging/flatpak/so.shep.Shep.json
flatpak run so.shep.Shep
```

`cargo_sources.py` produces the same crate entries as [flatpak-cargo-generator](https://github.com/flatpak/flatpak-builder-tools/tree/master/cargo) (checked against revision `41c20aa`), except that it names the Cargo configuration `config.toml`. It needs only the Python standard library, and refuses git or other registry sources, so every crate is pinned by its `Cargo.lock` checksum. Regenerate it whenever `Cargo.lock` changes.

The checkout source skips `target`, `artifacts`, `flutter`, `web` and `website`. For Flathub, replace the `dir` source with a `git` source pinned to a release tag and commit, and commit the generated `cargo-sources.json` in the Flathub repository.

Vendored native code (SQLCipher and its static OpenSSL, litehtml, libcurl) builds from source inside Cargo. Only `libdbus-sys` needs a system library, which the freedesktop SDK provides.

## Permissions

| Permission | Why |
| --- | --- |
| `--share=network` | IMAP, POP3, SMTP, CalDAV, Google and backup providers. Printing and Google sign-in also use a loopback server that the host browser must reach. |
| `--share=ipc` | Shared memory for X11. |
| `--socket=wayland`, `--socket=fallback-x11` | Native window; X11 only when Wayland is unavailable. |
| `--talk-name=org.freedesktop.Notifications` | New-mail popups. |
| `--talk-name=org.kde.StatusNotifierWatcher` | Tray icon registration. |
| `--talk-name=com.canonical.Unity` | Watches the dock that shows the unread launcher badge. |
| `--talk-name=org.freedesktop.secrets` | Passwords and Google tokens in the desktop keyring. |

There is no filesystem access. File choosers use the desktop portal (rfd's default backend), and links open through the OpenURI portal. There is no `--device=dri`: Shep renders with the tiny-skia software renderer and never opens a GPU device.

Inside Flatpak the tray registers its connection name rather than `org.kde.StatusNotifierItem-PID-ID`, because the sandbox cannot own that name.

## Sandbox differences

- Data lives in `~/.var/app/so.shep.Shep/`, separate from a script-installed Shep. Move it with Database export and import.
- Sound-only notifications use `canberra-gtk-play`, which the runtime lacks, so that mode shows its existing error. Popups with sound still work.
- The manifest does not embed the Google OAuth client, so Google sign-in reports that it is not configured in this build.
- Database export, attachment saving and local backup folders go through portal paths. Writing a temporary file beside the chosen file is expected to work in the document portal but has not been verified.

## Screenshots

With the `test-ui` build and the harness tools from the [native E2E skill](https://github.com/sam-ruff/shep.so/blob/main/.agents/skills/shep-e2e/SKILL.md):

```sh
python3 packaging/flatpak/capture_screenshots.py
```

It runs the saved `test_store_screenshots_*` tour on fictional fixture mail and converts the captures to PNG. The images show the fixture's TEST badge. The metainfo references them on `main` through `raw.githubusercontent.com`, so the URLs only resolve after merging.

## Other Linux stores

- **Snap**: a strict-confinement `snapcraft.yaml` would need the `network`, `desktop`, `wayland`, `x11` and `password-manager-service` interfaces. Tray and badge D-Bus access need interface review, and Snap Store review is separate from Flathub's.
- **AUR**: a `shep-bin` package could install the published Linux archive with its checksum, and a `shep` source package could build with `cargo --locked`. Both need published releases first.
- **AppImage**: straightforward for the static-heavy binary, but adds no update or sandbox benefit over the release archive.

Flathub comes first because it reaches most desktop distributions with one reviewed manifest.
