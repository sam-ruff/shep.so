# Completion audit

The user requested a complete, polished Rust + iced mail/calendar client. Passing the current suite is evidence for specific behavior, not evidence that the whole goal is complete. This audit records outstanding work; it does not replace or narrow the original specification.

## Implemented, with local evidence

| Requirement | Implementation and evidence | Remaining verification |
| --- | --- | --- |
| Native mouse-friendly UI, remappable shortcuts | iced mail/calendar/preferences, native MCP keyboard/mouse flows, saved remapping | Broader accessibility and large-font layout review |
| Multiple saved IMAP/POP3 accounts, SMTP wizard | SQLite settings, OS secrets, local IMAP transcript tests; authorized Fastmail authentication and Inbox download | POP3/SMTP wire contracts beyond fixtures; account lifecycle |
| Responsive inbox, preloading, resize, filters, fuzzy move | Independent bounded workers, background store, caches, virtual inbox; saturated-provider test; versioned preferences and message-detail results; native combined resize/settings flow | Pending save/shutdown and overload recovery review; final performance rerun |
| Compact inbox header | Single row for title/count/sync; native light/dark, idle/busy captures at 1440×920 and compact interaction at 900×640; updated production installation | Already-open windows retain the old executable until reopened |
| Optional Google login and Drive backups | Browser OAuth/PKCE with bounded loopback parsing; serialized refresh/sign-in and saved token rotation; encrypted rolling backups, destination-scoped setup/history/results, protected retention; durable upload journal and Drive resumable HTTP contracts; validated atomic restore and missing-password recovery | Genuine Google authorization, partial permission grants/account lifecycle, and large real-mailbox verification |
| Google Calendar and CalDAV | Background sync/create/edit/delete, conditional writes, native all-day editor | CalDAV discovery, Google read-only calendars, connection lifecycle; live server evidence |
| Safe calendar edits | Complete CalDAV resource preservation; stable Google create identity; source-scoped cache/editor; serialized per-calendar sync/write | Recurrence editing and invitations remain separate work |
| Reader, sender actions, attachments, image policy, quote display | Native reader/full-window reader, copy dialog, wrapping received attachments, image exceptions, collapsed/expanded/latest quote preferences | Separate-message conversation grouping and richer mail composition |
| Logo, WebP, appearance | Approved Swiss Shepherd assets and dark variant; cached WebP, Light/Dark/System | Continue visual review after layout changes |
| Testing, release, installer | Repo MCP skill, deterministic native scenarios, Rust/Python tests, hooks, semantic-release files, Linux installer | Final artifact/build verification after remaining changes; non-Linux execution/distribution |
| CI and performance gates | Dormant workflow definitions, strict backend/native budgets, Actions disabled by user request | Keep disabled until Sam asks; run measurements only at the end on an idle host |

## Remaining implementation audit

1. Cached queries, body loads and ordered saves have independent workers, verified with provider slots/queue occupied. Startup defers the Google keychain check. Preferences now use versioned acknowledgements; backup completion changes only metadata. Detail revisions reject stale bodies/errors after flag/move changes. Still review closing during a debounced/pending preference save and retrying a full persistence queue.
2. Complete account/calendar lifecycle and make connection errors recoverable without leaving stale sources or credentials. Respect Google calendar access roles and add CalDAV discovery/connection testing.
3. Complete common sending/reading workflows: outgoing attachments, CC/BCC, reply-all, server Sent handling, and grouping separate messages into conversations. Preserve drafts across failures.
4. Exercise POP3 and SMTP through deterministic protocol contracts. Google refresh now coalesces across callers, preserves/replaces refresh tokens according to the response, retries a failed keychain save before using the new grant, and serializes sign-in against refresh. Pending login grants can finish without another browser/code exchange. Revoked grants require reconnect; client-secret errors can be corrected. Tests cover these paths plus client binding, response bounds including chunked bodies, malformed/redirected responses, secret-safe diagnostics and fragmented/invalid loopback callbacks. Still finish partial permission grants and connection lifecycle; genuine Google authorization remains unverified. Drive has bounded pagination/JSON, ownership checks, pre-generated IDs, resumable chunks, checkpoints, checksums and restart recovery. Restore validates complete archives, reconstructs MIME display/search data off-thread, and commits new mail/configuration atomically before missing-password recovery. Existing mail/settings/passwords/drafts are preserved, and recovered mail absent from the server survives sync. Large imports still hold the cache connection during the transaction; finish independent cached reads during restore/export. Add lifecycle controls for abandoned uploads when removing a destination/account. Loopback contracts are not live-service evidence.
5. Review the current 25 MiB message and 256 MiB snapshot ceilings against real mailbox sizes. Large-mail/backup paths must remain bounded in memory while avoiding silent omissions.
6. Review scaled fonts, compact windows, empty/error states and keyboard/mouse parity. Finish with the full functional suite, final performance gates, release/install checks, and a push to `sam-ruff/shep.so`.

Live Google/CalDAV and Windows/macOS execution are not established by Linux fixture tests. Do not describe those checks as completed. Useful independent development remains, so the goal is not blocked on that verification yet.

## Calendar correctness evidence

`src/providers/calendar/*` contains loopback HTTP contract tests for Google pagination/conditional PATCH/DELETE, stable POST retry recovery, malformed committed responses, redirect rejection, and CalDAV REPORT/GET/PUT/DELETE with ETags. CalDAV edits retain alarms, attendees, timezones and extension properties. Conflict tests ensure another client's changes are not overwritten.

`tests/calendar.rs` covers all-day/default/DURATION semantics (including DST), escaped text, source-scoped IDs, atomic mixed-source rejection, and migration from the original cache keys. Engine tests cover source-scoped mutation and acknowledgment after a remote commit even when local caching fails. The native MCP suite edits and deletes one of two events with the same remote UID in different calendars.

Protocol references: [Google event IDs](https://developers.google.com/workspace/calendar/api/v3/reference/events/insert), [CalDAV resource ETags, RFC 4791 §5.3.4](https://www.rfc-editor.org/rfc/rfc4791.html#section-5.3.4), [iCalendar event duration, RFC 5545 §3.6.1](https://www.rfc-editor.org/rfc/rfc5545.html#section-3.6.1).
