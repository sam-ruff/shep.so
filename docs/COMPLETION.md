# Completion audit

The user requested a complete, polished Rust + iced mail/calendar client. Passing the current suite is evidence for specific behavior, not evidence that the whole goal is complete. This audit records outstanding work; it does not replace or narrow the original specification.

## Implemented, with local evidence

| Requirement | Implementation and evidence | Remaining verification |
| --- | --- | --- |
| Native mouse-friendly UI, remappable shortcuts | iced mail/calendar/preferences, native MCP keyboard/mouse flows, saved remapping | Broader accessibility and large-font layout review |
| Multiple saved IMAP/POP3 accounts, SMTP wizard | SQLite settings, OS secrets, local IMAP transcript tests; authorized Fastmail authentication and Inbox download | POP3 wire contracts; live sending and broader account lifecycle |
| Responsive inbox, preloading, resize, filters, fuzzy move | Independent bounded workers, background store, caches, virtual inbox; saturated-provider test; versioned preferences and message-detail results; native combined resize/settings flow | Pending save/shutdown and overload recovery review; final performance rerun |
| Compact inbox header | Single row for title/count/sync; native light/dark, idle/busy captures at 1440×920 and compact interaction at 900×640; updated production installation | Already-open windows retain the old executable until reopened |
| Optional Google login and Drive backups | Browser OAuth/PKCE with bounded loopback parsing; serialized refresh/sign-in and saved token rotation; encrypted rolling backups, destination-scoped setup/history/results, protected retention; durable upload journal and Drive resumable HTTP contracts; validated atomic restore and missing-password recovery | Genuine Google authorization, independent-process lifecycle coordination, and large real-mailbox verification |
| Google Calendar and CalDAV | Background sync/create/edit/delete, conditional writes, native all-day editor; CalDAV discovery and multi-calendar chooser; calendar access roles, read-only viewing and reviewed removal/reconnect | Live server evidence; independent-process coordination |
| Safe calendar edits | Complete CalDAV resource preservation; stable Google create identity; source-scoped cache/editor; serialized per-calendar sync/write | Recurrence editing and invitations remain separate work |
| Reader, sender actions, attachments, image policy, quote display | Native reader/full-window reader, copy dialog, wrapping received attachments, image exceptions, quote preferences; optional separate-message reader cards, bounded pages/preloading, account-scoped reference index | Broader real-mail conversation review |
| Outgoing recovery and Sent copies | Durable pre-SMTP MIME/envelope records, atomic local Sent/draft cleanup, explicit Outbox recovery, IMAP Sent discovery/APPEND and deduplication; Rust fault tests and native recovery flows | Live SMTP/Sent verification; broader authentication contracts |
| Logo, WebP, appearance | Approved Swiss Shepherd assets and dark variant; cached WebP, Light/Dark/System | Continue visual review after layout changes |
| Testing, release, installer | Repo MCP skill, deterministic native scenarios, Rust/Python tests, hooks, semantic-release files, Linux installer | Final artifact/build verification after remaining changes; non-Linux execution/distribution |
| CI and performance gates | Dormant quality/release workflows and strict backend/native budgets; documentation publishing has a separately recorded exception | Keep disabled until Sam asks; run measurements only at the end on an idle host |

## Active request tracking

[TODO.md](../TODO.md) contains every unfinished request, including subsequent corrections. [REQUEST_AUDIT.md](REQUEST_AUDIT.md) maps the full conversation to implemented evidence or active work. Add requests to TODO immediately; remove only after implementation, relevant verification and shipping, and keep the completed evidence here. This replaces the former mixed list of finished and unfinished requests.

## Remaining implementation audit

1. Cached queries, body loads and ordered saves have independent workers, verified with provider slots/queue occupied. Startup defers the Google keychain check. Preferences now use versioned acknowledgements; backup completion changes only metadata. Detail revisions reject stale bodies/errors after flag/move changes. Still review closing during a debounced/pending preference save and retrying a full persistence queue.
2. Complete account/calendar lifecycle and make connection errors recoverable without leaving stale sources or credentials. CalDAV discovery/connection testing and Google calendar access roles are now implemented; reviewed local removal and retryable credential cleanup now survive restart. Google can now disconnect on this device while retaining read-only calendar archives, with durable cleanup/retry and versioned status. Partial Google grants and staged transactional grant activation are now implemented within one engine. Remaining lifecycle work includes mail-account disconnection with an offline archive, source-specific credential editing and coordination across independent app processes.
3. Outgoing attachments, CC/BCC and Reply all are implemented with versioned draft persistence, file caching and native picker tests. SMTP loopback contracts verify recipient envelopes, hidden Bcc headers, binary MIME, rejection and lost acknowledgment diagnostics. Durable outgoing records now separate SMTP delivery from server Sent-copy recovery; Outbox provides explicit review/retry/local-copy actions without automatic resending. Related cached messages now have separate reader cards, linked by explicit message references within each account.
4. Exercise POP3 through deterministic protocol contracts; extend SMTP coverage to authentication. Server Sent discovery, exact identity checks, APPEND and lost acknowledgments now have deterministic protocol contracts. Google refresh now coalesces across callers, preserves/replaces refresh tokens according to the response, retries a failed keychain save before using the new grant, and serializes sign-in against refresh. Pending login grants can finish without another browser/code exchange. Revoked grants require reconnect; client-secret errors can be corrected. Tests cover these paths plus client binding, response bounds including chunked bodies, malformed/redirected responses, secret-safe diagnostics and fragmented/invalid loopback callbacks. Partial grants and staged switching now have failure/restart coverage; independent-process coordination and genuine Google authorization remain unverified. Drive has bounded pagination/JSON, ownership checks, pre-generated IDs, resumable chunks, checkpoints, checksums and restart recovery. Restore validates complete archives, reconstructs MIME display/search data off-thread, and commits new mail/configuration atomically before missing-password recovery. Existing mail/settings/passwords/drafts are preserved, and recovered mail absent from the server survives sync. Large imports still hold the cache connection during the transaction; finish independent cached reads during restore/export. Add lifecycle controls for abandoned uploads when removing a destination/account. Loopback contracts are not live-service evidence.
5. Review the current 25 MiB message and 256 MiB snapshot ceilings against real mailbox sizes. Large-mail/backup paths must remain bounded in memory while avoiding silent omissions.
6. Review scaled fonts, compact windows, empty/error states and keyboard/mouse parity. Finish with the full functional suite, final performance gates, release/install checks, and a push to `sam-ruff/shep.so`.

Live Google/CalDAV and Windows/macOS execution are not established by Linux fixture tests. Do not describe those checks as completed. Useful independent development remains, so the goal is not blocked on that verification yet.

## Calendar correctness evidence

`src/providers/calendar/*` contains loopback HTTP contract tests for Google pagination/conditional PATCH/DELETE, stable POST retry recovery, malformed committed responses, redirect rejection, and CalDAV REPORT/GET/PUT/DELETE with ETags. CalDAV edits retain alarms, attendees, timezones and extension properties. Conflict tests ensure another client's changes are not overwritten.

`tests/calendar.rs` covers all-day/default/DURATION semantics (including DST), escaped text, source-scoped IDs, atomic mixed-source rejection, and migration from the original cache keys. Engine tests cover source-scoped mutation and acknowledgment after a remote commit even when local caching fails. The native MCP suite edits and deletes one of two events with the same remote UID in different calendars.

Protocol references: [Google event IDs](https://developers.google.com/workspace/calendar/api/v3/reference/events/insert), [CalDAV resource ETags, RFC 4791 §5.3.4](https://www.rfc-editor.org/rfc/rfc4791.html#section-5.3.4), [iCalendar event duration, RFC 5545 §3.6.1](https://www.rfc-editor.org/rfc/rfc5545.html#section-3.6.1).


## Composer correctness evidence

`tests/composing.rs`, `providers/smtp_tests.rs` and UI ordering tests cover cached outgoing files, recipient validation/privacy, Reply-To/thread headers, migration and stale-save/sent-draft behavior. The native MCP suite uses the actual file chooser, saves and reopens attachments after source deletion, removes wrapped files, retains a draft after preview send refusal, and checks Reply all with both mouse and keyboard. Standard light/dark and 900×640 composer captures accompany these flows. The SMTP fixture exercises production delivery with a local transport; it does not establish live mail delivery. No new timing claims accompany this work.

## Conversation reader evidence

The reader groups downloaded messages across folders within one account using Message-ID, References and In-Reply-To. Subject matches alone do not merge mail. Duplicate mailbox copies display once, preserving the selected physical message for actions. The inbox continues to list individual messages. A General preference restores individual-message reading; quote display remains independently configurable.

Conversation metadata uses 20-message pages. Only the focused body and existing bounded neighbor cache load message contents; SQL ranking excludes raw MIME and body text. Older caches index in resumable 32-message transactions after the workspace becomes visible. New sync and restore writes update the index atomically. Store tests cover late parents, bridging references, account isolation, duplicate copies, deletion, flags, paging and reopening an unfinished backfill. UI tests reject stale results and keep replies targeted to the expanded message. The saturated-provider test also covers conversation reads.

Native MCP flows cover older Sent replies and attachments, mouse flagging, moving the expanded message, collapse/expand, the preference toggle, full-window reading, dark/compact layouts and long-thread paging during sync. Harness waits tolerate temporarily missing list entries while asynchronous pages refresh. These are functional checks; performance measurements remain deferred.

## Calendar connection and access evidence

`providers/calendar/discovery.rs` performs authenticated, bounded PROPFIND discovery through same-origin redirects, well-known/principal/home paths and direct collections. It handles unsupported optional properties, filters VEVENT collections, preserves collection URLs, and records separate create/update/delete privileges when supplied. Redirect loops, cross-origin targets, malformed/incomplete responses, authentication failures and response/request limits have loopback protocol tests. These are server-URL discovery tests; DNS SRV discovery and genuine homeserver verification are not established.

The native connection form finds calendars before saving, offers multiple selections and retains errors for retry. Discovery results carry form generations; credential edits/new dialogs invalidate old results. Calendar connection acknowledgments cannot close another editor. Reconnect deduplicates by URL/username and derives OS credential identifiers internally. Multi-source metadata saves use one atomic store write. Google list parsing is bounded and paginated, records access roles, and disables stale grants absent from a complete listing while keeping cached events. Read-only event views, writable defaults and provider/engine mutation guards enforce the advertised permissions. Legacy source records preserve their existing behavior until access is refreshed.

The saved MCP scenarios exercise selection/empty selection, connection failure and retry, repeated connection without duplicates, read-only viewing, and light/dark/900×640 layouts. Production never includes the fixture calendars or performs real service access in preview. Broader lifecycle work and live-service/non-Linux verification remain outstanding; no new timing claims accompany this feature.

Protocol references: [CalDAV discovery, RFC 6764 §6](https://www.rfc-editor.org/rfc/rfc6764.html#section-6), [calendar homes/components, RFC 4791](https://www.rfc-editor.org/rfc/rfc4791.html), [Google calendar list access roles](https://developers.google.com/workspace/calendar/api/v3/reference/calendarList).


## Connection removal evidence

Accounts and individual calendars have reviewed removal controls with exact local message/draft/event counts. A transactional fingerprint check rejects newly arrived mail, edited drafts or changed attachments even when counts are unchanged. Unfinished cross-account transfer journals need explicit cancellation. Account removal clears its cached MIME/search/conversation metadata, folders, drafts and attachment blobs; calendar removal is scoped by source even for shared remote event IDs. Server originals and existing backup copies are retained.

Local removal and credential cleanup jobs commit atomically. The OS deletion is retryable after failure/restart, treats missing secrets as success, and checks other current owners before removing a shared key. A lifecycle lock coordinates config/password writes, cleanup and restore; deterministic fake-keychain tests hold cleanup while reconnect waits. Tombstones stop delayed cache/draft writes. Calendar removal epochs outlive reconnection to reject old setup commands. Separate connection/calendar revisions stop delayed UI snapshots from restoring removed rows or events. Explicit backup restore can reconnect removed owners and cancel obsolete cleanup jobs.

Google calendar removal suppresses automatic re-import without deleting the shared Google credential. An explicit restore action reconnects calendars still accessible from the Google list. Native fixture flows cover reviewed removal/cancellation, draft counts, blocked confirmation for unfinished moves, remaining inbox navigation, calendar restoration and light/dark/compact layouts. These tests do not remove personal accounts or mutate live mail/calendars. Broader lifecycle and final performance verification remain outstanding.

This lifecycle increment passes formatting, Clippy with warnings denied, 138 Rust tests (two opt-in live tests ignored), 12 Python tests and all 34 native functional scenarios. Header light/dark/compact captures were reviewed again. Performance measurements remain deferred; this is progress toward the full goal, not a completion claim.

## Outgoing delivery and Sent-copy evidence

The outgoing journal commits immutable MIME, Message-ID and a separate private envelope before SMTP. A surviving submission cannot automatically resend after restart; an explicit review returns it to drafts or records it as sent. Known rejections retain an editable draft. The composer closes only after durable submission, leaving navigation available while provider work continues. Completion identifies its draft/revision and cannot close another editor.

SMTP acceptance is recorded before atomic local Sent insertion and draft/file cleanup. A failed acknowledgment write retains the earlier uncertain journal; failed local cleanup leaves an accepted record for restart repair. Copy recovery never calls SMTP. IMAP Sent discovery uses SPECIAL-USE and optional explicit folders, verifies exact Message-ID headers after SEARCH, and records Appending before upload. An unacknowledged copy requires checking or explicit review before another upload. Server-managed mode never APPENDs; local-only and POP3 modes never open an IMAP Sent connection.

Real Sent mappings drive the aggregate sidebar view. A matching synchronized server copy replaces the local Sent placeholder only within the same account/folder/identity. Recovery preserves flags and folder edits on existing local copies. Locally moved copies remain available; local flags/moves avoid server UID commands. Removal reviews and deletes outgoing records with account data. Recovery pages are bounded and versioned, and pending delivery drafts route to Outbox until reviewed.

Store/engine tests cover interrupted submissions across reopen, exact bytes and hidden recipients, stale attempts, transactional failures, lost SMTP/APPEND acknowledgments, all copy policies, local mutations, deduplication, folder aggregation and account removal. IMAP transcript tests drive discovery, exact header checks and APPEND acceptance/failure. Saved native scenarios exercise disabled actions, review, return-to-draft text preservation, mark-as-sent, retry errors, local completion, Sent navigation and SMTP copy settings. Light/dark/compact captures were reviewed.

This increment passes formatting, Clippy with warnings denied, 159 Rust tests (two opt-in live tests ignored), 12 Python tests and all 36 native functional scenarios. The compact inbox header remains verified in both themes and window sizes. Performance measurements are deferred; live SMTP/Sent and remaining full-product audit items are not claimed complete.

Protocol references: [SMTP acknowledgment ambiguity, RFC 5321 §4.5.3.2.6](https://www.rfc-editor.org/rfc/rfc5321.html#section-4.5.3.2.6), [IMAP special-use mailboxes, RFC 6154](https://www.rfc-editor.org/rfc/rfc6154.html).


## Google device disconnection evidence

Preferences now offers a reviewed Disconnect action for Google. It records a new connection revision and pending credential cleanup atomically, preserves calendar/event caches as read-only archives, and pauses automatic Drive backups. Destination identity, backup history and staged encrypted uploads remain available for explicit recovery after reconnect. Mail accounts, CalDAV and local-folder backups are preserved. Cleanup clears in-memory grants before trying OS deletion; failure remains retryable after restart. Reconnect finishes pending cleanup before installing a new login. No remote grant-revocation request is made.

Within the engine, Google provider requests use a shared read guard and connection changes take its write guard. Disconnect waits for active requests before committing, and provider mutation guards reject archived events. Status, preferences and archive snapshots reject older revisions. Stale reviews cannot disconnect a newer login. A fresh complete Google calendar list reactivates only the returned sources. Independent-process coordination is still outstanding; this increment establishes the lifecycle within one engine.

Store tests cover transactional failure, restart, cached data/backup identity retention, stale preference saves, cleanup/reconnect ordering and archive reactivation. Provider fake-keychain tests cover deletion failure, retry and fresh sign-in. Engine tests hold an active Google operation while disconnect waits and reject writes to archived events. UI ordering tests reject stale status/workspace updates and preserve unrelated editors. The saved native flows exercise Cancel, Escape, confirmation, offline event viewing and retained-mail navigation in light/dark/900×640 layouts. No personal Google connection was changed.

This increment passes formatting, Clippy with warnings denied, 168 Rust tests (two opt-in live tests ignored), 12 Python tests and all 38 native functional scenarios. Performance remains deferred. The broader completion audit, genuine Google authorization, partial permission grants, atomic grant switching and non-Linux execution remain open.

Design reference: [Google desktop OAuth: granted scopes and token revocation](https://developers.google.com/identity/protocols/oauth2/native-app). Google documents project-wide effects for revocation; the UI explicitly offers disconnection on this device.


## Partial Google permissions and staged switching

Calendar-only, Drive-only and read-only Calendar grants now work independently. Service calls check the actual token scopes, including after refresh. Calendar roles further limit editing. The UI displays granted permissions and retains cached events as read-only archives when Calendar permission is absent. A missing Drive grant pauses automatic Drive backup while keeping prior destination metadata and existing copies.

The credential store stages the new grant beside the committed grant in a bounded vault. Validation uses the candidate; one database transaction then commits its pointer/client/access, verified Drive identity and refreshed calendar permissions. Failed validation or transaction rollback preserves the working login. Restart selects the database's committed grant, and an explicit candidate marker distinguishes retryable sign-in from a leftover old credential. Pruning failure does not undo activation. A new sign-in action requests account selection; edited OAuth setup fields leave the committed client usable until activation succeeds.

Provider tests exercise real local HTTP requests with fake credentials for partial scopes, denied services, read-only roles, refreshed scope changes, keychain failure, pending retry and restart. Combined store/provider tests simulate API failure, database rollback and exit before credential pruning. Store/UI tests cover archived data, stale settings, active-client preservation and late snapshots. Saved native scenarios review permission controls and read-only events in light/dark/compact layouts and continue browsing mail. No personal Google account was used.

Independent-process coordination, live authorization, non-Linux execution and the other completion-audit items remain open. Performance measurements are still deferred until the end on an idle host.

This increment passes formatting, Clippy with warnings denied, 178 Rust tests (two opt-in live tests ignored), 13 Python tests and all 40 native functional scenarios. Linux release packaging and installation are checked separately; live Google and non-Linux execution remain unverified.


## Sidebar, contacts and inbox context evidence

The sidebar now measures/truncates long labels without horizontal overflow, reserves scrollbar space, persists collapsed account headings and supports Ctrl+click folder unions without duplicates or a misleading all-mail result for an empty selection. Account setup remains in Preferences. A native draggable edge saves sidebar width; window dimensions restore after cached startup. A narrower window clamps the visible sidebar without discarding the saved wider choice. Close flushes pending drag state and waits for the latest preferences acknowledgment. A coalesced retry handles a full persistence queue; a failed save keeps the window and unsaved edits open.

Inbox context actions retain the right-clicked identity, support mouse plus Shift+F10/arrow/Enter/Escape, and wait for the correct body before reply/move/export. Navigation cancels delayed actions. Flags use a red outline; supported button tooltips show remapped keys. Contacts has a dedicated Preferences tab. Explicit saves show a dismissible acknowledgment-based toast. Calendar uses a refresh icon and frees header/footer space for its grid.

Validation: formatting, Clippy with warnings denied, 184 Rust tests (two opt-in live diagnostics ignored), 15 Python tests and all 45 native functional flows pass. SQLite reopen verifies layout persistence; UI unit tests cover close during debounce, failed saves, full-queue coalescing, stale acknowledgments and late context-body results. Native light/dark/900×640 evidence was visually reviewed. No performance measurements were run. This completes the listed increment only; folder trees/mutations, inline multi-draft composition, palette editing, encrypted large-mail streaming and multiple backup destinations remain active requirements above.


## Reader selection, shortcuts and settings interaction evidence

This increment adds read-only native text selection/copy in preview, quoted history and the full reader, with off-thread buffer preparation and stale-result guards. Shortcut settings now have primary and optional secondary slots. Ctrl+D moves to Trash; Backspace and Delete archive. Sidebar-only I can be disabled/remapped. Legacy custom bindings survive migration. Captured text editing does not mutate mail.

Mail returns to the configured Inbox when clicked from another mail folder; sidebar unread counts remain independent of search/filter scope. The Move chooser labels INBOX as Inbox and highlights its Enter target. A wire test covers moving from a spaced custom folder to INBOX and preserves the MOVE acknowledgment if logout disconnects. The user's personal A. Keep report still needs confirmation; no personal messages were moved by automation.

Background mail updates preserve the right-click menu and refresh its owned target metadata; native tests wait through a simulated sync, select an action and check outside/Escape dismissal. Preferences search opens matching editable sections. Labeled controls have no tooltips; icon hints use only the primary key and have independent tooltip/key-hint toggles. Mail sync is a refresh icon.

Validation: formatting, Clippy with warnings denied, 191 Rust tests (two opt-in live diagnostics ignored), 16 Python tests and all 52 native functional flows. The last full invocation passed 51; the remaining conversation-paging flow passed separately after its click coordinate was updated for the refresh icon. Light/dark/compact screenshots were reviewed. Release checksum/extraction/bundled-installer verification passes. No performance measurements were run. Exact shipped commit and installation are recorded after publication below.

The complete conversation is mapped in REQUEST_AUDIT.md. TODO.md is the active checklist; the AGENTS rule requires immediate updates for new requests and retains incomplete work through compaction. Faithful HTML, Ctrl+F/relevance search, optimistic flags, bulk/drag actions and the larger storage/sync/draft/backend features are not claimed complete by these tests.
