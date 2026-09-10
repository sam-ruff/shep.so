# Client testing

Use [the parity matrix](CLIENT_PARITY.md) to distinguish preview behavior from real provider/platform evidence. Tests use `shared/preview.json`, fake transports and synthetic mail; never personal accounts. Keep every new scenario reproducible and keep screenshots/logs in ignored `artifacts/`.

## Flutter

Flutter is pinned to **3.44.2 / Dart 3.12.2**, with flutter_rust_bridge 2.13.0 and Rust 1.96.0. The native-assets hook builds `flutter/rust` against `shared/mail-core`; Android builds require SDK 37, NDK 28.2, Perl and make. The hook honors Android minSdk 24 for C/OpenSSL and handles the Linux Snap Perl mismatch. Apple native TLS uses platform trust. iOS now targets 14.0 for WebP support in WKWebView; this minimum is not an Apple execution claim. From `flutter/`:

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
python3 scripts/clients/flutter_web_e2e.py --formatted
```

This follows Walkie Textie Flutter: Playwright drives Flutter's accessibility tree and real pointer gestures; Appium/UiAutomator2 drives native Android. Semantics/state are observations, not an action API. No direct workspace calls replace native clicks/swipes. The hosted desktop client is tested separately in `web/`.

Create a dedicated AVD named `shep-e2e` with Android API 36 and start it. With Android SDK, Appium 3.5.2 and Node 24 installed:

```sh
python3 scripts/clients/android_e2e.py --device emulator-5554
```

The runner refuses personal devices, runs preview and actual native-bridge integration tests sequentially, rebuilds the isolated preview APK, then runs Appium. Never run the two native drivers against the same emulator concurrently, or run competing Flutter builds/tests in one checkout; generated shader assets and plugin registrants are shared. Only `so.shep.shep_mobile.preview` is reset. Native bridge scenarios save/reopen/edit/discard a draft through controls, reject an isolated loopback account probe roundtrip test-only credential pairs, and open the real production entry in this isolated preview package. Rust profiles use fresh temporary directories; credential entries use unique fixture identifiers and are removed afterward. No production app/account data is touched. The `test_driver/native_driver.dart` host callback saves captures from `flutter drive`; Flutter test cleanup can uninstall its test package, so screenshots must be collected before cleanup. Screenshots are in `artifacts/flutter/`; convert review copies to WebP with a standard lossless image converter.

The profile-history scenario runs the production Rust bridge and SQLite in two
isolated Android stores, with the fixture credential store locked. It exercises
metadata exchange, conflict review/resolution and reopening queued edits; the
same scenario runs in the host FFI suite. It does not drive Settings enrollment
or authenticate Google. Run only that backend integration with:

```sh
python3 scripts/clients/android_e2e.py --device emulator-5554 --profiles-only
cargo test -p shep-profile-core --features history
```

The wrapper deletes the previous report and requires the exact completion marker;
a driver exit without the scenario result fails. Keep provider transcript tests,
actual Settings controls and Apple/live execution separate. See
[the journal contract](agents/PROFILE_HISTORY.md).

The Android runner also pairs `attachments_android_test.dart` with `android_compose_fixture.py`. The host hands over a generated SQLite mailbox before the app opens it, then uses actual DocumentsUI input to cancel one picker and select two files. Flutter controls exercise cached Reply all, save/reopen, file removal, pending text saves and send refusal without credentials. `--compose-only` reruns just this scenario while developing; it is not the full Android suite. The helper accepts only the dedicated emulator and preview package. Apple file-picker coverage remains open.

The incoming-file scenario pairs `incoming_android_test.dart` with `android_incoming_fixture.py`. It seeds an isolated cached MIME profile before opening it, drives the actual Android save picker through cancellation and a successful save, and reads the selected file back to verify binary bytes. Reader controls move the message, refresh with locked credentials and retain the open message after its Inbox row disappears. `--incoming-only` reruns this scenario. Host tests also cover save errors, stale attachment identities and corrupt-file metadata without hiding the cached body. Apple export is implemented but uncompiled/unexecuted on this Linux host; simulator and interrupted-save cleanup coverage remain open.

The runner then pairs `outbox_android_test.dart` with `android_outbox_fixture.py` before rebuilding the preview for Appium. Its synthetic SQLite records are handed over before the native profile opens. A separate Android process verifies refusal to acquire the held profile lock, with an independent successful lock as a control. Real Flutter controls exercise light/dark Outbox review, disabled uncertainty actions, recovered text/Bcc/files, file removal, manual mark, local Sent and reopen. `--outbox-only` reruns this scenario. Recovery never sends; an explicit later Send without credentials remains refused with the recovered text intact. The native driver also runs the same Sent-handover control scenario as the host suite: a held sync, retained reader, late body result and pre-handover Undo, with transport-only fixture gates. The Outbox fixture additionally hands over a saved receipt plus cached provider row before startup, and real controls repair/deduplicate it through Rust. With the native credential reconnect/cleanup scenario, these were thirteen integration scenarios before the formatted-reader scenario below; the default runner now also includes its five Appium stages, followed by the existing six general Appium stages; test teardown callbacks are not additional scenarios.

Native handles for one canonical profile share their database and account-operation coordination. The companion owner-lock file is never removed; blocking SQLite work retains its lease through caller cancellation. Only a new exclusive owner reclassifies abandoned submissions as uncertain. Android uses Bionic `flock` because the pinned Rust standard-library Android implementation does not implement file locking. The emulator checks the actual OS lock, beyond the host subprocess test. Apple locking/runtime execution still requires macOS verification.

Sent-copy tests use the same shared `SentConnection` contract as the desktop. Native Rust fixtures hold an APPEND open, cancel its waiter, fail journal/cache writes before and after acknowledgment, change account/policy settings, restart an unfinished upload, and reconcile a synced copy without removing local edits. The Android Outbox scenario also repairs a preloaded provider acknowledgment without credentials, exercises refused lookup/retry with missing credentials, and saves/reopens Sent policy/folder preferences through real controls. The saved Android scenario additionally flags, marks unread and archives a local IMAP Sent message with the fixture credential store locked, then reopens it and checks the saved controls. The same scenario checks missing/locked credential rollback on cached server mail and visible reader recovery/dismiss controls. This proves device/cache behavior; the separate shared-core TLS transcripts prove the production IMAP lookup/APPEND path. Neither is live-provider verification.

Shared Sent lookup reads final command responses explicitly, rejecting NO/BAD, missing UID/header results and conflicting identities. Pinned gateway tests exercise Sent discovery, lookup and exact binary APPEND through both implicit TLS and STARTTLS; the wrong hostname must fail before authentication. See [IMAP command completion and APPEND](https://www.rfc-editor.org/rfc/rfc9051.html#section-6.3.12) and [special-use mailbox discovery](https://www.rfc-editor.org/rfc/rfc6154.html). Browser Sent gateway reservations and UI recovery now use the same transport. Its local/provider identity handover and logical grouping have the continuation evidence below; labels/history remain open.


On a Mac with Xcode and an installed iOS runtime:

```sh
python3 scripts/clients/apple_simulator.py
```

It creates and deletes only its own simulator. Apple execution, native XCUITest/Appium coverage and signing remain open until run on macOS; Android/Chromium evidence is not Apple verification.

## Browser and gateway

Browser builds/tests require the Rust WASM target and matching wasm-bindgen CLI. The npm pre-scripts compile the shared MIME crate and the shared profile crate (codec plus in-memory history) and generate ignored `web/src/wasm/` glue; no generated binaries are committed. Install the pinned tool once from the root:

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

Playwright drives the browser list/reader, input-safe shortcuts, resizing, compose/calendar forms and login presentation. Axe checks light/dark at 1440×920 and 900×640. Rust gateway tests check the actual middleware/session routes plus signed synthetic Google-token claims. The explicitly selected `real_browser_beta_gate` test runs production web assets against the real Rust router through an isolated local HTTPS proxy, checking anonymous assets/API denial, denied/allowed users, callback replay, secure cookies, CSRF and UI logout, and now the provider consent stages: denied consent leaves the saved connection unchanged, accepted consent binds Drive-only access through the fixture provider, and discovery plus a complete three-file publication run through the real app-data proxy without any token reaching the browser. The saved provider flows also exercise account probe failure/retry, streamed mail, flags and draft reload, password reconnect, and lost/uncertain SMTP delivery recovery through actual controls and IndexedDB. Google identity exchange and mail transport are object-scoped `cfg(test)` fixtures; no production bypass exists. Separate shared-core loopback TLS tests exercise real IMAP/POP3/SMTP and STARTTLS with hostname verification. Backend tests additionally disconnect an accepted SMTP HTTP waiter and inject a provider panic; the supervised operation still records its final or uncertain status. Its self-signed HTTPS test key is public fixture material. Browser evidence is in `artifacts/beta-browser/`. These do not establish live Google, SMTP or VPS behavior.

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


The formatted-reader scenarios use `shared/html-reader-fixture.eml`, a synthetic notification with tables, authored colors, a CID dog image, a remote banner and quoted history. `web/e2e/formatted-reader.spec.ts` seeds an isolated browser cache before mounting the production UI, then uses actual controls. It verifies Copy/paste, shared Find across spans, quote visibility, fallback highlights, stale-worker cancellation, CSP failure/retry and compact layouts. Sender-script/network/parent-access checks run inside the actual opaque frame; read-only DOM observations do not drive actions. `provider-flows.mjs` separately exercises the production build under the Rust HTTPS gateway's inherited CSP. Native Rust tests cover the same preparation contract; Flutter native WebView controls use the scenarios below; Apple execution remains open.

The formatted reader pairs `formatted_android_test.dart` with `android_html_fixture.py`, using a pre-open synthetic cache and read-only DOM observations. It captures the actual Android screen through the helper, avoiding the integration screenshot converter while a native WebView is mounted. The WebView DOM is a read-only observation oracle; Find, quotes and retries use actual Flutter controls. `formatted_native.mjs` separately uses Appium against the ordinary Flutter binding for selection/Copy/paste, links and dark mode; it includes interactive floating windows when inspecting Android’s selection toolbar. Use `--formatted-only` for this targeted scenario. Its browser equivalent uses `test/formatted_main.dart`; `generate_html_fixture.py --check` verifies that the test document matches current shared Rust preparation before Playwright and the Android formatted preview run. The native and browser runtime remains mounted across reader updates and releases inline blob resources when disposed. See the completion log for actual results and remaining gaps.


Android picker helpers delete their owned prior UI dump before each observation: `uiautomator dump` can report success without writing during transitions. Empty/malformed observations are retried within the existing scenario deadline. The incoming test publishes `cancel-file` immediately before its actual first Save click, so the helper cannot cancel a leftover picker while the new profile starts. `test_client_picker.py` covers a successful command that leaves an old dump, then recovery with current controls.

Native formatted Find tests await both the Flutter result count and the observed WebView highlight/scroll result. The native JS bridge applies commands asynchronously; `pumpAndSettle` only proves Flutter's scheduled frames settled. Keep these DOM observations read-only, with the existing bounded deadline, and preserve real Flutter controls for every action.


Complete-source Forward preparation uses `shared/forward-fixtures.json` in native Rust and WASM tests. It checks exact binary files, MIME types on duplicate names, isolated CID resources, alternative selection, damaged-file refusal and complete plain text. Shared core tests construct outgoing MIME with a reserved Message-ID, independent draft/file ownership and plain fallback when the quote is edited. WASM metadata excludes file bytes; callers copy each binary file from the owned result and free it. Retained outgoing HTML still requires the confined renderer for display. Client cache, gateway and actual control scenarios extend these preparation tests; their execution evidence is in the completion log. Apple execution and live delivery remain separate.


The saved Forward path is `android_e2e.py --device emulator-5554 --forward-only` (or the full wrapper). Its helper hands over synthetic cached MIME before the native profile opens; test-only acknowledgment loss wraps the real cache operation. Android controls verify blank recipients/new thread, exact retained file metadata, file removal, restart, missing-credential refusal, complete long source, damaged resources and a newer editor during preparation. External ADB captures avoid a mounted WebView screenshot hang; the same dedicated-emulator helper uses Android Back to dismiss the keyboard before the file-removal click. Browser `forward.spec.ts` exercises real WASM workers/IndexedDB and the same failure/restart controls, plus remapping and compact composer accessibility. The HTTPS provider flow separately exercises the production worker under gateway CSP. Composer accessibility checks target the active modal; authored-email contrast remains a separate rendering audit.


## Client printing

`web/e2e/printing.spec.ts` uses real browser controls and a separately owned Xvfb/Chromium profile for `window.print()` → Save as PDF. Install Xvfb and Poppler (`pdfinfo`, `pdftotext`, `pdftoppm`) on the Linux browser runner. It verifies full long-message output, headers, filenames, inline images, retry, independent navigation/editing and remappable/input-safe shortcuts. Stable synthetic PDF evidence is under `artifacts/web/print-output/`. The Rust HTTPS flow also verifies protected `print.html` and the production worker/runtime CSP.

Run the native printer scenario with:

```sh
python3 scripts/clients/android_e2e.py --device emulator-5554 --print-only
```

It is also included in the full Android wrapper. The fixture is handed over before the cache opens; Flutter drives Shep controls and the saved ADB helper selects Android's printer destination, cancels, retries and saves PDFs through DocumentsUI. The helper validates complete PDF text and pagination, then acknowledges completion before fixture cleanup. Review `artifacts/flutter/native/print/` PDFs and WebP captures. Android/Chromium results do not establish Apple or other browser-engine printing. Preparation/dialog launch is not a print receipt.


Counted move feedback uses `move_feedback_scenario.dart` from both host and Android native tests. Real swipes/buttons exercise two moves, partial rejection, pending Undo, failed reversal, Retry Undo, Dismiss and Undo from another open reader while read-on-leave is held. Query-only page projections restore the row and count without waiting for the provider. The pure model tests control the six-second clock and cover destination/account grouping, stale callbacks and acknowledged Undo warnings. `move-feedback.spec.ts` exercises browser controls/expiry; the real Rust HTTPS flows retain physical provider destinations. Keep general refresh/save status independent of the expiring move notification.


Captured-selection prerequisites have Rust tests in `flutter/rust/src/selection_tests.rs`, bounded-controller tests in `flutter/test/mail_selection_test.dart`, and an actual FFI contract in `native_repository_test.dart`. The 100,000-message fixture verifies cardinality and bounded responses; it is not a latency benchmark. Held selection/provider work and cancelled callers verify independent cached reads/saves and FIFO ownership. These contracts do not establish selection-control or durable bulk-action parity; those controls and their Android/browser scenarios remain active.


`web/e2e/selection-storage.spec.ts` runs the actual SQLite/WASM selection worker against Chromium IndexedDB. Ordinary captures stream metadata; only text searches read cached bodies. The 100,000-row fixture requires a production draft save to commit while capture remains pending, limits observations/review pages to 50 and checks the 32-request queue. Separate scenarios cover snapshot revisions, range/arrival behavior, current flags/aliases, missing mail, account removal, replay-window overflow, failed recapture rollback, tab isolation and close/reopen. `beta-gateway.mjs` loads the built worker under the real HTTPS gateway CSP. These are storage contracts; browser controls have the separate scenarios below and durable bulk E2E remains open. The concurrency/cardinality scenario is not a responsiveness benchmark.


`web/e2e/selection-controls.spec.ts` seeds a fictional 125-message cache and drives the production repository/worker through real checkbox, modifier, keyboard, paging and preference controls. It checks unread preservation, passive arrivals, scope changes, typed search, remapping/disable, startup failure/retry and abandoned capture with the actual WASM request held. The formatted-reader scenario separately verifies ordinary Ctrl+A selection inside the sandboxed frame. Production HTTPS includes Select all/Clear/Done and unread preservation. Controller tests retain bounded/lost/stale/offscreen behavior; Dart mirrors the pending offscreen/range fixes. Native selection controls, durable bulk recovery, Apple/other browser engines and final performance remain open.


Browser intent ordering has saved scenarios in `web/e2e/mail-intents.spec.ts`: two-tab same-value choices, per-field group Undo claims, schema-five migration, clock exhaustion, alias rollback and pending-account cleanup. Actual row clicks verify reservation before queued work and superseded display reconciliation; actual Preferences controls verify review gating and account removal. Provider unit tests cover accepted field subsets, no stale wire writes, acknowledged action-record failures and unknown outcomes. These do not establish a complete group runner or restart replay safety; pending records are not automatically replayed.


`web/e2e/bulk-executor.spec.ts` exercises the durable executor with frozen SQLite membership, real IndexedDB and Web Locks, and the production provider adapter using fictional responses. It verifies complete 125-message execution, partial/superseded field ownership, physical MOVE Undo, cache/applied-revision rollback, migration, identity recovery without repeated mutation, definite retry, lost acknowledgments, graceful stop and tab loss. Observe the specific Web Lock release after closing an owner tab; page-close completion alone is insufficient. The second tab must load cached state without reconnecting behind the deliberately occupied account lock. These API/transport scenarios do not replace the still-required visible group-control flows or live-provider verification.


The mail-schema-seven executor regression opens a real older writer, observes version-change closure, rejects both a late write and reopening at the older version, and preserves the prior intent clock and Sent roles. Cache-applied ownership must not be inferred from older acknowledgment-only status.


Flutter reader actions use `test/support/reader_actions_scenario.dart` from host tests and `integration_test/native_mail_test.dart`. Preparation and cache replies are held in synthetic adapters; real controls exercise a held Print touch, delayed detail/files, busy labels, duplicate refusal, Forward failure, scrolling and Reply. Host coverage includes 150% text with a bottom safe area; Playwright enforces real 44-pixel footer targets under desktop density. Its Find helper observes actual input focus and native Select all; prefilled Reply fields are observed after focusing their editing proxies. Wait for the actual route animation to finish before recording geometry. `printing_android_test.dart` requires Print to be visible without scrolling, including the long source; its saved helper still cancels and saves actual PDFs. Formatted Playwright and Appium additionally verify footer reachability and Move/Reply after Find and scrolling. The completion log records executed platforms and reviewed captures; Apple remains open.


`web/e2e/mailbox-storage.spec.ts` exercises the persistent SQLite/OPFS query worker against real Chromium IndexedDB and Web Locks. Its queries compare metadata pages, substring search, Unicode/quoted tokens, folder roles, pending fields, aliases and separate bodies. It recreates the source at the same revision, coordinates multiple tabs, and checks a 100,000-message rebuild while a production draft write completes independently. Failure/replay and worker-lifecycle evidence belongs in the completion log. Storage tests also hold the index lock while foreground body/metadata reads and indexed 50-row scans finish, and reject use after shutdown. `mailbox-controls.spec.ts` drives actual production paging, body/page Retry, stale body/query errors and a held row Flag press during asynchronous loading. `mail_paging.test.ts` covers retained-reader/page bounds, projection rollback, coalescing, removed accounts, evicted-metadata Undo and speculative cache budgets. Cardinality and correctness are distinct from final latency evidence, which remains open. The VFS follows SQLite's [cooperative SAH-pool lifecycle](https://sqlite.org/wasm/doc/trunk/persistence.md), and the derived search table follows its [FTS5 external-content contracts](https://www.sqlite.org/fts5.html#external_content_tables).


`web/e2e/bulk-controls.spec.ts` drives full mixed-account reviews, cancellation, all 125 messages across three pages, History/Undo, held provider requests, held native button presses, definite retry, explicit uncertain-result acceptance and conservative startup recovery. `bulk-projection.spec.ts` checks off-page counts/membership, newer field ownership, receipt/cache ordering and incremental journal observations. Keep light/dark/compact screenshots and axe checks; these synthetic controls complement the existing executor/provider transcripts, not live mail verification. History retains at most 20 groups and one 50-item page. Undo acknowledges input immediately and projects after its durable decision; synchronous row/count restoration while local decision storage is held remains an active parity gap.


The same file now covers the wider Undo lifecycle with real controls: Undo from History after moving to another page and after changing to the Archive scope, Undo of a group approved behind a held earlier group (all rows cancelled before sending, no provider call), Undo after a partial failure (only acknowledged rows restored, the failed row keeps no Retry) and two overlapping groups (the newer per-field choice wins, each Undo restores its own baseline). Its review-key scenarios press Y/N/Enter/Escape in the dark compact review and in History with checked folder reviews, verify that a declined or closed review is retired from the journal, and that Enter in the Move folder field opens the review without approving it. `preferences-controls.spec.ts` remaps the approve/decline review keys through the actual capture controls, checks conflicts, and verifies the review honours the new bindings after reload. `bulk_undo.test.ts` repeats the queued, partial-failure, overlapping and unconfirmed cases against the real journal and executor on an in-memory IndexedDB.


Abandoned-review cleanup has unit evidence in `bulk_journal.test.ts` (classification, bounded strict transactions of at most 50 item rows plus the job record, untouched approved work/receipts/pending repair, cancel refusals, rollback of an aborted cleanup transaction and executor startup retirement) and `bulk_client.test.ts` (tab liveness lock, periodic sweep scheduling that waits for live runs and ignores another owner's refusal). `bulk-recovery.spec.ts` seeds an ownerless 58-row review, a two-row tail and interrupted staging beside the older failed/unconfirmed fixtures; the startup scenario asserts they are retired before the held provider step while approved-but-paused groups keep the older entries beyond the first History page. A further scenario keeps a review open in one tab, opens a second tab whose startup sweep must leave it alone, closes the first tab and checks the reload retires the review with no mail change. The unit tests use `fake-indexeddb` and a small in-test Web Lock manager under `web/src/testing/`.


`web/e2e/preferences-controls.spec.ts` holds an ordinary fictional provider flag completion while actual shortcut controls receive a mouse press or modifier/key input. It verifies retained focus/activation, remapping persistence, conflicts, Escape, active Clear, switching captures, appearance and navigation cancellation. Light/dark screenshots are synthetic. These controls use the same isolated browser cache/transport fixtures as mailbox tests, never the owner’s phone or accounts.


The successive-group control holds the initial History observation, a provider result and Undo-preview queries separately. Progress must reach the actual completed count while the preview stays held; Undo activates only after preparation. A rejected-preview scenario retains a persistent error, retries through Refresh history and restores the group. These deterministic fault boundaries reproduce the observed stale History count without relaxing assertion deadlines.


`web/e2e/bulk-recovery.spec.ts` seeds historical group journals, holds unrelated provider work and uses actual startup, status-refresh, History/review and cache-retry controls. It covers older groups beyond the first 20 History entries, counts beyond a 50-item page, light/dark layouts, failed observations, acknowledged receipts, live-owner/tab-loss behavior and abandoned-review retirement. `bulk_recovery.test.ts` controls read/execution ordering, coalescing, retained status-check failures and disposal. These are synthetic client storage/transport fixtures; live-provider and native equivalents remain separate work.


## Shared profile metadata

`cargo test -p shep-profile-core` and `cargo test -p shep-mail-core --test profiles` validate the operation format and explicit account mapping. `shared/profile-cases.json` supplies the same 28 valid/invalid interoperability cases to Rust, `flutter/rust/src/profile_tests.rs`, the actual Dart FFI scenario in `native_repository_test.dart`, and `node scripts/clients/test_profile_codec.mjs`. The WASM script requires the pinned CLI/target above and writes only ignored `artifacts/profile-codec-*`. It runs the WASM ABI in Node, not browser Settings controls.

Cases retain unknown optional fields, reject unsupported versions/capabilities/security, duplicate JSON/targets/parents, malformed identities and known local/secret fields. Additional Rust/WASM checks cover size/depth bounds and output encoding; account mapping tests resolve legacy SMTP defaults and keep unknown connection extensions read-only. Native validation finishes with provider capacity held and preserves the original accounts/mail; actual Dart FFI keeps the credential store untouched. This is metadata validation, not working OAuth, enrollment, causal merge, credential transfer or complete database migration. See [the format subset](agents/PROFILE_FORMAT.md).

The same Node script now also drives the WASM `ProfileHistory` entry: import with a missing parent, the incomplete-edit refusal, a local edit, persisted records and an identical restore that rejects another device identity. `cargo test -p shep-profile-core --all-features --test memory` runs every history command on the SQLite journal and the in-memory journal side by side and compares state, overview, fields, versions, values and uploads after each step, plus restore round trips.

## Browser profiles and sync

```sh
cargo test --manifest-path backend/Cargo.toml profiles
npm --prefix web test -- src/profile_flows.test.ts
npm --prefix web run e2e -- e2e/profiles.spec.ts
cargo test --manifest-path backend/Cargo.toml real_browser_beta_gate -- --ignored
```

Backend tests use a scripted `FixtureProvider` (no network): exact requested scopes and PKCE parameters, omitted/broader/partial scope interpretation, the Drive principal binding, denied and failed exchanges and a mismatching Google subject preserving the active grant and choices, replayed states, server-side refresh before Drive calls with narrowed scopes and refresh failures reported as reconnect/retry instructions, proxy request validation, disconnect/logout/new-login cleanup, CSRF, and an unconfigured namespace reported explicitly. `web/src/profile_flows.test.ts` runs the real profile WASM with fake IndexedDB: record persistence and restore, port error kinds, discovery with 55 files and 52-profile paging, an incomplete listing saved as a failure and retried at the same revision, missing known files, altered media, changed identities, pause/resume, replayed additions, frozen publication reviews with a lost upload reply retried without duplicates, pause after an accepted step, enrollment copying/rows/application with newer local settings kept, a lost account reply retried with the same reserved identity and receipt after reload, changed connections offered separately, incomplete profiles refused, and preference revisions/receipts. `web/e2e/profiles.spec.ts` drives the actual Preferences controls in Chromium against a Node-side Drive fixture: consent choices and denial, disconnect, discovery failure/retry/paging/reload, the Create profile dialog with a lost reply, pause with mail browsing and resume, the Use profile dialog with a lost account reply, retry and a reviewed reconnect entering both passwords, and the three onboarding offers. Captures are saved under `artifacts/web/profiles/` in light and dark; axe runs on each state. These fixtures never contact Google and do not establish live provider access or cross-client interchange; see [browser profiles](agents/PROFILE_BROWSER.md).


## Desktop Google permission choices

The saved `test_google_requested_permissions_*` native scenarios use actual Drive checkbox/Calendar menu controls, saved/reopened settings and fixture-only sign-in refusal. A compact dark flow requests broader next-sign-in permissions, then opens an existing read-only event and verifies access is unchanged. Existing partial-grant/disconnect tests remain. Use explicit fictional `SHEP_GOOGLE_CLIENT_ID` and `SHEP_GOOGLE_CLIENT_SECRET` values for these fixture runs; no real credentials or Google authorization is used.

`cargo test --all-features google` covers exact requested URL scopes/PKCE, omitted-scope fallback, broader-grant restriction, refresh after editing choices, scope-bound candidate retry before/after restart, changed-settings activation refusal and UI save acknowledgment ordering. Native fixtures establish controls only; the real provider path remains covered by isolated protocol tests, with live Google and mobile/browser consent separate.


## Mobile Google consent

[Google configuration and current limits](agents/GOOGLE_MOBILE.md) distinguish native SDK sign-in from the still-open Calendar/Drive/profile integrations. Run `flutter test` for controller, compact controls and secure-store lost-reply cases. Run the actual SDK-adapter contract with fictional build configuration:

```sh
cd flutter
flutter test test/google_sdk_test.dart --dart-define=SHEP_GOOGLE_SERVER_CLIENT_ID=fixture-client.apps.googleusercontent.com
```

`google_controls_scenario.dart` is shared by host and `google_android_test.dart`: saved services, denied editing that retains read access, retry, dark mode, reviewed disconnect/cleanup and browsing/changing choices during held consent. Run `python3 scripts/clients/android_e2e.py --device emulator-5554 --google-only` for this development subset. The full Android wrapper includes it. Flutter Playwright and the preview Appium script also exercise consent/cancel/retry/disconnect through actual controls. Success fixtures live only under `flutter/test/`, use fictional identities and never contact Google. SDK-boundary tests inspect exact scopes/identity, no interactive background authorization and local sign-out rather than revocation. Platform configuration, real account switching/refresh/callbacks, Apple execution and cross-client app-data visibility remain separate verification.

The Flutter preview network gate rejects all external HTTP(S) requests. Native Google packages avoid web SDK registration; bundled Roboto supplies CanvasKit's otherwise downloaded fallback while the UI continues using Noto Sans. Font licenses are included as application assets. Google controls have separate semantics nodes so actual pointer/accessibility targets match the painted fields.


### Profile discovery controls

`flutter test` includes saved-identity and discovery lifecycle tests plus the
compact retry/paging/appearance and pending-mail-navigation controls shared with
`profile_discovery_android_test.dart`. The native bridge enables the shared Drive
transport; Rust session tests separately exercise held operations and catalog
ownership. UI providers are explicit fixtures, never personal Google accounts.

```sh
python3 scripts/clients/flutter_web_e2e.py --discovery
python3 scripts/clients/android_e2e.py --device emulator-5554 --discovery-only
```

The Android subset verifies named integration reports, then rebuilds the isolated
preview and runs Appium. `flutter/e2e/profile_discovery.mjs` runs the same real
retry/dark/disconnect/mail flow on Chromium and UiAutomator2; it saves captures
and refuses external browser requests or non-Shep/non-emulator native targets.
Standalone preview entry: `test/profile_discovery_main.dart`. Use the native
[configuration and boundaries](agents/PROFILE_MOBILE.md) for production builds.
Browser tests here exercise the Flutter automation surface, not the separate
hosted email client. Live Google, enrollment and Apple remain distinct work.


## Flutter first-profile publication

```sh
python3 scripts/clients/android_e2e.py --device emulator-5554 --creation-only
python3 scripts/clients/flutter_web_e2e.py --creation
```

The Android wrapper runs two saved integration scenarios, rebuilds the explicit
`test/profile_creation_main.dart` preview, then runs four UiAutomator2/Appium flows.
The Flutter web wrapper runs the same four flows through Playwright. Both use
`flutter/e2e/profile_harness.mjs`, also used by discovery. Re-run discovery controls
after changing this harness. Profile providers are isolated fixtures. In web mode a
control label may match the start of a clickable name or follow whitespace, because
Flutter web merges a list tile's title and subtitle into one node; Android already
uses contains-selectors.

`profile_creation_controls_test.dart` shares the Android review/paging/retry/
appearance and pause/browse/resume controls. Native Rust tests separately execute
the real SQLite publication path, receipt recovery and session retirement. See
[the publication contract](agents/PROFILE_PUBLICATION.md) and completion evidence;
these checks do not establish live Google, enrollment or continuous sync.


## Reviewed Flutter enrollment

Run `python3 scripts/clients/android_e2e.py --device emulator-5554 --enrollment-only`
for the two saved native review/application and pause/browse/resume scenarios, including
two Preferences taps before the next repaint, then
the five shared Appium flows. The wrapper requires the exact two completion names
in `integration-enrollment-result.json`; teardown callbacks and an interrupted
driver are not additional passes. Run
`python3 scripts/clients/flutter_web_e2e.py --enrollment` for the matching five
Playwright flows. Both use an isolated profile source and the real Flutter controls.

`profile_settings_test.dart` and `profile_application_test.dart` use production
preference persistence over object-scoped storage, with failed/lost writes, queued
edits, explicit resets and field revisions. `profile_enrollment_test.dart` covers
paged choices, category selection, lost account/settings replies and Google
disconnect while one accepted step is pending. Native Rust tests copy original
records into independent journals, review 75 accounts in bounded pages, apply
credentialless accounts, preserve mail/drafts and newer local metadata, reopen
receipts, activate Reconnect credentials and keep local removal suppression.
Original platform receipt revisions are checked independently of newer display
values, with failed native commits, restart, legacy receipts and changed retries.
The saved UI flow leaves a lost settings acknowledgment, changes appearance and
resumes the same application; its original receipt and newer local theme survive.
Preferences retain their scroll position, so the native/browser flow scrolls to
Theme after returning to that page.

The authenticated catalog export has separate scripted protocol tests. These
combined tests do not establish fully authenticated Google-to-Flutter interchange
or live provider login. Apple execution and automatic enrollment remain open;
ongoing preference sync is covered below. Record actual results, reviewed WebP
captures and the shipping commit in the completion log.

## Ongoing Flutter preference sync

```sh
python3 scripts/clients/android_e2e.py --device emulator-5554 --sync-only
python3 scripts/clients/flutter_web_e2e.py --sync
```

The Android wrapper runs two saved integration scenarios and requires exactly
`sync-controls-conflicts-receipts` and `sync-disconnect-pause` in
`integration-sync-result.json`, then rebuilds `test/profile_sync_main.dart` and
runs the seven shared UiAutomator2 flows from `flutter/e2e/profile_sync.mjs`. The
web wrapper runs the same flows through Playwright. Both use the isolated
`FixtureProfileSync` provider, real Preferences switches and the same
`profile_harness.mjs`, which now has a `toggle` helper for switch/checkbox roles;
re-run the discovery, creation and enrollment flows after changing it. Sync rows
sit below the fold of a lazily built list, so the flow reveals each result by real
scrolling before asserting it.

The saved flow seeds sync from the applied profile, turns it on, applies a remote
theme through the device receipt, loses that receipt's reply and retries the same
identity, turns one preference off, then changes the theme twice while the
fixture's other device does the same: the first 52-version conflict is resolved
with Keep mine after both pages were opened, the second with Use profile, and mail
browsing continues afterwards. `profile_sync_controls_test.dart` shares those
controls with the Android driver and also holds a cycle while Google disconnects.
`profile_sync_test.dart` drives the controller over production preference
persistence: paused seeding, remote application with frozen receipt revisions,
publication of local intent, per-field switches, lost confirmations retried with
the same identity, failed cycles, paged reviews, stale decisions, and a held cycle
that cannot update a replaced grant. Mobile Rust tests (`profile_discovery::sync`)
use an in-memory second device for seeding, admission before pull, convergence,
restart, kept receipts, conflicts, unproven fields, lost history acknowledgments,
controls, incomplete pulls, rebuilt sources and enrollment serialisation.

These fixtures do not establish live Google delivery between clients, account
definitions or the remaining portable categories; see
[Flutter profile boundaries](agents/PROFILE_MOBILE.md).
