# Current limits

Shep is still in development. Fastmail authentication and Inbox sync have been verified with a real account. Live Google, other providers and Windows/macOS remain unverified.

| Area | Limit today |
| --- | --- |
| Reading | Text rendering with a basic HTML-to-text fallback. External images follow your privacy settings and are blocked by default. |
| Incoming mail | 25 MiB per message; larger messages are skipped with a notice. |
| Attachments | Up to 32 files, 18 MiB total, within a 25 MiB outgoing message. |
| Backups | Up to 256 MiB of original mail per snapshot. |
| Gmail | Requires an app password and compatible account settings; Gmail OAuth is not supported. |
| Calendar | Sync covers 90 days back and 365 days ahead. Edit recurring CalDAV series on the server. |
| Local storage | The mail cache is not encrypted at rest. Backups are encrypted; credentials use the OS keychain. |

POP3 keeps folders and flags locally and leaves server originals intact. IMAP moves need server support. Full HTML layout, invitations, IMAP IDLE and general offline action queues are not yet available.

For implementation details, see the [agent reference](agents/limits.md) and [completion audit](COMPLETION.md).

Incoming MIME nesting beyond 128 multipart levels is refused before recursive parsing. This worker-safety check is separate from the open large-message streaming requirement. Client formatted HTML rendering remains pending; shared raw HTML representations are not safe page content.
