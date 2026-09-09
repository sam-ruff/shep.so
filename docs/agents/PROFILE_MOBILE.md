# Flutter profile discovery

Preferences now links to **Profiles and sync**, where the Android/iOS client can
find saved profiles, retry interrupted discovery, rescan, pause, and page through
50 summaries at a time. Names, account/setting counts, missing history, removal
and conflicts are observations. Applying accounts/settings and continuous sync
remain unfinished; no enrollment or credential-import success is displayed.

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

Continue initialized first-profile publication, owned upload receipts, bounded
reviewed enrollment, category controls and real account/preferences application.
Keep local mail/drafts and device identity; changed endpoints require reviewed
credential activation. The credential-protection choice, legacy migration,
desktop/browser integration, Apple and live cross-client verification remain open.
