# Shep development instructions

## Product direction

Shep is a native Rust + iced email and calendar client. Mouse usability and keyboard usability are equally important. Do not describe it as keyboard-first. Keep common actions visible and clickable; shortcuts are optional, native, remappable accelerators. The default move shortcut is `M`. Never run letter shortcuts while a text field or editor consumes the keystroke.

Keep the restrained shadcn-inspired design: clear hierarchy, comfortable spacing, rounded controls, subtle borders, quiet violet accents, useful empty/error states. Appearance has Light / Dark / System choices in Preferences. The user approved the flat Swiss Shepherd profile in `assets/swiss-shepherd.png`; preserve the design. App raster assets use WebP. Small logo variants are loaded once, not read from disk on every frame. Vector UI icons remain SVG.

## GitHub Actions are DISABLED — reminder to re-enable

**The user explicitly requested that all GitHub CI remain disabled for now.** Repository: `sam-ruff/shep.so`, private, default branch `main`. Direct pushes to `main` are currently authorized.

Workflow definitions are deliberately named `.github/workflows/ci.yml.disabled` and `release.yml.disabled`, and repository Actions permissions are set to `enabled: false`. Do not enable them as a side effect of ordinary development. Remind Sam to re-enable them when the self-hosted runners are ready.

To enable when Sam asks:

1. Provision trusted self-hosted runner labels from the CI matrix: `[self-hosted, Linux, X64]`, `[self-hosted, Windows, X64]`, `[self-hosted, macOS, ARM64]`. Adjust labels to the actual machines first. Do not run untrusted fork code on persistent self-hosted runners.
2. Install Rust with `rustfmt` and `clippy`, Python 3, Node 24, and platform development libraries. The Linux GUI harness additionally needs `Xvfb`, `xdotool`, and ImageMagick `import` with WebP support. Linux needs OpenSSL/dbus/X11/Wayland development packages and a Secret Service for real credentials.
3. Rename both `.yml.disabled` files to `.yml`.
4. `gh api --method PUT repos/sam-ruff/shep.so/actions/permissions -F enabled=true`
5. Run the quality workflow manually, inspect results, then let the release workflow run only after a successful push build on `main`.

## Build and quality gates

```sh
cargo run --release                         # normal local workspace; no fixtures
bash scripts/install-hooks.sh              # install repository Git hooks
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
python3 -m unittest discover -s tests -p 'test_*.py'
cargo bench --bench responsiveness
cargo build --profile test-ui --features test-support
python3 scripts/e2e.py                      # real native UI flows through MCP
bash scripts/check.sh                      # all of the above
```

The pre-commit hook runs formatting, Clippy and Rust tests. The commit-msg hook requires Conventional Commits. Do not skip failing hooks or weaken a budget just to get a commit through. `SHEP_SKIP_E2E=1 bash scripts/check.sh` runs the non-GUI checks on machines without Linux/X11; report that omission. CI repeats the checks; Rust compiles/tests run on all three platform runners, native E2E currently runs on Linux.

Defer performance measurements while the PC is saturated with other work; run them at the end on an otherwise idle machine. `python3 scripts/e2e.py --functional-only` runs correctness flows without the latency benchmark. Never weaken thresholds based on a loaded-host result.

During development run targeted tests after backend changes, the benchmark after storage/scheduling changes, and the MCP E2E suite after UI changes. Run the complete relevant set before pushing. Do not claim live Google, IMAP, POP3, SMTP, CalDAV, Windows or macOS verification based solely on fixture tests.

## MCP testing and automated equivalents

Read the repository skill [`.agents/skills/shep-e2e/SKILL.md`](.agents/skills/shep-e2e/SKILL.md) for UI work. `scripts/mcp_harness.py` is a stdio MCP server using JSON-RPC and real X11 input. Tools: `desktop.start`, `desktop.batch`, `desktop.state`, `desktop.screenshot`, `desktop.stop`.

Prefer batches of related clicks, drags, typing, shortcuts, short waits, state assertions and screenshots. Batches stop at the first error and save a failure screenshot. Use `wait_for` for asynchronous state transitions; fixed waits are only for visual settling or explicitly timing a scenario. A wait is at most 2 seconds, with at most 10 seconds of explicit waits in a batch.

**Every AI-driven MCP scenario must have an equivalent automated scenario in `scripts/e2e.py` (or a saved fixture consumed by it).** Both AI and automated runs use the same MCP server and real native controls. Do not replace mouse clicks with direct calls to update functions. The state file is an observation-only oracle, never an action API. Save screenshots for visual review; JSON state alone cannot prove layout quality.

Test/demo fixtures live in `tests/support/fixtures.rs`, gated by the nondefault `test-support` Cargo feature. Production builds do not contain fixture messages. The harness launches an isolated in-memory workspace with `--demo --test-state <file>`, owns its own Xvfb process, and cannot send real mail or write cloud data in this mode. Never point automation at personal data or use real credentials in fixtures.

**Do not leave log files in the repository root.** Write build/test logs to `artifacts/logs/`; remove stray root logs before finishing. The user explicitly requested this.

Artifacts go to ignored `artifacts/e2e/<run>/`: WebP screenshots, state, action timings and app log. Do not commit personal inbox screenshots, OAuth tokens, passwords or local databases. Harness tests are reproducible with Python and MCP; there is no web frontend requiring Playwright.

## Strict responsiveness and usability requirements

- UI update handlers: p95 < 8 ms, target < 2 ms. No filesystem, credential-store, SQL, network, compression, crypto or MIME parsing in iced `update`/`view`.
- Cached inbox/search page on 100,000 messages: p95 < 50 ms. Cached body load: p95 < 10 ms. Search debounces for 100 ms; stale results must not overwrite newer queries.
- Aim for 60 Hz interaction (16.7 ms frame budget), cached message navigation under 100 ms, and visible acknowledgement within 100 ms. Measure full native input-to-state latency separately from handler timing; handler timing is not a frame-rate claim.
- Keep 50 messages per page and render only visible rows plus a small overscan. Prefetch adjacent messages and the next page in background. Retain at most 8 bodies / 32 MiB in the prefetch cache. Avoid decoding assets repeatedly.
- Bounded foreground-read, persistence and provider-command channels (32 each), prefetch/download channels (8 each), and an event channel (32). `engine/dispatch.rs` reserves two foreground read workers and one prefetch worker; settings/drafts use a separate FIFO worker. Provider work has eight slots, with account sync limited to three. `try_send` must never wait on the UI thread. Interactive backpressure produces visible feedback; a full speculative prefetch queue quietly drops that optional request.
- Test browsing, typing, remapping and appearance while a backend operation is pending. Slow or unavailable servers must never disable navigation.
- Prefer 40–44 px click targets; visible focus, descriptive labels/tooltips, persistent errors with a clear recovery, no text clipping at 900×640 and 1440×920. Mouse and keyboard should reach the same core actions.
- User-visible messages should explain the problem and next action. Do not present sample data as live accounts, pretend a sync succeeded after errors, or silently lose unsent drafts.

Run `python3 scripts/performance_gate.py` after backend and native timing reports have been generated. It fails on missing, invalid, undersampled or over-budget evidence.

The inbox/reader divider must remain mouse-draggable with saved preferences and minimum widths. Filtering and sorting must invalidate stale page prefetches; flags map to IMAP `\Flagged` and remain local for POP3. Cover drag persistence, mouse flagging/filtering/sorting and page navigation in the MCP suite.

See `docs/PERFORMANCE.md` for measured results and what the measurements do and do not cover.

## Architecture and data safety

The full product goal is still active. Keep [docs/COMPLETION.md](docs/COMPLETION.md) current with implemented evidence and remaining work; do not infer feature completeness from a passing fixture suite.

`src/ui/` owns presentation and small caches. `engine.rs` bridges bounded channels and background work. `store.rs` runs SQLite WAL/FTS work through `spawn_blocking`. `providers::MailProvider`, `CalendarProvider`, and `backup::BackupProvider` are extension points: add a provider without teaching the UI its wire protocol.

Passwords and Google refresh tokens belong in the operating system keychain, never SQLite. Google login uses system browser + loopback callback, PKCE and state validation. Drive is opt-in and uses the app-private `appDataFolder` scope. Encrypted backups use Argon2id + AES-256-GCM, fresh salt/nonce, authenticated version header, and compress-before-encrypt. Retention runs only after a successful new upload. Never remove unrelated files. Restore validates/decrypts first and merges downloaded messages.

Incoming IMAP/POP3 support SSL/TLS or STARTTLS; SMTP has independent TLS and authentication settings. Fastmail uses implicit TLS on 993/995 and SMTP 465 with an app password. Never disable certificate verification. POP3 leaves server originals intact and has local folders/flags. IMAP mutations check UIDVALIDITY; same-account safe move requires MOVE. Cross-account moves require two IMAP accounts and source UIDPLUS; preserve the destination acknowledgement journal, never remove the source before APPEND succeeds, and never automatically repeat an ambiguous upload. CalDAV writes use ETags; expanded recurring series are not overwritten through a single-occurrence editor.

Current practical limits and unsupported behavior must remain explicit in README: 25 MiB individual-message download ceiling, 256 MiB raw-mail snapshot ceiling, text rendering with policy-controlled external images, Google Gmail uses app passwords rather than Gmail OAuth, calendar sync window -90/+365 days, recurring CalDAV edits belong in the server calendar UI. Improve these deliberately; do not hide them with success messages.

## Releases

Use Conventional Commits: `fix:`/`perf:` patch, `feat:` minor, `!` or `BREAKING CHANGE:` major. `main` is the only release branch. Node is development/release tooling only; the application remains Rust.

`npm ci` installs the locked release tooling. `.releaserc.json` runs commit analysis, notes/changelog, `scripts/release.py` (updates Cargo version and lockfile and creates a native archive/checksums), commits the version files, then publishes a GitHub tag/release. Release definitions are dormant while Actions are disabled. `npm run release:dry` inspects the intended release with authenticated GitHub access but does not publish.

The release workflow waits for a successful full quality workflow and checks that `main` still equals the tested SHA before publishing. Current automated release packaging produces a Linux archive on the Linux runner; Windows/macOS compilation is covered by the CI matrix, but signed installers and distribution builds for those platforms need additional packaging jobs. Do not call unsigned development binaries signed/notarized.


## Linux user installation

`scripts/install-linux.sh` builds the optimized production release and delegates installation to `scripts/install_linux.py`; extracted archives include both scripts and a launcher icon. Install per user without sudo. Match the application ID, desktop filename and StartupWMClass (`so.shep.Shep`) so GNOME/KDE can group and pin the window. `--pin` is an explicit optional GNOME action and must preserve existing favorites. `--uninstall` removes the binary, launcher and icon only, never user data. The installer has automated tests for install/update/uninstall, path quoting and pin idempotence. App raster assets remain WebP; the PNG launcher icon exists for desktop icon-theme compatibility.

## Reading, input and sync regressions

Keep the inbox header compact: title, count and one sync control in one row; avoid duplicate busy badges or privacy slogans in navigation. Up/Down moves within the focused inbox/sidebar, Tab switches those panes, and keyboard navigation scrolls the selection into view. Double-click opens the full-window reader or an all-day calendar event. Open/close reader actions are remappable. Child buttons capture iced mouse-area events: double-click behavior must be tested through real input.

The native background-sync flow captures the compact mail header in light/dark, idle/busy states; the 900×640 flow also clicks Sync and navigates messages. Review those WebP screenshots after header changes. Before diagnosing a screenshot that differs from source, compare the running executable with the installed build: replacing the binary leaves already-open windows on the old executable. Install updates atomically and have the user reopen old windows rather than force-terminating them. Allow the native layout to settle after switching tabs before starting a pane-divider drag; a changed state observation can precede presentation.

Block external images by default. Message/sender/domain exceptions and a manually entered Contacts list are explicit preferences. Image requests validate every redirect and resolved address, exclude local/private networks, bound payload/decoded allocation, and convert off-thread to WebP. Quoted history has collapsed/expanded/latest-only preferences. Attachments wrap in the reply bar; sender details expose copy actions.

The Fastmail sync regression was missing parentheses around IMAP FETCH attribute lists. `imap_sync_uses_valid_fetch_lists_and_batches_bodies` drives the production sync function against a local IMAP transcript and validates both metadata and batched BODY.PEEK[] requests. Live diagnostics are ignored tests requiring an explicit `SHEP_LIVE_ACCOUNT_ID`; they read the saved OS credential and never send, move or flag mail. `saved_account_inbox_sync_to_local_cache` limits downloads to Inbox while using the same sync path. Run live diagnostics only for an account the user has authorized.

Release preparation also runs `scripts/verify_release.py`: it checks SHA-256, extracts into a temporary directory, and exercises the bundled installer without Rust. You can rerun it with `python3 scripts/verify_release.py dist/shep-VERSION-linux-x86_64.tar.gz`. Native key injection uses an explicit 1 ms xdotool delay; performance budgets remain unchanged. The native suite has 21 functional flows plus the navigation performance gate.

Calendar provider writes return the committed event, including its server identity/ETag. Do not make a successful write depend on a subsequent calendar refresh, or retry it as a fresh create. Google creates use a stable per-form ID and verified conflict recovery. CalDAV edits GET the complete resource, retain alarms/attendees/extensions, and use If-Match; a successful PUT without an ETag requires a sync before another edit. Only 2xx acknowledges a commit; redirects are not success. Serialize sync and mutations per calendar. Remote IDs are scoped by calendar in the UI, command keys and storage; the v2 cache migration converts legacy composite keys. Completion events identify their form so they cannot close an unrelated dialog.

Calendar protocol tests use the scripted loopback server in `src/providers/test_http.rs` under `cfg(test)`, with calendar fixtures/re-exports in `src/providers/calendar/test_server.rs`. The shared server records binary request bytes and supports binary replies for Drive tests. Clients and credentials remain object-scoped fixtures. `cargo test --all-features calendar` covers wire failure/retry contracts, cache migration, source identity, iCalendar durations and escaped text. The native suite also edits/deletes duplicate remote UIDs across different calendars. Save logs under `artifacts/logs/`. Live calendar-provider verification remains distinct from these tests.

`cached_reads_and_ordered_saves_complete_with_all_network_jobs_and_queue_occupied` holds every provider slot open and fills the provider queue, then executes actual SQLite search/body reads and saves the latest preferences/draft through the production dispatcher. This is a deterministic correctness test, not a timing benchmark. Keep its blocked-provider condition intact; do not replace it with a short sleep that might finish before navigation. The test-only holding command is compiled only by `cargo test`, never into release or native test-support binaries. Preserve FIFO ordering for draft/settings persistence when changing concurrency.

Google connection status is checked by a provider command after the cached workspace is ready. Do not await OS credential-store access before emitting `Ready`. Failed speculative sends must clear their pending prefetch markers so the message/page can be requested again.

Preferences use client edit generations and persistent store revisions. `SavePreferences` returns a small `PreferencesSaved` snapshot; do not reload an entire workspace for each preference change. `ui/preference_sync.rs` preserves newer local edits and rejects older store snapshots, including while the pane divider's save is debouncing. Backup completion must use `Store::update_preferences` to change only its metadata; never write the preferences captured before a long upload back over current settings. UI saves preserve backend-owned `last_backup`. Google OAuth starts after its settings save is acknowledged, and the provider job must not rewrite the old settings. `tests/preferences.rs` covers persistence/revisions and the metadata race; the native suite combines appearance, unified/cross-account preferences and resizing and waits for `preferences_saved`.

Message-detail requests/results carry `detail_revision`, independent of page/search generations. Increment it and clear pending prefetch markers when mail changes. Ignore both data and errors from earlier revisions; a late body load must not undo new flags or a completed move. `ui/reading_tests.rs` controls backend-result ordering; native MCP mouse flagging/moving remains the control-level test.

Backup actions save the displayed preferences and wait for that generation's acknowledgment before entering the provider queue. Commands and list results/errors carry `BackupTarget`; list generations reject earlier results, and restore keeps the target selected with its copy. `last_backup`, `backup_ready` and `google_connection_id` are backend-owned metadata. `Store::record_backup` only updates a matching destination. Google identity is `drive:` plus the verified Drive `user.permissionId`, scoped by OAuth client in the target; reconnecting the same account preserves pending work, while another account resets readiness/history. Tokens record their issuing OAuth client and refuse use with another client. `Store::record_google_connection` rejects a client change during OAuth completion. The shared Google connection lock serializes login against Drive backup/list/restore operations. Do not restore these device-specific settings from a snapshot.

Every successful manual backup stores its passphrase under a destination-specific OS keychain entry, including when automatic backup is disabled. The first copy prepares the schedule; missing credentials pause it with a recovery instruction. The scheduler reads keychain credentials inside the provider job, never while polling the network dispatcher, and rate-limits failed automatic attempts to the configured interval. `BackupSaved` acknowledges upload before metadata/keychain/retention work; subsequent failures must say the copy was saved. Retention requires the committed ID in a complete, duplicate-free list and always protects it even when the clock changes. Local backups use temporary files and no-overwrite persistence, validate the timestamp/UUID filename, and bound reads even if a file grows.

`src/engine/backups_tests.rs` injects an object-scoped fake passphrase store and uses real temporary encrypted local archives; it never reads or writes the user's keychain. Tests cover manual-to-automatic setup, missing keychain access, metadata/retention failures after commit, destination changes, reopening a staged/committed copy without duplicate upload, and finishing failed setup on the same copy. `tests/backups.rs`, `tests/preferences.rs` and UI ordering tests cover protected retention, no-overwrite/temporary cleanup, destination metadata and stale list data/errors. The native Backups flow checks first-copy setup and saved form ordering in the isolated preview; preview intentionally cannot create/restore backups or contact Drive.

Production backups must use `BackupProvider::reserve` and `upload_prepared` through the durable journal in `backup/journal.rs`. Persist the reserved ID and exact encrypted archive before calling upload; persist each session URL before sending bytes. Never replace an unresolved record with a new archive, file ID or passphrase. A retry decrypts the staged archive with the supplied original passphrase before associating it with scheduled backup. The journal uses its own SQLite connection/file (`backup-uploads.sqlite`), WAL and synchronous FULL, so archive writes do not hold the mail-cache connection. In-memory test stores must get in-memory journals: SQLite can report their path as an empty string, which must not become a file in the repository root. Conditional checkpoints/removal protect a newer record from stale work; committed records remain until cleanup succeeds. Keep no root artifacts.

`backup/drive.rs` implements bounded Drive HTTP parsing, pagination-loop/incomplete-list/duplicate rejection, owned app-data checks, reserved IDs, 1 MiB resumable chunks, authoritative Range acknowledgments, session expiry and verified commit recovery. Only the documented same-origin resumable endpoint may receive archive bytes; never follow a session URL to another origin. Confirm returned ID/name/size/app properties and server SHA-256 (or fetch ciphertext if the checksum is absent). Uploads have per-request timeouts and bounded retries/no-progress detection, rather than an overall ten-minute cutoff. `backup/drive_tests.rs` uses the production API implementation with loopback HTTP to cover lost/partial responses, restart, checkpoint failure, expired sessions, conflicts, untrusted URLs, corrupt files and bounded listing/downloads. These are protocol contracts, not genuine Google verification. See [Drive uploads](https://developers.google.com/workspace/drive/api/guides/manage-uploads), [generated IDs](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/generateIds) and [Drive account identity](https://developers.google.com/workspace/drive/api/reference/rest/v3/about/get).

Restore validates the whole snapshot before changing storage or credentials. `backup/restore.rs` derives permitted incoming/SMTP/CalDAV credential keys from included sources, forbids device-key aliases and duplicate owners, checks calendar URLs/message IDs/limits, and rebuilds display/search metadata from original MIME off-thread. `store/restore.rs` uses one additive SQLite transaction: existing mail/flags/folders/configuration/preferences/drafts win. Do not turn restore into an upsert that rolls current flags or settings backward. Orphaned cached mail and locally moved stable IDs are valid. Credential-owner or cached-message identity collisions roll the transaction back.

Only after local commit does `engine/restore.rs` fill missing OS passwords for unchanged or newly imported connections. Existing passwords must survive an older backup. Distinguish `keyring::Error::NoEntry` from a locked/unavailable store; failed keychain work must report that mail was restored and explain retrying the same copy. Do not compensate by deleting restored mail or storing plaintext rollback secrets in SQLite. Account/calendar locks serialize import against sync/moves/connection edits; retain transfer's sorted account-lock order. Restore must observe its background SQLite transaction to completion rather than dropping the future at an overall timeout. `engine/restore_tests.rs` uses encrypted temporary archives and an object-scoped fake keychain, covering malformed ownership, late-insert rollback including FTS, existing state preservation and retry after partial keychain failure. These tests never touch personal credentials. The transaction still occupies the cache connection; independent read access during large restore/export remains in the completion audit.

Newly restored mail is pinned in `restored_messages` until a complete server listing confirms its remote identity. IMAP reconciliation must preserve pinned mail absent from the server; otherwise the next sync destroys the only recovered copy. Pins persist across restart, are inserted in the restore transaction and cascade away on explicit local deletion. A confirmed live identity resumes normal server reconciliation. Restore never uploads mail to a server implicitly.

Google token handling lives in `providers/google/tokens.rs`. One shared auth-state mutex serializes refresh, authorization-code exchange and OS credential writes; overlapping callers coalesce a refresh. Bind saved credentials to their issuing OAuth client, preserve an omitted refresh token only during refresh, and replace it when the response rotates it. A new login must supply its own offline grant. A failed refresh save leaves the new grant pending in memory; retry persistence before returning access or issuing another refresh. A failed login save keeps a separate candidate and blocks ordinary token use until Reconnect Google finishes it. Do not exchange a received authorization code again or silently borrow the previous account's refresh token. Keep these pending credentials only in memory/OS storage, never logs or the mail database. An app exit before a failed save is recovered can require signing in again.

`OsCredentialStore` also uses a FIFO `WriteQueue` whose owned guard moves into the blocking OS operation. A cancelled caller must not release ordering while its `spawn_blocking` write is still running; otherwise an old grant can overwrite a newer sign-in. The cancellation test holds the first operation, aborts its async caller, and verifies the next write remains pending until the first operation finishes, without timing measurements. Share one Google client per running profile; coordination across independent app processes remains lifecycle work.

Only successful HTTP token responses with validated bearer tokens/lifetimes may replace current credentials. Bound both declared and streamed JSON to 64 KiB, reject redirects/malformed responses, and keep remote payloads out of error chains. Revoked refresh grants stop further attempts in that process until login; application-credential errors allow correction. Connection checks validate saved credentials/client binding without issuing network refreshes or delaying the initial cached workspace. `providers/google/callback.rs` checks GET/path/Host/state and duplicate parameters, handles fragmented headers, limits headers to 8 KiB and concurrent local connections to eight, and uses per-connection/overall sign-in timeouts. Browser acknowledgments contain no code or state. Provider tests use an object-scoped fake keychain and `providers/test_http.rs` (including chunked replies), never personal Google credentials. See [Google desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app) and [refresh-token rotation, RFC 6749 §6](https://www.rfc-editor.org/rfc/rfc6749.html#section-6). Live Google authorization and partial-permission/account lifecycle work remain separate requirements.
