# Flutter profile discovery

Preferences now links to **Profiles and sync**, where the Android/iOS client can
find saved profiles, retry interrupted discovery, rescan, pause, and page through
50 summaries at a time. Names, account/setting counts, missing history, removal
and conflicts are observations. Reviewed enrollment and ongoing preference sync
are described below; no credential-import success is displayed.

## Connection and configuration

Enable Drive in the saved Google connection. The existing native SDK adapter
obtains a token silently for that exact account and enabled service. After an app
restart, reconnect may still be needed to restore its SDK session. Discovery
never opens consent in the background or sends its token to the beta server.

In addition to the [Google client configuration](GOOGLE_MOBILE.md), provide
`--dart-define=SHEP_PROFILE_NAMESPACE=YOUR_SHARED_APPLICATION_NAMESPACE`.
Use the same application namespace across registered Shep clients, independently
of their OAuth client IDs. An unconfigured build explains why discovery is
unavailable. Live same-project app-data visibility remains unverified; a namespace
label cannot establish that the registrations use the same project.

`GoogleConnectionRecord` now retains an optional `drive_principal` in device
secure storage. Older metadata without it remains readable. The shared Drive
transport verifies `about.user.permissionId`; Flutter commits this binding before
requesting any profile contents or advancing the catalog. A mismatch, failed save
or unconfirmed write cannot populate the screen with newly discovered contents.
Re-consent for the same saved account preserves the principal. Grants, tokens and
this local identity binding are never portable profile operations.

## Session ownership

`ProfileDiscovery` owns one running Dart operation and its eventual cleanup.
`NativeProfileDiscovery` sends commands through the existing background JSON/FFI
bridge; Flutter's native dependency now enables the shared `drive` feature.
`flutter/rust/src/profile_discovery.rs` owns a session with a verified Drive client
and the [durable catalog](PROFILE_DISCOVERY.md). Its storage directory appends
`.profile-discovery` to the mail-cache path and uses the scope digest as filename.
No mail-cache schema or account credential slot is imported by discovery.

Each session has an opaque UUID, one admitted open and one advance at a time.
State/summary reads use the catalog's bounded owner independently of held HTTP
and occupied mail-provider capacity. Closing removes the active session before
draining already accepted work. Old data and errors cannot reach a replacement
session; a stale close cannot close its replacement. Production uses the fixed
Google HTTPS transport; only Rust unit tests can inject a held provider adapter.

Google connect/disconnect, unconfirmed storage and disposal invalidate the Dart
grant generation. Changing requested consent alone leaves the committed grant
intact. Pausing allows the current accepted step to finish, then starts no further
GET. Leaving the screen keeps the owned operation available while browsing mail;
disposing the workspace drains it and closes its session. Retry gets a fresh SDK
token and reopens the same saved catalog. Unknown or failed close results retain
the original session identity for cleanup retry.

The view retains one summary page. Next uses the last profile cursor; First page
returns to the beginning without an unbounded cursor history. Saved results remain
explicitly incomplete until discovery catches up. Empty results, missing ancestry
and conflicts never imply successful enrollment or an empty legacy-backup store.

## Verification boundaries

Rust session tests hold both success and failure responses through close, re-open
the real catalog, and reject old session commands. Native bridge tests reject
invalid identities without contacting Google, echoing credentials or changing
mail, while provider capacity is occupied. Shared core protocol/storage tests
cover actual HTTP parsing, saved pages, receipts and restart separately.

Dart tests cover verified identity persistence, failed/unconfirmed saves, retained
grants after denied consent, stale data/errors/refreshes, pause/retry, 52-profile
paging, wrong sessions, disposal and failed close. The same saved widget scenarios
drive host and Android controls, with separate Playwright/UiAutomator flows.
Their providers are explicit isolated fixtures. See the latest completion log for
executed scenarios and reviewed captures; neither fixtures nor successful builds
establish live Google or Apple execution.

[First-profile publication](PROFILE_PUBLICATION.md) now adds frozen reviews,
initialized history and owned upload receipts. [Reviewed enrollment](PROFILE_ENROLLMENT.md)
applies accounts and the eight preferences. Ongoing preference reconciliation
is described below. Keep local mail/drafts and device identity; changed endpoints
require reviewed credential activation. Google-only credential sync (decided on
11 September 2026), legacy migration, browser integration, Apple and live
cross-client verification remain open.

## Ongoing preference sync

`flutter/rust/src/profile_discovery/sync/` keeps a native ledger in mail schema 12:
one subscription per Google scope (`profile_subscriptions`), staged/admitted/deferred
local edits (`profile_sync_edits`), device applications with their receipts
(`profile_sync_applications`) and open reviews (`profile_sync_reviews`). It reuses
the enrollment's history journal under `.published-profiles` and the discovery
catalog of the current session; nothing reads credentials or mail.

Seeding needs a completed enrollment. Fields whose platform receipt froze their
original revision get a proven basis; a kept field starts at its baseline revision so
the next cycle publishes the newer local value; legacy receipts, conflicts and
unsupported rows stay unproven. An unproven field never acknowledges a matching
value: a differing remote value opens a review, and a later local edit is still
published as intent. Each basis records the shared version, its scalar and the
device revision it was seen at. A subscription starts paused with every field enabled.

A cycle (`Cycle { snapshot }`) takes the device's current values and revisions.
It first retries staged edits, then admits new local intent (a revision newer
than the basis, including change-and-revert), then pulls: incremental catalog
refresh, bounded advance, and originals exported after the saved cursor, which
resets when the observation history's device identity changes. After draining,
each supported field is observed: a single shared version equal to the basis is
common; a version newer than a proven, unedited basis is applied through one
device request at a time; a conflict, a pending local edit or an unproven basis
opens a review. Publication then uploads queued operations. Every step is bounded
at 32 and the report says whether more remains.

Applications reuse the enrollment device path: `Application` returns a
`{id, baseline, changes}` request, the Flutter store writes it with the same
frozen-revision receipt, and `ConfirmApplication` records it. The platform store
retains one receipt, so enrollment refuses to prepare while an application is
unconfirmed and cycles refuse while an enrollment is pending. A receipt that keeps
the field marks newer local intent, which the next cycle publishes.

Reviews page the exact shared versions fifty at a time from the owned history.
`Decide` requires every page to have been opened and the device snapshot to match
the reviewed local intent. Keep mine stages one resolution operation carrying the
local value and the reviewed versions; Use profile with several versions stages the
same kind of resolution with the chosen value, while the only shared version needs
no new operation. Either choice that changes the device value stages an application.
The staged decision is committed before the history edit, so a lost reply retries
the identical operation, and a `Changed` or `Conflict` reply defers it to a refreshed
review rather than dropping it.

`ProfileDiscovery` (`flutter/lib/model/profile_sync.dart`) owns one sync task at a
time through the same verified session as discovery, opening one when none is
active. Enrollment completion seeds the subscription; the Workspace's 15-second
foreground timer runs a silent tick when the grant is connected, sync is enabled and
no preference save is pending. Google disconnect or a changed grant clears the
in-memory status and stops cycles without touching the durable subscription or
other devices; reconnecting the same account resumes it. Preferences shows the
master switch, eight per-preference switches, status, Sync now, the conflict entry
and an explicit paused message while Drive is not connected.

Rust tests cover seeding rules, admission before pull, two-device convergence,
receipt retry after restart, kept receipts, conflict and unproven reviews, lost
history acknowledgments, controls, incomplete pulls, rebuilt sources and pending
enrollment. Flutter host tests drive the controller and the actual controls; the
saved web and Android flows are listed in [client testing](../CLIENT_TESTING.md).
Account definitions, the remaining portable categories, automatic setup, browser
sync, credential transfer, live Google and Apple execution remain open.
