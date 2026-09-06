# Current limits

Shep is still in development. Fastmail authentication and Inbox sync have been verified with a real account. Live Google, other providers and Windows/macOS remain unverified.

| Area | Limit today |
| --- | --- |
| Reading | Static HTML layout with selectable text and a plain-text option. External images follow your privacy settings and are blocked by default. Advanced browser CSS and animations are not fully supported. |
| Incoming mail | 25 MiB per message; larger messages are skipped with a notice. |
| Attachments | Up to 32 files, 18 MiB total, within a 25 MiB outgoing message. |
| Backups | Up to 256 MiB of original mail per snapshot. |
| Gmail | Requires an app password and compatible account settings; Gmail OAuth is not supported. |
| Calendar | Sync covers 90 days back and 365 days ahead. Edit recurring CalDAV series on the server. |
| Local storage | The mail cache is not encrypted at rest. Backups are encrypted; credentials use the OS keychain. |

POP3 keeps folders and flags locally and leaves server originals intact. IMAP moves need server support. Invitations, IMAP IDLE and general offline action queues are not yet available.

For implementation details, see the [agent reference](agents/limits.md) and [completion audit](COMPLETION.md).
