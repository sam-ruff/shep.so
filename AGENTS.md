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

Calendar protocol tests run against a scripted loopback HTTP server in `src/providers/calendar/test_server.rs` under `cfg(test)`, using object-scoped clients and fixture credentials. `cargo test --all-features calendar` covers wire failure/retry contracts, cache migration, source identity, iCalendar durations and escaped text. The native suite also edits/deletes duplicate remote UIDs across different calendars. Save logs under `artifacts/logs/`. Live calendar-provider verification remains distinct from these tests.

`cached_reads_and_ordered_saves_complete_with_all_network_jobs_and_queue_occupied` holds every provider slot open and fills the provider queue, then executes actual SQLite search/body reads and saves the latest preferences/draft through the production dispatcher. This is a deterministic correctness test, not a timing benchmark. Keep its blocked-provider condition intact; do not replace it with a short sleep that might finish before navigation. The test-only holding command is compiled only by `cargo test`, never into release or native test-support binaries. Preserve FIFO ordering for draft/settings persistence when changing concurrency.

Google connection status is checked by a provider command after the cached workspace is ready. Do not await OS credential-store access before emitting `Ready`. Failed speculative sends must clear their pending prefetch markers so the message/page can be requested again.

Preferences use client edit generations and persistent store revisions. `SavePreferences` returns a small `PreferencesSaved` snapshot; do not reload an entire workspace for each preference change. `ui/preference_sync.rs` preserves newer local edits and rejects older store snapshots, including while the pane divider's save is debouncing. Backup completion must use `Store::update_preferences` to change only its metadata; never write the preferences captured before a long upload back over current settings. UI saves preserve backend-owned `last_backup`. Google OAuth starts after its settings save is acknowledged, and the provider job must not rewrite the old settings. `tests/preferences.rs` covers persistence/revisions and the metadata race; the native suite combines appearance, unified/cross-account preferences and resizing and waits for `preferences_saved`.

Message-detail requests/results carry `detail_revision`, independent of page/search generations. Increment it and clear pending prefetch markers when mail changes. Ignore both data and errors from earlier revisions; a late body load must not undo new flags or a completed move. `ui/reading_tests.rs` controls backend-result ordering; native MCP mouse flagging/moving remains the control-level test.

Backup actions save the displayed preferences and wait for that generation's acknowledgment before entering the provider queue. Commands and list results/errors carry `BackupTarget`; list generations reject earlier results, and restore keeps the target selected with its copy. `last_backup`, `backup_ready` and `google_connection_id` are backend-owned metadata. `Store::record_backup` only updates a matching destination. A successful Google login creates a new connection identity, and the shared Google connection lock serializes login against Drive backup/list/restore operations. Do not restore these device-specific settings from a snapshot.

Every successful manual backup stores its passphrase under a destination-specific OS keychain entry, including when automatic backup is disabled. The first copy prepares the schedule; missing credentials pause it with a recovery instruction. The scheduler reads keychain credentials inside the provider job, never while polling the network dispatcher, and rate-limits failed automatic attempts to the configured interval. `BackupSaved` acknowledges upload before metadata/keychain/retention work; subsequent failures must say the copy was saved. Retention requires the committed ID in a complete, duplicate-free list and always protects it even when the clock changes. Local backups use temporary files and no-overwrite persistence, validate the timestamp/UUID filename, and bound reads even if a file grows.

`src/engine/backups_tests.rs` injects an object-scoped fake passphrase store and uses real temporary encrypted local archives; it never reads or writes the user's keychain. Tests cover manual-to-automatic setup, missing keychain access, metadata/retention failures after commit and destination changes. `tests/backups.rs`, `tests/preferences.rs` and UI ordering tests cover protected retention, no-overwrite/temporary cleanup, destination metadata and stale list data/errors. The native Backups flow checks first-copy setup and saved form ordering in the isolated preview; preview intentionally cannot create/restore backups or contact Drive. Drive wire contracts and ambiguous-write recovery remain separate work in `docs/COMPLETION.md`.
