# Shep

A native Rust + iced email and calendar client, with a quiet interface, light/dark themes, and equally useful mouse controls and remappable keyboard shortcuts.

This is an initial working implementation. Fastmail IMAP/SMTP authentication and Inbox sync have been verified with the saved account. Google authorization, other live providers, and Windows/macOS execution still need verification with real accounts and platform runners. Automated tests use isolated fixture mail and local protocol streams.

## Run and install

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

## Mail and layout

- Add and edit multiple IMAP or POP3 accounts in Preferences → Accounts. The setup wizard separates identity, incoming IMAP/POP3 and outgoing SMTP settings, with SSL/TLS or STARTTLS, authentication choices, and independent connection tests. Fastmail has a preset; use its app password and full login address. Certificate verification is always enabled.
- Enable the unified inbox in General preferences, expand it to choose an account, or disable it for account-specific navigation. Custom folders appear under each account. Fuzzy search covers indexed sender, subject and body text. Filter All, Unread, Read, Flagged or Attachments; sort newest/oldest, sender or subject. Sorting is saved.
- Flag/unflag directly in an inbox row or the reader toolbar; mark read/unread in the toolbar. IMAP flags synchronize with the server; POP3 flags and folders are local. The Flagged sidebar view searches across folders.
- Drag the divider between inbox and reader to resize them. The split is saved after dragging, with minimum pane widths. Pages contain 50 messages; visible rows and a small margin are rendered. Adjacent bodies and the next page preload in the background.
- Move with the visible button or `M`, fuzzy-search a folder, then press Enter to use the top match. Opt-in moves between IMAP accounts preserve the source until the destination confirms receipt; interrupted transfers are recorded to prevent blind duplicate uploads. Reply, compose, save drafts, archive, move to Trash, export original `.eml` files and save attachments through native file dialogs.
- Drafts autosave locally after editing pauses and are saved when closing the composer or app. SMTP failure preserves the draft. Sent messages have a local Sent copy.
- Up/Down selects messages, Tab switches between inbox and sidebar navigation, and double-click opens a full-window reader. Close it with the button or remappable Escape shortcut.
- Reply history can be collapsed, expanded or hidden. Attachments wrap alongside Reply. Click the sender for copyable addresses. Remote images default to blocked, with message/sender/domain exceptions and Block all / Contacts / Allow all policies in Privacy preferences. Contacts are currently a manually maintained list.
- Preferences → General includes message font size and interface scaling, alongside Light, Dark and System appearance. Preferences → Shortcuts remaps every listed action and rejects duplicate bindings. Letter shortcuts do not activate inside text fields. `Mod` is Command on macOS and Control elsewhere.

## Calendar

The Calendar tab combines a month grid with an agenda. Connect Google Calendar or enter your homeserver's **CalDAV calendar collection URL** under Preferences → Calendars. CalDAV currently requires the collection URL itself, rather than automatic principal discovery; Nextcloud URLs commonly end with `/remote.php/dav/calendars/USER/CALENDAR/`.

Sync covers the previous 90 days and next 365 days. Double-click a day to add an all-day event. Create, edit and delete timed, multiday and all-day events. The last date in the all-day editor is inclusive. Existing remote events use ETags to detect conflicting writes. Expanded CalDAV recurring occurrences can be viewed; edit their series using the server's calendar interface. Calendar creation and edits are synchronized through the background engine. See Google's [calendar concepts](https://developers.google.com/workspace/calendar/api/concepts/events-calendars) for calendar and recurrence terminology.

CalDAV edits preserve existing alarms, attendees, timezones and extension fields. Successful writes update the local calendar without depending on another full sync. Interrupted creates keep the same identity when retried; conflicting edits ask you to sync. A server that omits its updated ETag can still save an event, but you must sync before editing it again.

## Optional Google login and backups

Google is optional. Without it, mail and CalDAV work with local settings and you can back up to a local folder.

1. Create a Google Cloud project and enable **Drive API** and **Google Calendar API**.
2. Configure its OAuth consent screen; add your Google address as a test user if the app is in testing.
3. Create an OAuth client of type **Desktop app**. Enter its client ID and desktop client secret in Preferences → General, or set `SHEP_GOOGLE_CLIENT_ID` and `SHEP_GOOGLE_CLIENT_SECRET` before the first launch.
4. Save preferences, then choose **Connect Google**. Authorization opens the system browser, using PKCE, state validation and a temporary loopback callback, following Google's [desktop OAuth flow](https://developers.google.com/identity/protocols/oauth2/native-app).
5. In Preferences → Backups, choose Local folder or Google Drive, set retention (1–100 copies) and interval (1–8760 hours), enter a passphrase of at least 12 characters, and make the first backup. Enable automatic backups if wanted.

Google refresh tokens and saved account passwords are kept in the OS keychain. Desktop OAuth client configuration is stored with local preferences; it is not a confidential server credential. Google login currently requests Drive app-data and Calendar permissions together.

Backups contain downloaded original email, account configuration, calendar-source settings and preferences. Enable **Include account passwords** to carry those passwords inside the encrypted snapshot. Google tokens are never included. Snapshots use compression, Argon2id and AES-256-GCM with fresh salts/nonces. The passphrase is saved in the OS keychain after a successful backup for scheduled use. Keep a separate copy of it for restore.

“Back up now” saves the displayed backup settings before starting. Make one manual copy at each destination to prepare its automatic schedule; this also works when you enable automatic backups afterward. Changing destination, Google account or OAuth application requires a new first copy. Reconnecting the same verified Google account preserves its pending uploads and history. Existing installations need to reconnect Google once to verify the account/application binding, and make a new manual copy to establish the destination-specific keychain entry. Missing keychain access pauses automatic backups and the Backups page explains how to resume. A cleanup or keychain failure after upload reports that the copy was saved and identifies the remaining step.

Drive copies live in its private application data area, accessed only with `drive.appdata`; they do not appear as ordinary files in My Drive. This follows the [Drive app-data model](https://developers.google.com/workspace/drive/api/guides/appdata). Use the same OAuth application when restoring on another machine. Retention deletes only Shep copies after a successful new upload. Restore decrypts and validates first, then merges mail/accounts; it keeps this device's Google login and backup preferences. Calendar events sync from their calendar providers again.

Restore imports new mail and account/calendar settings in one local transaction. Existing messages, flags, folder moves, connection settings, preferences and unsent drafts are retained. Included passwords fill only missing OS-keychain entries for matching connections; restoring an older copy never replaces a current password. If the keychain is locked, the mail import is still complete: unlock it and restore the same copy again to finish the missing passwords without duplicating mail. Credentials for connections whose settings have changed are skipped. Invalid credential owners, duplicate IDs, conflicting cached identities or unreadable originals stop the import before any local changes.

Recovered emails remain cached even if the server has already deleted them. They follow server deletions again only after a complete sync confirms the same remote identity. Restore does not upload mail back to the server.

Drive reserves each file ID before transferring a copy and uploads in 1 MiB resumable chunks, following the [Drive upload protocol](https://developers.google.com/workspace/drive/api/guides/manage-uploads). Shep saves the encrypted archive, reserved ID and session in `backup-uploads.sqlite` beside the mail cache, using a separate database connection. After interruption or an app restart, choose **Back up now** with the original passphrase to resume that exact copy. A missing final response is resolved by checking the reserved file's identity, size and ciphertext checksum. A confirmed copy with unfinished cleanup is retried without another upload. Healthy uploads are not stopped by an overall ten-minute deadline; individual HTTP requests and lack of progress remain bounded.

## Current limits

- The local SQLite mail cache is not encrypted at rest; encrypted backups and OS-keychain credentials have separate protection. Use device disk encryption when needed.
- Incoming messages over 25 MiB are skipped with a notice. A snapshot supports up to 256 MiB of original mail in this version. Email text previews are capped at 32,000 characters; export preserves the original message and attachments.
- The reader renders plain text with a basic HTML-to-text fallback. Remote images load only after the privacy policy allows them, with bounded downloads, address validation and WebP conversion. Scripts never execute. Full HTML layout, outgoing attachments, CC/BCC, grouping separate messages into threads, invitations and general offline mutation queues are not implemented yet. Quoted reply history within a message is supported.
- Gmail mail access currently needs an app password and compatible account settings; Google login connects Drive/Calendar, not Gmail OAuth. OAuth-only IMAP servers are not supported yet.
- Same-account IMAP moves require MOVE; cross-account moves require two IMAP accounts and source UIDPLUS. An upload interrupted before its acknowledgement is retained for manual destination inspection, while a confirmed copy can resume source removal without uploading again. POP3 requires UIDL and never deletes server originals. Periodic background sync is configurable from 1–60 minutes; IMAP IDLE is not implemented yet.
- SMTP sends retain a local Sent copy; server-side Sent APPEND is not implemented yet. CalDAV discovery and editing recurring series are not implemented.
- Automated UI testing currently runs on Linux/X11. Windows/macOS builds are in the dormant CI matrix but have not been executed here. Signed/notarized installers are not included.

## Development, tests and releases

```sh
bash scripts/install-hooks.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
python3 -m unittest discover -s tests -p 'test_*.py'
cargo build --profile test-ui --features test-support
python3 scripts/e2e.py --functional-only     # use while the machine is busy
# On an otherwise idle machine, at the end:
cargo bench --bench responsiveness
python3 scripts/e2e.py
python3 scripts/performance_gate.py
```

Linux native E2E additionally needs `xvfb`, `xdotool`, and ImageMagick with WebP support. The optimized `test-ui` profile avoids measuring debug rendering. `scripts/check.sh` runs the full suite; `SHEP_SKIP_E2E=1` explicitly omits GUI tests when X11 is unavailable.

The native MCP server is configured in `.mcp.json`. Read [the repository E2E skill](.agents/skills/shep-e2e/SKILL.md). Its batch tool performs real clicks, double-clicks, drags, typing and shortcuts, plus bounded waits, observed-state assertions and WebP screenshots. Every AI-driven scenario must have an equivalent automated test. Fixture mail exists only behind the nondefault `test-support` feature. The production application does not launch fixture workspaces.

All generated logs and evidence belong under ignored `artifacts/`; never leave logs in the repository root. Performance budgets, methodology and the validated Linux baseline are in [docs/PERFORMANCE.md](docs/PERFORMANCE.md). The [completion audit](docs/COMPLETION.md) distinguishes implemented behavior, remaining work and verification that still needs live services/platforms.

**All GitHub Actions are deliberately disabled.** Workflow files use `.yml.disabled` and repository Actions are disabled. [AGENTS.md](AGENTS.md) records the reminder and exact steps for re-enabling on the intended self-hosted Linux, Windows and macOS runners. Do not enable automatically.

Conventional Commits drive semantic-release on `main`. Pre-commit runs fmt, Clippy and Rust tests; commit-msg validates the commit format. Release tooling uses Node 24 and `npm ci`. `npm run release:dry` checks the proposed release without publishing. The dormant release workflow runs only after a successful quality build, verifies the tested commit, prepares the Cargo version/archive/checksums, and publishes the GitHub release. Current release packaging produces Linux archives.

## Extending Shep

`src/ui/` contains iced presentation and bounded prefetch caches. `engine.rs` dispatches bounded channels to background jobs; `store.rs` keeps SQLite work off the UI thread. Mail sync, flags and moves serialize per account to avoid racing stale server snapshots.

Cached search and message loads have reserved workers independent of provider operations. Speculative prefetch has its own smaller queue, and settings/drafts save in order on a separate worker. The dispatcher is tested with every provider worker blocked and its command queue full while local reads and saves continue.

Preferences use versioned acknowledgements so an older background update cannot undo newer choices or pane resizing. Message-detail results are invalidated after mail changes, including late prefetch errors and old flag states. Backup completion updates only its destination's timestamp and schedule readiness without overwriting settings changed during the upload. Copy lists and restore actions are bound to the selected destination. Retention protects the acknowledged copy even after a clock correction; local uploads never overwrite an existing file.

Implement `providers::MailProvider`, `providers::CalendarProvider` or `backup::BackupProvider` for a new provider, register its factory/configuration, and add deterministic contract tests. Wire-protocol code belongs in the provider; the UI only sends commands and handles events. Keep channel capacity, concurrency limits, cancellation, TLS verification and retention invariants intact.

The approved White Swiss Shepherd logo and dark variant are in `assets/`; [assets/README.md](assets/README.md) records image-generation prompts and derivations. Font licensing is included alongside the embedded fonts. MIT license.
