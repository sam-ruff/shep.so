# Shep

A calm, native email and calendar app built with Rust and iced. Comfortable with a mouse, with remappable shortcuts when you want them.

[Documentation](https://sam-ruff.github.io/shep.so/) · [Install](docs/installation.md) · [Agent docs](docs/agents/index.md)

![Shep in light mode, with a unified inbox and an open email](docs/images/mail-light.webp)

- **Mail in one place.** Multiple accounts, a unified inbox, search across folders, conversation reading and bulk actions with Undo.
- **Everyday essentials.** Replies, forwarding, printing, attachments, autosaved drafts and recovery for interrupted sends.
- **Calendars alongside.** Google Calendar and CalDAV, with a month view and agenda.
- **Make it yours.** Light, Dark or System appearance, resizable panes, configurable shortcuts and new-mail popup/sound controls.
- **Encrypted backups.** Save locally or to Google Drive. Google is optional.

![Shep's calendar in dark mode, showing the month and upcoming events](docs/images/calendar-dark.webp)

*Native Linux screenshots with fictional demo mail and events.*

## Get started

With stable Rust and the [Linux dependencies](docs/installation.md) installed, run from the checkout:

```sh
cargo run --release
```

Or install for your Linux user:

```sh
bash scripts/install-linux.sh
```

Add accounts and calendars in **Preferences**.

## Still in development

Fastmail login and Inbox sync have been verified. Live Google, other providers and Windows/macOS still need verification.

- Mail supports static HTML layout and selectable text, with a plain-text option. External images load only when your privacy settings allow them.
- Downloads: **25 MiB per message**. Backups: **256 MiB of original mail**.
- Gmail needs an app password; Google sign-in does not provide Gmail OAuth.
- Calendar sync: **90 days back, 365 days ahead**. Edit recurring CalDAV series in your server's calendar UI.
- The local mail cache is not encrypted at rest.
- Complete database export is in Preferences. Database import and continuous account/profile sync are still in development.

[Full limits](docs/limits.md) · [Contributing](docs/development.md) · MIT licensed.
