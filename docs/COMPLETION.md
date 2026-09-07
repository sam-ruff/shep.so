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

[TODO.md](https://github.com/sam-ruff/shep.so/blob/main/TODO.md) contains every unfinished request, including subsequent corrections. [REQUEST_AUDIT.md](REQUEST_AUDIT.md) maps the full conversation to implemented evidence or active work. Add requests to TODO immediately; remove only after implementation, relevant verification and shipping, and keep the completed evidence here. This replaces the former mixed list of finished and unfinished requests.

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

Shipped as `590ab1091f45a5bc4d02ac665684cd68b3aa1e9d` (`feat: refine mail interactions and track all requests`) on main. The optimized production binary is installed for the Linux user; its SHA-256 matches the release artifact: `6393dc749d61495ceb69363ef230ee1793c289564bdbcd0317adabf65427fe33`. Existing windows must be reopened to load it; no personal window was terminated. The corresponding completed TODO entries have been removed while this evidence and the audit remain. The larger full-product goal remains active.

## Documentation CI repair and interaction requirements

The failed Documentation run [34024807937](https://github.com/sam-ruff/shep.so/actions/runs/34024807937) rejected two links to root TODO.md outside the published docs tree. Both now point to the repository file. The pinned `zensical build --clean --strict` passes locally; strict validation remains enabled. Contributing instructions and AGENTS.md require the same strict build before docs pushes. Commit `eea1dfb` is pushed; [Documentation run 34026001754](https://github.com/sam-ruff/shep.so/actions/runs/34026001754) passed both build and deployment. Quality and release definitions remain disabled.

AGENTS.md now records immediate optimistic feedback as an app-wide requirement, including archive removal and failure rollback. R50/R60 and R62 track implementation and read/unread integration coverage; R61 tracks the requested three-platform download installers and prominent installation commands. These requests are not claimed implemented by this documentation change.

## Optimistic mail actions, read/unread and shortcut clearing

Flags and read/unread now update list, reader and conversation metadata immediately. Changes coalesce per field and message; an older acknowledgment or rejection cannot overwrite newer input. Backend requests have typed IDs. Full queues restore the previous UI, and background pages retain pending overlays. Read changes preserve Flagged and flag changes preserve Seen. Body/attachment buffers remain unchanged on the UI thread.

Same-account Archive, Trash and Move hide the source row immediately and allow reading other messages while they finish. Moves wait behind that message's pending flag changes. Rejection restores the source row without changing the folder the user is browsing; acknowledged server moves stay committed when later cache/refresh work fails. Closing waits for accepted mail changes; failures keep the window open. Durable pending-action recovery across forced termination, cross-account optimistic feedback and projection into newly selected filters remain in TODO.

The IMAP regression tests exposed a library behavior: async-imap 0.11.3's streamed UID STORE/EXPUNGE helpers discard the tagged response status. Production flag changes and transfer source cleanup now use `run_command_and_check_ok`. Loopback transcripts cover read/unread and flag/unflag, rejected STORE/EXPUNGE and disconnect after a successful acknowledgment. These tests do not claim a personal Fastmail mutation was performed.

Each shortcut slot now has its own clear ×. Primary and secondary can both be disabled, clearing cancels capture, errors remain visible, and empty bindings survive reload. Tests clear every slot and restore a binding; real native flows clear/remap both Move slots and inspect compact dark layout. Move Enter now resolves current form text at handling time, fixing a fast-typing race where the prior frame's destination could be used.

Validation: formatting and Clippy pass; 203 Rust tests pass (two live diagnostics remain ignored); 17 Python tests pass. All 58 native functional flows are verified: the full run passed 57 and exposed one incorrect assertion that right-click leaves the original message selected; the corrected saved context-action test passes separately. Evidence is under `artifacts/logs/mail-actions-*`. The strict documentation build passes. Commit `742b21e` is pushed and installed. The optimized production binary matches the installed executable at SHA-256 `8d892db4578f7954ab5e1674be49fc36c122eb9b928dc80ec4115e12696e97e8`; release checksum, extraction and bundled installer checks pass. [Documentation run 34027695701](https://github.com/sam-ruff/shep.so/actions/runs/34027695701) passes. Existing user windows were left open and need reopening for the update. WebP review covers immediate unread counts/open-envelope state, archive rollback, correct-row context actions and separate clear controls in light/dark/compact windows. Performance measurements remain deferred. R61/R64/R65 retain download installers, themed launcher and background-sync scheduling. AGENTS.md records the Dungeonwalk asset-tool credential discovery pointer without copying or exposing credentials.


## Monorepo clients and restricted beta — uncommitted worktree checkpoint

Work remains on `feat/mobile-web-clients` in `shep-clients`, based on `8af26f6`. The delegated promo agent built `website/` on `feat/promo-website` in `shep-website`; its reviewed source is copied into the combined worktree. **No client commit, main merge/push or VPS deployment has occurred.** Desktop background scheduling shipped independently upstream as `d4ecb21` / `2965ae2`; preserve it when integrating this older worktree.

Flutter includes mail/reader/calendar/preferences/composer previews, configurable K-9-style swipes with matching icons and visible alternatives, optimistic change/undo/rollback, persisted nonsecret preferences and separate Android preview/production packages. Its production provider bindings are unfinished. The separate browser now connects to authenticated Rust mail endpoints: account verification/reconnect, streamed sync, IMAP flags/MOVE, local POP3 actions, per-Google-subject/client IndexedDB mail/raw/draft storage and SMTP delivery records. Passwords stay in tab memory. Visible tabs check mail every 15 seconds while connected, with independently queued manual Refresh. The browser layout retains desktop-style panes, saved dividers, configurable input-safe shortcuts and Light/Dark/System appearance.

`shared/mail-core` now owns the actual common mail/MIME/reply and IMAP/POP3/SMTP implementations, with desktop compatibility exports. The VPS gateway pins administrator-approved endpoints and verifies TLS against their original hostnames, bounds concurrent operations and streams sync events. Google login protects all app/assets/API routes through verified allowlists, one-use OAuth state/PKCE, in-memory expiring sessions, CSRF/origin validation and no-store responses. Server mail/password persistence is absent; empty deployment allowlists/endpoints fail closed. SMTP requires a server-issued reservation durably saved with the browser draft before POST. Lost/uncertain/expired receipts never create automatic repeat sends. Accepted sends outlive HTTP disconnects, including supervised provider panics.

Verified evidence under ignored `artifacts/logs/` and client screenshot directories:

- All **203 existing Rust workspace tests** pass after extraction; two additional pinned TLS tests pass in the **13-test shared-core suite**, covering IMAP/POP3/SMTP, implicit TLS/STARTTLS and hostname rejection. Root formatting/Clippy pass. All **58 desktop native functional flows** pass in one run; synthetic native screenshots were reviewed. Performance measurements remain deferred on the busy host.
- **21 backend tests** pass plus the explicitly selected real HTTPS browser scenario. The optimized production backend builds; its test-fixture exclusion and refusal to start without deployment configuration are verified. That browser run exercises **17 access/provider/recovery stages**, production assets and actual controls/IndexedDB, including wrong-password retry, cache/draft reload and lost/uncertain delivery status. Google identity and mail transport are test-only fixtures; separate shared-core tests exercise real loopback wire/TLS behavior.
- **18 browser model/provider tests** and **14 Playwright scenarios** pass, including theme/compact axe scans. Account setup is additionally checked at 900×640 and 1440×920 through the Rust-backed browser flow.
- **18 Flutter unit/widget tests**, **three Android integration scenarios**, **five Appium stages** and **five Flutter Playwright stages** pass. The latest Android runner completed integration → preview rebuild → Appium sequentially. Native/browser swipe-icon evidence was reviewed. Production Android debug APK build and fixture-exclusion inspection pass; Apple execution is unverified.
- The promo site passes **61 Chromium/Firefox/WebKit tests**, with **two engine-specific clipboard skips** and **18 layout/accessibility scans**. Browser-aware install suggestions, approved assets and synthetic screenshots were reviewed. Store/private-beta links report actual unpublished availability.
- **21 Python tests** pass, including protected/public staging separation, fail-closed release prerequisites and synchronized client/lockfile version stamping. Pinned strict Zensical passes. Quality/release workflows remain `.yml.disabled`; documentation CI is unchanged.

This is partial implementation, not feature parity or a shipped beta. Remaining work includes Flutter native bindings/cache/credentials, browser worker-based bounded cache reads and encryption, remembered credentials, IMAP move/undo recovery, autosave/reply/attachment/Outbox/Sent behavior, calendar/backup/Google-provider parity, Apple simulator/signing, platform launcher assets, store submissions and complete coordinated release packaging. Exact permitted Google identity, OAuth configuration and VPS SSH target are still pending, so no live service was installed or tested. R67–R74 stay active. R75 tracks OAuth “Sign in with Google” in place of manual tokens; R76 tracks scheduled Automatic replies with account-group assignment. See CLIENT_PARITY.md and TODO.md for the full remaining scope.


## Native Flutter continuation (review worktree, uncommitted)

The production Flutter entry now opens a Rust profile rather than an unconnected repository. `flutter/rust` shares mail/MIME providers with the desktop and gateway, with SQLite WAL, two independent cache-read connections and bounded FIFO writes. An owned request retains its account lock/capacity through completion if its Dart waiter disappears. Password pairs use one ordered secure-storage write; no password field exists in the Rust cache schema. Account setup probes incoming/SMTP separately and displays persistent connection errors. Foreground checks run every 15 seconds; manual refresh can queue independently.

Flutter now loads metadata pages from SQLite and fetches bodies on demand, keeps metadata actions optimistic, and offers native account selection, draft autosave, restart/reopen and reviewed permanent discard. SQLite revisions/tombstones reject stale draft writes. SMTP MIME/reservation is persisted before transmission; a surviving submission cannot automatically resend. Full accepted-send/Sent-copy repair and uncertain/rejected review are still open, so this does not establish desktop Outbox parity.

Eight Rust tests cover paging/search/detail, POP3 local state, draft revisions/file cleanup, account identity, future-schema rejection, surviving submissions, independent reads and cancellation/FIFO ownership. Nineteen Flutter host tests pass, including a real FFI/SQLite reopen/discard test. Four additional Android scenarios pass through the real bridge/device storage: save/reopen/edit/discard controls, failed loopback account setup with untouched credentials, isolated secure credential-pair roundtrips, and the actual production entry opening its native cache/account setup without fixture mail. The existing three preview integration scenarios also pass. Native draft/error screenshots are reviewed; the shared runner preserves their host captures through `flutter drive` before rebuilding for Appium. Five Appium stages and all five Flutter browser Playwright stages pass. The production debug APK builds for arm64-v8a, armeabi-v7a and x86_64; the new artifact verifier confirms the packaged Rust libraries, Internet permission and absence of fixture markers. All 23 Python tests and pinned strict Zensical pass. This is a debug build, without distribution signing.

The native-assets setup pins Rust/bridge versions, includes manifest/lockfile changes in build dependencies, honors Android minSdk 24, uses the host Perl for OpenSSL when Flutter's Snap environment mixes Perl versions, and registers generated JNI libraries through AGP 9's Variant API. Android's production manifest now grants Internet access. Disabled CI and coordinated version stamping include the native Rust crate. No quality/release workflow was enabled.

This continuation remains uncommitted in `shep-clients`; no main merge/push or live/VPS action occurred. Remaining native work includes bounded body/attachment parsing and prefetch, complete outgoing recovery, account edit/remove, replies/attachments, Google/calendar/backups, cache encryption, new desktop read-on-leave/toast defaults, Apple execution and release/store packaging. Server identity/OAuth/SSH configuration is still pending. R67–R76 remain active; R75's OAuth replacement and R76's grouped, scheduled Automatic replies are recorded requirements, not implemented features.


## Move receipts and client Undo (review worktree, uncommitted)

The shared core now preserves acknowledged IMAP destination identities from COPYUID/APPENDUID and checks exact MIME fingerprints before recovering moved copies. Tagged success remains committed when logout fails; conflicting or absent mappings require recovery, and duplicate/changed copies are refused. The hosted gateway applies the same endpoint, identity and authentication restrictions to recovery. These contracts follow [IMAP MOVE](https://www.rfc-editor.org/rfc/rfc6851.html) and [UIDPLUS](https://www.rfc-editor.org/rfc/rfc4315.html).

Flutter SQLite and browser IndexedDB keep a stable local message identifier as its server folder/UID changes. Sync updates that record and retains its body/raw mail; destination duplicates are merged only when their original bytes agree. Move intent is saved before transmission. A lost response or failed acknowledgment save cannot automatically issue the operation again after reopening; complete reconciliation can re-establish an unchanged source for a subsequent explicit action. Missing/conflicting mappings use exact-content recovery, with persistent errors when one copy cannot be established. Further recovery-review UX remains active.

Flutter also retains the small metadata record needed by Undo after a paged folder refresh. Undo updates the display immediately while the original move waits, then runs in order. The production browser's saved HTTPS scenario holds an actual gateway response after commit, clicks Undo, and checks that the next request uses the destination identity; it also undoes after refresh. Android drives the same paged/queued cases through swipe and button controls. Its transport barrier is a test-only provider; SQLite recovery and real IMAP wire contracts have separate tests. This is not live IMAP/device-provider verification.

Validation: 209 Rust workspace tests pass, including 17 shared-core protocol tests; two live diagnostics remain ignored. The standalone backend has 22 passing tests, plus 19 stages in the explicitly run Rust-backed HTTPS browser scenario. The native Rust crate has 12 passing cache/FIFO/recovery tests. Flutter has 20 passing host tests, three preview Android scenarios and five further Android scenarios; the combined integration → rebuild → five-stage Appium run passes. Browser model/provider tests pass 20 cases and Playwright passes 14 scenarios; all five Flutter browser Playwright stages also pass. Formatting, Clippy and Flutter analysis pass. Reviewed WebP evidence includes `artifacts/flutter/native/paged-swipe-undo.webp` and the browser's pending/post-refresh Undo captures; logs use `artifacts/logs/move-*` and the saved Android runner logs. All 23 Python tests and the pinned strict Zensical build pass. The current production Android debug APK includes Rust libraries for all three ABIs, Internet permission and no preview fixture markers; its SHA-256 is `cc8d60450b4c4ba41a4408a1dfbf9bdf9fbad299bf3ed8905fd3a15f9a152ade`. Distribution signing is still open. Performance remains deferred.

This checkpoint is uncommitted in `feat/mobile-web-clients`. It does not close R67–R76 or establish full feature parity. Ambiguous/unrecoverable move review, cross-account moves, the newer desktop defaults, full outgoing/attachments/replies, calendars/Google/backups, encrypted/bounded caches, Apple execution and release/store distribution remain active. No main merge/push, personal-provider test or VPS deployment occurred; the exact VPS/owner/OAuth configuration is still pending. Quality/release CI remains disabled.


## Client replies and outgoing attachments — worktree checkpoint

Flutter and the separate browser now prepare Reply/Reply all from cached mail headers, preserving Reply-To, To/Cc deduplication, all configured sender exclusions, In-Reply-To/References and quoted text. Shared synthetic JSON fixtures run in both Rust and TypeScript, including deterministic September date formatting. Browser replies work before reconnecting after reload. This does not implement Google provider OAuth or scheduled Automatic replies; R75/R76 remain active TODOs.

Outgoing attachment bytes have separate ownership from draft text. Native SQLite stores independent file revisions; browser IndexedDB stores file blobs separately. Text autosaves cannot restore removed associations. Native file edits flush pending text; failed imports remain atomic, removed files stay removed across reopen, and saved binary files survive deletion of their original source. Sending checks the displayed file/text versions, prepares the shared MIME, and preserves immutable submitted content. Submitted/discarded native drafts and browser delivery records reject file edits. Full Outbox/Sent-copy recovery and incoming attachment downloads remain open.

The saved Android scenario uses `file_selector` and actual DocumentsUI controls: cancel, select two files, cached Reply all, save/reopen, remove/reopen, pending text persistence and send refusal without credentials. Its helper hands over a generated fixture database before the UI opens it; subsequent actions use real controls. The browser scenario uses real IndexedDB, the file chooser and the authenticated Rust HTTPS service, verifies exact binary MIME/reply headers/Bcc envelope, and recovers a lost send response without resending. Compact browser composition now keeps Save/Send visible while fields scroll. Reviewed WebP captures include `artifacts/flutter/native/native-reply-attachments.webp`, the picker cancellation/multiple-selection captures, and `artifacts/beta-browser/reply-attachments-900.webp` / `reply-attachments-1440.webp`.

Validation: 210 Rust workspace tests pass (18 shared-core tests included; two opt-in live diagnostics ignored), 15 native Rust tests, 22 backend tests plus 23 stages in the explicitly run Rust HTTPS browser scenario, 28 browser data/reply tests and 14 browser Playwright scenarios. Flutter analysis and 20 host tests pass. The full sequential Android runner passes three preview scenarios, five further native/paged scenarios, the additional reply/attachment scenario, then all five Appium stages. All five Flutter browser stages, 23 Python tests and the pinned strict Zensical build pass. Rust formatting/Clippy and Dart formatting pass. The owned emulator and test servers were stopped after verification. Logs are under `artifacts/logs/compose-*` and the saved Android/Flutter runner logs. Performance measurements remain deferred.

The production Android debug APK includes Rust libraries for all three ABIs, Internet permission and no checked fixture markers; SHA-256 is `1f052c83f61dfe3d8597b2f04060d5c03d2f31353096a56871733727703a0bd5`, with the report at `artifacts/android-compose-production-isolation.json`. This is not a signed distribution release. Apple execution/file selection, live providers, Google/calendar/backup parity, encrypted/bounded caches, newer desktop defaults and complete composition/move/lifecycle recovery remain active. No main merge/push, personal-provider test or VPS deployment occurred. Source remains uncommitted in `feat/mobile-web-clients`; this checkpoint closes no full client request. Quality/release workflows remain disabled, and deployment still needs the exact VPS/owner/OAuth configuration.


## Browser Outbox review and exact Sent copies — worktree checkpoint

The shared core now exports the desktop outgoing types and an exact prepared MIME/envelope contract. Browser delivery reserves an identity, saves the immutable draft, obtains prepared bytes from Rust, commits those bytes locally, then submits them. The gateway retains only a digest of the prepared content and non-secret account configuration; changed bytes/settings cannot use that reservation. Atomic cancellation prevents a still-unused reservation from later starting SMTP and cannot release a running operation. No persistent mail/password store was added to the VPS service.

Browser Outbox supports status checks, cancelled/rejected return to Drafts, explicit review before uncertain return or manual mark, and keeping the original local Sent copy. Recovery clones editable text and attachment ownership to a new draft; old editor saves cannot resurrect the submitted original. A new Send remains a separate user action. Manual mark retains uncertainty in the original delivery record. Confirmed delivery/rejection stays authoritative after the backend receipt expires or restarts. Local Sent uses the exact submitted MIME and survives reload, server reconciliation and newer local flags/folder choices. Adding Sent cannot replace a pending action or be erased by a stale refresh. Provider Sent lookup/append recovery and bounded Outbox paging remain open.

Validation: 210 root workspace tests, 15 native Rust tests, 25 backend tests, 39 browser model/provider/reply tests, 14 browser Playwright scenarios and all 31 stages of the explicitly run Rust HTTPS browser scenario pass. Rust formatting and Clippy with warnings denied pass for the workspace, native crate and backend; the browser production build passes. All 23 Python tests, the parity checker and the pinned strict Zensical build pass. The saved HTTPS flow observes IndexedDB before transmission, uses real Outbox/composer controls and asserts exactly four explicit SMTP attempts across accepted, uncertain and rejected cases; status checks, manual review and cancelled preparation never send. Light/dark Outbox captures at 1440×920 and 900×640 pass axe; the complete recovery card remains inside its scroll container. Reviewed WebP evidence includes `artifacts/beta-browser/outbox-review-light-1440.webp`, `outbox-review-dark-900.webp` and `outbox-reviewed-local-copy.webp`. Logs use `artifacts/logs/outbox-*`.

This continuation changes the separate browser UI and shared/backend contracts; no new Android or Apple execution is claimed. Native Outbox controls, provider Sent recovery, incoming attachments, calendars/Google/backups, encrypted/bounded caches, account lifecycle and full client parity remain active. Upstream selectable HTML in 1968e37 still needs integration and client equivalents. R75 Sign in with Google and R76 grouped, scheduled Automatic replies remain recorded TODOs. Performance remains deferred. No main merge/push, personal-provider test, deployment or release occurred; source remains uncommitted in `feat/mobile-web-clients`, quality/release workflows remain disabled, and VPS/owner/OAuth configuration is still pending.

## Native Outbox review and exclusive profile ownership — uncommitted checkpoint

Flutter now offers a paged Outbox from navigation and submitted drafts. It reads 20 metadata rows at a time, keeps Back usable while recovery runs, and shows storage failures with retry. Rejected submissions can return to a new draft; uncertain deliveries require explicit review before return or manual marking. Return copies the original text and attachment blobs to new identities in one transaction, retains the outgoing record, and prevents stale editors from saving or sending the original again. Manual marking keeps the uncertain SMTP status; it records the user's decision without inventing a provider acknowledgment. Local Sent preserves the exact submitted MIME and newer local flags/folder changes. Provider Sent lookup/append recovery remains open.

The native sender owns its account lock and admission permits through completion, including a cancelled caller or provider panic. It commits immutable MIME before transmission and the terminal SMTP result before local Sent work. A later cache failure cannot turn a known acceptance into a fresh send; an unsaved terminal result stays in shared memory for an explicit persistence retry. Canonical profile handles share their database and operation coordination. A persistent companion lock file enforces exclusive process ownership; blocking cache jobs retain the lease through cancellation. Only an exclusive replacement owner reclassifies surviving submissions as uncertain.

Real Android testing exposed an unsupported standard-library file-lock implementation on the pinned Android target. The native crate now uses Bionic `flock` there and retains the standard API on other platforms. The saved Android helper checks the actual held lock from a second process, with a separate successful lock as a control. Synthetic Outbox state is handed over only before opening an isolated preview profile, including Dart's private `code_cache` location. Subsequent recovery uses actual Flutter controls. Test-only SMTP transports and fixtures are excluded from production.

Validation for this continuation:

- **23 native Rust tests** pass, including active-send ownership, process handover, terminal-write/Sent-write failures, exact bytes before SMTP, cancelled waiters, panics, recovery rollback, attachment ownership, stale-draft refusal and 45-record paging. Native formatting and Clippy pass. **22 Flutter host tests** and analysis pass, including recovery errors, navigation while pending and pagination/review reset.
- The complete sequential Android runner passes **ten integration scenarios**: three preview, five native bridge, one real DocumentsUI picker and one Outbox recovery scenario. All **five Appium stages** then pass. Outbox controls cover both themes, review gating, returned files/Bcc/text, file removal, manual mark, local Sent, reopen and refusal to send without credentials. This is synthetic device/cache evidence, not live SMTP delivery.
- All **five Flutter Playwright stages** pass after the button-theme change. Buttons now use restrained borders, 44-pixel minimum targets and rounded corners. Reviewed WebP captures under `artifacts/flutter/native/` include `native-outbox-review-light`, `native-outbox-review-dark`, `native-outbox-recovered-draft`, `native-outbox-empty` and `native-outbox-local-sent`.
- The production debug APK builds for arm64-v8a, armeabi-v7a and x86_64. Its Rust libraries, Internet permission and fixture exclusion pass inspection; SHA-256 is `ad9f072c91bf5ddf579c6faf255fe70f405f8a879b7260f90a221e46261782c0`. This is not a signed distribution build. **23 Python tests**, the 17-contract parity review check and pinned strict Zensical build pass. Logs use `artifacts/logs/native-outbox-*` and the sequential runner's `android-*` files.

The root Rust, gateway and separate-browser implementation did not change in this continuation; their previous checkpoint remains the relevant evidence. Apple runtime/file-picker/locking execution is unverified, performance measurements remain deferred, and provider Sent recovery, full composition/incoming attachments, account lifecycle, Google/calendar/backups, encrypted/bounded caches and complete parity stay active. R75 OAuth and R76 grouped, scheduled Automatic replies remain recorded TODOs. Desktop main advanced independently to 6fed03b; committed HTML/focus changes still require integration, while uncommitted HTML/find work remains untouched. **No client commit, main merge/push, deployment or release occurred.** Quality/release definitions remain disabled; the authorized VPS deployment still awaits the exact server, owner identity and OAuth configuration.


## Native provider Sent recovery — uncommitted checkpoint

Native Flutter accounts now expose Sent-copy policy and an optional server folder at setup and under Preferences → Sent copies. The default for a new IMAP account matches the desktop's Automatic policy; existing settings are preserved and POP3 stays local. Saving Sent preferences updates only those fields, and a reconnect cannot overwrite a newer Sent preference with its earlier form snapshot.

SMTP acknowledgment and copying to Sent are separate operations. After confirmed delivery and local-cache persistence, the composer can close while an owned background task checks/copies Sent under the same account lock and capacity permits. The native journal snapshots the nonsecret account connection, stores the exact MIME, and commits the destination before APPEND. Unacknowledged copies remain uncertain across reopen and need a separate reviewed upload action. Checking Sent alone never uploads. Server-managed policy only looks up the copy; LocalOnly/POP3 require no upload. A changed incoming connection cannot repurpose an older submission.

An acknowledged copy survives later journal or local-cache failure. Its known receipt remains available in memory for persistence retry; a persisted receipt repairs the cache without credentials or another provider request. A matching provider copy resolves the local recovery flow while retaining the original uncertain SMTP record. It also prevents stale Return/Mark decisions from creating a new draft. Manual marking retains its review history and can subsequently keep a local copy. Synced provider copies remove only untouched local copies; committed local flag/move choices are preserved. Stable UI identity and pending actions during that local/server handover, offline actions on IMAP local copies, Sent-folder grouping and history cleanup remain active work.

The shared `SentConnection` contract is used by desktop and native code, with a pinned gateway adapter ready for the browser endpoints. Discovery and exact Message-ID lookup now inspect final tagged LIST/SEARCH/FETCH responses instead of accepting streamed results that discarded a final rejection. Tests reject NO, incomplete results, zero/missing/duplicate identities and conflicting headers. Real loopback TLS transcripts cover Sent discovery, lookup, binary APPEND, implicit TLS/STARTTLS and hostname refusal before authentication. These changes follow [IMAP completion and APPEND](https://www.rfc-editor.org/rfc/rfc9051.html#section-6.3.12) and [special-use discovery](https://www.rfc-editor.org/rfc/rfc6154.html); they do not establish live provider behavior.

Validation: **212 root workspace tests**, **32 native Rust tests**, **25 backend tests**, formatting and Clippy with warnings denied pass. Flutter analysis and **23 host tests** pass. The complete sequential Android runner passes all **ten integration scenarios** and **five Appium stages**; all **five Flutter browser stages** pass. The expanded native Outbox scenario repairs a saved provider receipt, checks missing-credential failures and separate copy review, persists Sent preferences and reopens the profile. It uses synthetic records handed over before startup and real controls afterward, including touch scrolling to reach lazy rows. The actual Android process-lock check also passes. No personal account or real email is used.

Reviewed WebP captures include `artifacts/flutter/native/native-sent-copy-review.webp` and `native-sent-preferences.webp`, alongside the existing light/dark Outbox captures. Duplicate error text and a redundant delivery-status control were removed from confirmed-copy review. The production debug APK contains the Rust library for all three Android ABIs and excludes the new fixture markers; its SHA-256 is `3d53c0b7e540c47336b97b572478c55afeb35aad34ae088e39252466f8eff900`. This is not a signed distribution artifact. **23 Python tests**, the **18-contract parity review check** and pinned strict Zensical build pass. Logs use `artifacts/logs/sent-*` and the Android runner's `android-*` files.

This remains uncommitted in `feat/mobile-web-clients`. No main merge/push, deployment, release or workflow enablement occurred. Native Sent handover/offline-local behavior and complete client parity remain open; browser Sent endpoints, journal and controls are the next provider parity task. The previous separate-browser control evidence remains applicable to its unchanged implementation; no new browser-provider success is claimed here. Apple execution and live provider/VPS verification are still unproven, and performance gates remain deferred while the host is busy. Exact VPS, owner identity and OAuth configuration are still pending. Desktop main independently advanced to `c266035` / `f5f13c9` with find-in-message; those committed changes need integration/client parity, while current uncommitted composition work remains untouched. R75 Sign in with Google and R76 grouped, scheduled Automatic replies stay in TODO.


## Offline local Sent actions and visible reader errors — uncommitted checkpoint

Flutter native mutations now let Rust inspect the current stored message under the account lock before requesting a password. Local IMAP Sent copies can be flagged, marked unread or moved without opening the device credential store, including before the message has been displayed. A server-backed message returns a typed credential requirement before any flags or move journal are changed; the frontend then obtains that account's password. The account settings are reread after acquiring the lock. Routing uses the stored remote identity rather than an old page cache or a local-looking UI ID. Missing and locked credentials still refuse server actions with rollback and a recovery instruction.

The inbox and open message reader now share a persistent error banner with Retry and Dismiss controls. A failed flag/read action is visible while the reader remains open, and cached content remains usable. R76's requested TODO is also clarified as named reusable Automatic replies entries: type one message, assign searchable accounts/saved groups/Select all, and set the start/end/timezone; add a separate entry for another message/group. Scheduled replies and Google-provider OAuth R75 remain recorded requirements, not implemented features.

Validation: **33 native Rust tests**, native formatting/Clippy with warnings denied, Flutter analysis and **25 Flutter host tests** pass. The shared synthetic Outbox fixture is prepared before each host/device profile opens. Host tests cover no credential reads for local edits/reopen and refusal for a server UID with a local-looking UI ID. The **expanded Android Outbox scenario** passes through actual flag/unread/archive/reopen controls, locked/missing-credential rollback, visible reader errors and dismissal; the separate Android profile-lock control also passes. The first device attempt caught an incorrect test expectation: mobile currently marks read on opening. The final test checks persisted unread state before opening and the current read transition afterward; desktop read-on-leave parity remains open. The other nine Android scenarios and Appium were not rerun for this checkpoint; their prior full-suite evidence remains separate. All **five Flutter Playwright stages**, **23 Python tests**, the **18-contract parity review check** and pinned strict documentation build pass.

Reviewed final WebP captures are `artifacts/flutter/native/native-imap-local-sent-offline.webp`, `native-imap-local-sent-reopened.webp` and `native-imap-credential-recovery.webp`. The last now shows the error inside the reader with readable cached content and recovery controls. The production debug APK contains all three native Android ABIs, Internet permission and no tested fixture markers; its SHA-256 is `d69f25bfeda9741c51fa56f1e9f9b5bf8e6f7c212c37d5e34da72aa9aa879f7d`. Report: `artifacts/flutter/production-sent-offline-apk.json`. This is not a signed distribution artifact. Logs are under `artifacts/logs/sent-offline-*` plus the runner's `android-outbox-*` and `flutter-web-*`; the owned emulator and browser server were stopped.

This is a partial uncommitted checkpoint in `feat/mobile-web-clients`, with no main merge/push, deployment, release or workflow enablement. Stable Sent identity and pending actions during local/server handover, logical Sent-folder grouping, browser provider Sent recovery, history cleanup and complete client parity remain active. Root/shared/backend code did not change in this continuation; no new shared-wire/live-provider or Apple evidence is claimed. VPS/owner/OAuth configuration is still pending, and performance remains deferred. Desktop Forward independently shipped in `9062dcf` / `ac515a3`; the main worktree is clean at the latest check. Its committed HTML/find/Forward changes and earlier defaults need deliberate integration and mobile/browser equivalents. Quality/release CI remains disabled until the runners are ready.


## Native Sent handover and combined checkpoint preparation

A matching acknowledged server Sent copy now adopts the untouched local copy's stable ID in one SQLite transaction. Previously issued provider IDs remain aliases for detail, reply and queued actions. Adoption checks the account, saved destination, unique Message-ID and acknowledged UID when present; ambiguous copies or edited local mail remain separate. Failures roll back provider insertion, aliases, body and search updates together. Schema version 6 migrates historical Sent-folder roles atomically, while ordinary Inbox paging retains its indexed path. Logical Sent includes configured/discovered/acknowledged folders; Undo retains the actual provider destination. Original submitted MIME is never rewritten.

Local flag/read/move actions finish even when all provider slots and the account lock are occupied. They commit the local-edit marker with the change, protecting it from a later sync. A POP3 UIDL that resembles a local outgoing identity cannot mark another account's Sent record. Flutter migrates body/error caches, selection, pending field generations and Undo through the returned aliases, keeping a displayed reader usable through a held handover and late body result.

Validation for this continuation: **39 native Rust tests**, native formatting/Clippy, Flutter analysis and **27 host tests** pass. The complete sequential Android runner passes **eleven integration scenarios** (three preview, six native, one actual DocumentsUI picker and one Outbox recovery), then **five Appium stages**. All **five Flutter browser stages** pass. Tests exercise transaction and migration rollback, reopen/reply aliases, ambiguous identity refusal, a queued provider action during sync, and real controls for retained readers, late bodies and pre-handover Undo. The real Rust Android Outbox flow repairs a preloaded acknowledged provider row and shows one grouped copy. Fixtures are supplied before profile opening; controls perform subsequent actions. The final POP3 ownership guard has its focused native regression test and the targeted real Android Outbox rerun also passes. Final artifact checks are recorded with the shipping evidence.

Reviewed WebP captures are `artifacts/flutter/native/native-sent-handover.webp`, `sent-handover-reader.webp` and `sent-handover-undo.webp`, covering the actual Rust repair and light/dark readable controls. Logs use `artifacts/logs/sent-handover-*`. The production debug APK inspection before the final POP3 guard passed all three Android native ABIs, Internet permission and tested fixture exclusion, at SHA-256 `f8c22ee851e53317cd992d5845ff05500a671bad69602e04eecf8a2b4dc65f1d`. It is not a signed distribution artifact.

The combined checkpoint also retains the earlier verified browser/gateway, native desktop extraction and delegated promo website work. The final root formatting/Clippy, **212 workspace tests**, **23 Python tests** and **18-contract parity check** pass. Previously recorded unchanged-source evidence includes **25 backend tests**, **39 browser tests**, **14 browser Playwright scenarios**, **31 Rust HTTPS browser stages**, **58 iced native functional flows**, and **61 promo-site tests** with two engine-specific clipboard skips. The final push evidence records strict documentation validation and the commit.

R77 authorizes promptly publishing this partial checkpoint to the review branch, without waiting for full feature parity or merging main. Native copy labels/history cleanup, reader retention outside current list membership, browser provider Sent recovery, full composition/attachments/account lifecycle, calendar/Google/backups, encryption/bounds, current desktop HTML/find/Forward/defaults, Apple execution/distribution and complete parity remain active. R75 OAuth and R76 grouped scheduled Automatic replies remain TODOs. Live providers and VPS deployment are unverified; the exact VPS/owner/OAuth configuration is still pending. Performance measurements remain deferred on the busy host. Quality/release workflows remain disabled.


## R77 — Combined work pushed for review

Published commit [`d80f539`](https://github.com/sam-ruff/shep.so/commit/d80f539ba56c0dd9631d4305d453c841bcfe5901) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), containing the combined Flutter/browser/backend/shared-core/promo-site implementation, tests, disabled CI/release definitions and request tracking. The remote branch SHA was verified against the local commit. The commit ran the required formatting, Clippy and Rust-test hooks successfully; strict pinned Zensical validation also passed. No hook was skipped. Source and intentionally public synthetic TLS/OIDC fixtures are committed; device profiles, build reports, generated binaries, logs and credentials are excluded.

After the final Sent ownership guard, the targeted Android Outbox scenario passed again through actual controls, including its independent profile-lock check. The final production debug APK builds and passes native-library/Internet/fixture exclusion inspection for all three ABIs: SHA-256 `34d213c6f124535a9fea9a2c0255ba2a011cd52974158b456bb8e663461ad5a9`, report `artifacts/flutter/production-push-apk.json`. It remains an unsigned-for-distribution development artifact and is not published as a release. The owned emulator was stopped. Push and final-check logs use `artifacts/logs/push-*`.

This completes the request to push the current checkpoint; it does not complete the mobile/browser feature-parity goal. Main was not merged or changed, no VPS deployment occurred, and quality/release workflows remain disabled. Re-enable those workflows only when the trusted runners and remaining release prerequisites are ready. R67–R76 and all other incomplete requests remain in TODO; the parity matrix describes their actual scope and limits. The exact VPS, permitted Google identity and OAuth configuration are still needed for the authorized deployment.


## Browser provider Sent recovery — verified increment

Browser Outbox now checks the provider's Sent folder, saves exact server copies and repairs local state after acknowledgment. Preferences → Mail accounts has an independent Sent-copy policy/folder editor; reconnect preserves newer choices. New accounts default to Automatic, while POP3/LocalOnly keep copies on the browser and ServerManaged performs lookup only. Original nonsecret account settings and immutable MIME remain in the outgoing record. A changed incoming connection cannot repurpose an older upload.

The authenticated Rust gateway adds bounded, identity-scoped Sent reservations and transient receipts through the existing pinned shared transport. The browser commits the destination, copy identity and copying marker before APPEND. Duplicate requests cannot repeat an active/acknowledged copy, unknown or expired identities cannot begin an upload, and accepted work retains admission through HTTP cancellation. Supervision records uncertainty after provider errors/panics; failed lookup never uploads. The server stores no persistent mail or passwords. Client acknowledgment persistence and local-cache repair are separate: a failed acknowledgment save retains the receipt in memory, a saved receipt repairs after reload without credentials, and neither causes another APPEND. An uncertain copy requires lookup and explicit review before a new upload reservation. A matching Sent copy retains the original uncertain SMTP history and prevents returning it as a new draft.

Validation: **33 backend tests** (including eight new Sent route tests), backend formatting/Clippy, **49 browser model/provider/recovery tests**, the production browser build and **14 Playwright scenarios** pass. The explicitly selected real Rust HTTPS browser test passes **36 stages**. Its new controls save/reopen Sent preferences, observe actual IndexedDB before upload, discard an HTTP response after acknowledgment, recover after reload without credentials, and resolve a reviewed uncertain APPEND through provider lookup. The Rust fixture verifies the exact original SMTP bytes and counts exactly **two explicit APPENDs** and the unchanged **four explicit SMTP attempts**. No recovery check sends or appends again. These are object-scoped synthetic transports; the existing shared-core TLS transcripts remain the wire evidence, and no live-provider success is claimed.

Light/dark Outbox and 1440×920/900×640 Sent preferences pass axe and visual review. Final WebP evidence includes `artifacts/beta-browser/provider-sent-review-light-1440.webp`, `provider-sent-review-dark-900.webp`, `provider-sent-recovered-after-reload.webp` and `provider-sent-preferences-900.webp`. A compact scrolled-card assertion initially observed layout before it settled; it now waits for the unchanged full-card containment condition, with geometric failure diagnostics. The harness clears its previous result before starting so a failed run cannot leave a stale success report. Logs use `artifacts/logs/browser-sent-*`; **23 Python tests**, the **18-contract parity check** and pinned strict Zensical validation pass.

Browser local/provider Sent identity handover, logical folder grouping, copy labels/history, bounded Outbox/cache reads and complete background/error lifecycle remain active. The native Flutter and shared/root provider implementation did not change in this increment; no new Android/Apple/native-desktop execution or timing evidence is claimed. Calendar/Google/backups, current desktop HTML/find/Forward/Print/defaults, distribution and full parity remain active. R75 Sign in with Google and R76 grouped scheduled Automatic replies stay in TODO. No main merge, VPS deployment or workflow enablement occurred; exact VPS/owner/OAuth configuration is still pending. Quality/release definitions remain disabled. The following shipping evidence identifies this increment's commit.


Browser Sent recovery shipped for review as [`d24d87a`](https://github.com/sam-ruff/shep.so/commit/d24d87a5c52adb205b8bfdc18743a49354c9c3b1) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the remote SHA matches the local commit. Required hooks pass formatting, Clippy and **212 root workspace tests** without skips. The optimized production gateway also builds and excludes the checked synthetic Sent/Google fixture markers; its inspection report is `artifacts/browser-sent-production-backend.json`. This is a review-branch increment, with no main merge, live account test, deployment or workflow enablement. The full parity goal and the remaining work above stay active.


## Browser Sent identity handover — verified increment

The browser now adopts an untouched local Sent copy and its acknowledged provider copy in one IndexedDB transaction, preserving the original client ID and aliases for previously visible provider IDs. Strict, bounded Message-ID parsing and the saved submission/account/folder/UID receipt determine eligibility; ambiguous submissions and edited or moved local copies remain separate. The outgoing journal retains the exact original MIME. A version-3 IndexedDB upgrade seeds acknowledged Sent roles from earlier records, adds a submission index and preserves existing mail/raw data. Synchronous write errors now abort the entire transaction, including earlier queued writes.

Logical Sent includes configured, discovered and acknowledged physical folders. Reader selection, queued flags, newer optimistic intent and existing Undo closures follow identity adoption; Archive/Undo retains the physical server destination. The open reader also retains its cached snapshot if a complete refresh removes the list row. Short account-cache locks let local edits persist while network sync is held; server mutations re-resolve aliases after acquiring provider ownership. Separate edited-copy labels, history cleanup, bounded cache/Outbox reads and the wider lifecycle audit remain active.

Validation: **57 browser tests**, the production build and **15 Playwright scenarios** pass, including the real IndexedDB migration and all-store rollback contract. **34 backend tests**, backend formatting/Clippy and the optimized gateway build pass. The real Rust HTTPS browser suite passes **40 stages** and exits cleanly: it caches a provider row before receipt repair, flags/reads that row, repairs in a reopened tab without passwords, then preserves the original tab's reader and pre-handover Undo. Actual requests verify current server UIDs and the physical Sent destination. Fixture counters establish two Sent flag operations, two Sent moves, the existing four Inbox moves, **two explicit APPENDs** and **four explicit SMTP attempts**. These fixtures exercise application and transport contracts, not live providers.

Reviewed light 1440×920 and dark 900×640 screenshots show the single selected Sent row and retained reader; axe passes. Evidence is under `artifacts/beta-browser/sent-handover-*.webp` and `artifacts/logs/browser-handover-*`. The expanded two-tab test exposed proxy cleanup leaks after successful controls; the harness now closes its owned raw TLS sockets and upstream HTTP agent, and writes its success report only after cleanup. Failed cleanup runs remain in separate ignored logs; the final run passes in 11.65 seconds. **23 Python tests**, the **18-contract parity check** and pinned strict Zensical validation pass. `artifacts/browser-handover-production-backend.json` records the optimized binary hash and absence of checked synthetic fixture markers.

Root/shared/native Flutter implementation is unchanged by this increment. No new Android/Apple/native-desktop execution, performance measurement or live-provider success is claimed. Full parity, current-main integration, Google OAuth R75, scheduled grouped Automatic replies R76, packaging and VPS deployment remain active. No main merge or workflow enablement occurred; quality/release definitions stay disabled, and exact VPS/owner/OAuth configuration remains pending. Shipping commit evidence follows after the authorized review-branch push.

Browser Sent identity handover shipped for review as [`b132fa2`](https://github.com/sam-ruff/shep.so/commit/b132fa2fa4f622315440b4a578409623d07ef1c0) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required commit hooks passed formatting, Clippy and **212 root workspace tests**, with no skips. All current combined implementation is on that review branch. Full parity and the remaining requirements above stay active; no main merge, deployment or workflow enablement occurred.


### 2026-09-06 — Cached incoming attachments across clients (R67/R69/R71/R73/R74)

Flutter and the separate browser now save cached incoming files using one `shared/mail-content` Rust implementation. Native decoding runs after releasing the SQLite read connection; browser decoding runs as WASM in a serial worker with bounded admission. Shared fixtures establish exact binary and quoted-printable Unicode bytes, duplicate-name identities, safe suggested filenames and Content-Type name-only attachments. Shared list parsing now counts those named files. Download requests reject an old content identity when the cached bytes change. A corrupt native attachment exposes an error/reload control while preserving the readable cached body.

Flutter keeps the current reader independently of the filtered/paged list and retains it through move/refresh and alias adoption. Native Save distinguishes cancellation, write failure and successful completion. Android uses the actual DocumentsUI destination picker and writes off-thread. iOS has a protected temporary-file/document-export implementation, but it has **not been compiled or executed on this Linux host**. The browser retries failed worker/WASM loading and downloads from its identity-scoped IndexedDB cache without mailbox credentials or network access after its own code is loaded. It reports “Download started,” because browser downloads do not acknowledge destination persistence to the app.

Validation: **41 native Rust tests**, **30 Flutter host tests**, Flutter analysis, **59 browser tests**, the production browser build, **15 Playwright scenarios**, **34 backend tests** and gateway/native formatting/Clippy pass. The real Rust HTTPS browser suite passes **43 stages**, including blocked-WASM retry and offline saves that compare downloaded bytes. The complete dedicated Android run passes **twelve integration scenarios** and **five Appium stages**; the new scenario cancels then saves through DocumentsUI, checks the destination bytes and retains the moved reader after a failed refresh. The Flutter browser runner passes its five stages. The production Android debug APK contains all three native ABIs, Internet permission and no checked fixture markers; it is not a signed distribution release. The optimized gateway and browser assets also pass recorded synthetic-marker/hash inspection.

Reviewed light 1440×920 and dark 900×640 browser captures show distinct file controls, wrapping and visible status; axe passes. Android captures show the actual cancel/save picker and retained reader with its expected credential-store error and saved-file result. Evidence is under `artifacts/beta-browser/incoming-attachments-*.webp`, `artifacts/flutter/native/`, `artifacts/logs/incoming-attachments-*`, `artifacts/logs/android-*`, `artifacts/logs/flutter-web-*` and the two `artifacts/incoming-attachments-production-*.json` reports. An initial undersized emulator had a System UI failure; the dedicated AVD was restarted with more memory and the final complete suite passed. No test threshold was weakened.

The shared parity review now records **19 contracts**; **23 Python tests** include coordinated manifest/lockfile stamping for the new content crate. Quality/release definitions remain disabled. Full parity, current-main integration, large-message streaming and pre-parse deep-MIME protection, cache/reader lifecycle, Apple execution/signing, Google OAuth R75, grouped scheduled Automatic replies R76 and VPS deployment remain active. No live-provider, native-iced, Apple or performance execution is claimed by these fixtures. Deployment still needs the exact VPS and verified owner/OAuth configuration. The authorized review-branch commit/push evidence follows after required hooks and strict documentation validation.


Incoming attachment saving and reader retention shipped for review as [`abc44dc`](https://github.com/sam-ruff/shep.so/commit/abc44dcd0d85d38e607d8ff8c30315303800a03c) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required hooks passed formatting, Clippy and **215 root/shared tests**; the two opt-in personal-account diagnostics remain intentionally ignored. Pinned strict Zensical validation passed. R77's prompt-push instruction is fulfilled for this increment; full parity and the remaining requests above stay active. No main merge, deployment or workflow enablement occurred.


## Reviewed client account removal — verified increment

Flutter and browser Preferences now review local message, draft, file and delivery counts before removing an account. Both recheck the review inside a single database transaction; new mail, draft/file edits or recovery changes require another review. Active account/provider operations are protected. Discarding unfinished delivery or move records needs explicit acknowledgment and never cancels/undoes a server operation. Removal deletes owned local data, with no provider deletion request. Native FTS and dependent records participate in rollback.

SQLite schema 7 records removed identities and retryable credential cleanup jobs. Reconnect cannot reuse a removed identity, and delayed draft saves cannot recreate it. Device credential failure leaves the account removed and a visible Retry cleanup action; a fresh profile handle sees the pending job. The lifecycle FIFO covers connect/removal/cleanup across native handles. IndexedDB schema 4 checks removed identities in each write transaction, so a late tab result cannot undo deletion. Browser tombstones retain only identifiers and a random retry token, without removed mail metadata, draft text or secrets. Draft entries and retained readers clear immediately, and late mutation/save results cannot restore them.

Validation: **44 native Rust tests**, native formatting/Clippy, **32 Flutter host tests**, Flutter analysis, **62 browser tests**, production browser build and **16 Playwright scenarios** pass. Actual IndexedDB tests prove stale-review refusal, rollback of mixed writes, unchanged neighboring accounts, nonblocking refusal under an occupied Web Lock, absence of removed content from tombstones and stale reconnect rejection before network calls. The Rust HTTPS browser flow passes **47 stages**, including a second-tab draft edit during review, reload/cancel, removal, immediate disappearance of drafts, stale-tab reconnect/refresh and reopening. Existing SMTP/APPEND fixture counts remain unchanged. No live-provider success is inferred.

The complete Android suite passed its **twelve integration scenarios and five Appium stages**. Final draft-list cleanup was then verified by rerunning the affected native incoming/removal scenario, all host tests and the five Flutter browser stages. The Android scenario uses real Preferences and file-picker controls, cancels removal, reviews light/dark counts, removes the account with a fixture credential-store failure, verifies draft disappearance and retries cleanup. The fixture is installed before the production native profile opens. The final production debug APK builds and passes three-ABI/permission/fixture-exclusion inspection; it is not a signed distribution build. Browser production assets also pass recorded fixture-marker/hash inspection.

Reviewed captures under `artifacts/beta-browser/account-removal-*.webp` and `artifacts/flutter/native/native-account-removal-*.webp` show the review, counts, cancellation/removal controls and cleanup recovery. Browser review spacing was corrected after visual inspection; light/dark 1440×920 and 900×640 axe checks pass. Logs live under `artifacts/logs/account-removal-*` and the Android/Flutter runner logs. Production reports are `artifacts/account-removal-production-{android,web}.json`. **23 Python tests**, the **20-contract parity review** and pinned strict documentation validation pass.

Connection editing, wider lifecycle notifications, large-cache/streaming performance, current-main integration, full mail/calendar/Google/backup parity, Apple execution/distribution and VPS deployment remain active. Main independently advanced to d3b34e7 with HTML preparation/compact layout/interface scaling; its integration/client equivalents remain tracked. R75 OAuth and R76 scheduled grouped Automatic replies stay in TODO. No native-iced, Apple, live-provider or performance execution is claimed for this increment. Quality/release workflows remain disabled. Required hook and prompt review-branch shipping evidence follows.


Reviewed account removal shipped as [`18bf033`](https://github.com/sam-ruff/shep.so/commit/18bf0332ec4075004d5ce3ae42eb1a3271722f77) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote commit verified. Required formatting, Clippy and all **215 root/shared tests** passed; the two opt-in personal-account diagnostics remain intentionally ignored. Final pinned strict documentation validation passed. R77's prompt-push request is fulfilled for this increment. Full parity, Apple execution and VPS deployment remain active; quality/release workflows remain disabled and main was not changed by this work.


## Atomic native credential handover — verified prerequisite

Flutter reconnect now saves a candidate incoming/SMTP pair under an independent device key. SQLite schema 8 commits the matching account settings and active key pointer together, using the existing WAL/FULL durability policy. A failed OS write or activation preserves the previous pair; a lost activation acknowledgment leaves the committed new pair usable. Stale competing candidates cannot overwrite a newer connection. Reconnect preserves the current Sent preferences, and changing them during activation requires another attempt. Removal reviews also detect an intervening credential activation.

Cleanup checks current owners before returning unused keys, remains within the shared Dart lifecycle FIFO through the OS acknowledgment, and never includes the active pair. Failed cleanup remains visible and survives reopening. Removed accounts include all owned credential slots in cleanup. Only slot IDs/configuration enter SQLite; passwords remain in device storage. Incoming, mutation, SMTP and Sent-recovery requests recheck their credential binding while holding the account lock, before provider work or outgoing/move journaling. Local cache actions still avoid credential reads. Legacy accounts keep their original key until reconnect, including effective implicit SMTP STARTTLS settings. Unauthenticated SMTP requests do not read a password from a locked device store; that Flutter call contract is tested separately from delivery.

Validation: **49 native Rust tests**, formatting and Clippy; **37 Flutter host tests** and clean analysis. Tests cover schema-7 migration, atomic rollback/retry, idempotent activation, restart, stale settings/candidates/removal reviews, a queued mutation across activation, provider dispatch with the current binding, stale sync/move/Send/Sent refusal, lost OS/bridge acknowledgments and a held credential write against queued cleanup. The actual Rust bridge/cache remains in the host tests; probes and failure acknowledgments use object-scoped fixtures. No live-provider success is inferred.

The full Android suite passed **13 integration scenarios and five Appium stages**. After the final ownership-query refinement, the affected seven-scenario native run and the incoming/removal scenario passed again. Real password fields and reconnect/retry controls retain the old pair on a simulated activation failure, activate the new pair on retry, show a cleanup failure and clear it without deleting the active pair. The existing isolated Android device-credential roundtrip also passes. Reviewed light/dark WebP captures are `artifacts/flutter/native/native-credential-activation-failure.webp`, `native-credential-cleanup-retry.webp` and the removal captures. The cleanup action now sits beneath its text to preserve readable mobile width.

All **five Flutter browser stages** pass. The production debug APK builds for all three Android ABIs and passes permission/fixture-exclusion inspection, including the new connection fixture markers; its hash is recorded in `artifacts/credential-handover-production-android.json`. This is not a signed distribution build. **23 Python tests** and the **21-contract parity review** pass. Logs are under `artifacts/logs/credential-handover-*` and the saved Android/Flutter runner logs. Required root hooks and strict documentation validation are recorded with shipping evidence below.

Full connection editing and reviewed mailbox-identity migration remain open in both clients, alongside broader lifecycle, calendar/Google/backups, current-main integration, Apple execution/distribution, performance and VPS deployment. Browser remembered credentials remain separate work. R75 Sign in with Google and R76 scheduled grouped Automatic replies remain TODOs. No desktop-iced, Apple, live-provider or performance run is claimed for this increment. Quality/release workflows remain disabled.


Atomic native credential handover is committed and pushed as [`b9a0102`](https://github.com/sam-ruff/shep.so/commit/b9a0102397778444745a4afee8578284fea78748) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required formatting/Clippy hooks and **215 root/shared tests** pass; two opt-in personal-account diagnostics remain intentionally ignored. Pinned strict documentation validation passes. The owned Android emulator was stopped after verification. R77's prompt review-branch push is fulfilled for this increment; full parity remains active. Main, deployment and quality/release workflow enablement are unchanged.


## Find in displayed client mail — verified text controls

Flutter/native Rust and the separate browser share the desktop literal Unicode/whitespace search contract. Matches retain original UTF-16 offsets, case matching, counts, next/previous wrapping and visible highlights. Searches include expanded quoted text and exclude hidden history. One pending request coalesces newer edits; old query, message, quote, case and error results cannot restore obsolete hits. Native search has a separate blocking permit, and the browser preloads a dedicated Rust WASM worker for cached searches while offline.

Both readers retain selectable text. Native controls reveal the entire active line with surrounding space and inherit Shep's Noto Sans theme. Browser Find supports input-safe shortcuts, remapping/disable, focus retention and worker failure/Retry. New shortcut defaults leave existing custom bindings intact. Browser-history suspension retains the Find model instead of disposing a page that can resume. Full HTML Find and large-text bounds remain separate work.

Validation passes **50 native Rust tests**, native/root/backend Clippy, **41 Flutter host tests** and clean analysis, **65 browser unit tests**, **19 Playwright scenarios**, **34 backend tests** and the **48-stage real Rust HTTPS browser flow**. Shared cases exercise Unicode folding, wrapped whitespace, literal metacharacters and astral offsets. Native search also passes with every provider slot occupied; the actual FFI test verifies zero credential reads. HTTPS controls search cached MIME while networking is disabled and retain the existing exact-file and SMTP/APPEND fixture checks. These are synthetic protocol and control results, not live mail or Google verification.

The **13 Android integration scenarios** have passing runs; affected incoming/Find and Outbox flows were rerun after reader and assertion updates. **Six Appium stages** also pass, including dark Find with the real keyboard. The shared host/native scenario tests case, wrapping, quotes, full-hit visibility, native selection and Copy. It keeps the fixture keychain locked while foreground polling can resume after DocumentsUI; unrelated poll reads are distinct from the zero-read FFI search contract. One emulator System UI interruption was recovered with Wait, followed by successful reruns. The picker supervisor now stops its owned driver after helper failure, and Python checks ensure it never dismisses an application failure as System UI.

Reviewed captures include `artifacts/flutter/native/native-find-{tail,quoted}.webp`, `find-dark.webp` and `artifacts/web/find-{light-1440,dark-900}.webp`. Browser light/dark compact axe checks pass. The scroll assertion was strengthened after reviewing an edge-aligned native highlight. **26 Python tests**, the **22-contract parity review** and pinned strict documentation validation pass; final production-artifact and shipping evidence follows below. Logs remain under `artifacts/logs/`.

Full HTML, large-text layout/search, complete mobile keymaps, current-main integration, complete mail/calendar/Google/backup parity, Apple execution/distribution, final performance and VPS deployment remain active. Main independently added Linux unread dock badges in `90776fa` / `3c7acc6`; preserve that work during integration. R75 Sign in with Google and R76 scheduled grouped Automatic replies remain TODOs. No iced, Apple, live-provider or performance execution is claimed for this increment. Quality/release workflows remain disabled.

The production debug APK builds and passes all three Android ABI, Internet-permission and fixture-exclusion checks, including the Find sentinel. It is not a signed distribution build. Production browser assets also pass fixture-marker and SHA-256 inspection. Reports are `artifacts/message-find-production-{android,web}.json`. The owned Android emulator was stopped after verification.

All **six Flutter browser stages** now pass, including dark Find and case controls. The saved Playwright helper reads Flutter's merged input accessibility labels as well as ordinary semantics/live regions; screenshots independently verify the visible match count and highlight. Final host analysis and all 41 host tests pass with the strengthened full-hit visibility check. The production artifact reports, 26 Python tests, 22-contract parity registry and strict documentation build are complete. Required commit-hook and prompt review-branch shipping evidence follows.


Client Find is committed and pushed as [`50eab04`](https://github.com/sam-ruff/shep.so/commit/50eab047b3b9986b29c94b1810076e5c819b3f8a) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required formatting/Clippy hooks and **217 root/shared tests** pass; two opt-in personal-account diagnostics remain intentionally ignored. Pinned strict documentation validation passes. R77's prompt review-branch push is fulfilled for this increment. Full parity, Apple execution and VPS deployment remain active, and R75/R76 remain in TODO. Main and the disabled quality/release workflows were not changed by this work.


## Shared MIME representation selection — verified reader prerequisite

Cache ingestion and replies now select supported MIME alternatives once, prefer the last nonempty plain/HTML choice, honor multipart/related roots and detect complete mislabeled or once-escaped XHTML. Mixed HTML sections retain separate Content-ID scopes; inner resources shadow outer ones and ambiguous duplicate IDs stay unresolved. Selected inline payloads are stored once by digest. Named inline images no longer become downloadable attachments unless explicitly attached. Cache text and attachment counts do not decode optional files/images, so damaged attachments preserve readable native cache text while explicit saves still fail visibly. The root reader retains its existing legacy attachment byte API pending current-main integration.

Untrusted incoming MIME passes an iterative nesting preflight before pinned mailparse 0.16.1 recursion. Tests reject 4,096 MIME levels on a small native stack and in WASM, compare accepted boundary behavior with that parser and keep the WASM instance usable after refusal. HTML text fallback also traverses iteratively; 4,096 HTML levels and 1,000 sections sharing one payload have regression tests. The 25 MiB limit remains. This is **not formatted HTML rendering**: RawHtmlPart and the low-level WASM message_body result are untrusted sender data. Resource confinement, sanitization, WebView/frame rendering, HTML Find/selection, image policy and broader parity remain open.

Validation passes **225 root/shared tests** (two personal-account diagnostics intentionally ignored), **51 native Rust tests**, **41 Flutter host tests**, **67 browser unit tests**, **19 Playwright scenarios**, **34 backend tests** and the **48-stage real Rust HTTPS browser flow**. Root/native/backend formatting and Clippy pass, as does Flutter analysis. Fourteen shared representation fixtures run in native Rust and WASM; native cached-detail tests run with provider capacity occupied. The changed incoming Android scenario passes actual DocumentsUI cancellation/exact saving, reader retention, Find/Copy and removal/cleanup. **Six Appium stages** and **six Flutter browser stages** pass. Other Android integration scenarios were not rerun for this increment; Apple/live-provider execution remains open.

The iced functional suite was run twice: **57/58 each time**, with different intermittent input misses during concurrent builds. Both failing scenarios pass unchanged in isolation. A first-frame settling interval now precedes the initial narrow divider drag, and that flow passes in the second full run. Rapid Archive/Delete shortcut reproducibility remains explicitly active in TODO; neither full run is represented as a clean pass. Performance measurements remain deferred. Flutter browser automation also exposed an inactive semantics input after Match case; the saved scenario now clicks the painted field and sends actual keyboard events. Its final rerun passes; original failure evidence is retained. No thresholds or assertions were weakened.

Reviewed synthetic screenshots show the selected text, three real file controls and readable native/browser/iced layouts, including dark 900×640 browser and Appium Find. Evidence is under `artifacts/logs/mime-selection-*`, `artifacts/e2e/`, `artifacts/beta-browser/incoming-attachments-*.webp` and `artifacts/flutter/`. The production debug APK passes three-ABI, permission and fixture-exclusion inspection; eleven production browser files pass fixture-marker/hash checks. The optimized Rust gateway also builds and passes recorded fixture-marker/hash inspection. These are development artifacts, not signed distribution builds. **26 Python tests**, the **23-contract parity review** and pinned strict documentation validation pass. Flutter's build hook explicitly watches both shared manifests.

R75 Google OAuth, R76 scheduled grouped Automatic replies, full HTML/client parity, current-main integration and VPS deployment remain active. Main independently added bounded selection groundwork in 0833456 / 2e50d50. Quality/release definitions stay disabled. Prompt review-branch shipping evidence follows after the required hooks; no main merge or deployment is included.


Shared MIME selection and nesting protection shipped for review as [`4226f57`](https://github.com/sam-ruff/shep.so/commit/4226f577fd38f5d2e3bd353d5b15a4ab8ab36eec) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the remote SHA was verified. Required formatting/Clippy hooks and all **225 root/shared tests** pass, with two opt-in personal-account diagnostics intentionally ignored. Pinned strict documentation validation passes. The dedicated Android emulator is stopped and logs/screenshots remain under ignored artifacts. The full-run iced input limitations above remain active; R77's prompt review-branch push is fulfilled for this increment. Full parity, R75/R76, current-main integration and VPS deployment remain open. Main and disabled quality/release definitions were not changed by this work.


### Confined browser HTML — review-branch increment

Cached browser mail now retains authored HTML tables, CSS, backgrounds and bounded inline images. Shared Rust preparation sanitizes active content, tokenizes CSS resource references, converts inline images to deduplicated WebP and inventories blocked remote images. Preparation runs in an independent, cancellable browser worker. The hosted app keeps its opaque sandboxed frame mounted through control updates, preserving selection/Copy, scroll position, shared Unicode Find across inline spans, quoted-history policy and plain-text choice. Next/previous reveals the message; a compact full-reader grid regression is corrected. External links expose an address review with real Open/Copy controls. Failed preparation or a mismatched containing CSP has visible plain-text fallback and Retry.

The frame permits only the exact fixed display runtime, bounded blob images and local styles. Sender scripts/forms/frames and automatic network resources are removed; the opaque origin cannot read the containing app. The gateway includes the same runtime hash because a srcdoc document inherits its parent's CSP. Build/deploy the web app and gateway from the same source revision.

Verification: four shared Rust document tests cover authored layout, CSS escapes/raw-text boundaries, resource inventory, case-insensitive CID/data schemes and deduplicated conversion/error reporting. The browser has **69 unit tests** and **26 Playwright scenarios**, including seven new formatted-reader flows: actual Copy/paste, cross-span Find, quote visibility, next/previous, resize/flag updates, plain choice, hostile mail isolation, reviewed links, failed/obsolete workers, old-browser highlight fallback and mismatched-CSP recovery. The real Rust HTTPS beta flow passes **49 stages**, including the added formatted/CSP/worker-retry stage; all **34 gateway tests** pass. The native Rust bridge's **51 host tests** and root/native/gateway Clippy checks pass. All **26 Python tests** pass and the parity registry validates **24 contracts**. Pinned strict Zensical validation passes. The optimized Rust gateway and all **12 production browser files** pass fixture-marker exclusion and SHA-256 inspection, recorded in `artifacts/html-production-backend.json` and `artifacts/html-production-web.json`. Final hook/shipping evidence follows below.

Reviewed synthetic WebP captures under `artifacts/web/` show the retained authored dark notification in light 1440×920 and dark 900×640 Shep controls, with visible Find highlights and usable full width. Logs use `artifacts/logs/html-*`; runtime tests use no personal accounts, tokens or live mail. The earlier iced 57/58 failures and their isolated passes remain recorded; this increment does not rerun or claim a clean iced suite. No Android/iOS HTML control execution is claimed.

Remaining R38/R44/R67/R71 work: Flutter native WebView integration and Android/Apple execution, remote-image preferences/exceptions/loading, reading anchors during remote-image reflow, large-document streaming/layout bounds and full current-desktop integration. Existing 25 MiB MIME/128-level nesting limits remain; inline images currently allow dimensions up to 2048×2048 and at most 32 MiB of aggregate decoded pixels. The complete product, R75 Sign in with Google, R76 grouped Automatic replies and VPS deployment remain active. Work stays on the review branch; main and disabled quality/release definitions are unchanged.


### Confined HTML shipping checkpoint

Browser HTML rendering shipped for review as [`a9b07e2`](https://github.com/sam-ruff/shep.so/commit/a9b07e25769e4ba1424bc7016f1f66487635f9e7) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the remote SHA was verified. Required formatting/Clippy hooks and **229 root/shared tests** pass, with two opt-in personal-account diagnostics intentionally ignored. Evidence includes **69 browser unit tests, 26 Playwright scenarios, 49 Rust HTTPS stages, 34 gateway tests, 51 native Rust host tests, 26 Python tests**, strict pinned documentation validation, 24 parity contracts and inspected production artifacts. Synthetic initial-reader and Find WebPs were reviewed in light 1440×920 and dark 900×640 layouts. Logs/artifacts remain ignored. R77's prompt review-branch push is fulfilled for this increment. Flutter HTML/device execution, remote-image policy, full parity, R75/R76 and VPS deployment remain open; main and disabled quality/release definitions remain unchanged. Re-enable quality/release only when the trusted runners and release prerequisites are ready.


### Flutter confined HTML reader — review-branch increment

Flutter now prepares the shared sanitized document through two bounded native Rust slots, independent of provider work and Find. The cached alias is resolved before preparation, and a saturated renderer leaves plain text, Find and retry available. A persistent Android WebView/iOS WKWebView retains the document through flag updates, Find, quote visibility and plain-text switching. Generation/layout checks reject late events, commands coalesce, and disposal releases converted inline blob resources. Navigation, device file/content access and permission grants are denied. Reviewed external links open through the system handler. iOS now targets 14.0 for WebP; Apple execution remains open.

Actual Android integration exercises the production FFI/SQLite preparation and Rust Find with locked fixture credentials: authored table/CSS and inline image observations, cross-span matches, quote counts, next/previous, scroll retention through flag updates, plain choice and damaged-message Retry. Native Appium passes **five stages**, including real selection-menu Copy and Paste, Find/quotes/plain choice, link review/address Copy and dark mode. It runs the ordinary Flutter binding with a test-only shared Rust-prepared document; it is separate from the production-bridge integration. **Nine Flutter browser stages** pass using the same prepared fixture, plus the existing **six browser stages**. Tests caught and corrected cross-origin window comparison, Find accessibility covering the frame, and retained quote visibility when switching to plain text. Native floating toolbar inspection uses all interactive windows and actual native controls. The integration helper captures the Android screen because the screenshot converter cannot reliably capture a mounted hybrid WebView; failed diagnostic logs remain under ignored artifacts.

Validation passes **229 root/shared tests** (two opt-in personal-account diagnostics intentionally ignored), **52 native Rust tests**, **43 Flutter host tests**, **69 browser unit tests**, **26 separate-browser Playwright scenarios**, **34 gateway tests**, **49 actual Rust HTTPS stages** and **26 Python tests**. Flutter analysis and root/native/gateway formatting/Clippy pass. The production APK contains all three native architectures and excludes the new fixture markers; twelve production browser files and the optimized gateway pass fixture exclusion/hash inspection. These development artifacts are not signed distribution releases. The 24-contract parity registry and pinned strict documentation build pass. The existing Android incoming-file scenario also passes actual save/cancel/exact bytes, retained reader, plain-text Find and account removal/cleanup after this reader change. Other Android integration scenarios and the six general Appium stages were not rerun for this increment. Final shipping evidence follows below.

Reviewed synthetic WebP captures under `artifacts/flutter/native/picker/`, `artifacts/flutter/native/formatted-appium/` and `artifacts/flutter/web-formatted/` show the authored message, visible Find matches, quote/plain controls, real Copy selection and readable fallback, including dark mode. No personal accounts, credentials or live provider verification are involved. Earlier iced 57/58 limitations remain recorded; no iced suite or performance measurement is claimed for this increment. Full remote-image policy, large-document/layout bounds, Apple execution, hardware keymap parity and current-main integration remain open, as do R75/R76 and VPS configuration/deployment. Main independently shipped bulk selection/durable Undo in `af056a9` / `10d8569`; those changes need deliberate integration. Quality/release definitions remain disabled and now include the new saved scenarios.


### Flutter reader shipping checkpoint

The Flutter formatted-reader increment shipped for review as [`6e8475f`](https://github.com/sam-ruff/shep.so/commit/6e8475f4470534fd4d16836c8a9efefa8915f1f6) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the remote SHA was verified. Required formatting/Clippy hooks and all **229 root/shared tests** pass, with two personal-account diagnostics intentionally ignored. The platform/browser/protocol evidence, production artifact inspection and remaining limits are recorded above. Pinned strict documentation validation passes. R77's prompt push is fulfilled for this increment; full parity, Apple execution, R75/R76 and VPS deployment remain active. The owned emulator is stopped, artifacts remain ignored, and main was not merged or modified by this work. Quality/release definitions remain disabled; re-enable them when trusted runners and release prerequisites are ready.

## Imported desktop main history through 10d8569

The following records describe the independent desktop main worktree at their cited commits. They do not establish verification of this merged client branch. Desktop request IDs R67–R71 here belong to the `desktop-main:` namespace.

## Frequent background mail and independent refresh

Mail checks now start after the cached workspace opens and repeat every 15 seconds by default. Existing saved minute-based preferences gain the new default automatically; General preferences accepts 5–3600 seconds and stores it across restarts. Calendar scheduling retains its separate cadence. Each completed account check flushes cached changes without waiting for a slower account.

A dedicated capacity-one command channel coalesces repeated Refresh clicks. The scheduler runs one cycle at a time and retains a follow-up requested during automatic or manual work. Automatic checks leave the manual refresh icon idle, while clicking it gives immediate feedback. Shared provider slots preserve bounded background work and independent cached reads/saves. Interval changes take effect after saving, and timeout or failure leaves the worker available for retry.

Typed sync completion clears a recovered sync error without dismissing unrelated errors. Corrected settings clear their prior validation/save error on the successful explicit save, alongside the existing Changes saved toast. Native fixtures deliver a fictional new message and exercise failure/retry without contacting a personal account.

Validation: 213 Rust tests pass (two opt-in live diagnostics ignored), 17 Python tests pass, and the full 61-flow native functional run passes. Virtual-time tests cover immediate/repeated checks, interval changes, coalesced manual requests, error and timeout recovery; queue saturation and SQLite reopen tests cover isolation and persistence. WebP review covers automatic arrival, queued refresh, server-error recovery and valid/invalid interval saves. Logs are under `artifacts/logs/mail-sync-*`. Formatting, Clippy and the strict documentation build pass. Performance measurements remain deferred. Release installation and the shipped commit are recorded below after publication.

The optimized production release passes archive checksum, extraction and bundled-installer verification. It is installed for the Linux user; the installed executable and release binary both have SHA-256 `1f903cd013c9324e2edd19d2281f37afbad4e0f54958ee0a72ec427d9c9ba37c`. Existing personal windows were left running and need reopening to load the update.

Shipped as `d4ecb215c5ba7ce9bb078c68d6a9f0e383609f4c` on main. R65 is complete and removed from TODO; the other full-product requirements remain active. The Documentation workflow is tracked in [run 34029689068](https://github.com/sam-ruff/shep.so/actions/runs/34029689068). Quality and release CI remain disabled.


## Relevance search and Move matching

New inbox searches use **Best match**, while the sort menu can override ordering for that search. Clearing search or returning through Mail restores the saved browsing sort. SQL ranks before paging, preserves account/folder/filter scope and uses timestamp/ID ties. Exact-term ranking is separate from typo-expanded ranking; sender matches carry less weight than subject/body matches. A short whole-body equality bonus makes the requested `test` body rank above keyword repetition, including ordinary surrounding line endings. Its metadata size guard avoids reading long message text for that bonus; longer messages remain searchable through the index.

RapidFuzz 0.5 replaces the handwritten edit matrix and supplies edit, subsequence and ratio matching. Vocabulary candidates are retrieved through indexed edits and a bounded shared-prefix neighborhood. Folder ranking handles exact names/leaves, prefixes, accents, transpositions and abbreviations; Enter resolves the current query. Latin normalization preserves Japanese marks and Hangul syllables. Nonempty input without indexable tokens returns an empty result, and query punctuation cannot become SQL/FTS syntax.

The native Move scenario exposed a loading race. Move/read/flag now resolve the current reader identity from matching detail, conversation or inbox metadata and work while its body is pending. A stale body from another message cannot become the action target. Closing Move clears focus observation. The unrelated POP3 caption was removed from the Move chooser. The saved native flows inspect Best match/date switching and refresh, then move into Café and Archive and verify the destination contents.

The ranked-query benchmark now selects Relevance; it has not been run because performance measurements remain deferred until the final idle-host pass. See [RapidFuzz](https://docs.rs/rapidfuzz/latest/rapidfuzz/), [FTS5 ranking](https://www.sqlite.org/fts5.html#the_bm25_function) and [SQLite octet_length](https://www.sqlite.org/lang_corefunc.html#octet_length) for the underlying APIs. Test and shipping evidence follows after validation.

Validation: formatting and Clippy pass, 221 Rust tests pass (two opt-in live diagnostics ignored), 17 Python tests pass, and all 63 native functional flows pass in one complete run. Tests cover the exact `test` body, repetition, line endings/case, typo/prefix ranking, stable pages/reopen, scopes, Unicode, stale results and actions while bodies load. Reviewed WebP evidence shows Best match, date override, highlighted Café and the correct Archive destination. Logs are under `artifacts/logs/search-*`. The strict documentation build passes. Release checksum/extraction/bundled-installer verification passes; the installed Linux executable matches the optimized production binary at SHA-256 `7c663f0dfde36967316334325987e05d0b2d8c2ec015a56547ec49a3809080d7`. Existing personal windows were left running and need reopening for this update. Shipping is recorded after publication below.

Shipped as `d3a530a` on main. The fuzzy-matching/relevance entry is complete and removed from TODO. R44 still tracks finding text within the reader, and R50/R60 retain their other optimistic-action requirements. The full-product goal remains active.

## Collapsible drafts and permanent discard

Drafts appear in a counted sidebar group with a saved collapse preference. Each draft has a native context menu for opening or discarding; a background mail refresh preserves its target and menu. The editor also has a bin, and the discard confirmation is red in both themes. Both discard controls open the same subject/attachment review, with mouse cancellation and Escape/N, or Enter/Y to confirm. Failed storage keeps the original editor and cached attachments available for retry; pending deletion prevents further edits and window closure.

Deletion runs through the independent persistence queue. One SQLite transaction rejects unresolved outgoing delivery, permanently retires the draft ID, removes text and cached attachment blobs, and returns a versioned snapshot. Retirement survives reopening storage, newer delayed save/file revisions, and old send cleanup. Beginning a delivery checks the same tombstone transactionally. Existing Outbox review remains the route for resolving an interrupted delivery before its draft can be discarded. Known rejections can be discarded directly, with their cached MIME/envelope record and Outbox entry removed in the same transaction.

Rust tests cover storage reopen, attachment deletion, late saves/imports, rollback/retry, send/discard exclusion and stale UI acknowledgments. The occupied-provider test also discards a real stored draft while network workers remain blocked. Native scenarios use actual right-click, group toggles, file picker, bin and keyboard review. The one-failure preview fixture contains no personal data or server access. Validation and shipping results are recorded below after completion. Inline composition and switching among active drafts remain tracked separately by R35.

Validation: formatting and Clippy pass; 229 Rust tests pass (two opt-in live diagnostics ignored), and 17 Python tests pass. The 65-flow native run passed 64 flows and exposed a file-picker harness race: GTK received an empty fixture path before clipboard ownership was ready. The harness now waits for native focus and verifies the isolated clipboard serves the complete path before pasting. Its Python regression includes a stale initial clipboard value. The previously failing attachment flow, discard/retry/compact flow and search-sort flow then all passed. All 65 functional scenarios are verified across that full run and targeted rerun; this is not a claim of a single entirely green run after the harness correction.

An earlier full run also exposed a stale search-focus observation after choosing a sort. Sort/filter changes now clear the old observation and cancel pending retries; a UI ordering test verifies late acknowledgments cannot revive it, and the saved native scenario waits for a fresh focus result. No performance measurements were run. Reviewed WebP captures show the red discard button in light/dark/900×640, the attachment/error review, collapsed/expanded drafts and a menu surviving mail refresh. Logs are under `artifacts/logs/drafts-*`.

The strict documentation build passes. Optimized production release checksum, extraction and bundled-installer checks pass. The Linux user installation matches the release binary at SHA-256 `3d234115369e536c8dbf9618ef50da685062053ac6f2d2dccb5cd325f243cc48`. Personal windows were left running and need reopening for this build. Shipping is recorded below after publication. R35, R38 and the newly requested R67/R68 remain open in TODO.

Shipped as `5aab267bb84392756b1fe8f3658155649015ed85` on main. R36 is complete and removed from TODO, including the requested red confirmation button. The full-product goal remains active. R67 read-on-leave, R68 immediate counted toasts/Undo and R38 faithful HTML are not included in this delivery.

## Read on leaving and immediate action toasts

Deliberately selecting an inbox message arms a read acknowledgment when the user leaves it. Clicking another message, navigating folders/tabs, composing, closing the reader/window and leaving the application update its unread indicator immediately through the existing background flag queue. Startup selection, hover, prefetch and background refresh do not mark unseen mail read. Explicit Mark unread cancels automatic acknowledgment for that visit. Read completion preserves later flag changes, failed saves restore unread state without changing the current selection, and keyboard navigation through the Unread filter keeps the next row selected when the previous row disappears.

Folder and cross-account moves wait behind the selected message's pending read/flag changes and receive the confirmed flags, including after a rejected read. Cross-account transfers now have typed completion IDs, hide the source immediately, retain their pending overlay through refresh, and restore on failure. The existing destination-upload journal and source-before-delete rules remain. A confirmed source move is not reclassified as rejected if subsequent cache cleanup fails. Durable action recovery and projection into newly selected destination/filter scopes remain in TODO.

Archive/delete/move toasts are created during the optimistic UI update, including while source flags are still saving. Repeated actions increment the count and restart the six-second lifetime. Archive/delete continue across accounts in a unified inbox; other folders are scoped to their exact destination account/folder. Each failed action removes only its own count. An older result cannot replace a newer toast, and acknowledgments do not recreate dismissed/expired feedback or hide an existing error. Save-preferences and mail-action toasts stack independently. Undo remains open under R68; no Undo control is claimed delivered here.

Rust coverage includes source flag/transfer ordering, queue exhaustion, explicit-unread intent, failed saves, typed transfer dispatch/cache results, counted feedback and injected-clock lifetime checks. Saved native scenarios cover immediate repeated actions, dismiss-before-save, failure, read navigation and cross-account transfer with isolated slow/failing backends. They also review light/dark/900×640 WebP evidence. A native regression exposed archive/delete counts resetting at a unified-inbox account boundary; the counter now aggregates those standard actions across accounts. The account-removal harness now retries a transient null review object instead of raising ValueError while walking its pending count; its Python test reproduces that transition. Validation and shipping results follow after completion.

The broad native run also exposed Ctrl+D reaching the mail shortcut handler when iced left that chord uncaptured in the focused search field. Mail-target shortcuts now check the native search input focus before dispatch, including remapped modified chords; typed focus responses are ignored after changing tabs or opening another interaction. A new saved mouse-focus scenario tests default/remapped Delete in search and then outside it. Removing the old completion notice increased the visible shortcut-list height, so the Inbox-clear scenario now clicks its actual row and asserts Delete remains unchanged.

Validation: formatting and Clippy pass; 242 Rust tests pass (two opt-in live diagnostics ignored) and 17 Python tests pass. The final broad native run passed 68 of 72 functional flows and exposed the focus check swallowing keys when the full-window reader had no search widget. The guard now applies only to the inbox layout. All four affected reader/selection/context flows, the default/remapped search-isolation flows and the counted-toast flow pass in the saved targeted rerun. All 72 scenarios are verified across that broad run and corrected rerun; this does not claim a single entirely green suite run after the final correction. Evidence is under `artifacts/logs/action-toasts-*`; reviewed WebP captures include counted archive/delete/move, failure, read-on-leave and compact dark toasts. Performance measurements remain deferred. The strict documentation build passes. Optimized packaging, installation and publication evidence follow below.

The optimized production package passes SHA-256, extraction and bundled-installer checks. The Linux user installation matches that release binary at SHA-256 `8d56b26f1da56cefd3c67983d3a4bfc1eecb4617fa79bd4d2b6a30aedc0cf4a7`. Personal application windows remain open and need reopening for this build. Publication is recorded below after pushing. R68 Undo and R38 faithful HTML remain open.

Shipped as `9c907d2c8d974033ff0455ac5d7ef7b2e5a6474a` on main. R67 and the newly discovered search-focus isolation regression are complete and removed from TODO. R68 now retains Undo; its immediate counted feedback is delivered. The full-product goal remains active.

[Documentation run 34037090661](https://github.com/sam-ruff/shep.so/actions/runs/34037090661) passes for the shipped code. Quality and release workflow definitions remain disabled as requested.


## Undo for immediate action toasts

Archive, delete and move feedback offers Undo for the visible counted group. Clicking it immediately restores the original rows and shows a counted Restored toast. A move waiting for a flag write can be cancelled locally; a move already accepted by the backend is reversed after its receipt arrives. Navigation remains available, and source placeholders cannot issue actions against obsolete server identities. Undo from a destination folder clears its retired reader target. A rejected reversal keeps Retry Undo/Dismiss available after normal toast expiry; retry preserves the acknowledged receipt.

MOVE/APPEND parse their tagged completion and COPYUID/APPENDUID explicitly. Missing mappings after success use exact destination lookup for Undo: account connection checks, UIDVALIDITY, size/Message-ID candidates and full-byte SHA-256 verification. Duplicate matching copies produce an actionable error rather than guessing. Cache relocation updates the identity transactionally, preserving raw content, search and flags. POP3 reverses locally. Cross-account Undo retains the upload journal and may reverse an already authorized transfer after its preference is disabled. Protocol fixtures and isolated cache tests cover acknowledgment, rejection, lost replies, ambiguity and fresh identities; these are not live Fastmail verification.

The first native pass verified pending grouped archive Undo and move Undo in light/dark/900×640. Visual review exposed a stale destination-reader request, now corrected; the retry scenario's click position was corrected to the actual native control. Final validation, packaging and publication evidence follow below. Session-only Undo does not complete R50/R60 durable recovery or all filtered-folder projection work.


Validation: formatting and all-target/all-feature Clippy pass; all 258 Rust tests pass (two opt-in live diagnostics ignored), and all 17 Python tests pass. The final complete native run passes all 76 functional scenarios in one run, including four saved Undo flows. Reviewed WebP evidence covers immediate grouped restoration, delete rejection/retry, cross-account reversal after disabling the preference, destination-reader cleanup and light/dark/900×640 controls. The strict documentation build passes. No performance measurements were run. Logs are under `artifacts/logs/undo-*`; packaging, user installation and publication results follow below.


The optimized production package passes checksum, extraction and bundled-installer checks. The per-user Linux installation matches the release binary at SHA-256 `993c786f1080608bc747b5ee7378f9ce3db060c371dcd9c3c42402fa19f438a1`. Personal windows remain running and need reopening. Publication is recorded below after pushing; the full-product goal remains active.


Shipped as `551f86c4f428668e633c2be70ea2df1d08d0016f` on main. R68 is complete and removed from TODO. The full-product goal remains active; R38 faithful HTML, R50/R60 durable optimistic recovery and all other listed work remain open. Quality and release workflows remain disabled as requested.

[Documentation run 34040557571](https://github.com/sam-ruff/shep.so/actions/runs/34040557571) passes for the shipped code.


## 2026-09-06 — Formatted HTML reader and preserved immediate feedback

R38 now selects the correct MIME representation and renders static HTML layout, typography, tables, backgrounds, links and inline images. Complete mislabeled or escaped XHTML no longer exposes raw tags. A Formatted/Plain text switch preserves the alternative representation; explicit HTML attachments remain attachments and Content-ID images stay scoped to their related part. Fictional fixtures reproduce the supplied styles without copying private authorization codes.

A dedicated litehtml worker owns parsing, fonts, layout, image decoding and viewport rasterization. The iced canvas scrolls with the reader, clips short messages away from reply controls, and offers horizontal scrolling for wide tables. New messages start at the top. Visible-text geometry supports native drag selection, Select all and clipboard copy, excluding collapsed quoted text. Quote controls retain the existing collapsed/expanded/latest-only preference. Metadata-only flag updates preserve the open document. HTML source/inline bytes count toward the existing prefetch cache budget.

External images retain the existing default block/Contacts/Allow all policy and per-message/sender/domain exceptions. A compact image menu saves vertical space. CID/data resources convert to WebP in the worker, and small images preserve their natural dimensions. The renderer has no JavaScript, CSS-import, filesystem or automatic network loader. A small licensed upstream drawing adapter is included with corrected image sizing, positioning, repetition and display scale. HTTP(S) links use background system-browser dispatch; mailto creates an unsent draft. Advanced browser CSS/animations are not fully supported, and the separate R23 download/snapshot/plain-preview streaming work remains open.

Validation: formatting and Clippy with warnings denied pass; 270 Rust tests pass, with two opt-in live diagnostics ignored. All 17 Python tests pass. The complete 80-scenario native functional run passes; the existing quote scenario was then extended with plain-mode collapse/expand assertions and also passes. Native evidence includes styled/XHTML/plain reading, actual clipboard paste, long and wide documents, right-column drag selection, link activation, image exceptions, light/dark/full/900×640 layouts, and archive feedback while a flag save remains pending. The existing counted-toast, grouped Undo, read/unread and context-menu regressions pass. Pixel tests verify table backgrounds, image sizing/repetition at normal/doubled scale and viewport-sized frames. Singular message-count labels are corrected.

Performance measurements remain deferred. Logs and reviewed WebP evidence stay under ignored `artifacts/logs/html-*` and `artifacts/e2e/`; no root logs or private mail fixtures were added. The repository MCP skill and agent instructions document the new flows and CMake/C++ build prerequisite. Quality/release workflows remain disabled; the strict documentation build passes.

The optimized production package passes checksum, extraction and bundled-installer checks, including the adapter license/provenance. Installation for the Linux user matches the release binary at SHA-256 `eee40e452ef60cbb2ea118955d363be715d3d945998dfb4417983e08da97c947`. Existing personal windows were left running and need reopening. Publication is recorded below after pushing.


The core HTML reader shipped as `1968e37b3d2792a9eaa87dab0a2e77ea1939e632`. [Documentation run 34045888746](https://github.com/sam-ruff/shep.so/actions/runs/34045888746) passes. Final keyboard review found that uncaptured arrows in the HTML body could navigate the inbox; the follow-up captures scrolling keys only while the body is focused and restores inbox navigation after clicking outside.

The expanded 81-scenario run passed 80 flows and exposed a native file-picker harness race. The path field was populated before GTK finished validating it, so the first Return did not always accept the dialog. The harness now verifies the exact entered path through GTK clipboard ownership and uses bounded, picker-targeted confirmation retries within the existing close timeout. Deterministic Python tests cover ignored input, missing acceptance and delayed filename validation; the saved real multi-attachment flow passes after the correction. The corrected complete run passes all 81 native functional scenarios in one run.


The follow-up shipped as `32120b40400b1fd6c9e1986f94d97a82fe481f8d` on main. Formatting, all-target/all-feature Clippy and all 270 Rust tests pass (two opt-in live diagnostics ignored); all 20 Python tests pass. The optimized production package passes checksum, extraction and bundled-installer checks. Its Linux user installation matches SHA-256 `2b1fef6de0f264f61062867fb54261f26e5fa2d5e1b7811930c430e8b7db46c7`. Existing personal windows remain open and need reopening. The strict documentation build passes; [the follow-up documentation run](https://github.com/sam-ruff/shep.so/actions/runs/34047461991) tracks publication.

Reviewed follow-up evidence includes the long-reader End/Home and inbox-focus transition captures in `artifacts/e2e/a711a8c83e2f/` and wrapped attachments in `artifacts/e2e/642e66b89f15/`. Final logs are `artifacts/logs/html-verified-native-tests.log`, `html-final-python-tests.log`, `html-navigation-commit.log`, `html-final-release.log` and `html-final-install.log`. R38 is complete and removed from TODO. The latest responsive-toast request remains delivered and reverified with controlled slow/failing actions, counting and Undo. R44 find, R23 large-message streaming and the rest of the full-product backlog remain open. Performance measurements remain deferred; quality and release workflows remain disabled.


## 2026-09-06 — Find within formatted and plain messages

R44 adds a remappable Mod+F action and a reader toolbar control. The find bar counts and highlights literal matches, distinguishes the active match, supports case matching and wraps next/previous through Enter/Shift+Enter or mouse buttons. Escape closes Find before leaving a full-window reader. Search follows the current message and its visible quote state while preserving the inbox query, native text selection and image privacy controls.

Matching and geometry run on the reader worker. HTML uses actual visible text runs, excluding collapsed/hidden content; plain text uses an independent font system so background shaping cannot take iced's UI font lock. Whitespace normalization retains source offsets, Unicode case folding handles non-ASCII text, and regex punctuation is literal. Result revisions, coalescing and cancellation reject obsolete work. Indexed highlight geometry visits visible rows, and reveal scrolls both the parent reader and wide HTML documents when needed. Compact toolbar spacing preserves Export.

The first native iterations exposed a global-Shift race on Enter, stale focus observations on close/navigation and tests clicking controls before the find bar finished changing the layout. Event modifiers/revisions now drive Enter, known focus transitions clear observations, and saved tests wait for the current native field and visible controls. Five saved Find scenarios pass, with reviewed light/dark/900×640/full-reader screenshots. Complete regression, optimized installation and publication evidence follows after final checks. Performance measurements remain deferred; R44 stays in TODO until shipping is verified.


Validation: formatting and all-target/all-feature Clippy with warnings denied pass. All 277 Rust tests pass, with two opt-in live diagnostics ignored; all 20 Python tests pass. The complete native run passes all 86 functional scenarios in one run, including the five new Find paths and existing immediate-action, Undo, read/unread, context, draft, calendar and shortcut regressions. The strict documentation build passes. Reviewed captures include `artifacts/e2e/508d31adf4fb/find-html-first.webp`, `67444689804a/find-plain-first.webp`, `9ad371cc859d/find-plain-quote.webp` and the wide compact/full-reader evidence. The final compact capture additionally moves the pointer away from Export so its tooltip does not cover the find controls.

Optimized packaging passes checksum, extraction and bundled-installer verification. The Linux user installation matches release SHA-256 `8a3ca8c00386b21ba5ce7ebdea68c22fa4df2059f400eb5778d94df586e701b8`. Personal windows remain running and need reopening. Logs are under `artifacts/logs/find-*`; no root logs or private mail fixtures were added. Quality and release workflows remain disabled; performance measurements remain deferred. Publication evidence follows below.


Shipped as `c266035ad5a3d76c7bd535e8934cab281b4a5063` on main. [Documentation run 34050082487](https://github.com/sam-ruff/shep.so/actions/runs/34050082487) passes. The final wide-table capture rerun also passes, with all compact controls reviewed at `artifacts/e2e/dc9b4f87a783/find-wide-dark-compact.webp`. R44 is complete and removed from TODO; its previous inbox relevance work remains covered. The full-product goal stays active, including forward/print, bulk actions, folder and composer work, settings sync/backups, cache encryption/large mail, installers, palette/icon work and final provider/platform/performance verification.


## Forward messages with retained content and attachments

The preview footer has a Forward arrow and a remappable F shortcut, including the optional secondary slot and disable behavior. A forward uses the expanded physical message in a conversation. It starts an independent draft on the original account, with a new identity, Fwd subject, blank To/Cc/Bcc and no inherited reply-thread headers. Focus goes to the recipient field. Preparation runs through the persistence worker and shows Preparing immediately; repeated requests coalesce. Navigation and other editors remain available. A late result stays in Drafts, a failed preparation leaves no partial draft, and retry clears only its own error.

Forward construction reads complete cached MIME instead of the shortened reader. It retains public quoted headers, full original text, selected HTML/styles/body attributes, CID resources and ordinary attachment bytes/media types. It does not fetch remote images or copy Bcc/transport headers. Text and independent attachment blobs commit together; inline metadata cascades with file deletion. HTML and files survive save/reopen. A note above the unchanged quote preserves HTML; editing the quoted original sends the edited plain text and retains inline-image bytes as ordinary attachments. This behavior is documented. Existing sending/attachment ceilings remain R23; this increment does not claim arbitrary-size download/storage support.

Validation: all 284 Rust tests pass, with two opt-in live diagnostics ignored. Coverage includes new-thread/recipient privacy, complete text beyond the reader ceiling, MIME/HTML/CID/binary roundtrips, native editor text roundtrip, reopening, atomic rollback/retry, stale results and forwarding with all network jobs/queues occupied. Clippy with warnings denied and formatting pass; all 20 Python tests and the strict documentation build pass. One complete run of all 91 native functional scenarios passes, including five new forwarding flows and the existing read/unread, context-menu, Find, attachment/reply, immediate toast and Undo regressions. No personal mail was sent or mutated. Performance measurements remain deferred.

Reviewed evidence includes the four wrapped files and reopened composer at `artifacts/e2e/44c4970e0ebf/`, the older Sent message at `artifacts/e2e/0d087fffaaea/forward-conversation-target.webp`, and the dark 900×640 composer at `artifacts/e2e/5fd130da30b0/forward-dark-compact.webp`. The final composer capture at `artifacts/e2e/fe6483569ecf/forward-composer-files.webp` is also reviewed; its saved native rerun passes after waiting for presentation to settle. Logs are under `artifacts/logs/forward-*`. Optimized installation and publication evidence follow after those steps finish. R41 Print, the inline composer and the other full-product TODO entries remain open.


The optimized production package passes checksum, extraction and bundled-installer verification. The Linux user installation matches the release binary at SHA-256 `a270f17cc51c2bb15cd5051389b7c5aca0c8e93df967f1507b588d5a7db10128`. Personal windows remain running and need reopening for this build. Publication is recorded below after pushing; the full-product goal remains active.


Shipped as `9062dcf656ec22df438c4eabbcdebfa376be7739` on main. R41 Forward is complete and removed from TODO. R41 Print and all other remaining requests stay open; the full-product goal remains active. Quality/release workflows remain disabled as requested, and performance measurements remain deferred.


## Print messages through the browser printer/PDF dialog

The reader footer now has a printer icon and remappable Mod+P (Command+P on macOS), including both shortcut slots and disable behavior. Preparation snapshots the expanded message and the current Formatted/Plain choice. It reads complete cached MIME, including quoted history, public headers, attachment names and scoped CID images. Previously loaded remote images are included only when the current message policy permits them; no new external image requests occur. Rendering and image conversion run off-thread on a dedicated bounded queue, independent of provider and cached-read capacity. Repeated pending requests coalesce, navigation stays available and a delayed preparation retains its original target. Failure permits retry and preserves unrelated errors.

A temporary memory-only HTTP document opens in the default browser for printer or PDF selection. It uses a random loopback port/token, Host/method/path validation, bounded clients/headers/timeouts, no-store/no-referrer headers and restrictive CSP. The message itself is an inert sandboxed srcdoc with scripts/forms/navigation blocked; its trusted parent prints the child window so long messages paginate. Headers are inserted after the real srcdoc loads, and encoded CID references resolve to embedded WebP. The server stops after serving, dropping the preview or five-minute expiry. No extra plaintext mail file is written by Shep. Browser launch is not reported as proof of a completed print job.

Validation so far: 289 Rust tests pass, with two opt-in live diagnostics ignored, plus formatting, all-target/all-feature Clippy with warnings denied and 24 Python tests. Rust coverage includes complete text beyond the reader ceiling, template-like header text, inert HTML/resources, token/Host/method rejection, one-use/no-store behavior, cancellation/capacity release and printing while every provider slot/queue is occupied. Four saved native Print scenarios pass through the real MCP input path and an isolated X11 Chrome profile: actual styled/plain/multi-page PDFs, preparation failure/retry while navigating, primary/secondary remapping/disable/input isolation, and real dialog cancellation with compact dark controls and wrapped attachments. The full native regression run and publication are recorded below when complete.

Reviewed artifacts include `artifacts/e2e/1edd1a6e6d3f/print-styled.webp`, `print-plain.webp`, `print-long.webp`, and `artifacts/e2e/18b6191749b8/print-native-dialog.webp`. The printed headers, inline logo, final paragraph and attachment names are verified. The compact reader remains cramped above wrapped attachments; that and the newly reported HTML layout movement/artifacts/readiness are explicitly tracked as R69. New dock/taskbar unread-count badges are tracked as R70. Existing download/cache limits remain R23, and Firefox/Safari/macOS/Windows print execution remains unverified.

Release packaging passes checksum, extraction and the bundled Linux installer. Logs stay under `artifacts/logs/print-*`, with no root logs. The current immediate archive/delete/move toasts were reverified against delayed/failing actions before Print work; they still appear with the optimistic action and dismissed feedback cannot return on acknowledgment. Performance measurements remain deferred, quality/release workflows stay disabled, and the full-product goal remains active.


Final checkpoint: the complete native run passed 94 of 95 functional scenarios; the sidebar-resize test began before initial body/layout presentation. Its saved scenario now waits for reader readiness plus a short presentation interval before dragging, and its targeted rerun passes. All 95 functional scenarios therefore pass across the complete run and that rerun, including all four Print flows, existing forwarding/find remaps, toasts/Undo/read state and attachment controls. No performance thresholds changed. Evidence is in `artifacts/logs/print-full-native-tests.log` and `print-sidebar-rerun.log`; the final Python rerun still passes all 24 tests.

The optimized user installation and release binary match SHA-256 `a8c928de57d867bcfcc962fc2a12756ffc0b129ee4503d267fd2b50239f1428b`. Personal windows remain running and need reopening for this version. Strict docs build passes. R69 HTML rendering, R70 dock badges and R71 the malformed-refresh-icon report are recorded in TODO and the request audit; the last awaits clarification because its screenshot includes browser controls. The user requested pushing at the next working checkpoint; publication follows now, without waiting for the entire backlog.


Shipped as `b120721e6c6c3314becf47605be19f8922225cc2` on main. R41 Print is complete and removed from TODO; its platform/browser execution limits remain explicit above and under R01/R06. The full-product goal remains active. The repository skill and AGENTS.md now describe the print harness, actual PDF evidence, X11/profile isolation and the updated shortcut positions.


## 2026-09-06 — Refresh geometry and prepared HTML frames

The shared Mail/Calendar refresh SVG now joins its circular strokes to both
arrowheads. The previous path left disconnected segments. Saved native scenarios
exercise refresh while navigating and capture light/dark, 900×640 and 120%
interface scaling. Reviewed evidence is under `artifacts/e2e/4820b8a914ff/` and
`artifacts/e2e/2b315964044a/`. The supplied browser crop's exact location remains
unconfirmed; these checks establish the native controls specifically.

HTML now waits for the actual canvas viewport before its first Load and keeps
Find behind that Load. Loading stays inside the body allocation. Current-message
frames for obsolete viewport/scroll geometry cannot replace the displayed frame;
resource discovery survives rejection, and incompatible width/scale bitmaps
are never stretched into a new layout. Each renderer reuses its own font system
across documents, without locking iced's fonts. Same-size repaints clear/reuse
the pixel allocation, and redundant unchanged viewport requests skip repainting.

A separate background worker prepares the first viewport of up to two adjacent,
already cached messages. Its replaceable mailbox drops obsolete queued work.
The UI retains at most four prepared frames / 32 MiB, keyed by message/body,
geometry, font, quotes, image permissions and cached-image revision. Preparation
never fetches resources; permitted cached WebP bytes decode only if the document
uses them. Document handles, glyphs and decoded resources remain isolated. The
active renderer remains available independently and builds the selection/Find
state after a prepared frame is shown.

All 294 Rust tests pass (two opt-in live diagnostics ignored), along with all 24
Python tests, formatting and all-target/all-feature Clippy with warnings denied.
Five new Rust tests cover geometry/Find ordering, stale-frame isolation, image
permission/source/layout cache keys, bounded LRU behavior and replaceable
preparation with seeded images. Three saved native scenarios pass; the scaled
scenario was corrected after visual inspection of the actual upward-opening
preferences menu. The HTML flow verifies a cache hit before adjacent-message
navigation, final-message text after rapid selection, End/Home, pane dragging
and compact resize. Reviewed evidence is in `artifacts/e2e/5eba7ad959ed/`.

The full native suite, optimized installation and publication are recorded below
after completion. R69 remains open for late-discovered image/horizontal controls,
compact reading space above wrapped attachments and remaining visual readiness
work. R70 badges and the rest of the full product backlog remain open. No latency
claim is inferred from these functional tests; performance measurements stay
deferred and quality/release workflows stay disabled. Logs use
`artifacts/logs/html-preparation-*`.


The complete native run passes all 98 functional scenarios in one run, including
existing HTML selection/Find/quotes/images, wrapped attachments, forwarding and
actual browser printing, context menus, read/unread, immediate toasts and Undo.
The log is `artifacts/logs/html-preparation-full-native.log`. Additional reviewed
captures include `artifacts/e2e/7c888b317ab3/` (dark/compact HTML) and
`artifacts/e2e/e5674c505059/html-wide-table-right.webp` (horizontal panning).
No performance measurements were run.


The optimized production package passes SHA-256, extraction and bundled-installer
verification. The user installation matches `target/release/shep` at SHA-256
`72db7cbaf528a24a2bcb9a2a628836f13b144279537271bc0a0e4fe7a3d20dbe`.
Existing personal windows remain open and need reopening for the new build.
Strict documentation validation passes. No root logs were created. Publication
is recorded below after the authorized main push.


Shipped to main as `cd8f7321aa0360c8ad05cb7648f4f86359a2636e`. The native refresh-control
interpretation of R71 is complete: both Mail and Calendar use the corrected SVG,
with normal/scaled/light/dark/compact visual evidence and saved native tests.
R71 is removed from TODO on that basis; the browser crop itself was not reproduced
as a separate browser UI defect. R69 remains in TODO for its remaining layout and
readiness work, and the full-product goal remains active. The verified Linux
installation is already in place; personal windows were not terminated.

[Documentation run 34057767845](https://github.com/sam-ruff/shep.so/actions/runs/34057767845) passes for the shipped code commit.

## HTML control placement and compact reading space — R69 follow-up

Image-policy metadata now includes CSS/legacy backgrounds and base-relative image
URLs, prepared alongside the parsed HTML on the backend. The blocked-image bar
exists before rendering, including a CSS-only image fixture. The renderer still
requests only resources used by the document, and existing image permissions and
network validation still govern downloads. Metadata discovery does not fetch.

Wide-message panning now uses a track inside the visible body, with a small band
reserved from the initial layout. It no longer inserts a control above rendered
text. The thumb responds to input before the worker repaints; obsolete horizontal
frames cannot move it back. Dragging, Shift+wheel and focused Left/Right support
panning, while Find input retains its own arrow keys. Selection and Find
highlights are clipped above the track.

Compact readers use a shorter sender header with measured address ellipses,
two-column attachment buttons and a combined action/navigation row. The 900×640
dark Prototype fixture now has readable body space with all four attachment
controls visible. Full sender addresses remain available through the sender
dialog. Reply all uses its icon and existing shortcut-aware tooltip in this view.

Superseded document loads are discarded before unnecessary parsing. A native
resize regression also exposed Find results arriving before a usable layout
frame: one current-query result is now retained until its matching frame arrives,
with stale query/document rejection. The regression is covered by both a Rust
ordering test and the existing real-input compact Find flow.

The new automated native scenarios use a controlled renderer delay to compare
loading/ready body origins and exercise the in-body track, and a compact preview
to check visible reading space and forwarded attachment retention. Reviewed WebP
evidence includes `artifacts/e2e/7ef264c09b04/html-css-loading.webp` and
`html-css-ready.webp`, `artifacts/e2e/3bbd2c4fa8f0/html-compact-reading-space.webp`,
`artifacts/e2e/3aeebf82797e/find-wide-dark-compact.webp`, and
`artifacts/e2e/fc35200f7734/html-compact-image-menu.webp`.

All 100 native functional scenarios pass in one full run, alongside formatting,
Clippy with warnings denied, 299 Rust tests (two opt-in live diagnostics ignored),
and 25 Python tests. The optimized production package passes checksum, extraction
and bundled-installer verification. Logs are under `artifacts/logs/html-layout-*`;
no root logs were created. R69 remains active for preserving reading position when late images change document
dimensions and the remaining readiness review. Performance measurements remain
deferred; controlled fixture delays are correctness evidence only.

The user installation matches the tested optimized binary at SHA-256
`8de511f5d3f57cf511b8219fc10e5db1a01ea035cd537f54c3aaa7b4d7257483`.
Personal windows were left running. Strict documentation validation passes.

Shipped to main as `6d83b728a0b8fbec239a42a454b179026629d7ba`. The Linux
installation is verified, all 100 native functional scenarios pass in one run,
and the full-product goal remains active. R69 stays in TODO for the explicit
remaining image-arrival and readiness work.

[Documentation run 34061562636](https://github.com/sam-ruff/shep.so/actions/runs/34061562636) builds and deploys the shipped code successfully.


## Reading position during image arrival and renderer recovery — R69

A saved native fixture reproduced a 400-pixel displacement when two images
without a fixed height arrived above the visible paragraph. The renderer now
retains a visible text-node anchor across image layout, prepares the corrected
viewport pixels, and coordinates the native scroll adjustment. Subsequent image
layouts coalesce until acknowledgement; input and replacement documents continue
through the existing channel. Images below the viewport do not move its content.
The native operation checks document/view version, scroll position and geometry,
so an old result cannot overwrite newer navigation. Acknowledgements never replace
newer viewport observations. Find and selection use the corrected layout.

The renderer regression compares the before/after pixel buffers exactly, exercises
multiple arrivals and Copy while the scroller acknowledgement is pending, and
checks images below the viewport and old document acknowledgements. Native tests
cover the same two-image arrival in light mode and a compact dark reader with an
active Find match, plus navigation to another message before images arrive. The
reviewed captures keep paragraph 24 and the paragraph-30 Find match at the same
screen position. Evidence includes `artifacts/e2e/d0907ab46ccf/`,
`artifacts/e2e/4e0da012e9be/` and `artifacts/e2e/355ea02c14b9/`.

Recoverable renderer errors now offer Retry formatted message. Retry preserves
the selected mail, creates a new render generation, waits for its real viewport
and rejects old errors. Plain text stays available. An unavailable worker keeps
its explicit reopen instruction. The isolated failure/native Retry capture is
`artifacts/e2e/68eb7d07359e/html-render-failure.webp`.

The first full run also exposed two older tests that could capture nullable
read/flag values before the initial detail arrived. Those tests now await known
initial metadata before taking their snapshots. Immediate-feedback and rollback
assertions are unchanged, and the failing scenario passes after that correction.

Formatting, Clippy with warnings denied, 305 Rust tests (two opt-in live diagnostics
ignored), and 27 Python tests pass. All 104 native functional scenarios pass in
one full run. Final reviewed captures include `artifacts/e2e/c5f648fefa94/`,
`artifacts/e2e/1e0e97d0a409/` and `artifacts/e2e/d4b91c137b88/`.
Release checksum, extraction
and the bundled installer pass. Performance measurements remain deferred to the
final idle-host gates; no latency claim follows from these controlled delays.

The installed Linux release matches `target/release/shep` at SHA-256
`a7803f07d0a3b6b3baed85c98a73ca5834b31db61bd97c635b0ac892c05f2273`.
Personal windows remain untouched; reopening starts the new binary. Logs are
under `artifacts/logs/html-anchor-*`. Strict documentation validation passes.

Shipped to main as `cc38af9d04eca6284214a030d5015262d19effed`. R69 is
closed after the full native run, reviewed captures, installer verification and
push. Final idle-host performance gates remain R03/R09; the full product goal
remains active.

## Linux unread launcher integration — R70, with R50/R60 count reconciliation

The Linux adapter publishes the native Unity LauncherEntry protocol for Shep's
installed desktop ID. A dedicated async worker receives only the newest unread
count through a watch channel, retains its session-bus connection, republishes
on dock-owner changes and reconnects after bus loss. Zero hides the badge.
Preferences has a searchable, saved toggle. Counts include all connected mail
accounts, independently of the currently selected folder or unified-inbox choice.

The existing optimistic count adjustment depended on visible page rows. Pending
read/move/Undo identities are now observed in the same database snapshot as global
counts. The UI projects their intended membership separately from page contents,
rejects snapshots requested before a new intent, and avoids applying changes twice
when SQLite has committed before iced receives an acknowledgement. This includes
Inbox destinations and known cross-account destination identities. Remaining
ambiguous provider outcomes and durable recovery stay in R50/R60.

Rust tests exercise actual private-bus Update/Query messages, hiding at zero,
dock-owner replacement, rapid count replacement and lost-bus recovery. Other tests
cover empty filtered pages, read failure, Inbox moves, rekeyed cross-account Undo,
SQLite membership snapshots and preference persistence after reopening. All 313
Rust tests pass (two opt-in live diagnostics ignored), alongside formatting,
Clippy with warnings denied and 29 Python tests.

Four saved native badge scenarios pass through the MCP harness, which starts an
owned private bus and observes actual protocol messages. They exercise background
arrival, read changes across folder navigation, archive/delete/move with Undo,
failed archive rollback and the preference. Recent count history catches transient
regressions. Final full-suite, compact visual and shipping evidence follow below.
No personal desktop bus, mail account or process was used by these tests.

This increment implements Linux publication. Actual dock rendering review,
Windows/macOS adapters and execution remain open under R70; unsupported platforms
do not show an ineffective preference. Protocol observations alone are not a
GNOME screenshot. Performance measurements remain deferred to R03/R09.

The first full native run completed 108 scenarios with one failure in the new
compact preference test: it clicked before the complete search query/result had
settled. Waiting for that exact native query/result fixes the scenario; all four
badge scenarios then pass again with service activation disabled on the private
bus. A fixture-bus unit test also verifies that no portal/keyring service is
activatable. No processes with a stopped fixture's badge-bus address remain.
The final full rerun uses that corrected harness and scenario.

Reviewed preference evidence includes `artifacts/e2e/702328ea573b/` and
`artifacts/e2e/8c4fb25cf9f1/badge-preference-compact-dark.webp`. The compact capture
also exposes an existing clipped tab/scale-fragment rendering issue, retained
explicitly in R15/R17/R21. Badge controls are visible and operable; this is not a
claim that the remaining preferences polish is complete. The optimized release
passes checksum, extraction and bundled-installer checks.

The final full native rerun passes all 108 functional scenarios. Its badge
preference evidence includes `artifacts/e2e/b6b3d7b40d33/`; the private-bus log
confirms no service activation. Formatting, Clippy, 313 Rust tests (two ignored
live diagnostics), 29 Python tests and strict documentation validation pass.
Logs remain under `artifacts/logs/badge-*`; quality/release CI stays disabled.

The installed Linux binary matches the verified optimized release at SHA-256
`745bae0c21cf1c908c4b5f17185242f7e25aeffaf12495f22e974c6b17b5b738`.
Personal windows were left running; reopening uses the new binary. R70 and the
full product goal remain active for their explicitly recorded remaining work.

Shipped to main as `90776fad7d344eeada6ba910e0aad4277170b157`. The Linux
installation and all 108 native functional scenarios are verified. R70 remains
open for its platform/rendering/recovery follow-ups; the full goal is active.

## 2026-09-07 — Preferences clipping and refresh verification

Compact Preferences could draw part of the Interface size dropdown below its
scroll viewport. Searching for a setting left those pixels behind until a full
repaint. The saved native regression reproduces that behavior before the fix
(`artifacts/e2e/685033e171ef/`) and checks the empty margin after filtering and
after resizing by one pixel and back.

The released iced 0.14 software renderer treated a cached text viewport as if it
were the glyph bounds, sometimes omitting its clip entirely. Shep now patches
that renderer to intersect the local viewport and damaged layer for cached text.
Raw text also resets its shared mask so a preceding label cannot clip it. Normal
partial redraws remain enabled. Only `engine.rs` differs from the upstream src/
copy; the release archive includes its MIT license and patch provenance.

Three direct renderer tests cover partially/fully scrolled text, damage-region
intersection and clip-mask ordering, using the bundled Noto Sans font. All 316
Rust tests pass (two opt-in live diagnostics ignored), along with Clippy with
warnings denied and 29 Python tests. The saved native clipping test passes and
its corrected captures are in `artifacts/e2e/b5e139f565dd/`.

The existing Mail/Calendar refresh scenarios pass again, including 120% interface
size and compact dark mode. The native icons were visually reviewed in
`artifacts/e2e/1741714f5abe/` and `artifacts/e2e/7480b4601efe/`; their geometry is
correct. This verifies the native icon already shipped for R71, not the browser
chrome in the original crop. No performance timings were measured.

The first full run completed 109 scenarios with two failures: the Google
permissions flow clicked Backups before the returned Preferences layout was
ready, and rapid HTML navigation queued subsequent clicks without observing each
intermediate selection. Both saved flows now observe native UI state before the
next click. They pass individually; HTML navigation still does not wait for
intermediate body rendering. The final complete rerun passes all 109 functional
scenarios. Reviewed final preference evidence is in
`artifacts/e2e/fae64a84b9dc/`, and rapid HTML navigation/resize evidence is in
`artifacts/e2e/84a416a2c2e2/`. Logs are under `artifacts/logs/preferences-clip-*`.

Shipped to main: `0d2feb1ab5cf4885aa8a02ec3cb24134204072de`. Formatting,
Clippy, all Rust/Python tests and strict documentation validation pass. The
optimized archive passes checksum, extraction and bundled-installer checks.
The installed user binary matches the release at SHA-256
`e3f8577cd11b9671874179cf6663bf9fd7d40801b3494641287751449b0cdc28`.
Personal windows were left running. Quality/release workflows remain disabled;
performance and the remaining product TODO stay open.

Built-in imagegen was used for transparent logo extraction. Light and dark
candidates and prompt provenance are saved under ignored
`artifacts/imagegen/launcher-alpha/`. The dark extractions have visible edge
defects and were rejected. Approved production assets remain unchanged; R64 is
open for a clean cutout and desktop theme integration.

## 2026-09-07 — Selection storage groundwork for R42

Inbox queries and captured selections now share a single scope/ranking plan.
Selection membership and its ordered positions live in process-local SQLite
tables. Capturing a query includes all matching pages without sending all IDs,
message bodies or MIME to iced. Counts and requested visible membership are small
results, and selected metadata is read in pages of at most 50 messages.

The store API supports individual selection/deselection, replacement, ranges in
both directions across page boundaries, additive ranges, all and clear. Revisions
reject stale changes and page continuations; failed changes roll back atomically.
A review gets its own immutable membership snapshot, independent of later inbox
selection or scope changes. New arrivals do not join an existing selection, and
selected versus available counts make disappeared messages explicit. Metadata
pages read current flags/folders. Snapshots must be released by their caller and
do not survive closing or opening another Store connection.

Six integration tests exercise matching membership/order for every sort and
search mode, combined folders, Sent mappings, unread/read/flag/attachment filters,
cross-page ranges, stale/failing changes, review isolation, arrivals/deletions,
current flags and connection/restart isolation. All 322 Rust tests pass (two
opt-in live diagnostics ignored), as do Clippy with warnings denied and 29 Python
tests. Five existing native MCP regressions pass for best-match search, flags and
paging, combined sidebar folders, unread dock counts and cached navigation. These
native flows exercise the existing inbox after the query refactor; they do not
demonstrate multi-selection controls. Logs are in `artifacts/logs/selection-*`.

R42 remains open. Next work must connect native controls and optimistic selection
through bounded channels, clean up abandoned snapshots, and implement reviewed
bulk execution with immediate feedback, per-message outcomes, grouped Undo and
durable recovery. Temporary selection snapshots alone are not an operation
journal. No new multi-selection UI is claimed by this checkpoint. Performance
measurements remain deferred.

Shipped to main: `0833456`. Formatting, Clippy, all Rust/Python tests and strict
documentation validation pass. The optimized release passes checksum, extraction
and bundled-installer verification. The installed user binary matches SHA-256
`2670efb78c57edd8a3bc6da1123c6259524be68ba5149ca6203edc95a6ee23d2`.
Personal windows were left running. Quality/release CI remains disabled, and the
full product goal and R42 remain active.


## 2026-09-07 — Native selection controls in development (R42)

The working tree now connects selection storage to native Select/Done, padded
checkboxes, Ctrl-click, Shift-click ranges and a remappable Select All action.
Selection survives paging, clears immediately when the query scope changes, and
keeps the reader anchor separate. Clear unchecks messages; Done/Escape leaves
selection mode. Repeating Select All explicitly captures arrivals that did not
join the earlier snapshot. Checkbox/modifier gestures preserve the open message
and do not count every chosen row as read. Modified double-clicks do not open the
full reader; an ordinary double-click still does.

A separate bounded FIFO channel handles selection independently of provider work
and draft/settings saves. The UI projects one visible page immediately, keeps at
most 32 queued gestures and one request in flight, and releases an obsolete
snapshot before starting another. Five controller tests cover immediate
projection, cross-page ranges, bounded input, cancellation/cleanup, stale replies,
arrival handling, input focus and errors. The provider-saturation regression also
executes selection capture/change while every network slot and queue is occupied.

All 327 Rust tests pass (two opt-in live diagnostics ignored), along with Clippy
with warnings denied, formatting, 29 Python tests and strict documentation build.
The first full native run passed 111 of 112 functional scenarios. Its sole failure
was the existing Preferences-to-Mail resize scenario: the drag did not change
the divider. Its saved flow now allows 150 ms for presentation and captures the
returned layout before dragging. On the rebuilt current test binary, that flow
and all three selection scenarios pass. This is a targeted rerun, not a claim
that the complete 112-scenario run was green. Logs use the ignored
`artifacts/logs/selection-controls-*` and `selection-native-*` paths.

Reviewed current selection evidence is in `artifacts/e2e/f4503f4c6b5b/`,
`artifacts/e2e/0ca3ac406efe/` and `artifacts/e2e/e45b797e0557/`; corrected resize
evidence is in `artifacts/e2e/4e337cae83fa/`. The saved refresh scenarios also pass
in light/dark, compact and 120% layouts (`eb6fea58a85d/` and `65f4c08de024/`).
These verify native Shep controls, not browser chrome in the user's original crop.

This UI work is **uncommitted, uninstalled and unshipped**. R42 remains open:
preview toolbar/keybinds still need selected-group scope, exact frozen reviews,
Y/N/Enter/Escape, bounded execution, immediate projected results, partial failures,
grouped Undo and durable recovery. Do not treat the controls alone as delivery or
install them over the user's working app before that integration. Finish remaining
selection conventions alongside bulk work, including retaining a range anchor
when Select All recaptures membership.

The installed optimized binary remains the shipped `0833456` checkpoint, SHA-256
`2670efb78c57edd8a3bc6da1123c6259524be68ba5149ca6203edc95a6ee23d2`.
Main and origin/main are at audit commit `2e50d50`; its documentation workflow
and the preceding code workflow succeeded. Personal windows remain running.
Performance measurements remain deferred; quality and release CI remain disabled.


## 2026-09-07 — Integrated selection and durable bulk actions

The selection UI now drives group Archive/Trash/Move, read/unread and flags.
Multi-message actions review frozen membership and accept Y/N/Enter/Escape.
Default moves keep each message in its own account; enabled cross-account moves
retain the existing IMAP policy. Select All preserves the range anchor, and
mouse selection remains independent of reading/text selection.

The persistent group journal stores metadata and exact per-message receipts.
Indexed counters, bounded pages, a coalescing wake channel and one provider slot
keep group membership outside UI memory. Pending effects drive ordinary queries
while provider operations use actual cached identities. Whole-group unread
counts and weighted toasts project immediately. Undo cancels queued work,
reverses acknowledged moves/flags, and waits for a running forward receipt.
Page phase observations prevent double projection before acknowledgment.
Unconfirmed outcomes are retained for explicit review; definite inverse failures
can retry. A file lease prevents another process replaying a live job.

Graceful close finishes the current receipt and leaves queued items for a fresh
engine. Account-removal reviews fingerprint related group entries, require
cancellation for unfinished work and remove only affected history. Ten bulk
storage tests, six engine group tests, UI snapshot/Undo tests, the native-widget
click regression and account-removal coverage join the suite: **347 Rust tests pass**, with
two opt-in live diagnostics ignored. Clippy, formatting and 29 Python tests pass.

Four saved native bulk flows cover mouse and keyboard scope, multi-page review,
red Trash confirmation, cancellation, forced slow pending Undo, read/unread,
flags, mixed-account Projects moves, failures and History details in normal and
compact dark layouts. Final verification also caught queued clicks using the
batch-final pointer location; the root wrapper now preserves each motion before
scroll transforms. A deterministic native-widget test covers two clicks in one
batch. Stop-at-entry now acknowledges a pending close without claiming a message.

All **116 native functional scenarios pass**, run as two consecutive 58-case
batches on the same final binary: artifacts/logs/bulk-final-native-a.log and
bulk-final-native-b.log. The final Rust/Clippy logs are bulk-pointer-all-rust.log
and bulk-pointer-clippy.log; Python reports 29 passing tests. Strict Zensical
builds pass. Optimized checksum/extraction/bundled-installer verification passes
in bulk-pointer-release.log.

Installed atomically for the Linux user; the existing running window was left
alone. The installed binary matches target/release/shep, SHA-256
`f189f322b567bb07b02fdcc1fc773a17b633085ecc0cfcff8db59684d9aa93d9`.
Shipped to main as `af056a9c7f925b901052ae61b56afa840417143d`.
Quality and release workflows remain disabled; documentation CI stays enabled.

The existing native refresh-icon fix was reverified in light/dark, compact and
120% layouts. Final reviewed captures include artifacts/e2e/7a94b5a1f5a6/
and artifacts/e2e/2bd3f73aad10/. The arrows render cleanly. These are native Shep
checks, not reproduction of the browser chrome in the original crop.

The full product goal remains active. Remaining platform/provider verification,
optimistic ambiguity/restart work and the rest of TODO are not completed by
these fixture results. Performance measurements remain deferred.

R42 remains open for explicit selection of new arrivals without losing existing
membership, group/individual mutation coordination and the remaining native
History/recovery paths. These are tracked explicitly in TODO; passing the current
fixture suite does not establish live-provider or cross-platform coverage.


## Client worktree integration of committed desktop main

The review worktree integrates desktop main through `10d8569` (33 commits) while preserving the client work under `flutter/`, `web/`, `website/` and `backend/`. The separate main worktree's uncommitted selection/mutation work is untouched. Imported main evidence above describes its original commits; the following checks exercise the merged source.

Mail protocols remain in `shared/mail-core`, including pinned/routed gateway connections, acknowledged move recovery and exact SMTP identities. Desktop render payloads extend the shared detail type without introducing iced or storage dependencies into the transport crate. Forward formatting metadata and MIME assembly are shared; `build_with_message_id` and browser reply contracts survive the merge. The desktop renderer and printing now consume shared MIME representation selection. A single-document adapter rebinds CID references before combining related sections, handles CSS URL tokens and each section's base URL, and leaves ambiguous references unresolved. It does not sanitize HTML itself; native rendering and print confinement remain responsible for that boundary. Tests cover independent/ambiguous CID scopes, CSS references, mixed text and section bases, retained forward files/formatting and rejection of damaged forward resources without a partial draft. Readable cached text remains available when an optional inline resource is damaged.

The merged source passes formatting/Clippy and **372 root/shared Rust tests**, with two opt-in personal-account diagnostics intentionally ignored; **52 native bridge tests**, **34 gateway tests**, **43 Flutter host tests**, **69 browser unit tests**, **26 browser Playwright scenarios**, **49 actual Rust HTTPS stages**, **39 Python tests** and **27 parity review contracts** also pass. Both Flutter browser paths pass: nine formatted-reader stages and six standard stages. The first standard Flutter browser runner terminated with exit status 143 before its result; the unchanged rerun passed. The complete iced functional run passes **116/116** in one run. This does not erase the earlier two 57/58-run limitations recorded above or establish final timing budgets.

The optimized Linux production archive passed checksum verification, extraction and the bundled installer in temporary directories. No personal desktop installation was replaced. Reviewed native captures include immediate archive/group Undo, compact per-message failures, Find's final visible match, styled HTML and its inline image in dark compact mode, forwarded attachments, the actual print dialog and a browser-created styled PDF. Full header, reflow, selection, native printing and other saved scenarios remain in the suite. Evidence is under ignored `artifacts/logs/desktop-integration-*` and `artifacts/e2e/`.

The first full Android run passed swipe, native account/draft and composition scenarios, then its incoming picker helper acted on an old DocumentsUI dump and stalled. The failure log and screenshot are preserved. Helpers now remove the previous dump before observing, reject empty/malformed observations and wait for an explicit first-save intent. A regression reproduces a successful dump command that writes nothing, then recovery with current controls. The next full run passed through incoming save/Find/removal, then the formatted-reader assertion observed Flutter’s Find count before the native WebView applied its highlights. The test now awaits the actual read-only DOM highlight/scroll observations with the existing bounded deadline. Its failed log is preserved. The older Outbox handover assertion also expected a plain text widget despite the current editable/selectable reader. It now uses the same read-only body observation as the preceding Sent assertion, allowing MIME terminal whitespace. The native repository test still checks exact stored body text. Targeted completion and final shipping evidence follow below.

Mobile/browser Forward/Print, selection/bulk, current read-on-leave/toast defaults, remote-image policies/anchors, OS badges/background lifecycle and the broader provider/calendar/backup gaps remain active. Apple execution, live providers, final idle-host performance and VPS deployment are unverified. R75 Google provider sign-in and R76 scheduled grouped Automatic replies remain TODOs. This integration ships only to the authorized client review branch; quality/release workflows stay disabled.


### Android integration completion

All **14 Android integration scenarios** are verified across the full runs and targeted reruns after the automation corrections, with **five formatted-reader Appium stages** and **six standard Appium stages** passing. This is not a claim of a single uninterrupted green Android wrapper run. Actual controls cover swipe/remapping/Undo, native profiles and secure-storage roundtrip, drafts and credential handover, real attachment selection/save/cancel, cached reader/Find/removal, formatted HTML/Copy/links, durable Outbox/local/provider Sent recovery and independent-process lock exclusion. The formatted and Outbox reruns pass after awaiting the native render result and observing the current selectable body. Current light/dark captures were regenerated from this run's PNGs and reviewed, including native selection, visible Find highlights, account removal and Sent handover. Production APK inspection and the prompt review-branch shipping commit are recorded below.


The production Android APK contains all three Rust ABI libraries, has Internet permission and excludes fixture markers; its SHA-256 is `4ae8c6141583c4b8e3fb95d2b6c30af3dc7c1b077234234e0efda0545437f4a4`. It is a development-signed build, not a store-ready signed release. The Linux production binary SHA-256 is `2cf7a67aced01eaf394bd4f41f9f531532469d95dd2c7d5a20e67171adf0cd64`; archive `3d9176fc540ff7df49459e6ea37d9a1853c25374e4cf77e9acb599729a7ea30f` passed the bundled installer verification. Final Flutter analysis and pinned strict documentation validation pass. No client feature is marked complete solely by this integration, and no personal installation or VPS deployment is included. The merge commit and verified remote push follow in the shipping record.


### Desktop integration shipping record

Committed and pushed as [`72ca625`](https://github.com/sam-ruff/shep.so/commit/72ca625961bee619747e98110adcd2dfe22eb8c0) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required hooks passed formatting, Clippy and all **372 root/shared tests**; two personal-account diagnostics remain intentionally ignored. The tests, production artifacts, native visual review and corrected Android automation are recorded above. R77’s prompt-push request is fulfilled for this increment. The dedicated emulator is stopped, artifacts remain ignored, and the separate main worktree and personal installation were not changed. Full parity, Apple execution, R75/R76 and VPS deployment remain active. Quality/release definitions remain disabled; re-enable them when trusted runners and release prerequisites are ready.


## Shared complete-source Forward preparation

Forward preparation now lives in `shared/mail-content`, callable by native Rust and browser WASM workers. The desktop delegates to a shared-core wrapper that assigns independent draft/file identities and leaves recipients and reply-thread headers empty. Complete source text, retained HTML/styles and exact attachment/inline bytes share one selector. Editing the original quotation switches outgoing MIME to the edited plain text and keeps inline images as ordinary files; a note above the intact quote retains formatting. Existing root transactional draft tests still cover reopen, failure rollback and damaged resources without partial drafts.

Attachment metadata comes from its actual MIME part. Duplicate names and identical bytes no longer inherit another file's media type; explicit attachments cannot supply an inline image's type. Suggested names use the shared path-safe filename policy. Native/WASM fixtures also cover independent and ambiguous CID scopes, alternative selection, exact binary bytes and malformed-resource recovery. Count, byte and pre-parse nesting limits remain unchanged; this does not fulfill streaming/large-mail work. WASM owns the prepared files separately from metadata JSON and exposes exact byte copies; callers must free the result and atomically persist the new draft with all files. Retained outgoing HTML is not a safe display document.

The browser passes **71 unit tests**, **26 Playwright scenarios** and **49 actual Rust HTTPS stages**. The native bridge passes **52 Rust tests** and Clippy; the gateway passes **34 tests** and Clippy, with the separately invoked HTTPS test accounting for its ignored browser harness. Python reports **39 passing tests** and the parity checker validates **27 contracts**. Shared core tests verify new draft/file ownership, a reserved SMTP Message-ID, complete MIME roundtrip and edited-quote fallback. All five existing desktop Forward flows pass. Reviewed current captures include `artifacts/e2e/e9d894c86f1b/forward-composer-files.webp`, `4739ff2b1a87/forward-dark-compact.webp` and `106c4f7fbcfe/forward-pending-preserves-editor.webp`; attachments, blank-recipient recovery and independent editors remain usable. Full-suite, packaging and shipping results follow below. Logs are under ignored `artifacts/logs/shared-forward-*`.

This increment does not add Flutter/browser Forward controls. Their atomic draft/file storage, quote metadata through autosave, inline identities through native persistence/Outbox and gateway attachment payloads, and actual control/device execution remain in TODO. Apple/live-provider/final performance evidence, broader parity, R75/R76 and VPS configuration remain open. Current work ships only to the review branch; no personal installation, main merge or VPS deployment is included. Quality/release workflows remain disabled.


The complete desktop functional run passes **116/116** in one run (409.421 seconds), including Forward, HTML/Find, Print, selection/bulk, read-on-leave, account/settings and recovery regressions. Shared-content Clippy also passes for the actual WASM target with warnings denied. The inspected production browser WASM has SHA-256 `645ad52b01117b03bebdcdc49e7237f63105d78b81be001fbee81d2ad5c6f19f`; the build excludes the new synthetic message/file markers. No Android or Apple UI execution is claimed for this preparation-only increment. Optimized Linux packaging and final hook/shipping evidence follow.


The optimized Linux archive passed SHA-256 verification, extraction and the bundled installer in temporary directories. Binary SHA-256: `93fca421da12c47c97efd699b1ee30bfae31eb229167c007287d1bca6fbda821`; archive: `e4eb45879046c0522aed8bc0eabf97d4e8b494a5511589d0d8ec62196094986c`. New fixture markers are absent. No personal desktop installation was replaced. Pinned strict documentation validation and the required commit hooks precede review-branch shipping.


### Shared Forward preparation shipping record

Committed and pushed as [`64d4936`](https://github.com/sam-ruff/shep.so/commit/64d493605a14f6a7c44e627be1d5423cc3f01772) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **377 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The complete 116-flow native run, shared native/WASM fixtures, browser/HTTPS regressions, native bridge/gateway tests, reviewed screenshots and production package checks are recorded in the completion log. Artifacts stay ignored and the temporary target symlink is removed. R77's prompt push is fulfilled for this increment. Full parity, Flutter/browser Forward controls and durable metadata, Apple/live verification, R75/R76 and VPS deployment remain open. Quality/release workflows remain disabled; re-enable them when trusted runners and release prerequisites are ready.


## Native Flutter and browser Forward controls

Both clients now prepare a Forward from complete cached MIME on independent background capacity. They atomically save a new draft and all exact file bytes, with blank recipients and no reply-thread headers. A retained quotation keeps authored HTML and inline Content-IDs through autosave, restart and reviewed Outbox recovery; editing the quotation uses the shared plain-text fallback. Preparation uses two slots independently of provider/reader work. A failed or lost acknowledgment retries the same draft identity without overwriting edits, removed files or protected delivery/discard state. A late completion remains in Drafts and leaves a newer editor or reader open. Browser Forward is remappable, clearable and inactive in text fields.

Native cache version 9 adds inline-file and Forward-origin tables without changing existing file rows. One transaction owns draft creation and all files; tests inject a failed inline insert, account removal and response loss. Text saves retain backend-owned quotation/file metadata. Outbox return clones file identities and preserves Content-IDs. The gateway accepts inline identities through its attachment contract and tests retained HTML/exact binary MIME, a reserved Message-ID and refusal of Content-ID header injection before any transport call.

The native bridge passes **57 Rust tests** and Clippy; the gateway passes **35 tests** and Clippy, plus the separately selected real HTTPS browser harness. Flutter analysis and **44 host tests** pass. The browser passes **74 unit tests**, **30 Playwright scenarios** in one complete run, and **50 real Rust HTTPS stages**, including production Forward worker/CSP and saved-draft reopening. Python reports **39 passing tests**, and the parity checker validates **27 contracts**. The Android Forward scenario passes against actual FFI/SQLite, including restart, missing-credential Send refusal, damaged source, text beyond the cached preview and a held preparation while editing another draft. Composition/Outbox/Appium regressions and final production artifact results follow below.

The initial browser scenario had a Node JSON-import error and incorrect labels/fixture-text expectations. A full run then exposed an observation racing optimistic file removal; it now awaits enabled controls before comparing persisted files. The HTTPS scenario now waits for Save to close the editor before reloading. Android retries exposed an offscreen field observation and a remove-file click under the toolbar after the native keyboard appeared. The saved scenario now dismisses the keyboard through actual Android Back input, observes its closure, centers the file control and verifies hit testing before clicking. Failed logs remain under ignored `artifacts/logs/client-forward-*`; failed browser traces are retained under `artifacts/web/forward-failed-*`. No test budget or production failure guard was weakened.

Current light/dark composer, retry and independent-editor captures are reviewed under `artifacts/flutter/native/forward/`, `artifacts/web/forward-*` and `artifacts/beta-browser/forward-real-https-reopened.png`. The mobile title was shortened to Forward after the first capture showed truncation. Composer accessibility is tested on the active modal. The sender-authored purple-on-white HTML fixture exposed an existing lost legacy body-background/dark-contrast issue; it is explicitly retained in R38, separate from the new composer checks. Full visual/reader lifecycle parity is not claimed.

Apple execution, live providers, final idle-host performance, Print, full composition/multiple editors and the broader parity backlog remain open. R75 Google provider sign-in and R76 grouped scheduled Automatic replies remain TODOs. VPS installation still needs the actual target and owner/OAuth configuration. This increment ships only to the authorized client review branch; no personal desktop installation or main merge is included. Quality/release workflow definitions remain disabled.


The targeted Android attachment-composition and Outbox regressions pass, followed by all **six standard native Appium stages**. This turn reruns those three integration scenarios (Forward, attachments, Outbox), not the full Android wrapper or Apple simulator. Final Flutter analysis and all **44 host tests** pass. The production Android APK includes all three native ABI libraries, Internet permission and no fixture markers; SHA-256 is `8338f59a7f992fc60a7f6545749cc1d614e3a3498318d64df54e5a10001be783`. It is development-signed, not a store-ready release. The optimized Rust gateway builds successfully (SHA-256 `51fee2f9f03af892be8916018bd36e1ed981074a667f8d98c68a8a588742159b`). The inspected production browser build excludes fixture markers; its shared WASM hash remains `645ad52b01117b03bebdcdc49e7237f63105d78b81be001fbee81d2ad5c6f19f`. Pinned strict documentation validation passes. Root iced behavior and shared preparation are unchanged from the previously verified 116-flow/optimized-package checkpoint; that full native suite is not rerun for this client-control increment. Required hooks and exact remote shipping evidence follow.


### Client Forward shipping record

Committed and pushed as [`21ca299`](https://github.com/sam-ruff/shep.so/commit/21ca299907ef943aa4046166a3c934e15b4779b3) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the exact remote SHA verified. Required hooks passed formatting, Clippy and **377 root/shared Rust tests**; two personal-account diagnostics remain intentionally ignored. Client native/protocol/browser/HTTPS tests, reviewed captures, corrected automation and production artifact inspection are recorded in the completion log. The dedicated emulator is stopped and artifacts remain ignored. R77's prompt push is fulfilled for this increment. Main and the personal desktop installation were not changed. Full parity, Apple/live/performance evidence, R75/R76 and VPS deployment remain active. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Client complete-source printing

Flutter and the separate browser now expose Print from the reader. Shared Rust/native/WASM preparation uses the complete cached physical message (including aliases), the chosen Formatted/Plain representation, full quoted history, Subject/From/To/Cc/Date and attachment names. Bcc and reply-thread headers are excluded. The resource sanitizer retains bounded inline WebP images without fetching remote content. Native preparation has two independent slots; the browser owns at most two independent previews/workers. Navigation and a newer composer remain usable while the original source prepares. Errors offer retry/plain-text recovery; no dialog launch or cancellation is reported as a print receipt.

Android retains an owned, confined WebView until its native print adapter finishes. The saved Flutter/ADB scenario opens the actual system dialog, cancels, retries, selects Save as PDF and uses DocumentsUI to save formatted and long mail. The resulting ten-page long PDF contains the complete source ending. The formatted PDF was reviewed for readable headers, retained filenames, the authored purple text/white background and inline Shepherd image. Browser printing uses a separate protected preview with an opaque sandbox and only its fixed CSP-hashed runtime. Actual Chromium `window.print()` output is paginated and retains the same source content. The UIKit adapter is added with ephemeral WKWebView and native printing, but has not been compiled or executed on Apple hardware.

Verification: **381 root/shared Rust tests**, **58 native bridge tests**, **35 gateway tests**, **51 real Rust HTTPS stages**, **46 Flutter host tests**, **75 browser unit tests**, **39 Python tests** and **27 parity contracts** pass. The browser has **35 passing scenarios across the full and targeted runs**: the final full run passed 34/35 and failed only when exporting the already-validated PDFs because a test helper import was missing; the corrected PDF scenario passes. Earlier selector errors and the export failure remain in the logs. The original 33-scenario full run and both added pending/remapping scenarios also passed. The actual HTTPS flow verifies anonymous print-page denial, authenticated source display, production worker and containing CSP. Formatting, Clippy, final analysis, production artifacts and shipping evidence follow below.

The Android printer helper initially stopped at an unselected printer destination; it now explicitly selects Save as PDF. Its corrected full saved scenario passes, including an acknowledged completion handshake before fixture cleanup. **Six standard native Appium stages** pass afterward. Android captures and PDFs are under ignored `artifacts/flutter/native/print/`; browser PDFs are retained in `artifacts/web/print-output/`, with light/dark compact and HTTPS captures beside the existing evidence. These were visually reviewed. Build/test logs are under `artifacts/logs/client-print-*` and the saved `android-print-*` logs. Root iced presentation/provider behavior is unchanged; the prior 116-flow desktop run remains its latest native-control evidence, rather than claiming a new desktop UI run for these client controls.

The shared reader now respects legacy body background/text attributes instead of overwriting a white message background with the dark app theme; unspecified text on that authored background defaults to dark ink. Shared contracts and an actual dark browser reader check cover this correction. The wider R38 contrast/remote-image and native/Apple audit remains open.

Remaining Print parity includes Apple and other browser engines, native keymap configuration, policy-permitted cached remote images, large-document/lifecycle and final idle-host performance. Existing 25 MiB incoming and bounded resource limits remain. Full composition, selection/bulk, calendar/Google/backups and the rest of the product backlog remain active. R75 Google provider sign-in and R76 grouped scheduled Automatic replies remain TODOs. VPS deployment still needs the explicitly configured owner Google identity, OAuth configuration and SSH target. No main merge, personal desktop installation or live-provider/VPS verification is included. Quality/release definitions remain disabled.


Final Flutter analysis passes. The production Android APK includes all three Rust ABIs, Internet permission and no fixture markers; SHA-256 `ceec75aac0ac15e57207f5cb15a65ecb70603bd2ca08f19407b4fdba605859e8`. It is development-signed, not store-ready. Browser WASM SHA-256: `af68dd35fc8b176495f1b149845c558a31bc013343c51dccb4045c373f28dff9`. The optimized Linux binary (`93fca421da12c47c97efd699b1ee30bfae31eb229167c007287d1bca6fbda821`) and archive (`cbef659466e6533a6e2fbc8c8e554b470f874a9df152d4957fb5fb225d8ba3d9`) pass extraction/checksum and bundled-installer verification in temporary directories. No personal installation is replaced. The owned emulator is stopped. Pinned strict Zensical validation passes; required commit-hook and remote shipping evidence follow.

The final optimized gateway SHA-256 is `dd064c38fb2b487c1cebc54252de10f6a53743be0099c8cebba803b3ea6164c0`, with synthetic fixture markers absent. Its print-runtime hash matches the production browser; no VPS installation is included.


### Client Print shipping record

Committed and pushed as [`67f206c`](https://github.com/sam-ruff/shep.so/commit/67f206caf2a8cdab9ccf46ab66c2012f332360c8) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The client/native/protocol/browser checks, actual Android/Chromium PDFs, corrected automation, reviewed captures and production package checks are recorded above. R77’s prompt push is fulfilled for this increment. Artifacts remain ignored and the owned emulator is stopped. Main and the personal desktop installation were not changed. Full parity, Apple/other-engine/live/performance evidence, R75/R76 and VPS configuration remain active. Quality/release definitions remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Client read-on-leave and independent cached paging

Flutter and the browser now retain unread status while a deliberately opened message is being read. Leaving for another message, navigation, a composer or loss of foreground/focus queues a quiet read update. Startup selection, refresh and body preloading do not arm reading. Explicit read/unread controls cancel that visit so Mark unread survives later navigation. Read updates use the existing ordered optimistic mutation path before Move, preserve newer flags, retain a different reader, and expose rollback/retry without replacing Move feedback or Undo.

Flutter Back uses the actual route-pop callback; disposing a widget is only cache cleanup. Native page reads no longer wait for pending provider actions. Query-only Rust projections apply the latest fields before folder/account/search/filter/sort/paging and count calculations. One SQLite read snapshot returns the projected page, global unread count, aliases and confirmed metadata for rollback, without persisting optimistic fields. Identity adoption retains queued actions and the latest per-field versions. Pages captured across an action acknowledgment are retried independently of provider completion. The unprojected query keeps its indexed path; projected queries split edited rows from unchanged rows rather than joining every message against every pending edit.

The native bridge passes **59 Rust tests** and Clippy, including projected search/filter/paging/counts, aliases, original-cache preservation and reads with all provider capacity held. Flutter analysis and **53 host tests** pass; browser unit tests report **79 passes**, and one complete Playwright run passes **38 scenarios**. Python passes **39 tests** and the parity registry validates **28 contracts**. Eight Android native integration scenarios pass, including the new held-read/folder-navigation/failure/explicit-unread control flow. The additional incoming-file Android scenario verifies actual FFI/SQLite unread state before opening, after Back and after explicit Mark unread, alongside native save/cancel, move/refresh, Find and account-removal regressions.

Initial host assertions incorrectly assumed an empty Archive fixture and observed a write before its acknowledgment. The widget teardown check also exposed a read side effect during disposal; it now belongs to actual route navigation. An Android command omitted Rust from its PATH; the corrected environment passed. The new HTTPS observer initially looked outside the stored record’s `core` field. Its next run exposed overlapping saved-draft buttons: Drafts inherited the mail grid’s 5 px divider column. Drafts now uses its own scrollable list with correct counts, and a compact control test reopens all four saved drafts. The final Rust HTTPS run passes **52 stages**, including provider acknowledgment and actual IndexedDB read/unread state. Final production checks are recorded below. Failed evidence remains in ignored `artifacts/logs/client-read-*`. No failure guard, test budget or performance threshold was weakened.

The Android failure/new-reader and explicit-unread captures and browser read/failure captures were reviewed for legible feedback, preserved selection, spacing and visible controls. Root iced code is unchanged from the previously verified 116-flow/package checkpoint; that full native suite and Apple execution were not rerun for this client increment. Window/tab-close durability, forced termination, OS polling/error-retention lifecycle, large-cache/idle-host performance, counted move notifications and the wider parity backlog remain open. R75 Google provider sign-in, R76 grouped Automatic replies and VPS target/owner/OAuth configuration remain active. Quality/release workflows remain disabled. Commit hooks, artifact and exact review-branch shipping evidence follow.


The final production APK contains all three Rust ABI libraries and Internet permission, with no fixture markers; SHA-256: `898a31af8fbb3252a7fbf51a183d6443ba8c7252af175f04cad51f3ab3bf5b40`. It is development-signed, not store-ready. The browser production build succeeds; shared WASM SHA-256: `af68dd35fc8b176495f1b149845c558a31bc013343c51dccb4045c373f28dff9`. Final Flutter host checks pass **53 tests**, including held read/newer unread across alias adoption. The compact Drafts count/click regression passes after the full 38-scenario browser run. The real HTTPS result is a local scripted provider/Google fixture, not live Google, IMAP or VPS verification. Final Appium, strict documentation and commit-hook results follow.

All **six standard native Appium stages** pass against the rebuilt preview. The final browser production HTML/JavaScript/WASM contains no fixture markers. The Drafts list and real HTTPS unread captures were reviewed, and the pinned strict documentation builder passes. Only the nine relevant Android integration scenarios and standard Appium path were rerun in this increment; the full Android wrapper, Apple and idle-host performance remain separate work. Required commit hooks and the exact review-branch push are recorded next.


### Read-on-leave shipping record

Committed and pushed as [`5fc9560`](https://github.com/sam-ruff/shep.so/commit/5fc9560fda68109592e1be2aa6fcf16b0579d1c9) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The client native/cache/browser/HTTPS tests, nine Android integration scenarios, six Appium stages, reviewed captures, corrected failures and inspected production artifacts are recorded above. R77’s prompt-push request is fulfilled for this increment. The dedicated emulator is stopped and artifacts remain ignored. Main and the personal desktop installation were untouched. Full client parity, close/OS/Apple lifecycle, performance, counted notifications, R75/R76 and VPS deployment remain active. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Counted client move notifications and grouped Undo

Flutter and the separate browser now show the desktop’s six-second Archive/Delete/Move/Restored notifications. Repeated Archive/Trash actions group across accounts; other destinations group by exact account/folder. Each group keeps operation identities so a late failure removes only its own count. Dismissal and expiry survive late acknowledgments, and stale Undo callbacks cannot affect a newer group. General refresh/save feedback is independent of the move notification.

Grouped Undo restores optimistic metadata immediately, cancels moves still waiting behind read-on-leave, and waits for dispatched moves before reversing their acknowledged physical destinations. Failed reversals stay in a persistent review with Retry Undo/Dismiss. An acknowledged reversal with a metadata warning offers Refresh restored mail and cannot issue a second reverse operation. Current source-scope cache confirmation is required to clear that review. The existing native/browser durable provider intents and alias contracts remain authoritative; this increment does not complete move-history/restart/cross-account recovery.

Native Undo now retains metadata for every active group/failure, reconstructs off-page pending values from query projections and restores matching rows in date order. Page reads remain independent of provider completion. The footer uses the Scaffold’s measured navigation area, keeping Compose above notifications and recovery controls. Recovery buttons wrap on compact layouts. A retained reader’s flag update does not reinsert a row excluded by its authoritative page.

Verification so far: **59 Flutter host tests**, **84 browser unit tests**, **40 Playwright scenarios** in one complete run, **52 real Rust HTTPS stages**, **39 Python tests** and **29 parity contracts** pass. Flutter analysis passes. All nine Android native integration scenarios pass, including the new counted partial-failure/Undo/retry flow, paged pending restoration, read-on-leave, physical Sent identity/Undo, device cache/drafts, connection failure/reconnect and isolated device credential checks. A final native rerun also checks Undo inside a different open reader and independent Refresh feedback; final Appium, production and shipping evidence follows below.

Initial host failures exposed obsolete single-move labels, an unsent move correctly cancelled before dispatch, redundant unchanged-unread writes and a pending notification timer at teardown. Later control tests found missing off-page projection values and restored-row ordering, followed by Compose covering Retry Undo. These production issues are corrected and the saved controls pass; no assertion is bypassed. The first reader flow returned to Inbox after Archive. Its saved scenario now opens another reader before Undo and asserts its route; Flutter’s converted-surface capture retained an old Inbox frame, so the ordinary Appium binding provides the separate reader capture. Flutter lint findings were corrected. Failed logs remain under ignored `artifacts/logs/client-toast-*`. Android and browser captures are reviewed for readable feedback, row position, reachable recovery controls and preserved reader state.

The shared Rust/provider implementation and root iced UI are unchanged from their previous verified checkpoints; the full root native suite, Apple execution and idle-host performance are not rerun for this client increment. Full client parity, durable move history/recovery, close/OS lifecycle, Google provider sign-in (R75), grouped scheduled Automatic replies (R76), and VPS owner/OAuth/SSH configuration remain active. Quality/release workflows remain disabled. This increment ships to the authorized review branch only, preserving main and the personal desktop installation.


The final Android rerun passes all **nine native scenarios**. All **six standard Appium stages** and the **six Flutter browser stages** pass. Appium’s `reader-move-undo.png` shows the actual reader with Restored feedback; its screenshot is reviewed separately from the integration binding’s stale converted frame. Lossless WebP review copies are saved beside the counted Archive/partial-failure and browser captures. The final production browser build excludes fixture markers; shared WASM SHA-256 remains `af68dd35fc8b176495f1b149845c558a31bc013343c51dccb4045c373f28dff9`. The dedicated emulator and owned preview/Appium servers are stopped. This increment reruns the relevant nine Android scenarios and standard automation paths; the full Android wrapper, formatted-reader path and Apple simulator remain separate evidence. APK inspection, strict documentation and required commit-hook results follow.


The production APK passes inspection for all three Rust ABIs, Internet permission and fixture exclusion; SHA-256: `4b918a341780b4951c42e961eaa58a6a07cfb1f4d32d253b1ab46ab017965a92`. It is development-signed, not store-ready. Root Rust/shared code is unchanged; required formatting/Clippy/Rust commit hooks and the exact review-branch shipping record follow.


### Counted move feedback shipping record

Committed and pushed as [`96275a1`](https://github.com/sam-ruff/shep.so/commit/96275a18e5c623bc6837578bf4227dc16c339a0e) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the exact remote SHA verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Pinned strict documentation validation passes. The client host/native/browser/HTTPS checks, nine Android scenarios, six Appium stages, six Flutter browser stages, corrected failures, reviewed captures and production APK/browser inspection are recorded above. R77’s prompt-push request is fulfilled for this increment. The dedicated emulator is stopped and artifacts remain ignored. Main and the personal desktop installation were untouched. Full parity, durable move recovery/history, Apple/OS/close lifecycle, performance, R75/R76 and VPS configuration remain active. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Captured client selection prerequisites

Native selection now retains full query membership and ranks in a dedicated SQLite connection's temporary tables, using the same projected filter/search plan as ordinary mail pages. Calls return counts/groups and at most 50 observed identities or review rows. Revision checks protect changes and frozen membership; explicit arrivals preserve earlier choices, passive refresh does not grow the capture, aliases reconcile in bounded batches, and missing mail remains visible in selected/available counts. A separate 32-slot FIFO keeps selection independent of cache reads, saves and provider capacity, including cancelled callers. No mail, flag or credential write is performed by selection.

The new Dart controller bounds gestures, projects pending choices, retains offscreen intent, recovers exact committed revisions after a lost acknowledgment, retries failed changes/releases and reobserves refreshes arriving during an older response. It is a prerequisite: existing mail controls have not yet switched to captured selection. Browser selection storage/controls, native control wiring, exact reviewed durable bulk execution, range/arrival UX and final performance/Apple evidence remain active.

Verification: **66 native Rust tests**, **69 Flutter host tests** (including actual FFI capture/freeze/Clear with a locked credential store), **39 Python tests**, native Clippy and Flutter analysis pass. The 100,000-message test proves complete membership and bounded bridge output, not latency. Initial Rust integer/coercion errors and Flutter syntax/lint findings were corrected; regression tests also cover scrolling with pending deselection, queued range intent, queue-overflow anchors and stale observations. Failed logs remain under ignored `artifacts/logs/client-selection-*`. Android, production artifact, strict documentation, required hook and review-branch shipping results follow.

Root iced/shared/browser presentation is unchanged from its previous verified checkpoints. New selection controls have no visual evidence yet. Full parity, durable bulk/recovery, R75/R76 and VPS target/owner/OAuth configuration remain open. Quality/release workflows remain disabled.


All **nine existing Android native scenarios** pass against the rebuilt bridge, including cache startup, drafts, connection failures, credentials and swipe/Undo controls. Reviewed dark startup and light restored-inbox captures retain readable spacing and accessible feedback; WebP copies remain in ignored artifacts. These are regressions for existing controls, not new selection UI evidence. The initial driver started before emulator boot and found no device; the boot-ready run passes. The first production build wrote its APK but its command exited 143; a separate final rebuild exits successfully. Production inspection verifies all three Rust ABIs, Internet permission and fixture exclusion; SHA-256: `388718769e63207e2c6d5ef5cbc39a288a35ae2fc10f1238ff8e675b3d2e29c4`. Signing remains developmental. Pinned strict Zensical validation and **30 parity contracts** pass. The dedicated emulator is stopped. Appium/browser suites, root iced native controls, Apple and final idle-host performance were not rerun for this prerequisite; their previous evidence and active gaps remain explicit. Required hooks and the exact review-branch shipping record follow.


### Captured selection prerequisite shipping record

Committed and pushed as [`646638d`](https://github.com/sam-ruff/shep.so/commit/646638d1e782eb938192ac83fa4e71243551a8bb) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 66 native Rust tests, 69 Flutter host tests, nine Android scenarios, 39 Python tests, 30 parity contracts, reviewed regression captures and production APK inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this increment. Full native/browser selection controls, durable bulk execution and the wider parity backlog remain active. Main and the personal desktop installation were untouched; artifacts remain ignored and the dedicated emulator is stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Browser captured-selection storage

The browser now has a lazy repository adapter and a dedicated SQLite/WASM worker for complete ordered captures, revision-checked changes, ranges, explicit arrivals, Clear, immutable reviews and pages of at most 50 metadata rows. The worker bounds queued requests to 32; each worker owns private temporary storage, and closing or losing it expires its captures. Ordinary captures stream metadata from a readonly IndexedDB snapshot; text search alone reads cached bodies. Browser list and capture share folder/filter/search matching and deterministic identity ordering for equal timestamps. No selection operation marks mail read or contacts a provider.

Mail metadata and a bounded 1024-entry change journal commit atomically with cache writes. Sleeping workers rebuild current flags, folders, aliases and missing/account state after the replay window expires, without adding passive arrivals. Alias collisions preserve chosen membership and original rank; missing targets retain account ownership. Version-five migration preserves newer acknowledged Sent roles and outgoing indexes. Failed recaptures roll SQLite membership and revisions back together. SQLite package provenance and the wrapper license ship with browser assets.

The earlier same-IndexedDB selection design is not shipped. Its 100,000-row scenario timed out before draft saving committed; separate real Chromium diagnostics confirmed that disjoint readwrite transactions serialize, while a readonly snapshot allows the save to finish. The SQLite replacement passes that concurrency contract. Initial SQLite tests also exposed an empty binding-array error, which is corrected. Failed diagnostic/test evidence remains ignored under `artifacts/logs/browser-selection-*`.

This remains a prerequisite: the bounded browser gesture controller, actual native/browser selection controls, reviewed durable bulk execution/recovery and full parity are open. Root iced and Flutter presentation/provider code are unchanged, so Android/Appium, Apple, root native UI and idle-host performance are not rerun for this increment. Cardinality/concurrency tests do not establish latency or live provider success. R75 Google provider sign-in, R76 grouped scheduled Automatic replies and VPS target/owner/OAuth configuration remain active. Quality/release workflows stay disabled. Final regression, production artifact, documentation, required hook and review-branch shipping evidence follows.


Final verification passes **85 browser unit tests**, **49 Playwright scenarios**, **53 real Rust HTTPS beta/mail stages**, **39 Python tests** and **30 parity contracts**. TypeScript and the production browser build pass. The HTTPS suite loads the bundled SQLite worker under the production gateway CSP and exercises capture/freeze/page; mail and identity exchange remain synthetic fixtures. Reviewed light 1440×920 and dark 900×640 captures preserve readable list/reader spacing and reachable controls. These are existing-layout regressions, not new selection UI evidence. The production artifact contains the selection worker, SQLite WASM and license, excludes the preview entry and checked fixture markers, and preserves shared MIME WASM SHA-256 `af68dd35fc8b176495f1b149845c558a31bc013343c51dccb4045c373f28dff9`. SQLite WASM SHA-256 is `02d7e48164395fa68f81c6ec33e9da5461be397dc57602ac0cd89b4bbba1d312`. Pinned strict Zensical validation passes. Required commit hooks and exact review-branch shipping are recorded next.


### Browser selection storage shipping record

Committed and pushed as [`8295e3c`](https://github.com/sam-ruff/shep.so/commit/8295e3c40ff2a19cb211398f47e30fb9603c80d1) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 85 browser unit tests, 49 Playwright scenarios, 53 real Rust HTTPS stages, 39 Python tests, 30 parity contracts, reviewed layout regressions and production artifact inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this checkpoint. Native/browser selection controls, durable bulk execution and the full parity backlog remain active, including R75/R76 and missing VPS/owner/OAuth configuration. Main and the personal desktop installation were untouched; artifacts remain ignored and owned test servers are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Browser selection controls

The browser now connects complete captured membership to Select, Select all, Clear, Done, row checkboxes, Ctrl/Meta-click, Shift ranges and focused-list Mod+A. Remapping and disabling persist; ordinary search/reader text selection remains native. Mode survives page and preference changes, resets immediately when the mailbox scope changes, and preserves typed search before its 100 ms debounce. Selecting never arms reading or dispatches a retained reader's action. The summary reports captured counts, current account/folder groups and unavailable selected messages. Durable group execution is still open.

The controller keeps one request in flight, at most 32 queued gestures and one observed page. It preserves newer/offscreen choices, recovers exact committed revisions after lost responses, re-observes changes arriving behind older snapshots and releases abandoned captures before replacement. Startup errors offer Retry; a stopped worker's expired storage does not prevent a new capture. Both Dart and browser controllers now keep deselection counts when Select all is pending and a row leaves the viewport, and visibly choose range endpoints before their ranks arrive. Flutter's existing loaded-row controls are not yet switched to this controller; they require the reviewed durable bulk path.

Saved tests now drive six production-adapter browser selection scenarios across 125 fictional cached messages, including paging/preferences while the actual WASM request is held. Review found and fixed lost first-character typing during immediate mode exit and excluded the new Select all shortcut from the formatted-frame interception list. The existing handover test now asserts real captured choices; its first stronger run exposed missing logical Sent roles in the synthetic preview transport, which is corrected without weakening the assertion. Preview membership remains a small fixture transport excluded from the production bundle. Logs remain under ignored `artifacts/logs/browser-selection-controls-*`. Final regression, visual, artifact and shipping evidence follows.

The full product goal stays active: native selection controls, reviewed durable bulk execution/receipts/history/Undo, Apple/other browser-engine execution, final performance, Google provider sign-in (R75), grouped scheduled Automatic replies (R76) and VPS target/owner/OAuth configuration remain TODOs. Root iced/provider code and Flutter presentation are unchanged; root native UI, Android/Appium and Apple suites are not rerun for these browser controls and unwired Dart-controller fixes. Fixture success does not establish live-provider or complete parity. Quality/release workflows remain disabled.


Final verification passes **96 browser unit tests**, **55 Playwright scenarios**, **54 real Rust HTTPS beta/mail stages**, **71 Flutter host tests**, **39 Python tests** and **30 parity contracts**. Flutter analysis, TypeScript, the production browser build and pinned strict documentation validation pass. The full browser run retains the 100,000-row capture/draft-save and journal contracts; the HTTPS run adds actual Select all/Clear/Done controls without reading mail. The light 1440×920 and dark 900×640 captures are reviewed; shorter visible Select all/Clear labels keep both buttons on one compact row while retaining descriptive accessible names. The production bundle excludes preview/fixture markers and includes the SQLite license; SQLite and shared MIME WASM hashes remain those in the prior storage checkpoint. Root native/Android/Appium/Apple and idle-host performance are not rerun for this increment, as detailed above. Required hook and exact review-branch shipping evidence follows.


### Browser selection controls shipping record

Committed and pushed as [`7f6d525`](https://github.com/sam-ruff/shep.so/commit/7f6d52573f04456a01a1a5d18618d1ae3bea483e) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 96 browser unit tests, 55 Playwright scenarios, 54 real Rust HTTPS stages, 71 Flutter host tests, 39 Python tests, 30 parity contracts, reviewed compact/large layouts and production artifact inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this checkpoint. Native control replacement and reviewed durable bulk execution/receipts/history/Undo remain active alongside full parity, R75/R76 and missing VPS/owner/OAuth configuration. Main and the personal desktop installation were untouched; artifacts remain ignored and owned test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Browser durable group journal prerequisite

Frozen selections now export exact membership and current physical identities from a readonly mailbox snapshot into worker SQLite, then copy at most 50 metadata rows per transaction to a separate per-profile IndexedDB journal. Completed staging requires a new review decision; partial preparation cannot execute or silently replace its membership after restart. The journal stores no MIME, passwords or OAuth data.

Exclusive tab ownership spans claimed work and receipt persistence. Status indexes and counters allow one claimed step across groups, 20-job history and 50-item pages. The storage state machine retains acknowledged forward/inverse identities, waits for a pending forward result before Undo, excludes unsent steps after Undo, rejects stale attempts/revisions and retries only definite failures. Closing the owner tab preserves an unconfirmed forward/inverse result with its receipt for explicit review.

Eight real Chromium storage scenarios pass, including complete 100,000-message export while an independent draft commits, partial staging/atomic rollback, aliases/missing mail/passive arrivals, cross-tab exclusion, abrupt owner closure, Undo/retry and bounded profile-isolated history. This is cardinality/concurrency evidence, not a latency benchmark or real provider execution. Final browser regression, production, documentation and shipping checks follow.

This remains a prerequisite: provider execution, current-state and individual-intent coordination, cache/receipt recovery, account-removal review/cleanup, explicit ambiguous-result resolution, optimistic query effects, native equivalent and visible review/History/Undo controls remain active. No production control invokes the new journal yet. Flutter/root iced presentation is unchanged, so Android/Appium, Apple and root native UI are not rerun for this increment. Full parity, R75/R76 and VPS target/owner/OAuth configuration remain open; quality/release workflows remain disabled.


Final verification passes **96 browser unit tests**, **63 Playwright scenarios**, **55 real Rust HTTPS beta/mail stages**, **39 Python tests** and **30 parity contracts**. TypeScript, production browser build and pinned strict documentation validation pass. The HTTPS worker acquires the real journal lock under the gateway CSP and refuses empty reviews. Production inspection excludes checked fixtures and retains the SQLite license; SQLite and shared MIME WASM hashes remain those recorded in the prior checkpoint. Existing light 1440×920 and dark 900×640 selection captures are reviewed; these are layout regressions, not new group controls.

The first full browser run passed 62 scenarios and failed the new tab-close scenario: Chromium had acknowledged page closure before releasing its Web Lock, so the journal correctly refused ownership. The scenario now observes actual lock release before opening recovery; the complete 63-scenario rerun passes. Failed trace/capture evidence remains under ignored `artifacts/browser-bulk-journal-failures/`, with logs under `artifacts/logs/browser-bulk-journal-*`. No production guard or timeout budget was weakened. Android/Appium, Apple, root native UI and idle-host performance are not rerun for this storage prerequisite; the host is busy with separate work. Required commit-hook and exact review-branch shipping evidence follows.


### Browser durable group prerequisite shipping record

Committed and pushed as [`bae6776`](https://github.com/sam-ruff/shep.so/commit/bae677639921759d866f1147b2d08e51c5ef7263) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 96 browser unit tests, 63 Playwright scenarios, 55 real Rust HTTPS stages, 39 Python tests, 30 parity contracts, reviewed layout regressions, corrected tab-close observation and production inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this increment. The journal is a storage prerequisite; provider execution, account cleanup/current-state coordination, native equivalent and visible review/History/Undo controls remain open alongside the full parity and deployment backlog. Main and the personal installation were untouched; artifacts remain ignored and owned browser test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Acknowledged browser mutations and group cache recovery

Browser mutations now return physical receipts independently of message-list reload. Remote acknowledgment reaches the durable writer before fallible cache updates; local/POP3 changes acknowledge their actual cache commit. Source guards reject changed physical messages before mutation. Missing destination UIDs retain exact-content size/SHA-256 proof with an explicitly absent destination identity. Cache or receipt-writing failures cannot downgrade a known server acknowledgment.

Individual controls retain their confirmed flag/move after a failed display read. Counted Archive/Restored feedback retains acknowledged successes; cached receipt-backed Undo remains usable after a display-only warning. Incomplete cache/identity recovery still requires refresh, and acknowledged inverse warnings cannot issue another inverse operation.

Journal schema 2 separates an acknowledged result from pending cache/identity repair, indexes runnable work, and allows read-only history observation without recovering a live owner's step. Further provider claims wait until pending cache work reconciles; attempt/source guards protect completion and recovered UIDs do not overwrite forward flag metadata. Version-one migration preserves existing progress and conservatively recovers abandoned work. This is still an execution prerequisite: the full provider loop, persistent individual-intent ordering, account-removal integration, optimistic group queries and visible approval/History/Undo controls remain active.

Targeted tests cover receipts before cache failure, post-commit list failure for IMAP/POP3, exact MOVE recovery proof, no repeated MOVE, changed-source refusal, cache-repair reopen/stale results, read-only ownership and schema migration. Saved real browser controls cover retained flags and Retry without another flag write; the Archive/Undo control scenario and final regression results follow. Flutter/root Rust presentation and native provider code are unchanged; Android/Appium, Apple, root native UI and final idle-host performance are not rerun here. Full parity, R75/R76 and missing VPS target/owner/OAuth configuration remain tracked.


The two saved production-adapter browser flows now drive actual row Flag and reader Archive/Undo through cache-committed display failures. They verify persistent flags, retained Archive/Restored counts, reachable Undo, and Retry without another flag write. Compact flag-warning and reader Archive-warning captures are reviewed for readable errors, confirmed state and accessible controls. The first Archive scenario omitted opening a message and correctly encountered a disabled action; its corrected real-input flow passes. A prior broad run was invalidated when a development source edit triggered Vite navigation during the 100,000-message test. The final run freezes source for its duration. Both failed traces remain ignored under `artifacts/browser-bulk-execution-failures/`; no product guard or timeout was weakened. Final counts and shipping evidence follow.


Final verification passes **101 browser unit tests**, **67 Playwright scenarios** in a complete unchanged-source run, **55 real Rust HTTPS beta/mail stages**, **39 Python tests** and **30 parity contracts**. TypeScript, the production browser build and pinned strict documentation validation pass. Production inspection excludes checked fixtures and preserves the SQLite license; SQLite/shared MIME WASM hashes remain those from the prior checkpoint. Ten journal storage scenarios now include migration, pending-cache/identity completion and read-only observation, alongside the existing 100,000-row/concurrency, receipt, tab-loss and rollback contracts. Those are correctness/cardinality contracts, not latency measurements or live mail evidence. Required commit-hook and prompt review-branch shipping records follow.


### Acknowledged mutation and cache recovery shipping record

Committed and pushed as [`d2074db`](https://github.com/sam-ruff/shep.so/commit/d2074dba95a462cdc326135364477af9c4b6c2bf) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 101 browser unit tests, all 67 Playwright scenarios in a complete unchanged-source run, 55 real Rust HTTPS stages, 39 Python tests, 30 parity contracts, reviewed committed-action warning captures and production inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this increment. Full group execution, persistent individual-intent coordination, account lifecycle integration, optimistic queries and native/visible group controls remain open alongside the original parity/deployment backlog. Main and the personal installation were untouched; artifacts remain ignored and owned browser test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Persistent browser action ordering

Browser controls now reserve per-field revisions when the user acts, before waiting for earlier provider jobs. Dispatch rechecks ownership under the operation lock and reports only accepted fields; superseded flags/moves cannot become false confirmations or counted successes. Unsent move/Undo reservations retire without a provider write. Known acknowledgments retain their receipts even if the action record cannot finish; uncertain wire outcomes remain pending and are never automatically replayed.

Mail schema 6 adds a persistent clock and metadata-only intent records. Revisions distinguish newer same-value choices from an older group's ownership, and inverse claims affect only fields still owned by the original decision. Alias adoption merges field ownership inside the cache transaction; ordinary sync cannot erase it. Account removal includes pending flags and missing-message intent in its reviewed counts, deletes only the removed account's ownership and rejects stale writes. The compact review has readable controls and singular counts.

Targeted verification passes 107 browser unit tests and six new real Chromium scenarios, covering two-tab ordering, group claim/Undo primitives, stale completion, aliases/atomic rollback, schema migration, clock exhaustion, queued row controls and actual account removal. These establish the individual-control and storage increment, not a visible group executor. Initial TypeScript errors in generic review sorting and a new test observation were corrected. Full regression, production/HTTPS checks, strict documentation, required hooks and shipping evidence follow.

Root iced/shared/native Flutter implementation is unchanged; Android/Appium, Apple, root native controls and idle-host performance are not rerun for this browser increment. Full group execution/recovery/history, optimistic bounded queries, group cleanup and Flutter parity remain active, as do R75/R76 and missing VPS target/owner/OAuth configuration. Main and the personal desktop installation remain untouched; quality/release workflows stay disabled.


The first full browser run passed 72 of 73 scenarios; the remaining version-two migration test still expected schema 5. Its expected current version is corrected to 6 while preserving all data/role/rollback assertions. The failure trace remains under ignored `artifacts/browser-intents-failures/schema-expectation/`. Review also extended removal tombstones to missing intent-owned and aliased identities so late raw-body writes cannot recreate removed content; the saved Chromium removal scenario checks this. The final unchanged-source run and production/HTTPS evidence follow.


Final verification passes **107 browser unit tests**, **73 Playwright scenarios** in a complete unchanged-source run, **39 Python tests**, **30 parity contracts**, TypeScript and the production browser build. The real Rust HTTPS beta/mail integration test passes against the new production assets. The compact account-removal capture is reviewed and saved as a lossless WebP under ignored artifacts. Production inspection excludes the preview entry and checked fixture markers, includes bundled licenses, and preserves the previously recorded SQLite/shared MIME WASM hashes. Pinned strict documentation validation passes; required commit hooks and exact review-branch shipping follow.


### Persistent browser intent shipping record

Committed and pushed as [`353d259`](https://github.com/sam-ruff/shep.so/commit/353d2597ecb57fb649adea54dd312d4a01b1b6ab) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Verification includes **107 browser unit tests**, **73 Playwright scenarios**, **55 real Rust HTTPS beta/mail stages**, **39 Python tests**, **30 parity contracts**, TypeScript, production browser inspection and pinned strict documentation validation. The reviewed compact removal capture and retained migration-assertion failure evidence are recorded above.

R77's prompt push is fulfilled for this increment. Group execution/recovery, bounded optimistic queries, native client parity and the wider product/deployment backlog remain active, including R75/R76 and missing VPS/owner/OAuth configuration. Main and the personal installation were untouched; artifacts remain ignored and owned test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Browser durable provider executor

Frozen membership now feeds an owned executor that claims one message at a time, binds the browser profile and approved intent revision, validates physical source identity, and stores accepted fields plus the actual receipt before finishing cache work. Undo reverses acknowledged destination identities and only fields still owned by the original action; superseded work is counted separately from success. Unsent membership remains cancelled by the job's Undo phase. Definite failures can retry, uncertain steps cannot automatically retry, and graceful stop finishes the owned receipt before a fresh executor resumes queued work.

Cache writes now atomically record their applied field revisions. Receipt recovery fills only missing cache changes and uses read-only exact-identity lookup when a MOVE omitted its destination UID. It cannot repeat IMAP mutations or overwrite newer cached fields, including a later action whose separate completion record failed. Journal schema 3 preserves existing pending-cache gaps, adds saved ownership/skipped outcomes, and refuses legacy execution without the required decision metadata.

Thirteen real Chromium storage/provider scenarios cover the executor and actual production adapter: complete 125-message frozen execution and Undo, same-value newer intent, partial/superseded fields, physical/source/profile refusal, transactional rollback and aliases, late cache repair after newer flags/moves, lost journal replies, unknown destination UID recovery, definite retry versus ambiguous failure, graceful stop and tab loss. These use fictional transports and real browser storage/locks; they are not visible group-control or live-provider evidence. The executor is not activated from application controls yet. Optimistic query integration, group account cleanup, review/History/Undo UI and native equivalents remain active requirements.

The first fixture omitted the required frozen review and was correctly rejected. The next tab test tried reconnecting behind the intentionally held account lock; it now loads cached state without reconnecting. Chromium also releases a closed tab's Web Lock after page-close completes, so the replacement observes that specific lock ending before recovery. No production guard or timeout was weakened. Failed traces remain under ignored `artifacts/browser-executor-failures/`. Targeted scenarios and 107 browser unit tests pass; full regression, production/HTTPS, documentation, hook and shipping results follow. Root Rust/native Flutter implementation is unchanged; Android/Appium, Apple, root native UI and idle-host performance are not rerun here. Full parity, R75/R76 and missing VPS/owner/OAuth configuration remain open.


Final verification passes **107 browser unit tests**, **86 Playwright scenarios**, **55 real Rust HTTPS beta/mail stages**, **39 Python tests** and **30 parity contracts**. TypeScript, the production browser build and pinned strict documentation validation pass. All 13 executor scenarios pass in the complete unchanged-source browser run. Light 1440×920 and dark 900×640 layout regressions are reviewed; lossless WebP copies remain ignored. These captures show existing controls, not new group UI. Production inspection excludes the preview entry and checked fixture markers, includes bundled licenses and preserves the previously recorded SQLite/shared MIME WASM hashes. Required hooks and prompt review-branch shipping follow; quality/release workflows remain disabled.
