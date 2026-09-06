# Completion audit

The user requested a complete, polished Rust + iced mail/calendar client. Passing the current suite is evidence for specific behavior, not evidence that the whole goal is complete. This audit records outstanding work; it does not replace or narrow the original specification.

## Implemented, with local evidence

| Requirement | Implementation and evidence | Remaining verification |
| --- | --- | --- |
| Native mouse-friendly UI, remappable shortcuts | iced mail/calendar/preferences, native MCP keyboard/mouse flows, saved remapping | Broader accessibility and large-font layout review |
| Multiple saved IMAP/POP3 accounts, SMTP wizard | SQLite settings, OS secrets, local IMAP transcript tests; authorized Fastmail authentication and Inbox download | POP3/SMTP wire contracts beyond fixtures; account lifecycle |
| Responsive inbox, preloading, resize, filters, fuzzy move | Independent bounded foreground/prefetch/persistence/provider workers, background store, caches, virtual inbox, native functional flows; saturated-provider correctness test | Review stale preferences during rapid changes; final performance rerun |
| Compact inbox header | Single row for title/count/sync; native layout gallery at 1440×920 and 900×640 | Installed/running windows need to use the latest build |
| Optional Google login and Drive backups | Browser OAuth/PKCE, app-data scope, encrypted rolling backups; local retention/encryption tests | HTTP contracts for OAuth refresh/Drive and genuine Google authorization |
| Google Calendar and CalDAV | Background sync/create/edit/delete, conditional writes, native all-day editor | CalDAV discovery, Google read-only calendars, connection lifecycle; live server evidence |
| Safe calendar edits | Complete CalDAV resource preservation; stable Google create identity; source-scoped cache/editor; serialized per-calendar sync/write | Recurrence editing and invitations remain separate work |
| Reader, sender actions, attachments, image policy, quote display | Native reader/full-window reader, copy dialog, wrapping received attachments, image exceptions, collapsed/expanded/latest quote preferences | Separate-message conversation grouping and richer mail composition |
| Logo, WebP, appearance | Approved Swiss Shepherd assets and dark variant; cached WebP, Light/Dark/System | Continue visual review after layout changes |
| Testing, release, installer | Repo MCP skill, deterministic native scenarios, Rust/Python tests, hooks, semantic-release files, Linux installer | Final artifact/build verification after remaining changes; non-Linux execution/distribution |
| CI and performance gates | Dormant workflow definitions, strict backend/native budgets, Actions disabled by user request | Keep disabled until Sam asks; run measurements only at the end on an idle host |

## Remaining implementation audit

1. Cached queries, body loads and ordered saves now have independent workers, verified with all provider slots and their queue occupied. Startup also defers the Google keychain check until the cached workspace is ready. Next review stale preference snapshots during rapid UI changes and long-running backups; a late workspace update must not undo newer local settings. Check stale detail/prefetch results across flag and move mutations as well.
2. Complete account/calendar lifecycle and make connection errors recoverable without leaving stale sources or credentials. Respect Google calendar access roles and add CalDAV discovery/connection testing.
3. Complete common sending/reading workflows: outgoing attachments, CC/BCC, reply-all, server Sent handling, and grouping separate messages into conversations. Preserve drafts across failures.
4. Exercise Google OAuth refresh, Drive upload/list/restore/retention, POP3 and SMTP through deterministic protocol contracts. Handle pagination loops, partial failures and ambiguous writes explicitly.
5. Review the current 25 MiB message and 256 MiB snapshot ceilings against real mailbox sizes. Large-mail/backup paths must remain bounded in memory while avoiding silent omissions.
6. Review scaled fonts, compact windows, empty/error states and keyboard/mouse parity. Finish with the full functional suite, final performance gates, release/install checks, and a push to `sam-ruff/shep.so`.

Live Google/CalDAV and Windows/macOS execution are not established by Linux fixture tests. Do not describe those checks as completed. Useful independent development remains, so the goal is not blocked on that verification yet.

## Calendar correctness evidence

`src/providers/calendar/*` contains loopback HTTP contract tests for Google pagination/conditional PATCH/DELETE, stable POST retry recovery, malformed committed responses, redirect rejection, and CalDAV REPORT/GET/PUT/DELETE with ETags. CalDAV edits retain alarms, attendees, timezones and extension properties. Conflict tests ensure another client's changes are not overwritten.

`tests/calendar.rs` covers all-day/default/DURATION semantics (including DST), escaped text, source-scoped IDs, atomic mixed-source rejection, and migration from the original cache keys. Engine tests cover source-scoped mutation and acknowledgment after a remote commit even when local caching fails. The native MCP suite edits and deletes one of two events with the same remote UID in different calendars.

Protocol references: [Google event IDs](https://developers.google.com/workspace/calendar/api/v3/reference/events/insert), [CalDAV resource ETags, RFC 4791 §5.3.4](https://www.rfc-editor.org/rfc/rfc4791.html#section-5.3.4), [iCalendar event duration, RFC 5545 §3.6.1](https://www.rfc-editor.org/rfc/rfc5545.html#section-3.6.1).
