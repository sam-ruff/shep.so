# Shep

A calm, native email and calendar app built with Rust and iced. Comfortable with a mouse, with remappable shortcuts when you want them.

[Documentation](https://sam-ruff.github.io/shep.so/) · [Install](docs/installation.md) · [Agent docs](docs/agents/index.md)

## Install

On Linux, install [the build dependencies and current stable Rust](docs/installation.md#build-dependencies), then run:

```sh
curl -fsSL https://raw.githubusercontent.com/sam-ruff/shep.so/main/scripts/install-release-linux.sh | bash
```

This adds Shep to your applications menu. It installs a checksum-verified release when available; until releases are published, it downloads a pinned revision of `main` and builds it first. Source builds take time and require the dependencies above. Installation defaults to your home directory, with all-user and cancel choices in an interactive terminal.

Release CI remains paused. [Windows and macOS installers](docs/installation.md#other-platforms) require published platform assets and are not available for installation yet.

![Shep in light mode, with a unified inbox and an open email](docs/images/mail-light.webp)

- **Mail in one place.** Multiple accounts, a unified inbox, search across folders, conversation reading and bulk actions with Undo.
- **Everyday essentials.** Replies, forwarding, printing, attachments, autosaved drafts and recovery for interrupted sends.
- **Calendars alongside.** Google Calendar and CalDAV, with a month view and agenda.
- **Make it yours.** Light, Dark or System appearance, editable color palettes, resizable panes, configurable shortcuts and new-mail popup/sound controls.
- **Encrypted backups.** Save to multiple local folders, Google Drive, S3-compatible storage, SFTP or FTP/FTPS. Google is optional.

![Shep's calendar in dark mode, showing the month and upcoming events](docs/images/calendar-dark.webp)

*Native Linux screenshots with fictional demo mail and events.*

## Get started

With current stable Rust and the [Linux dependencies](docs/installation.md#build-dependencies) installed:

```sh
git clone https://github.com/sam-ruff/shep.so.git
cd shep.so
cargo run --release --locked --no-default-features --jobs 4
```

Or install for your Linux user:

```sh
bash scripts/install-linux.sh
```

Add accounts and calendars in **Preferences**.

Mobile (`flutter/`), a separate browser client (`web/`), the promo site (`website/`) and a Rust beta gateway (`backend/`) are being developed in this monorepo. [Client parity and remaining work](docs/CLIENT_PARITY.md) records what is available; the new clients are previews, not replacements yet.

## Still in development

Fastmail login and Inbox sync have been verified. Live Google, other providers and Windows/macOS still need verification.

- Mail supports static HTML layout and selectable text, with a plain-text option. External images load only when your privacy settings allow them.
- Downloads: **25 MiB per message**; MIME nesting beyond 128 multipart levels is refused. Backups: **256 MiB of original mail**.
- Gmail needs an app password; Google sign-in does not provide Gmail OAuth.
- Calendar sync: **90 days back, 365 days ahead**. Edit recurring CalDAV series in your server's calendar UI.
- The local mail cache is not encrypted at rest.
- Complete database export/import is in Preferences. Imported profiles require credential reconnection; continuous account/profile sync is still in development.

[Full limits](docs/limits.md) · [Contributing](docs/development.md) · MIT licensed.
