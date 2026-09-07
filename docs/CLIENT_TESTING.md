# Client testing

Use [the parity matrix](CLIENT_PARITY.md) to distinguish preview behavior from real provider/platform evidence. Tests use `shared/preview.json`, fake transports and synthetic mail; never personal accounts. Keep every new scenario reproducible and keep screenshots/logs in ignored `artifacts/`.

## Flutter

Flutter is pinned to **3.44.2 / Dart 3.12.2**, with flutter_rust_bridge 2.13.0 and Rust 1.96.0. The native-assets hook builds `flutter/rust` against `shared/mail-core`; Android builds require SDK 37, NDK 28.2, Perl and make. The hook honors Android minSdk 24 for C/OpenSSL and handles the Linux Snap Perl mismatch. Apple native TLS uses platform trust. From `flutter/`:

```sh
flutter pub get
flutter analyze
flutter test
flutter build web --target test/preview_main.dart --no-web-resources-cdn
```

The host suite includes an actual FFI/SQLite reopen test. It also uses the same generated Outbox fixture as Android to test local IMAP Sent flag/read/move/reopen without touching a locked credential store, and missing/locked credential refusal for server-backed mail. Python 3 prepares each temporary profile before the native bridge opens it. It loads the library from Flutter’s native-assets output because the bridge’s legacy widget-test loader still expects Cargo’s old target directory. Run `cargo test --manifest-path flutter/rust/Cargo.toml` from the root for paging/search, POP3 local state, FIFO/cancellation, draft/discard and outgoing contracts. Outgoing tests hold an SMTP operation open, reject a second process, inject terminal-record/Sent-cache write failures, and verify immutable recovery, paging and no repeated send. These tests use temporary profiles and fictional data.

From the root, run the self-contained browser runner:

```sh
npm --prefix flutter/e2e ci
npm --prefix flutter/e2e exec -- playwright install chromium
python3 scripts/clients/flutter_web_e2e.py
```

This follows Walkie Textie Flutter: Playwright drives Flutter's accessibility tree and real pointer gestures; Appium/UiAutomator2 drives native Android. Semantics/state are observations, not an action API. No direct workspace calls replace native clicks/swipes. The hosted desktop client is tested separately in `web/`.

Create a dedicated AVD named `shep-e2e` with Android API 36 and start it. With Android SDK, Appium 3.5.2 and Node 24 installed:

```sh
python3 scripts/clients/android_e2e.py --device emulator-5554
```

The runner refuses personal devices, runs preview and actual native-bridge integration tests sequentially, rebuilds the isolated preview APK, then runs Appium. Never run the two native drivers against the same emulator concurrently, or run competing Flutter builds/tests in one checkout; generated shader assets are shared. Only `so.shep.shep_mobile.preview` is reset. Native bridge scenarios save/reopen/edit/discard a draft through controls, reject an isolated loopback account probe roundtrip test-only credential pairs, and open the real production entry in this isolated preview package. Rust profiles use fresh temporary directories; credential entries use unique fixture identifiers and are removed afterward. No production app/account data is touched. The `test_driver/native_driver.dart` host callback saves captures from `flutter drive`; Flutter test cleanup can uninstall its test package, so screenshots must be collected before cleanup. Screenshots are in `artifacts/flutter/`; convert review copies to WebP with a standard lossless image converter.

The Android runner also pairs `attachments_android_test.dart` with `android_compose_fixture.py`. The host hands over a generated SQLite mailbox before the app opens it, then uses actual DocumentsUI input to cancel one picker and select two files. Flutter controls exercise cached Reply all, save/reopen, file removal, pending text saves and send refusal without credentials. `--compose-only` reruns just this scenario while developing; it is not the full Android suite. The helper accepts only the dedicated emulator and preview package. Apple file-picker coverage remains open.

The incoming-file scenario pairs `incoming_android_test.dart` with `android_incoming_fixture.py`. It seeds an isolated cached MIME profile before opening it, drives the actual Android save picker through cancellation and a successful save, and reads the selected file back to verify binary bytes. Reader controls move the message, refresh with locked credentials and retain the open message after its Inbox row disappears. `--incoming-only` reruns this scenario. Host tests also cover save errors, stale attachment identities and corrupt-file metadata without hiding the cached body. Apple export is implemented but uncompiled/unexecuted on this Linux host; simulator and interrupted-save cleanup coverage remain open.

The runner then pairs `outbox_android_test.dart` with `android_outbox_fixture.py` before rebuilding the preview for Appium. Its synthetic SQLite records are handed over before the native profile opens. A separate Android process verifies refusal to acquire the held profile lock, with an independent successful lock as a control. Real Flutter controls exercise light/dark Outbox review, disabled uncertainty actions, recovered text/Bcc/files, file removal, manual mark, local Sent and reopen. `--outbox-only` reruns this scenario. Recovery never sends; an explicit later Send without credentials remains refused with the recovered text intact. The native driver also runs the same Sent-handover control scenario as the host suite: a held sync, retained reader, late body result and pre-handover Undo, with transport-only fixture gates. The Outbox fixture additionally hands over a saved receipt plus cached provider row before startup, and real controls repair/deduplicate it through Rust. With the native credential reconnect/cleanup scenario, these are thirteen integration scenarios in total, followed by six Appium stages; test teardown callbacks are not additional scenarios.

Native handles for one canonical profile share their database and account-operation coordination. The companion owner-lock file is never removed; blocking SQLite work retains its lease through caller cancellation. Only a new exclusive owner reclassifies abandoned submissions as uncertain. Android uses Bionic `flock` because the pinned Rust standard-library Android implementation does not implement file locking. The emulator checks the actual OS lock, beyond the host subprocess test. Apple locking/runtime execution still requires macOS verification.

Sent-copy tests use the same shared `SentConnection` contract as the desktop. Native Rust fixtures hold an APPEND open, cancel its waiter, fail journal/cache writes before and after acknowledgment, change account/policy settings, restart an unfinished upload, and reconcile a synced copy without removing local edits. The Android Outbox scenario also repairs a preloaded provider acknowledgment without credentials, exercises refused lookup/retry with missing credentials, and saves/reopens Sent policy/folder preferences through real controls. The saved Android scenario additionally flags, marks unread and archives a local IMAP Sent message with the fixture credential store locked, then reopens it and checks the saved controls. The same scenario checks missing/locked credential rollback on cached server mail and visible reader recovery/dismiss controls. This proves device/cache behavior; the separate shared-core TLS transcripts prove the production IMAP lookup/APPEND path. Neither is live-provider verification.

Shared Sent lookup reads final command responses explicitly, rejecting NO/BAD, missing UID/header results and conflicting identities. Pinned gateway tests exercise Sent discovery, lookup and exact binary APPEND through both implicit TLS and STARTTLS; the wrong hostname must fail before authentication. See [IMAP command completion and APPEND](https://www.rfc-editor.org/rfc/rfc9051.html#section-6.3.12) and [special-use mailbox discovery](https://www.rfc-editor.org/rfc/rfc6154.html). Browser Sent gateway reservations and UI recovery now use the same transport. Its local/provider identity handover and logical grouping have the continuation evidence below; labels/history remain open.


On a Mac with Xcode and an installed iOS runtime:

```sh
python3 scripts/clients/apple_simulator.py
```

It creates and deletes only its own simulator. Apple execution, native XCUITest/Appium coverage and signing remain open until run on macOS; Android/Chromium evidence is not Apple verification.

## Browser and gateway

Browser builds/tests require the Rust WASM target and matching wasm-bindgen CLI. The npm pre-scripts compile the shared MIME crate and generate ignored `web/src/wasm/` glue; no generated binaries are committed. Install the pinned tool once from the root:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version 0.2.128 --locked --root artifacts/wasm-tools
```

```sh
npm --prefix web ci
npm --prefix web test
npm --prefix web run e2e
cargo test --manifest-path backend/Cargo.toml
cargo test -p shep-mail-core --all-features
npm --prefix web run build
cargo test --manifest-path backend/Cargo.toml real_browser_beta_gate -- --ignored
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
```

Playwright drives the browser list/reader, input-safe shortcuts, resizing, compose/calendar forms and login presentation. Axe checks light/dark at 1440×920 and 900×640. Rust gateway tests check the actual middleware/session routes plus signed synthetic Google-token claims. The explicitly selected `real_browser_beta_gate` test runs production web assets against the real Rust router through an isolated local HTTPS proxy, checking anonymous assets/API denial, denied/allowed users, callback replay, secure cookies, CSRF and UI logout. The saved provider flows also exercise account probe failure/retry, streamed mail, flags and draft reload, password reconnect, and lost/uncertain SMTP delivery recovery through actual controls and IndexedDB. Google identity exchange and mail transport are object-scoped `cfg(test)` fixtures; no production bypass exists. Separate shared-core loopback TLS tests exercise real IMAP/POP3/SMTP and STARTTLS with hostname verification. Backend tests additionally disconnect an accepted SMTP HTTP waiter and inject a provider panic; the supervised operation still records its final or uncertain status. Its self-signed HTTPS test key is public fixture material. Browser evidence is in `artifacts/beta-browser/`. These do not establish live Google, SMTP or VPS behavior.

`shared/compose-fixtures.json` is consumed by both the Rust and browser reply tests. Browser provider controls also cover offline Reply all, real multi-file selection, removal after reload, compact composer actions and exact MIME bytes/headers at the Rust send boundary. Draft blobs live separately from text in IndexedDB/SQLite; native file revisions and browser attachment snapshots prevent sending stale file selections. These contracts do not establish live SMTP delivery or complete Sent/Outbox recovery.

Use `python3 scripts/clients/check_parity.py --base REV` when desktop or shared-core code changes. `python3 -m unittest discover -s tests -p 'test_*.py'` covers separated static staging, fail-closed release readiness and aligned client/lockfile version stamping. Existing iced MCP flows stay in `scripts/e2e.py`.

The browser Outbox flow observes IndexedDB before allowing Send through, proving the exact prepared MIME is committed. It drives loss of preparation and Send responses, local Sent after reload, disabled uncertainty controls, explicit review/return, attachment ownership, a separate new Send, manual mark, rejected recovery and atomic reservation cancellation. The Rust fixture counts SMTP calls, so status/recovery cannot hide a resend. Compact layout checks compare cards with their scrolling container, alongside axe and reviewed light/dark screenshots. Provider tests cover storage failure, stale editors, active-send refusal and terminal acknowledgments surviving receipt expiry. The Sent continuation records the copy reservation/account/destination in IndexedDB before APPEND, loses its HTTP response after the Rust acknowledgment and recovers after reload without credentials. A second explicit, reviewed copy receives an uncertain result and is resolved by lookup without repeating APPEND. It also saves/reopens Sent preferences and scans light/dark/compact recovery controls with axe. The Rust fixture verifies exact original SMTP bytes and counts two APPENDs across these explicit copies; the four explicit SMTP attempts remain unchanged. Local/provider identity handover and logical Sent grouping now have the continuation evidence below; bounded browser Outbox reads remain open.

## Worktree staging and release

The promo agent built `website/` in `shep-website`; its reviewed source is also copied into the combined `shep-clients` worktree. The combined review branch is `feat/mobile-web-clients`; checkpoint shipping evidence is recorded in [the completion log](COMPLETION.md). Main is unchanged by this client work. After building both clients, stage their **separate** public/protected directories with:

```sh
python3 scripts/clients/assemble_site.py \
  --website website/dist --web web/dist \
  --output artifacts/staged-beta
```

The Rust gateway alone serves the protected `web/` directory; do not put it under the public promo root. Staging does not deploy. Quality/release workflow filenames stay `.yml.disabled`. The production APK check is `python3 scripts/clients/verify_android.py flutter/build/app/outputs/flutter-apk/app-production-debug.apk`; it verifies native libraries, Internet permission and fixture exclusion, while signing verification remains separate. One quality workflow now includes all client jobs; release readiness, signed artifacts and coordinated version/publication work remain explicit prerequisites. Re-enable workflows only when Sam's trusted runners and release requirements are ready.

Sent route tests in `backend/src/mail/sent_tests.rs` cover policy/pin/identity/CSRF refusal, duplicate and changed-content requests, expiration, occupied capacity, HTTP cancellation, lookup errors and supervised APPEND panics. `web/src/sent.test.ts` covers immutable reservation ordering, browser storage failure before upload/after acknowledgment, credential-free repair, cross-tab locks, changed connections, policy/reconnect behavior and explicit ambiguous retries. These are synthetic contracts, not live-provider claims.


Browser Sent handover adds `handover.test.ts` for strict identity/receipt checks, atomic rollback, alias reopening, held-sync local edits, queued server actions, newer intent/rollback and retained readers. `storage.spec.ts` runs the real IndexedDB version-2 upgrade and synchronous write failure rollback in Chromium; its isolated synthetic database exists before the production store opens. These storage contracts supplement control tests, and do not act through a UI state oracle.

The real Rust HTTPS scenario syncs a provider Sent copy before receipt recovery, opens and flags that row, then repairs through a newly opened tab with no passwords. Refresh in the original tab must adopt the provider row under the original local ID while retaining its open reader and existing Undo. Actual Archive/Undo requests must use current UIDs and the physical `Sent Mail` destination. Read-only IndexedDB observations check the alias and immutable SMTP bytes. Light 1440×920 and dark 900×640 captures run axe and receive visual review. The local HTTPS fixture owns and closes raw TLS sockets as well as HTTP connections; it saves success only after browser/proxy cleanup.

Cached incoming files use `shared/mail-content` in native Rust and browser WASM. `shared/attachment-fixtures.json` covers binary bytes, quoted-printable Unicode text, duplicate filenames, sender path sanitization and Content-Type name-only attachments. The browser worker opens the verified identity's cache, resolves stored aliases and returns metadata or one selected transferable buffer. The real HTTPS control scenario first blocks WASM loading and retries, then disables networking and saves exact cached bytes. Reviewed light/dark desktop and compact captures accompany axe checks. This does not establish large-message streaming, pre-parse nesting protection, or Apple/native-desktop execution.


Account removal uses the same incoming-file Android profile after the save scenario. Real Preferences controls review and cancel, switch light/dark, remove with the fixture keychain locked and retry cleanup after it unlocks. Captures are `native-account-removal-{light,dark,cleanup}.webp`. Host tests cover retained readers and delayed mutation failures; Rust tests cover transactional rollback/FTS, current draft/file revisions, unfinished moves, occupied provider capacity, restart and stale account/draft saves. SQLite schema 7 records removed IDs and cleanup jobs without credentials; opening a cached workspace only reads job metadata.

Browser removal uses IndexedDB schema 4 and a write transaction that rechecks the review before deleting owned data. Tombstones contain identities and a random retry token, never the review's draft text or mail metadata. Normal commits read tombstones in their own write transaction to reject late tab/editor results. The real storage test holds an actual Web Lock, rejects stale confirmation and proves a mixed valid/removed write rolls back entirely. Real HTTPS controls edit a draft in a second tab while review is open, reload/cancel, review light/dark/compact layouts, remove and refuse that tab's stale reconnect. Account removal never calls a provider deletion API. Account connection editing, Apple execution, broader lifecycle notifications and large-cache performance remain open.


`connections_tests.rs` exercises native schema-7 migration, atomic credential activation/rollback, idempotent lost-response recovery, competing/stale candidates, active-key cleanup refusal and obsolete credentials rejected before provider work or journaling. Flutter host tests use the actual bridge with object-scoped probe/acknowledgment failures and a held credential write. The saved `native_mail_test.dart` reconnect scenario drives real password fields, failure/retry and Preferences cleanup controls; its probe fixture never sends mail. New slots and cleanup jobs contain no passwords in SQLite. The existing preview secure-storage scenario remains the separate actual Android credential-store roundtrip. Apple device execution remains open.


Find in message uses the shared `find-cases.json` contract in native Rust, actual Flutter FFI, preview Dart and WASM tests. Cases include Unicode case folding, astral UTF-16 offsets, literal metacharacters and wrapped whitespace. The native request has its own blocking permit and passes while provider capacity is occupied. Both client models coalesce requests and reject obsolete query/message/quote/error results.

`message_find_scenario.dart` drives the same controls from Flutter host tests and the isolated Android incoming-file profile: counts, wrapping, case, Unicode, next/previous, offscreen reveal, expanded quotes and native word selection/Copy. The host clipboard is an isolated test service; Android reads the actual emulator clipboard. `message-find.spec.ts` adds browser focus, remapping/disable, input isolation, changed messages and full-reader Escape, with light/dark compact axe and screenshot review. `message-find-flow.mjs` also runs against cached MIME through the real Rust HTTPS flow while networking is disabled. These scenarios establish text-reader behavior; faithful HTML, large-text performance, complete mobile keymaps and Apple execution remain open.

The Appium and Flutter browser scripts also search the dark reader through actual fields and case controls. `android_e2e.py --appium-only` rebuilds the isolated preview and reruns those native controls after targeted integration checks; default CI still runs all scenarios. The picker supervisor terminates only its owned driver when its helper fails. A System UI interruption may choose Wait, while an application failure remains visible and fails normally.

`shared/reader-fixtures.json` covers MIME alternatives, related roots, mislabeled XHTML, mixed bodies, attachment classification and CID shadowing/ambiguity. Shared Rust and browser WASM compare the same representations; native bridge tests check cached detail with provider slots occupied. The incoming Android and real HTTPS browser scenarios also reject the obsolete alternative and extra inline-file control. The low-level `message_body` WASM result contains **untrusted HTML**, not a document safe to insert into the app. Formatted rendering remains separate work. Depth tests reject 4,096 MIME levels before recursive parsing, compare accepted boundary behavior against the pinned parser and exercise deeply nested HTML text conversion without recursive traversal.


The formatted-reader scenarios use `shared/html-reader-fixture.eml`, a synthetic notification with tables, authored colors, a CID dog image, a remote banner and quoted history. `web/e2e/formatted-reader.spec.ts` seeds an isolated browser cache before mounting the production UI, then uses actual controls. It verifies Copy/paste, shared Find across spans, quote visibility, fallback highlights, stale-worker cancellation, CSP failure/retry and compact layouts. Sender-script/network/parent-access checks run inside the actual opaque frame; read-only DOM observations do not drive actions. `provider-flows.mjs` separately exercises the production build under the Rust HTTPS gateway's inherited CSP. Native Rust tests cover the same preparation contract; Flutter native WebView controls and Apple execution remain open.
