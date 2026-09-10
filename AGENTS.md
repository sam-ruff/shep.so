# Shep development instructions

## Monorepo and feature parity

The Rust + iced desktop application stays at the repository root. `flutter/` is the Android/iOS client; `web/` is the separate browser client matching the desktop behavior; `website/` is the promotional website. The earlier separate `shep.flutter` repository and hosted Flutter-client proposals are superseded. Since the 2026-09-09 merge of `main` into `feat/mobile-web-clients`, `main` is the single integration branch for the desktop, mobile and website sessions: desktop changes reach the client branch through merges from `main`, never through hand-ported commits, and root desktop code follows `main` when the two disagree. Mobile/web/website development continues in Git worktrees; combining agent work inside a review worktree is allowed. The shipping request authorizes committing and pushing combined client work promptly to `feat/mobile-web-clients` for review, without waiting for full parity; keep incomplete features tracked, and treat merging client work into `main` as an explicit integrator step rather than part of a checkpoint push. The latest request authorizes installing the Rust beta service on the email VPS once its target and OAuth/owner configuration are supplied; do not invent another permission step.

`shared/mail-core` owns common mail/MIME/reply and IMAP/POP3/SMTP contracts. `shared/mail-content` owns MIME representation/CID selection, cached attachment identities, filenames and exact decoding for native Rust and browser WASM workers. Untrusted raw input must pass `mime::parse`; its iterative boundary preflight is coupled to pinned mailparse 0.16.1 and must be reviewed when upgrading that dependency. RawHtmlPart/message_body is untrusted sender HTML, never safe to insert directly into a page or WebView. Formatted rendering requires resource confinement and sanitizer/CSP tests. Keep shared byte fixtures in agreement; browser builds generate ignored WASM glue using the pinned CLI in docs/CLIENT_TESTING.md. Include both shared crates in coordinated version stamping. Root compatibility exports and the VPS service use this crate; platform credentials/cache/UI stay outside it. Keep gateway endpoint pinning and TLS hostname validation intact. Browser mail/drafts/submission IDs belong in per-Google-identity client storage, passwords in tab memory until secure remembered credentials are implemented. Persist a reserved SMTP identity before sending and never turn an unknown/uncertain delivery into a new automatic send.

`shared/mail-content::forwarding` prepares complete cached source and exact files off-thread; its retained outgoing HTML is not a safe display document. Shared core attaches new draft/file identities without inheriting recipients or thread headers. Keep file bytes out of WASM metadata JSON, free owned WASM results, and persist the draft plus every file atomically in each client. Text autosaves must preserve backend-owned forward metadata and inline-file associations. Native cache version 9 stores inline Content-IDs separately from file bytes and records each Forward source/target identity; retries must return the current editable draft and never resurrect sent/discarded drafts. Browser creation uses one IndexedDB transaction under the draft lock. Retain quote metadata through text autosaves and clone inline IDs through reviewed Outbox recovery. Two independent preparation slots/workers keep provider/reader capacity separate. A late completion saves to Drafts without replacing a newer reader/editor. Preserve native/WASM, cache, gateway, Android and Playwright failure/restart scenarios; Apple execution and full composition remain tracked.

Client account removal reviews current metadata before an atomic local delete. Never remove provider mail. Native SQLite retains removed-account/draft identities and credential cleanup jobs; a locked keychain must not undo committed removal. Account/credential lifecycle writes share a Dart FIFO across native handles, while the native profile lock excludes other app processes. Browser removal takes draft → account → cache Web Locks without waiting on active work, then rechecks the review inside IndexedDB. Ordinary writes must check removed identities in the same transaction. Keep tombstones free of removed mail, draft bodies, passwords and OAuth data; stale tab/reconnect/autosave results cannot recreate them. Delayed mutation results must tolerate a removed reader/confirmed state.

Native reconnect stages an independently keyed credential pair before atomically activating `account_credentials` with the account settings. SQLite stores only slot identifiers/configuration; credential bytes stay in device storage. `connections.rs` owns staging, compare-before-activation, legacy migration and cleanup. Do not overwrite the active OS key or delete it after a lost activation response. Cleanup runs through the shared Dart lifecycle FIFO and checks the durable journal; abandoned/superseded keys remain retryable. Provider requests carry their credential slot and recheck it under the account operation lock before sync/mutation/SMTP/Sent work. Unbound requests are accepted only for legacy profiles; local cache actions need no credential access. Reconnect still preserves existing connection settings; full editing needs reviewed mailbox-identity migration and matching browser behavior.

Whenever a desktop feature, setting, default, provider contract or failure/recovery behavior changes, review its mobile and browser equivalents. Update `docs/CLIENT_PARITY.md` and the shared client scenarios in the same change. Implement and test the equivalents, or add an explicit active parity gap to TODO.md; never silently allow the clients to diverge or claim that fixture tests establish provider parity. The mobile layout may adapt for touch and configurable K-9-style swipes; browser behavior must match desktop. Every gesture also needs a visible accessible control.

Client selection prerequisites live in `flutter/rust/src/selection.rs` and `flutter/lib/model/mail_selection.dart`. Keep captured IDs/ranks in background SQLite, use the common paging query plan, return at most 50 observed rows, and preserve the separate 32-slot FIFO and cancellation ownership. Keep the scope/revision/alias/arrival, failed recapture, lost acknowledgment, stale observation and cleanup tests. Browser selection uses `web/src/selection_store.ts`: worker-owned temporary SQLite, a readonly IndexedDB mailbox snapshot and at most 1024 cache-change records. Never put temporary selection membership in a readwrite transaction on the mail database: real Chromium tests show it blocks independent draft saves. Keep metadata/journal writes atomic with mail/aliases, preserve newer Sent roles on schema upgrades, and retain the 100,000-row concurrent-save, queue, rollback and tab-isolation browser contracts. Worker loss expires ephemeral captures; durable bulk jobs require a separate journal. Browser controls now use `web/src/mail_selection.ts` with one request in flight, at most 32 gestures and 50 observed rows. Preserve pending offscreen intent, unknown-rank target feedback, lost replies, refresh generations and abandoned-capture release. Exclude Select all from formatted-reader shortcut interception so Ctrl+A continues selecting text; never let selection mode dispatch a single-reader shortcut. Keep typed search text while exiting mode before the debounce. Native controls and durable bulk execution remain open: wire rendered-row observations without retaining an entire inbox in Dart, and never implement captured Select All by looping over loaded rows.

Browser mailbox queries live in `web/src/mailbox_store.ts` and its bounded worker/client. IndexedDB mail schema 12 preserves the authoritative cache incarnation UUID and indexes account/folder/server identities for metadata scans while preserving its change revision/floor, intents and acknowledged Sent roles. Derived SQLite/OPFS storage must compare both incarnation and revision, replay at most 1024 changes or rebuild from a readonly cursor, and never hold a mail readwrite transaction while indexing. Own the per-profile Web Lock before installing/unpausing the SAH pool; close every SQLite handle and pause the VFS before releasing that lock. Retain one source body while indexing, at most 50 returned metadata rows, and separate single-body reads. Failed index work must roll back its checkpoint. The Gateway/Workspace now uses this path for list/startup reads. Keep one running page request plus its latest replacement, independent foreground body/metadata workers, separate speculative/scan capacity, a 50-row metadata page, the active reader and at most eight cached bodies/32 MiB. Failed projections must retire before a late query can overwrite rollback. Individual counted Undo may retrieve evicted metadata; an absent source must remain an explicit review failure. Schema upgrades preserve cache clocks/epoch and protected intents. Sync scans return at most 50 metadata entries and flag batches retain one body at a time. Preserve real controls, query/body Retry, stale data/errors, held row/footer input, cross-page selection and account removal. Whole-client bounds are not established: Outbox/drafts/account-removal snapshots, provider known-ID/reconciliation arrays, group projection performance, fuzzy relevance, large-message parsing/streaming and final performance remain open. Derived account removal is applied by the next successful query/rebuild; physical cleanup retry/lifecycle remains tracked.

Browser Reply/Reply all/Forward/Print live in a stable footer outside `.reader-content` scrolling. Keep their visible labels and responsive column widths stable while preparation runs; busy state uses a disabled control, descriptive accessible label and reduced-motion-aware icon. `web/src/reader_actions.ts` retains that same-message action branch, same-scope mail-row controls and their structural ancestors without removing/reinserting them, because even momentary disconnection cancels native pointer activation. Rebind callbacks/attributes to current data while replacing surrounding controls. The sandboxed formatted frame retains its existing independent lifetime. Preserve saved real pointer-position, held-button, keyboard-focus and body-scroll regressions; never compensate with a synthetic click, direct controller call or forced hit test. Respect already-handled key events and native Enter/Space on action controls. Mail-row secondary controls retain native activation; opening a focused message follows the configured reader shortcut, including remapping and disabling. Shortcut capture belongs to the mounted UI; retain its button/ancestors through background redraws, rebind key/blur callbacks to current preferences, and cancel on focus departure without rendering during the native focus transfer. Preserve preferences-controls.spec.ts for held presses, modifier input, conflicts, cancel/clear, navigation and persisted settings. The wider native/list-input lifecycle audit remains active.

Browser bulk prerequisites live in `web/src/bulk_journal.ts`. Export frozen membership/current physical identities through worker SQLite and readonly mailbox snapshots, then stage at most 50 metadata rows per strict transaction in a separate profile IndexedDB journal. Never put MIME or secrets in the journal. Keep the profile Web Lock through a provider step and its durable receipt; a replacement owner classifies abandoned forward/inverse steps as unconfirmed and cannot blindly retry them. Interrupted preparation cannot execute. Status indexes/counters bound claiming and history (20 jobs/50 items). Undo prevents unsent forward claims and retains actual destination and inverse receipts. `web/src/bulk_executor.ts` now connects exact membership to the provider adapter, with profile binding, one owned step, physical-source checks, per-field approval/Undo revisions, distinct skipped outcomes, conservative restart and graceful stop. The production browser starts approved work and exposes frozen reviews, Archive/Delete/Move/read/flag actions, bounded History, Pause/Resume, Undo and explicit failed/uncertain recovery. Unknown results require a checked folder review; accepting current state retires only that local intent, without classifying the provider outcome. Keep newer decisions through receipt progress and retain native pointer targets through redraws. Group Undo prepares one additional metadata page in the query worker, then paints its rows/counts synchronously before saving a decision. Counterfactual reads change derived SQLite only; they cannot reserve intent, write the durable journal or run providers. Preserve newer local fields, the physical baseline of restored rows, scope/cache/account invalidation and rollback after a rejected decision. History progress must remain independent of preview preparation: start the preview outside the awaited refresh, with a separate request token that cannot enable Undo for another selection. Preview failures retain visible Refresh-history recovery while other actions remain usable. Closing History cannot hide decision failures. Release the source mail snapshot before derived query work; observe the receipt journal afterward and use per-field applied revisions to reconcile journal/cache ordering. An in-flight successive move uses its actual cached dispatch folder until its receipt supplies the inverse identity. Saved-group recovery observations use the journal’s existing status/cache/state indexes and retain at most four category targets across all History pages. Coalesce one observation and one replacement, preserve known targets on failed inspection, reject reads across execution boundaries and suppress normal active receipt/cache handshakes. Publish recovered attention before unrelated provider work finishes; never recover a live owner through an observation. Preserve bulk-recovery.spec.ts and bulk_recovery.test.ts for older groups, retry, acknowledged cache receipts, live-owner/tab loss and close. Full native parity, cross-account moves, abandoned-review cleanup and the wider Undo lifecycle/performance audit remain active gaps. Preserve real Chromium cardinality, concurrent draft, tab-loss, atomic rollback and stale-result tests.

Browser `GatewayRepository.mutateWithReceipt` returns physical before/after identities independently of list reload, and delivers remote acknowledgment to the durable writer before cache work. A committed `MutationFailure` carries its receipt and cache-applied state; a display-only failure must not roll back the action, erase counted success or disable a safe cached Undo. Unknown MOVE destinations use an empty destination UID plus bounded SHA-256/size proof, never the old source UID. Journal schema 6 retains approved intent/accepted fields and distinguishes skipped work, alongside runnable jobs and pending cache repair: no provider step may start until an acknowledged cache/identity gap reconciles. Read-only inspection never recovers a live owner; migration/restart cannot repeat an unconfirmed running step. Retain forward flags while enriching a recovered UID, and preserve provider/control, cache-phase, schema and stale-attempt tests. The executor completes the cache/action-record handshake before another step; worker projections and actual review/History/Undo controls now have saved browser evidence. Each journal item carries its last changed revision, and each journal incarnation has a UUID; query workers replay changed metadata without transferring whole groups to the UI. Read-only inspection must not briefly take the executor lock on an already-current schema. Long group query/capture snapshots derive active owners from accounts; never include removedAccounts there, because independent draft writes need that fence store. Protect both dispatch and cache-only repair against a replaced source cache.

Browser field ownership lives in `web/src/mail_intents.ts` and mail database schema 12. Intent-only writes advance the authoritative cache change revision so worker group projections observe newer choices and final ownership. Version 7 closes older writer connections before relying on atomic cache-applied revisions; version 8 fences removal code that lacks group ownership; retain prior clocks/statuses without inferring cache success from an acknowledgment-only status. Reserve individual revisions at input time, before waiting for older queued operations. Recheck accepted fields under the cache/account operation lock and confirm only those fields in the UI; superseded actions are not provider successes. Alias adoption merges ownership inside the cache transaction, and sync cannot overwrite it. Known acknowledgments survive action-record failure; unknown wire outcomes stay pending. Account removal reviews and deletes owned intent, including missing messages, and rejects stale writes. Keep the real Chromium migration, cross-tab same-value/Undo, alias rollback, queued-control and removal regressions. The claim API does not authorize replay: the durable group journal must still govern dispatch, receipts, uncertain-result review and restart. Cache writes atomically record the applied revision for each accepted field. `GatewayRepository.repairMutation` replays only missing cache changes, uses read-only identity recovery when necessary and never repeats a server mutation. Keep newer applied fields even when their intent completion failed or sync changed the cached value afterward. Preserve cache-transaction/alias rollback, same-value Undo, lost receipt replies and newer-move regressions. The browser now connects group controls to full-membership query/selection projections; broader lifecycle/performance and native parity remain open.

Browser metadata carries a physical-identity lineage from mail schema 12. Ordinary writes retain it only for the same cached account/folder/UID; acknowledged local/provider moves and verified recovery attach exact before/after transitions in the same transaction. Unproven replacements get a fresh lineage. Canonical aliases retain both original and target lineage, including validated flattening, atomically with ownership and cache writes. Frozen groups retain the original review; provider dispatch proves current metadata/alias continuity under the existing operation locks and receipts record the actual dispatch and inverse identities. Journal schema 6 fences older owners before relying on those proofs. Legacy reviews retain strict physical checks. Query/selection projections use the same origin rule and discard stale derived schema checkpoints. Never infer a new UID from a projection or automatically replay uncertain moves. Keep successive-group, individual-move, alias rollback/cache-restart, unknown-UID and replacement regressions. History selection clears previous details/actions immediately; late results or decisions must not target the newly selected group.

Browser account removal takes group ownership before the draft → account → cache locks and verifies both mail and group reviews. Include queued, failed, uncertain and inverse/cache work in the discard requirement. The mail transaction and its content-free removal token are authoritative; no server mutation or journal deletion occurs before it commits. Every new journal owner reconciles committed removals before recovering or claiming work. Journal schema 4 indexes source/destination account ownership, removes at most 50 entries per strict transaction, adjusts remaining counts and retains unrelated receipts. Its completed fences reject stale frozen captures; interrupted cleanup resumes from the same mail token. Lost mail replies check that token, and failed post-commit cleanup keeps the account removed with visible recovery instructions. Read-only startup inspection must not upgrade/close an older executor's receipt connection while its Web Lock is held. Preserve actual Preferences light/dark/stale-review/cleanup-warning flows and Chromium transaction, page-rollback, migration, cross-tab and tab-loss scenarios. Review observations hold one job/item while counting; the global group revision deliberately invalidates a review if other group work changes. General bounded cache reads and other-browser/Apple execution remain separate active requirements.

Quality and release workflows remain disabled for every client. Coordinate one source revision/version across future desktop, mobile and website/browser releases. R74 authorizes a Rust backend on the email VPS for browser beta verification and IMAP/POP3/SMTP transport, superseding the static-only/local-bridge proposal. Enforce verified Google login and an administrator allowlist server-side, initially the exact owner identity supplied by the user; reject everyone if it is not configured. Keep mail/passwords out of persistent server storage and all logs. Browser TCP protocols use the authenticated backend, never a fake direct-browser socket implementation. Deployment needs the actual VPS/OAuth configuration; development remains in worktrees.

Shared profile metadata lives in `shared/profile-core`, with explicit native account adapters in `shared/mail-core::profiles`. Preserve the common fixtures, unknown optional fields, duplicate-key/target rejection, explicit security and bounded encoding. Structural validation is not ownership, causal merge or local activation; never use it to replay mail actions or import credentials. Keep complete raw operations through local mappings, and require an update before applying unknown connection extensions. `docs/agents/PROFILE_FORMAT.md` records the implemented subset; the OAuth/profile handover remains the full contract. Include this crate in coordinated release stamping.

## Mobile Google connection

Shared profile history lives in `shared/profile-core/src/history/` behind the native `history` feature. Keep one background connection owner, its 32-command channel and lock through cancellation/close; never add a profile snapshot connection to the mail cache. Preserve exact operation/request bytes, durable reserved upload identities, causal dependencies, explicit conflict reviews, extension data and account/profile tombstones. Field/version observations stay at 50; apply at most 32 ready records per command. A complete received ancestry is not a complete cloud discovery. Bindings must come from authenticated transport, and upload confirmation requires verified owned file bytes. Neither the journal nor its Flutter bridge applies accounts/preferences or accesses credentials. Preserve the shared Rust, actual Dart FFI and isolated Android history tests. `docs/agents/PROFILE_HISTORY.md` explains API boundaries and pending integration; keep local journals/device IDs out of portable imports until explicit rebinding exists.

Flutter Google consent uses `model/google_connection.dart` and `data/google_native.dart`. The SDK owns tokens; device secure storage holds only application/subject/permission metadata and local cleanup state. Preserve separate requested versus committed permissions, one SDK operation, coalesced choices, failed-save/changed-choice rejection and durable disconnection before local SDK sign-out. Never replace local signOut with project-wide disconnect/revocation. Silent provider requests cannot invoke potentially interactive authentication. The initial adapter reconnects the saved subject; safe seamless switching, automatic session restoration and provider/profile integration remain open. Keep real controls in `test/support/google_controls_scenario.dart`, shared host/Android execution and preview Playwright/Appium scenarios, plus the configured SDK-boundary and secure-store lost-reply tests. No success fixture is imported by `lib/main.dart`. See `docs/agents/GOOGLE_MOBILE.md` for platform configuration and current limits; live/Apple verification remains distinct.

Flutter first-profile publication uses mail schema 10 frozen reviews and exact staged requests, plus an independently owned shared history journal. Preserve the causal initialization marker and record own-upload identities in discovery before confirming the local queue. Pause/close drains an accepted step; disconnected generations cannot begin another. The complete contract and saved regressions are in [PROFILE_PUBLICATION.md](docs/agents/PROFILE_PUBLICATION.md). Publication does not authorize account application, credential import or cloning device history.

## Desktop profile application

The desktop profile implementation is main's: `src/profile_sync/`,
`src/engine/profile_sync.rs`, `src/store/profile_sync/` and the Preferences
profile pages, described under "Database import and local profiles" and
"Continuous profile transport" below and in `docs/agents/profiles.md` and
`docs/agents/profile-drive.md`. The client branch's own desktop enrollment and
reconciliation (`src/profiles/enrollment/`, `src/profiles/sync/`,
`src/store/profile_enrollment.rs`, `src/ui/profiles/enrollment.rs`) were
replaced at the 2026-09-09 merge; `docs/agents/PROFILE_DESKTOP.md` and
`docs/agents/PROFILE_RECONCILIATION.md` describe that superseded implementation
and remain as history and as the invariants the Flutter/browser equivalents
still follow. Carry these invariants forward whichever implementation owns the
path: keep original records in the independent publication/history owner;
imported accounts retain a durable reconnect guard through both explicit
password writes and checked activation, which provider/backup/restore paths must
never bypass; portable preference saves preserve actual field intent, including
reverted edits, and never replace patches with an old whole-preferences
snapshot; keep the exact staged local request until its own history receipt is
saved; an idempotent history Edit reply reports current history, not the
original field revision, so never use that whole-history revision to acknowledge
unseen remote intent; scope remote record cursors to the observation history's
device identity; pausing cannot authorize a workspace switch; background sync
must serialize history ownership with publication/enrollment, check the active
Google grant and own provider capacity/lifecycle locks; and checked preference
conflict decisions freeze and page exact versions, recheck local intent before
staging, replay the same staged request after a lost receipt, never retire an
uncertain pending edit without its exact History.Edit result, and cannot be
cancelled as if they had never been recorded.

## Request tracking — required every turn

`TODO.md` is the authoritative active request list. At the start of each turn, read it alongside the original goal and relevant completion evidence. When the user adds, changes or reports a requirement, add/update its TODO entry immediately, before implementation; record corrections so obsolete defaults are not restored. Check the whole conversation when auditing scope, not just the latest message.

Do not remove an entry because code exists, a plan was proposed, or unrelated tests pass. Remove it only after the requested behavior and relevant unit/protocol/native tests pass, visual evidence is reviewed where appropriate, and the authorized shipping step is complete. Record the completed behavior, tests, limitations and commit in `docs/COMPLETION.md`; update `docs/REQUEST_AUDIT.md` so every original request remains traceable. Partially completed features stay in TODO with their remaining work stated. Never silently drop a request at compaction or replace the full product goal with the newest request.

The current stopping point and restart order are in [handover.md](handover.md). Keep TODO authoritative; a pushed handover is not completion of the full product goal.

## Parallel delivery

The user explicitly requests parallel agents in isolated Git worktrees to finish
the entire TODO list. Assign independent feature lanes, with the primary agent
owning integration and pushes to `main`. Each lane must add relevant tests,
review native visual evidence where applicable, and report exact commits,
verification and remaining limitations before integration. Keep each worktree's
Cargo target separate and cap its builds at four jobs. Isolated correctness-only
native flows may run alongside unrelated capped builds; every harness owns its
display, credentials fixture and files. Reserve quiet windows for latency or
renderer pixel measurements, and never relax a timeout or budget because a host
is busy. Parallel coding continues throughout. Do not bypass hooks or mark work
complete before verified shipping.

## Product direction

Keep README and human documentation short, concise and easy to read. The Zensical home page links to a separate `docs/agents/` section for detailed behavior and implementation reference. Keep operational agent instructions in this file, and link to it from the agent docs rather than maintaining a second copy.

Shep is a native Rust + iced email and calendar client. Mouse usability and keyboard usability are equally important. Do not describe it as keyboard-first. Keep common actions visible and clickable; shortcuts are optional, native, remappable accelerators. The default move shortcut is `M`. Never run letter shortcuts while a text field or editor consumes the keystroke.

Keep the restrained shadcn-inspired design: clear hierarchy, comfortable spacing, rounded controls, subtle borders, quiet violet accents, useful empty/error states. Appearance has Light / Dark / System choices in Preferences. The user approved the flat Swiss Shepherd profile in `assets/swiss-shepherd.png`; preserve the design. App raster assets use WebP. Small logo variants are loaded once, not read from disk on every frame. Vector UI icons remain SVG.

## Asset editing credentials

The user says the Dungeonwalk tools repository contains AI vectoriser and remove.bg API credentials that may be used when editing existing icons. Locate that repository and its own instructions when needed; preserve the approved Shepherd design. Keep credentials in their original secret configuration or process environment, never in Shep source, documentation, shell history, logs or tool output. This is a discovery pointer, not a request to copy credentials into this repository.

## Documentation CI is enabled; quality and release are DISABLED

**The user authorized the public repository and automatic Zensical documentation publishing to GitHub Pages.** Repository: `sam-ruff/shep.so`, public, default branch `main`. Direct pushes to `main` are currently authorized. `.github/workflows/docs.yml` builds documentation on GitHub-hosted Linux runners and deploys changes from `main`; pull requests only build. Repository Actions must be enabled for this workflow. Before pushing documentation changes, run the pinned Zensical builder with `zensical build --clean --strict`, matching CI. Links from published docs to files outside `docs/` (such as root TODO.md and AGENTS.md) must use their GitHub URLs; relative links cannot escape the published site.

Use `Sam R <sam@technesci.co.uk>` for this owner's Git author/committer identity. Do not add the owner's private contact details or exact workstation hardware to public documentation. Keep old private history backups and audit reports under ignored `artifacts/`.

Quality and release workflow definitions remain deliberately named `.github/workflows/ci.yml.disabled` and `release.yml.disabled`. Do not enable them as a side effect of ordinary development or docs publishing. Remind Sam to re-enable them when the self-hosted runners are ready.

To enable when Sam asks:

1. Provision trusted self-hosted runner labels from the CI matrix: `[self-hosted, Linux, X64]`, `[self-hosted, Windows, X64]`, `[self-hosted, macOS, ARM64]`. Adjust labels to the actual machines first. Do not run untrusted fork code on persistent self-hosted runners.
2. Install Rust with `rustfmt` and `clippy`, CMake and a C++ compiler for vendored litehtml, Python 3.11+, Node 24, and platform development libraries. The Linux GUI harness additionally needs `Xvfb`, `xdotool`, `zenity`, `xclip`, `dbus-daemon`, `busctl`, and ImageMagick `import` with WebP support. Print flows need Chrome/Chromium, Poppler (`pdfinfo`, `pdftotext`, `pdftoppm`) and ImageMagick `convert`. Linux needs OpenSSL/dbus/X11/Wayland development packages and a Secret Service for real credentials.
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
python3 scripts/test_profile_core.py # shared codec/history/Drive contracts
cargo test -p shep-html-pixbuf             # dependency cache regressions
python3 -m unittest discover -s tests -p 'test_*.py'
cargo bench --bench responsiveness
cargo build --profile test-ui --features test-support
python3 scripts/e2e.py                      # real native UI flows through MCP
python3 scripts/html_latency.py --samples 20 --output artifacts/performance/html.json
bash scripts/check.sh                      # all of the above
```

The pre-commit hook runs formatting, Clippy, `cargo test --all-features`, the `shep-html-pixbuf` tests and `scripts/test_profile_core.py`. The commit-msg hook requires Conventional Commits. `core.hooksPath` is the relative `.githooks` (set by `scripts/install-hooks.sh` from the checkout root); keep it relative so every worktree runs its own checkout's hooks rather than another checkout's, and re-run `git config core.hooksPath .githooks` if `git config --show-origin core.hooksPath` shows an absolute path. Do not skip failing hooks or weaken a budget just to get a commit through. `SHEP_SKIP_E2E=1 bash scripts/check.sh` runs the non-GUI checks on machines without Linux/X11; report that omission. CI repeats the checks; Rust compiles/tests run on all three platform runners, native E2E currently runs on Linux.

Defer performance measurements while the PC is saturated with other work; run them at the end on an otherwise idle machine. `python3 scripts/e2e.py --functional-only` runs correctness flows without the latency benchmark. Never weaken thresholds based on a loaded-host result.

The latest user priority is HTML opening latency (R72). They explicitly authorize measuring native selection-to-visible-HTML now and iterating on it until the wait is imperceptible. This supersedes the earlier measurement deferral for HTML opening work. Preserve the distinction between body loaded, HTML prepared/rendered, and actually presented pixels; cover cold, warm and prefetched navigation and document the host conditions without inventing an idle-host claim.

During development run targeted tests after backend changes, the benchmark after storage/scheduling changes, and the MCP E2E suite after UI changes. Run the complete relevant set before pushing. Do not claim live Google, IMAP, POP3, SMTP, CalDAV, Windows or macOS verification based solely on fixture tests.

## MCP testing and automated equivalents

Read the repository skill [`.agents/skills/shep-e2e/SKILL.md`](.agents/skills/shep-e2e/SKILL.md) for UI work. `scripts/mcp_harness.py` is a stdio MCP server using JSON-RPC and real X11 input. Tools: `desktop.start`, `desktop.batch`, `desktop.state`, `desktop.screenshot`, `desktop.stop`.

Prefer batches of related clicks, drags, typing, shortcuts, short waits, state assertions and screenshots. Batches stop at the first error and save a failure screenshot. Use `wait_for` for asynchronous state transitions; fixed waits are only for visual settling or explicitly timing a scenario. A wait is at most 2 seconds, with at most 10 seconds of explicit waits in a batch.

**Every AI-driven MCP scenario must have an equivalent automated scenario in `scripts/e2e.py` (or a saved fixture consumed by it).** Both AI and automated runs use the same MCP server and real native controls. Do not replace mouse clicks with direct calls to update functions. The state file is an observation-only oracle, never an action API. Save screenshots for visual review; JSON state alone cannot prove layout quality.

Test/demo fixtures live in `tests/support/fixtures.rs`, gated by the nondefault `test-support` Cargo feature. Production builds do not contain fixture messages. The harness launches an isolated in-memory workspace with `--demo --test-state <file>`, owns its own Xvfb process, and cannot send real mail or write cloud data in this mode. Never point automation at personal data or use real credentials in fixtures.

**Do not leave log files in the repository root.** Write build/test logs to `artifacts/logs/`; remove stray root logs before finishing. The user explicitly requested this.

Artifacts go to ignored `artifacts/e2e/<run>/`: WebP screenshots, state, action timings and app log. Do not commit personal inbox screenshots, OAuth tokens, passwords or local databases. The Rust desktop harness is reproducible with Python and MCP. Separate web, website and Flutter surfaces use the client automation described in docs/CLIENT_TESTING.md; Playwright does not test the iced window.

## Strict responsiveness and usability requirements

**Optimistic interaction is an app-wide requirement.** For reversible actions, show the expected successful result immediately and reconcile persistence/server state in the background. Archive/move removes a message from the current folder immediately; flags and read/unread indicators update immediately. Do not wait for SQLite, credentials, network requests or account sync before displaying that change. If an operation fails, restore the affected state and show an actionable error. Preserve newer user intent when older results arrive; keep pending changes through background refreshes and test slow success, failure/rollback and rapid repeated input. Responsiveness takes priority over waiting for confirmation, while correctness must converge and failures remain visible. This does not turn a pending operation into a confirmed server success.

Read-on-leave and action feedback requirements: selecting an inbox message and then leaving it marks it read; explicit mark-unread intent must survive. Archive/delete/move toasts appear in the same optimistic UI update, refresh their timeout and increment their count on repeated actions. Each offers Undo, including while the original write is pending. Track original account/folder and acknowledged server identities for reversal; never reuse an obsolete IMAP UID after moving. Rollbacks and failures remain visible. Read-on-leave, immediate counted feedback and session Undo are delivered. Preserve their protocol/cache/native regressions when changing mail actions.

- UI update handlers: p95 < 8 ms, target < 2 ms. No filesystem, credential-store, SQL, network, compression, crypto or MIME parsing in iced `update`/`view`.
- Cached inbox/search page on 100,000 messages: p95 < 50 ms. Cached body load: p95 < 10 ms. Search debounces for 100 ms; stale results must not overwrite newer queries.
- Aim for 60 Hz interaction (16.7 ms frame budget), cached message navigation under 100 ms, and visible acknowledgement within 100 ms. Measure full native input-to-state latency separately from handler timing; handler timing is not a frame-rate claim.
- Keep 50 messages per page and render only visible rows plus a small overscan. Prefetch adjacent messages and the next page in background. Retain at most 8 bodies / 32 MiB in the prefetch cache. Avoid decoding assets repeatedly.
- Bounded foreground-read, persistence and provider-command channels (32 each), prefetch/download channels (8 each), and an event channel (32). `engine/dispatch.rs` reserves two foreground read workers and one prefetch worker; settings/drafts use a separate FIFO worker. Provider work shares eight slots, with account sync limited to three. Manual mail refresh has its own capacity-one coalescing channel, independent of provider backpressure. `try_send` must never wait on the UI thread. Interactive backpressure produces visible feedback; a full speculative prefetch queue quietly drops that optional request.
- Test browsing, typing, remapping and appearance while a backend operation is pending. Slow or unavailable servers must never disable navigation.
- Prefer 40–44 px click targets; visible focus, descriptive labels/tooltips, persistent errors with a clear recovery, no text clipping at 900×640 and 1440×920. Mouse and keyboard should reach the same core actions.
- User-visible messages should explain the problem and next action. Do not present sample data as live accounts, pretend a sync succeeded after errors, or silently lose unsent drafts.

Run `python3 scripts/performance_gate.py` after backend, native navigation and HTML pixel timing reports have been generated. It fails on missing, invalid, undersampled or over-budget evidence.

The inbox/reader divider must remain mouse-draggable with saved preferences and minimum widths. Filtering and sorting must invalidate stale page prefetches; flags map to IMAP `\Flagged` and remain local for POP3. Cover drag persistence, mouse flagging/filtering/sorting and page navigation in the MCP suite.

See `docs/PERFORMANCE.md` for measured results and what the measurements do and do not cover.

## Account synchronization and channel ownership

`engine/account_work.rs` owns account and calendar scheduling in a background coordinator. Its request channel and waiting queue each hold at most 32 entries; one-shot channels grant work, interrupt read-only sync and acknowledge completion. Pending writes retain FIFO order; separate accounts can proceed independently. Abandoned requests must release their place, and a queued sync must yield to writes without starting another download. Do not restore shared account/calendar mutex maps.

`engine/account_sync.rs` keeps an owned task until already-started cache writes settle, even when the containing refresh cycle is cancelled. Interrupt only read-only provider/setup work. Drain received cache items before releasing account ownership; `try_join!` would detach an active SQLite `spawn_blocking` operation on provider error and can let old state overwrite a newer write. An interrupted check is not a complete folder listing or Inbox baseline. Preserve the deterministic held-provider/cache-commit, failure, timeout, cancelled-owner, bounded-queue and ordering regressions.

The saved native held-sync flows use `desktop.start(held_account_sync=true)`. They exercise the production coordinator/download pipeline against an indefinitely held fictional provider, without real network or keychain access. Closing must persist an explicitly selected message's read-on-leave change; flagging must interrupt the read-only provider, and a failed write must restore the flag and allow another attempt. Actual personal-server sync/close diagnosis and temporary close-to-tray remain separate unfinished work.

## Architecture and data safety

Use bounded channels and state-owning workers for application state coordination, rather than shared lock-managed state. The user explicitly corrected the account scheduling approach during R90: interactive writes must interrupt read-only sync through the coordinator, and durable completion acknowledgments must preserve ordering. Keep UI sends nonblocking, bound queued work, and handle abandoned requests without leaving an account occupied. Audit other application-level coordination when changing those paths.

The full product goal is still active. Keep [docs/COMPLETION.md](docs/COMPLETION.md) current with implemented evidence and remaining work; do not infer feature completeness from a passing fixture suite.

`src/ui/` owns presentation and small caches. `engine.rs` bridges bounded channels and background work. `store/worker.rs` owns the mail-cache SQLite connection and local operation leases on one thread, behind a bounded 32-command FIFO. Accepted SQL jobs drain even if their observation is cancelled or the last Store handle is dropped; a request cancelled before admission must not become a later write. Local lease release uses a one-shot channel's closure, so it cannot wait for queue space. Keep worker ordering/cancellation/failure tests, temporary selection tables and independent-process file leases intact. Long database copies must use a separate online snapshot path rather than holding this worker for the entire transfer. Backup-journal/Google/lifecycle coordination remains in the channel-ownership audit. `providers::MailProvider`, `CalendarProvider`, and `backup::BackupProvider` are extension points: add a provider without teaching the UI its wire protocol.

The cache worker exposed a bundled SQLite 3.51.1 Unix WAL open/close deadlock in the existing notification restart test. A debugger trace showed opposite inode/global mutex acquisition inside SQLite itself. Use the updated bundled SQLite (3.53.2 through rusqlite 0.40.2); do not restore 3.51.1 or hide the fault with an application lock. Preserve `partial_initial_import_stays_quiet_across_restart_then_new_mail_alerts_once` and the cache/worker close tests. See SQLite's [3.51.2 deadlock fix](https://www.sqlite.org/releaselog/3_51_2.html) and [3.51.3 WAL fix](https://www.sqlite.org/releaselog/3_51_3.html). This was fixture evidence, not a diagnosis of the personal account's remaining delays.

The R22 foundation uses vendored SQLCipher 4.19 / SQLite 3.53.4 without
activating existing-cache migration yet. Preserve its source hashes, licenses and
`vendor/libsqlite3-sys/shep-lifecycle.patch`: automatic process-exit/finalizer
cleanup freed codec state beneath a channel-owned cache worker, reproduced as
SIGSEGV in the saved subprocess/GDB fixture. The static build omits those global
exit handlers and initializes OpenSSL before its first RNG call; explicit
`sqlite3_shutdown()` still cleans up after all connections close. Keep the exit
read, OpenSSL positive control, explicit shutdown/reinitialize, keyed WAL,
wrong-key/corruption, staged migration and scratch drain/reopen regressions.
Preserve the native temporary-storage policy: default/plain connections use FILE;
a keyed main forces MEMORY even after an authorizer change, while keying scratch
alone does not change a plain main. Keep the VFS spill positive/negative controls.
Do not activate keyed startup before unbounded selection/catalog summaries and
recovered-view automatic indexes have bounded encrypted scratch plans.

Keep `vendor/libsqlite3-sys/shep-export.patch`: SQLCipher export must preserve
unindexed rowids as well as indexed mail/external-FTS identities. The explicit
raw SQLite export remains unencrypted and excludes credentials; its atomic
candidate in the chosen export folder is intentional user output. Implicit
cache/import/migration candidates must stay encrypted.
Selection/frozen-review scratch lives in a private attached encrypted database;
its connection must close before the owning temporary directory is removed.
Conversation duplicate selection/rank also uses indexed scratch and returns at
most 20 metadata rows. Session scratch uses DELETE/OFF with normal rollback but
no crash-durability guarantee; restart creates fresh scratch. Main cache and
receipt journals remain FULL. Qualify `PRAGMA main.journal_mode=WAL` so it cannot
overwrite the attached scratch policy. Preserve index-plan, duplicate/focus,
spilled rollback, cleanup and orphan-restart regressions.
Shared ancestry uses transaction-cleared indexed main-database scratch with a
bounded frontier. Finish legacy publication/recovery, remaining sorter/temp
paths, profile catalog/portable transfer routing and native key recovery before
activating encryption for personal data. See
[the encryption boundary](docs/agents/CACHE_ENCRYPTION.md).

The software renderer is patched through `vendor/iced_tiny_skia` (released iced 0.14.0, MIT). Cached dropdown text must intersect its own viewport with the damaged layer, and raw text must reset a shared clip mask after preceding text. Shadows must honor damage/layer clipping and include their full bounds in invalidation, including when only the shadow intersects the changed area. Otherwise moving or scrolled controls leave stray pixels that only a full repaint clears. Keep `tests/software_rendering.rs`, the filtered-preferences and scrolled mail-drag native regressions when updating iced; remove the patch only after these pass upstream. The release archive includes the vendor license and patch provenance. Do not edit the Cargo registry cache or replace partial redraws with continuous full-window redraws to hide defects.

Multi-selection storage lives in `store/selection.rs`; native controls in `ui/mail_selection.rs` use their own bounded FIFO channel in `engine/selections.rs`. Native group actions use frozen reviews and the durable journal described below. Keep shipping and remaining verification status in the completion log. `store/mail_query.rs` owns the common scope/ranking plan for inbox pages and captured membership. Keep selected IDs/ranks in SQLite and return at most one metadata page to iced. The controller keeps one request in flight and at most 32 pending gestures, projects visible selection immediately, and releases an abandoned snapshot before capturing a new scope. Scope changes clear selection immediately; page changes preserve it. New arrivals do not silently join a selection, but another explicit Select All captures them. Clear unchecks messages; Done/Escape exits selection mode. Checkbox/modifier gestures must not mark mail as read. Select All is remappable and scoped to the native field focus captured with its key and the controller pane scope when dispatching that key. Preserve normal text Ctrl+A, sidebar focus and double-click reading.

`capture_selection` ignores page offset, `change_selection` checks the expected revision atomically, and `freeze_selection` copies exact selected membership for a review. Missing mail remains explicit in selected versus available counts. Snapshot metadata pages read current flags/folders; immutable membership does not freeze message content. Temporary selection tables disappear on connection close; the bulk journal copies reviewed membership into durable jobs before execution. Preserve `tests/selections.rs`, controller ordering/cleanup tests, provider-saturation coverage and the saved `test_mail_selection_*` native scenarios when changing query scopes, ranking, pagination or selection lifecycle. Preserve the group toolbar, confirmation, failure and Undo native scenarios when changing this path.


Bulk mail changes live in `bulk.rs`, `store/bulk.rs`, `engine/bulk.rs` and `ui/bulk.rs`. Persist exact membership and original metadata with per-message receipts; keep MIME in the mail table. Pending effects project query membership/flags without giving a provider a speculative destination UID. Status counters and indexes on job/position avoid scanning the whole group for every receipt. History reads at most 20 jobs or 50 items; iced retains at most one original metadata page for immediate Undo.

A capacity-one wake channel coalesces requests; queued job IDs remain durable in SQLite. The worker uses one shared provider slot at a time, and updates progress at bounded intervals. Successful provider receipts are observed through persistence; do not drop them at a generic timeout. An owned `fs2` file lock prevents two processes from recovering/executing the same job. Memory stores must never create lock files in the repository. Startup does not repeat an unacknowledged running step: retain its ownership and show an unconfirmed result for explicit review. Never treat that conservative classification as proof that the server rejected the write.

Undo cancels unsent steps and reverses acknowledged steps using their actual receipt identities, including when a forward write is still running. Preserve newer unrelated flag intent. Definite inverse failures may retry; ambiguous outcomes need explicit acceptance after checking folders. Query snapshots observe forward/Undo phase alongside counts, preventing double projection when a page arrives before its acknowledgment. Temporary restored rows cannot issue body/provider requests with obsolete IDs. Bulk toasts carry weighted group counts and adjust after failures.


Close intent must survive independent draft, account/calendar, Google and mail
save acknowledgments, in either order with the bulk stop barrier. A failed
current write cancels close before the busy dependency is released; an obsolete
draft error must not cancel a newer pending save. Track write admission before
provider capacity is available. Bulk work acquires interruptible capacity before
claiming a journal item, while an already claimed step retains its receipt.
`lifecycle::Signal` uses a capacity-one watch channel for stop/activity state;
folder preflight cancellation must not restore a polling loop.
The native `held_provider_slots=true` fixture owns all eight provider permits,
skips unrelated fixture startup cleanup, and checks that closing preserves
unstarted group steps. `close_request` sends WM_DELETE_WINDOW to the owned window
without waiting, so pending/failure UI can be exercised in a batch.

Closing requests the bulk worker to stop after its current receipt is durable; remaining queued work resumes with a fresh engine. An error cancels pending close. An explicitly resumed/new group can continue after another close dependency failed. Account-removal reviews include related group state and history; changed reviews are rejected, unfinished changes require the existing cancellation checkbox, and removal deletes only affected account entries/receipts. Preserve the independent-process lock test, query/receipt/restart/close/account-removal tests and all saved `test_bulk_*` native scenarios.

Passwords and Google refresh tokens belong in the operating system keychain, never SQLite. Google login uses system browser + loopback callback, PKCE and state validation. Drive is opt-in and uses the app-private `appDataFolder` scope. Encrypted backups use Argon2id + AES-256-GCM, fresh salt/nonce, authenticated version header, and compress-before-encrypt. Retention runs only after a successful new upload. Never remove unrelated files. Restore validates/decrypts first and merges downloaded messages.

Incoming IMAP/POP3 support SSL/TLS or STARTTLS; SMTP has independent TLS and authentication settings. Fastmail uses implicit TLS on 993/995 and SMTP 465 with an app password. Never disable certificate verification. POP3 leaves server originals intact and has local folders/flags. IMAP mutations check UIDVALIDITY; same-account safe move requires MOVE. Cross-account moves require two IMAP accounts and source UIDPLUS; preserve the destination acknowledgement journal, never remove the source before APPEND succeeds, and never automatically repeat an ambiguous upload. CalDAV writes use ETags; expanded recurring series are not overwritten through a single-occurrence editor.

Current practical limits and unsupported behavior must remain explicit in README: 25 MiB individual-message download ceiling, 256 MiB raw-mail snapshot ceiling, static HTML/text rendering with policy-controlled external images, Google Gmail uses app passwords rather than Gmail OAuth, calendar sync window -90/+365 days, recurring CalDAV edits belong in the server calendar UI. Improve these deliberately; do not hide them with success messages.

## Releases

Use Conventional Commits: `fix:`/`perf:` patch, `feat:` minor, `!` or `BREAKING CHANGE:` major. `main` is the only release branch. The root desktop application remains Rust. Node builds/tests/releases the browser and promotional site; Flutter builds the mobile app; the hosted beta service is Rust.

`npm ci` installs the locked release tooling. `.releaserc.json` runs commit analysis, notes/changelog, `scripts/release.py` (updates Cargo version and lockfile and creates a native archive/checksums), commits the version files, then publishes a GitHub tag/release. Release definitions remain dormant while the release workflow is disabled. `npm run release:dry` inspects the intended release with authenticated GitHub access but does not publish.

The release workflow waits for a successful full quality workflow and checks that `main` still equals the tested SHA before publishing. Current automated release packaging produces a Linux archive on the Linux runner; Windows/macOS compilation is covered by the CI matrix, but signed installers and distribution builds for those platforms need additional packaging jobs. Do not call unsigned development binaries signed/notarized.


## Linux user installation

`scripts/install-linux.sh` builds the optimized production release and delegates installation to `scripts/install_linux.py`; extracted archives include both scripts and a launcher icon. Install per user without sudo. Match the application ID, desktop filename and StartupWMClass (`so.shep.Shep`) so GNOME/KDE can group and pin the window. `--pin` is an explicit optional GNOME action and must preserve existing favorites. `--uninstall` removes the binary, launcher and icon only, never user data. The installer has automated tests for install/update/uninstall, path quoting and pin idempotence. App raster assets remain WebP; the PNG launcher icon exists for desktop icon-theme compatibility.

## Reading, input and sync regressions

Keep the inbox header compact: title, count and one sync control in one row; avoid duplicate busy badges or privacy slogans in navigation. Up/Down moves within the focused inbox/sidebar, Tab switches those panes, and keyboard navigation scrolls the selection into view. Double-click opens the full-window reader or an all-day calendar event. Open/close reader actions are remappable. Child buttons capture iced mouse-area events: double-click behavior must be tested through real input.

The native background-sync flow captures the compact mail header in light/dark, idle/busy states; the 900×640 flow also clicks Sync and navigates messages. Review those WebP screenshots after header changes. Before diagnosing a screenshot that differs from source, compare the running executable with the installed build: replacing the binary leaves already-open windows on the old executable. Install updates atomically and have the user reopen old windows rather than force-terminating them. Allow the native layout to settle after switching tabs before starting a pane-divider drag; a changed state observation can precede presentation.

Block external images by default. Message/sender/domain exceptions and a manually entered Contacts list are explicit preferences. Image requests validate every redirect and resolved address, exclude local/private networks, bound payload/decoded allocation, and convert off-thread to WebP. Quoted history has collapsed/expanded/latest-only preferences. Attachments wrap in the reply bar; sender details expose copy actions.

The Fastmail sync regression was missing parentheses around IMAP FETCH attribute lists. `imap_sync_uses_valid_fetch_lists_and_batches_bodies` drives the production sync function against a local IMAP transcript and validates both metadata and batched BODY.PEEK[] requests. Live diagnostics are ignored tests requiring an explicit `SHEP_LIVE_ACCOUNT_ID`; they read the saved OS credential and never send, move or flag mail. `saved_account_inbox_sync_to_local_cache` limits downloads to Inbox while using the same sync path. Run live diagnostics only for an account the user has authorized.

Release preparation also runs `scripts/verify_release.py`: it checks SHA-256, extracts into a temporary directory, and exercises the bundled installer without Rust. You can rerun it with `python3 scripts/verify_release.py dist/shep-VERSION-linux-x86_64.tar.gz`. Native key injection uses an explicit 1 ms xdotool delay; performance budgets remain unchanged. The native suite discovers functional flows in `scripts/e2e.py` alongside separate navigation and HTML pixel performance gates; distinguish selected reruns from full-suite evidence in the completion log.

Calendar provider writes return the committed event, including its server identity/ETag. Do not make a successful write depend on a subsequent calendar refresh, or retry it as a fresh create. Google creates use a stable per-form ID and verified conflict recovery. CalDAV edits GET the complete resource, retain alarms/attendees/extensions, and use If-Match; a successful PUT without an ETag requires a sync before another edit. Only 2xx acknowledges a commit; redirects are not success. Serialize sync and mutations per calendar. Remote IDs are scoped by calendar in the UI, command keys and storage; the v2 cache migration converts legacy composite keys. Completion events identify their form so they cannot close an unrelated dialog.

Calendar protocol tests use the scripted loopback server in `src/providers/test_http.rs` under `cfg(test)`, with calendar fixtures/re-exports in `src/providers/calendar/test_server.rs`. The shared server records binary request bytes and supports binary replies for Drive tests. Clients and credentials remain object-scoped fixtures. `cargo test --all-features calendar` covers wire failure/retry contracts, cache migration, source identity, iCalendar durations and escaped text. The native suite also edits/deletes duplicate remote UIDs across different calendars. Save logs under `artifacts/logs/`. Live calendar-provider verification remains distinct from these tests.

`cached_reads_and_ordered_saves_complete_with_all_network_jobs_and_queue_occupied` holds every provider slot open and fills the provider queue, then executes actual SQLite search/body reads and saves the latest preferences/draft through the production dispatcher. This is a deterministic correctness test, not a timing benchmark. Keep its blocked-provider condition intact; do not replace it with a short sleep that might finish before navigation. The test-only holding command is compiled only by `cargo test`, never into release or native test-support binaries. Preserve FIFO ordering for draft/settings persistence when changing concurrency.

Google connection status is checked by a provider command after the cached workspace is ready. Do not await OS credential-store access before emitting `Ready`. Failed speculative sends must clear their pending prefetch markers so the message/page can be requested again.

Preferences use client edit generations and persistent store revisions. `SavePreferences` returns a small `PreferencesSaved` snapshot; do not reload an entire workspace for each preference change. `ui/preference_sync.rs` preserves newer local edits and rejects older store snapshots, including while the pane divider's save is debouncing. Backup completion must use `Store::update_preferences` to change only its metadata; never write the preferences captured before a long upload back over current settings. UI saves preserve backend-owned `last_backup`. Google OAuth starts after its settings save is acknowledged, and the provider job must not rewrite the old settings. `tests/preferences.rs` covers persistence/revisions and the metadata race; the native suite combines appearance, unified/cross-account preferences and resizing and waits for `preferences_saved`.

Pending individual moves attach bounded `MailMoveProjection` hints only to cloned read queries, never to the logical selection scope. `store/read_moves.rs` projects folder/account/flags in a connection-local temporary view within the read transaction; it preserves full-text ranking and page boundaries without updating source data or publishing an obsolete remote UID. Selection capture rejects display hints. Temporary destination rows use cached bodies and cannot issue provider mutations; receipt handling replaces their metadata with the actual acknowledged identity without losing the selected reader. Keep the move-projection storage/controller tests and native pending-destination, failure/Undo and cross-account scenarios. A move without a destination UID still needs durable recovery (R73); this display projection does not resolve that backend gap.

Message-detail requests/results carry `detail_revision`, independent of page/search generations. Increment it and clear pending prefetch markers when mail changes. Ignore both data and errors from earlier revisions; a late body load must not undo new flags or a completed move. `ui/reading_tests.rs` controls backend-result ordering; native MCP mouse flagging/moving remains the control-level test.

Backup actions save the displayed preferences and wait for that generation's acknowledgment before entering the provider queue. Commands and list results/errors carry `BackupTarget`; list generations reject earlier results, and restore keeps the target selected with its copy. `last_backup`, `backup_ready` and `google_connection_id` are backend-owned metadata. `Store::record_backup` only updates a matching destination. Google identity is `drive:` plus the verified Drive `user.permissionId`, scoped by OAuth client in the target; reconnecting the same account preserves pending work, while another account resets readiness/history. Tokens record their issuing OAuth client and refuse use with another client. `Store::activate_google` rejects client settings or lifecycle changes during OAuth completion. The shared Google connection lock serializes login against Drive backup/list/restore operations. Do not restore these device-specific settings from a snapshot.

Every successful manual backup stores its passphrase under a destination-specific OS keychain entry, including when automatic backup is disabled. The first copy prepares the schedule; missing credentials pause it with a recovery instruction. The scheduler reads keychain credentials inside the provider job, never while polling the network dispatcher, and rate-limits failed automatic attempts to the configured interval. `BackupSaved` acknowledges upload before metadata/keychain/retention work; subsequent failures must say the copy was saved. Retention requires the committed ID in a complete, duplicate-free list and always protects it even when the clock changes. Local backups use temporary files and no-overwrite persistence, validate the timestamp/UUID filename, and bound reads even if a file grows.

`src/engine/backups_tests.rs` injects an object-scoped fake passphrase store and uses real temporary encrypted local archives; it never reads or writes the user's keychain. Tests cover manual-to-automatic setup, missing keychain access, metadata/retention failures after commit, destination changes, reopening a staged/committed copy without duplicate upload, and finishing failed setup on the same copy. `tests/backups.rs`, `tests/preferences.rs` and UI ordering tests cover protected retention, no-overwrite/temporary cleanup, destination metadata and stale list data/errors. The native Backups flow checks first-copy setup and saved form ordering in the isolated preview; preview intentionally cannot create/restore backups or contact Drive.

Production backups must use `BackupProvider::reserve` and `upload_prepared` through the durable journal in `backup/journal.rs`. Persist the reserved ID and exact encrypted archive before calling upload; persist each session URL before sending bytes. Never replace an unresolved record with a new archive, file ID or passphrase. A retry decrypts the staged archive with the supplied original passphrase before associating it with scheduled backup. The journal uses its own SQLite connection/file (`backup-uploads.sqlite`), WAL and synchronous FULL, so archive writes do not hold the mail-cache connection. In-memory test stores must get in-memory journals: SQLite can report their path as an empty string, which must not become a file in the repository root. Conditional checkpoints/removal protect a newer record from stale work; committed records remain until cleanup succeeds. Keep no root artifacts.

`backup/drive.rs` implements bounded Drive HTTP parsing, pagination-loop/incomplete-list/duplicate rejection, owned app-data checks, reserved IDs, 1 MiB resumable chunks, authoritative Range acknowledgments, session expiry and verified commit recovery. Only the documented same-origin resumable endpoint may receive archive bytes; never follow a session URL to another origin. Confirm returned ID/name/size/app properties and server SHA-256 (or fetch ciphertext if the checksum is absent). Uploads have per-request timeouts and bounded retries/no-progress detection, rather than an overall ten-minute cutoff. `backup/drive_tests.rs` uses the production API implementation with loopback HTTP to cover lost/partial responses, restart, checkpoint failure, expired sessions, conflicts, untrusted URLs, corrupt files and bounded listing/downloads. These are protocol contracts, not genuine Google verification. See [Drive uploads](https://developers.google.com/workspace/drive/api/guides/manage-uploads), [generated IDs](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/generateIds) and [Drive account identity](https://developers.google.com/workspace/drive/api/reference/rest/v3/about/get).

Restore validates the whole snapshot before changing storage or credentials. `backup/restore.rs` derives permitted incoming/SMTP/CalDAV credential keys from included sources, forbids device-key aliases and duplicate owners, checks calendar URLs/message IDs/limits, and rebuilds display/search metadata from original MIME off-thread. `store/restore.rs` uses one additive SQLite transaction: existing mail/flags/folders/configuration/preferences/drafts win. Do not turn restore into an upsert that rolls current flags or settings backward. Orphaned cached mail and locally moved stable IDs are valid. Credential-owner or cached-message identity collisions roll the transaction back.

Only after local commit does `engine/restore.rs` fill missing OS passwords for unchanged or newly imported connections. Existing passwords must survive an older backup. Distinguish `keyring::Error::NoEntry` from a locked/unavailable store; failed keychain work must report that mail was restored and explain retrying the same copy. Do not compensate by deleting restored mail or storing plaintext rollback secrets in SQLite. Account/calendar locks serialize import against sync/moves/connection edits; retain transfer's sorted account-lock order. Restore must observe its background SQLite transaction to completion rather than dropping the future at an overall timeout. `engine/restore_tests.rs` uses encrypted temporary archives and an object-scoped fake keychain, covering malformed ownership, late-insert rollback including FTS, existing state preservation and retry after partial keychain failure. These tests never touch personal credentials. The transaction still occupies the cache connection; independent read access during large restore/export remains in the completion audit.

Newly restored mail is pinned in `restored_messages` until a complete server listing confirms its remote identity. IMAP reconciliation must preserve pinned mail absent from the server; otherwise the next sync destroys the only recovered copy. Pins persist across restart, are inserted in the restore transaction and cascade away on explicit local deletion. A confirmed live identity resumes normal server reconciliation. Restore never uploads mail to a server implicitly.

Google token handling lives in `providers/google/tokens.rs`. One shared auth-state mutex serializes refresh, authorization-code exchange and OS credential writes; overlapping callers coalesce a refresh. Bind saved credentials to their issuing OAuth client, preserve an omitted refresh token only during refresh, and replace it when the response rotates it. A new login must supply its own offline grant. A failed refresh save leaves the new grant pending in memory; retry persistence before returning access or issuing another refresh. A failed login save keeps a separate candidate; the committed grant remains usable while Reconnect Google retries saving the candidate. Do not exchange a received authorization code again or silently borrow the previous account's refresh token. Keep these pending credentials only in memory/OS storage, never logs or the mail database. An app exit before a failed save is recovered can require signing in again.

`OsCredentialStore` also uses a FIFO `WriteQueue` whose owned guard moves into the blocking OS operation. A cancelled caller must not release ordering while its `spawn_blocking` write is still running; otherwise an old grant can overwrite a newer sign-in. The cancellation test holds the first operation, aborts its async caller, and verifies the next write remains pending until the first operation finishes, without timing measurements. Share one Google client per running profile; coordination across independent app processes remains lifecycle work.

Only successful HTTP token responses with validated bearer tokens/lifetimes may replace current credentials. Bound both declared and streamed JSON to 64 KiB, reject redirects/malformed responses, and keep remote payloads out of error chains. Revoked refresh grants stop further attempts in that process until login; application-credential errors allow correction. Connection checks validate saved credentials/client binding without issuing network refreshes or delaying the initial cached workspace. `providers/google/callback.rs` checks GET/path/Host/state and duplicate parameters, handles fragmented headers, limits headers to 8 KiB and concurrent local connections to eight, and uses per-connection/overall sign-in timeouts. Browser acknowledgments contain no code or state. Provider tests use an object-scoped fake keychain and `providers/test_http.rs` (including chunked replies), never personal Google credentials. See [Google desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app) and [refresh-token rotation, RFC 6749 §6](https://www.rfc-editor.org/rfc/rfc6749.html#section-6). Live Google authorization and independent-process lifecycle coordination remain separate requirements.


Draft composition lives in `compose.rs` (recipient/reply headers and MIME), `store/drafts.rs` (versioned text plus separate attachment blobs) and `ui/composing.rs` (native controls). Never put attachment bytes or filesystem reads into UI messages. Text autosaves deliberately omit file associations; only backend add/remove operations own those associations. Draft edit revisions reject late text saves, a persistent sent-version tombstone prevents old saves from reviving delivered drafts, and global draft snapshots reject stale file lists. Closing waits for save acknowledgment and failures preserve the open editor. Tag acknowledgments by draft ID/revision so they cannot close another dialog or discard newer text.

SMTP construction validates To/Cc/Bcc together, supports quoted display names, deduplicates the envelope and never emits Bcc headers. Reply-all uses Reply-To, excludes all configured sender addresses and preserves In-Reply-To/References. Its default `Shift+R` binding is appended to the keymap so older saved mappings inherit it. Copy selected regular files in a blocking worker, then store their bytes separately; limits are 32 files, 18 MiB total attachments and 25 MiB encoded mail. Keep per-request SMTP timeouts, but do not cancel post-delivery local cleanup at a generic provider-job timeout. A confirmed SMTP send must remain acknowledged if local cleanup fails; a lost acknowledgment must not be described as a definite rejection. See outgoing recovery below for the durable delivery and server-copy contracts.

`tests/composing.rs` verifies recipient privacy, reply headers, old-draft migration, attachment MIME/Unicode/binary roundtrips, reopening without source files, atomic failed imports, stale saves/removals and sent revisions. `shared/mail-core/src/providers/smtp_tests.rs` exercises the production delivery function through an object-scoped loopback SMTP transport, including rejected recipients and lost DATA acknowledgments; only that fixture transport uses plaintext. UI ordering tests cover stale file/workspace snapshots, failed saves and unrelated dialogs. These tests do not send personal mail.

The native composer flows use `desktop.batch`'s `choose_file` action after clicking Attach files. Its optional `path` must resolve to a fixture file inside the current run's artifact directory; omit the path to cancel. The harness disables access to the user's desktop portal and isolates GTK config/data/cache directories. It operates the real Zenity picker with X11 input, pastes the complete path using an owned Xvfb clipboard, verifies GTK copied back the exact entered path after replacing the harness clipboard owner, waits for the dialog to close and restores app focus (Xvfb has no window manager). Filename validation is asynchronous: bounded Return retries target only the picker window, so its closing cannot send a key to the composer. GTK path completion can corrupt paths injected character by character. `xclip` and Zenity are additional Linux harness dependencies; the owned clipboard process is stopped on cleanup. Keep the actual picker flow, its protocol/unit tests and the saved native scenarios together. A 400 ms visual settling wait after file import is not a performance measurement; final timing gates remain deferred until the host is idle.

Conversation reading lives in `store/conversations.rs` and `ui/conversations.rs`. Link only explicit Message-ID/References/In-Reply-To within an account, including cached mail from different folders. Keep individual inbox rows and anchor selection distinct from the expanded message: reply/move/flags/export/image actions target the expanded physical message. Duplicate folder copies display once; prefer the selected/focused UID. A General preference disables grouping independently of quote-history display.

Index old caches after Ready in resumable 32-message transactions; parse at most 64 KiB of headers per message. Index newly stored/restored mail inside its transaction. Keep conversation token links after message deletion so surviving replies stay connected; account removal also cleans up that account's token index. Metadata pages contain 20 entries; exclude raw MIME and body text from SQL ranking/window-function input. Expand one body at a time and use the existing bounded neighbor cache. Tag results and delayed scroll actions by generation, preserve current focus across sync, and reject stale results after selection or preference changes.

Use `desktop.start` with `conversation_mail: true` for isolated short/long multi-message fixtures. Observe `conversation_rows`, `conversation_total`, `conversation_offset`, `reader_message_id`, `loaded_message_id` and `group_conversations`; wait for `loaded_message_id` before replying. `tests/conversations.rs` and UI ordering tests cover index/backfill/action invariants. The saved native flows verify flags, moves, older-message replies/attachments, collapse, preferences, dark/compact/full-window layouts and paging during sync. Missing nested list entries during asynchronous refresh are failed observations to retry until the normal wait deadline, not immediate harness errors.

CalDAV connection setup lives in `providers/calendar/discovery.rs`, `engine/calendar_connections.rs` and `ui/calendar_setup.rs`. Find calendars before selecting/saving them. PROPFIND is read-only; it follows bounded same-origin redirects, well-known/principal/home properties and direct collection URLs. Never forward credentials to another origin or disable certificate checks. Parse only namespace-correct successful properties; optional 403/404 properties are common. Preserve collection URLs, filter for VEVENT and bound requests/results/bodies. Discovery has a 90-second overall timeout and no persistence. Form generations reject late discovery results after edits/close/reopen.

Connecting selected calendars deduplicates existing URL/username pairs and derives new `caldav:` credential IDs internally; never trust a server/caller to name an OS secret. Serialize connection saves with a separate setup lock, then take sorted source IDs consistently with restore/sync locks. Never put synthetic connection keys in the same lock namespace as restored source IDs. Save selected source metadata atomically after password writes. Retry uses the same credential identities; do not automatically roll back or erase a successfully written password. Connection acknowledgments identify their form and must not close another editor. Do not drop a pending keychain/store save at the generic provider timeout; window close waits for this operation. Closing/saving the setup dialog restores preference fields behind it.

`CalendarAccess` distinguishes create/update/delete, with backward-compatible defaults for legacy sources. Known read-only calendars have a view-only event dialog; new event choices/defaults require create access. Enforce permissions in UI, engine and provider writes. CalDAV uses advertised privileges where available and lets the server enforce unknown permissions. Google list parsing (`calendar/google_sources.rs`) is bounded, rejects malformed/repeated pages and records roles. A complete refresh disables old grants absent from the list while retaining cached events; it must not erase cached calendars on a failed/partial response. Do not describe calendar-level roles as proof of access to every private event.

The native harness supports `readonly_calendars: true` to make the fixture Home calendar read-only. For discovery use an isolated `empty_calendars: true` preview and `https://calendar.example.test/`, username `alex`, password `fixture-password`. These values are handled only in `tests/support/fixtures.rs`; no network/keychain is used in preview. Observe `calendar_choices`, `calendar_selected`, `calendar_sources`, `calendar_error`, `calendar_discovering`, `calendar_saving` and `event_access`; never inject actions through state. Saved scenarios cover empty selections, multiple connections, errors/retry, idempotent reconnect, read-only viewing and dark/compact layouts. Provider tests use local HTTP transcripts; genuine Google/CalDAV authorization remains separate evidence.


## Connection removal and cleanup

`store/connections.rs`, `engine/removals.rs` and `ui/removals.rs` implement explicit removal of mail accounts and individual calendars. Review exact local counts and a fingerprint of the connection, message IDs, draft text/file metadata, events and associated transfer journals. Recheck that fingerprint in the deletion transaction; a changed snapshot requires another review. Unfinished transfers require a separate cancel checkbox. Remove only local data/journals; never issue server deletes or erase existing backups from this action.

Local deletion and pending OS credential cleanup jobs commit atomically. Failed cleanup must leave the connection removed and its jobs retryable; NoEntry is already cleaned. Retry after Ready and from Accounts/Calendars preferences. Before deleting a secret, check every current account/SMTP/CalDAV owner. A global connection lifecycle lock precedes per-account/calendar locks for connection saves, restore and cleanup, so reconnect cannot race an old deletion. Do not cancel these OS/SQLite mutations at the generic provider timeout. Connection and event snapshots have separate store revisions; older snapshots cannot restore deleted UI data.

Active tombstones reject delayed sync/draft saves. Calendar removal epochs survive an explicit reconnect and reject old queued setup attempts. Regular Google list refresh skips removed calendars; only the explicit restore-calendars action or a confirmed backup restore revives them. Removing one calendar must never erase shared Google authorization. Connection restore clears obsolete cleanup jobs inside its transaction. Multi-process lifecycle coordination remains an audit item; device disconnection is described below.

`tests/connections.rs` covers account/source isolation, FTS/conversation/draft-file cleanup, changed reviews (including attachments), move-journal consent, rollback and reopen. Engine tests use injected fake secret removers for failures, shared aliases and controlled cleanup/reconnect ordering; never test removal on a personal account. The native suite removes/cancels fixture connections, navigates remaining mail, restores Google calendar entries and reviews light/dark/900×640 dialogs. `desktop.start(pending_transfer=true)` seeds one fixture unfinished move; its saved scenario checks the disabled confirmation and explicit checkbox. Keep these flows functional-only until final performance testing on an idle host.

## Outgoing recovery and server Sent

`outgoing.rs`, `store/outgoing.rs`, `engine/outgoing.rs` and `ui/outgoing.rs` separate delivery from Sent-copy recovery. Commit the exact MIME, private SMTP envelope, immutable attempt/Message-ID and account configuration before any SMTP call. Reject stale draft text/files and duplicate attempts transactionally. A surviving Submitting record is uncertain, never an automatic retry. Known rejections permit an explicit new Send. Do not include server response text or credentials in delivery diagnostics.

Release the composer only after the durable submission acknowledgment; subsequent completion events must identify their draft/revision and leave unrelated editors open. SMTP acceptance precedes atomic local Sent insertion, sent-version tombstone and draft/file cleanup. Preserve local flags/folder changes when repeating cleanup. Startup repair performs local work for accepted mail only. It never calls SMTP or blindly retries APPEND. Outbox counts, blocked draft IDs and pages have store revisions; old workspace/results cannot restore an obsolete recovery state. Empty trailing pages return to the last populated page.

`providers/outgoing.rs` injects object-scoped transports for deterministic failure tests. Production keeps SMTP credentials independent from incoming credentials and skips credential reads for unauthenticated SMTP. `shared/mail-core/src/providers/mail/sent.rs` discovers selectable Sent mailboxes with SPECIAL-USE, bounded names and unambiguous fallback; account settings can override the folder. Exact Message-ID headers verify SEARCH candidates. SMTP/IMAP connection certificates remain verified. Automatic/server-managed/local-only policies are saved per account; POP3 is local-only. Server-managed mode checks but never APPENDs.

Persist Appending before upload; tagged APPEND OK is the commit point. Failed acknowledgment stays uncertain until an exact lookup or explicit reviewed retry. Sent-copy recovery cannot resend mail. The account lock serializes sends, copies, sync and removal. Server Sent sync replaces the local copy only for the same account, logical identity and confirmed folder; retain a local copy moved out of Sent. The sidebar Sent query includes actual per-account Sent mappings and local-only copies. Local copy flags/moves never use IMAP; cross-account moves require an existing server copy. Account removal reviews/fingerprints/deletes its outgoing records and mapping.

`tests/outgoing.rs` covers durable wire/envelope privacy, restart, atomic failures, stale attempts, bounds, deduplication, local edits, Sent aggregation and reviewed removal. Engine tests use fake SMTP/Sent transports to cover lost acknowledgments, failed acknowledgment persistence, all policies and local mutations; protocol tests exercise actual IMAP discovery/SEARCH/FETCH/APPEND transcripts. These are not live SMTP/Sent verification. Native `outgoing_mail: true` scenarios exercise disabled recovery, explicit review, return to drafts, record as sent, copy retry errors, keep-local completion and light/dark/compact views. Do not test recovery on personal outgoing mail.

## Google device disconnection

`store/google_lifecycle.rs` commits a versioned disconnect and pending credential cleanup before touching the OS keychain. Keep cached Google events and calendar names, mark their access read-only, and track them in `google_archived`. Pause automatic Drive backup/readiness while retaining destination identity, history and upload journals for explicit recovery after reconnect. Local-folder backups, mail accounts, CalDAV and server data are unaffected. This control removes the local Google login; it does not call Google's project-wide revocation endpoint.

`engine/google_lifecycle.rs` takes the Google connection write guard before the lifecycle guard. Google provider work uses read guards; login, disconnect and cleanup use the write guard. Do not invert that order with calendar/account/removal/restore locks, and do not cancel keychain mutations at the generic provider timeout. Show the cached workspace before startup cleanup. If deletion fails, clear in-memory tokens, preserve the pending cleanup state and offer a retry; reconnect must finish that cleanup first. The credential store's FIFO covers delete as well as write so an older blocking save cannot restore a deleted login.

`GoogleLifecycle` is backend-owned preference metadata. Late UI saves/acknowledgments must not clear it or re-enable automatic Drive backups. Connection status events carry its revision; archived-source snapshots follow connection revisions. A stale disconnect review cannot remove a newer login. A successful fresh list reactivates only returned calendars; sources missing from that list remain archives. Restore and source saves must preserve read-only archives while disconnected. A late disconnect result cannot close an unrelated editor. Window close waits for active Google connection changes.

`tests/google_lifecycle.rs`, provider fake-keychain tests, engine lock/mutation tests and UI ordering tests cover rollback/reopen, retained cache, cleanup failure/retry, stale reviews/settings/status and archived edits. Native light/dark/compact flows cancel and confirm the actual dialog, inspect retained read-only events and navigate mail afterward. Do not disconnect a personal Google account for these tests. Independent-process coordination and live Google verification remain audit items; the staged grant implementation below establishes switching within one engine.


## Google grants and activation

`providers/google/consent.rs` builds the exact request from Preferences’ next-sign-in service choices; `providers/google/scopes.rs` interprets the actual returned OAuth scopes. Keep desired `google_services` separate from the active grant. Fresh setup is opt-in; explicit choices override legacy grant-derived defaults. Omitted fresh-response scopes inherit only the requested set. Persist that set with the candidate, retain it on refresh, and intersect broader returned grants with it. Retry requires matching requested services; saved-form acknowledgment, queued execution and final activation reject changed setup. Preserve the protocol, lifecycle and saved native consent controls when changing this path. Calendar sync needs list plus event read permission; editing also needs event write permission. Drive backup needs `drive.appdata`. Handle Calendar-only, Drive-only and read-only Calendar grants independently; unsupported services return an actionable reconnect error before making a request. Calendar roles further restrict the OAuth grant. Omitted fresh token-response scope inherits the requested scopes per OAuth; legacy saved entries without scope metadata remain server-enforced until reconnect. Never infer a missing refresh token from another account.

The OS credential entry is a bounded two-grant vault, accepting the original single-grant format on read. Stage the candidate alongside the grant selected by `Preferences.google_grant.id`. Service validation runs against that candidate only. `Store::activate_google` atomically commits the non-secret grant ID/client/access, verified Drive identity, lifecycle revision and refreshed/archived calendar access. It rejects changed client settings and cleanup state. The DB pointer is the activation point: a crash before it keeps the previous login; a crash afterward selects the candidate even before credential pruning. Never put token material in SQLite.

Keep an explicit candidate ID in the vault. A leftover previous credential after failed pruning is not a pending sign-in. Reconnect retries a persisted candidate without another browser/code exchange; **Start a new sign-in** deliberately replaces the candidate and requests Google's account chooser. A failed candidate keychain save remains in memory while the previous grant stays usable. Refresh saves the whole bounded vault and preserves the other grant. Do not prune the previous secret before the DB activation commits. Pruning failure cannot turn a successful connection into an uncommitted retry. Device disconnect clears all grants.

The active OAuth client remains distinct from edited setup fields. Backups keep using its client/verified Drive destination; new client settings become active only on commit. Refresh uses the committed client's saved configuration when another client is being edited. Backend-owned grant metadata survives late preference saves/results. A Calendar-only grant pauses Drive automation but retains its prior destination/history; a Drive-only grant archives cached Google calendars. Source setup, refresh and backup restore cannot bypass the current read-only scope.

Provider tests use object-scoped fake keychains and loopback API endpoints to exercise partial scopes, refresh scope changes, staged retries/restarts, keychain failures, transactional rollback, old-client preservation and safe cleanup. Saved native tests use `desktop.start(google_permissions="calendar" | "drive" | "read-only")`; these are isolated fixture grants and never authenticate to Google. Observe `google_grant.access`, source/event permissions and continued mail navigation. Review light/dark/compact WebP captures. Live authorization and coordination between independent app processes are still unverified.

References: [Google native authorization and granted scopes](https://developers.google.com/identity/protocols/oauth2/native-app), [Calendar scopes](https://developers.google.com/workspace/calendar/api/auth), [OAuth token response scope, RFC 6749 §5.1](https://www.rfc-editor.org/rfc/rfc6749.html#section-5.1).


## Sidebar layout, context actions and explicit save feedback

The sidebar divider is a native drag widget with a nine-pixel hit area and a one-pixel resting rule. Store the chosen width in `Preferences.sidebar_width`; clamp its displayed width against the available scaled viewport without overwriting the saved choice on a smaller window. `window_size` is restored with an iced window task after cached preferences arrive, never by opening storage in the UI. Resize saves debounce for 350 ms. Closing flushes a pending reader drag and waits for the latest settings acknowledgment; draft completion re-enters the same close path. Do not close the window on a save failure.

General settings use one coalesced retry slot when the bounded persistence queue is full. A newer accepted save clears any older queued retry. Typed `PreferencesSaveFailed` events preserve unsaved edits, cancel only dependent pending operations and keep the window open. An explicit Save changes/Save contacts action shows the dismissible Changes saved toast only after the current edit generation is acknowledged. Automatic layout/appearance saves do not produce a toast for every adjustment.

Ctrl+click (Command+click also works on macOS) toggles account-scoped folders into a combined query; a normal click selects one. SQL binds selected scopes as JSON to avoid parameter limits and deduplicates overlapping global/account scopes. Some(empty) means no selected folders; it must never fall through to all mail. Account headings collapse with a chevron; their state persists. Long labels shape against actual available width; labeled sidebar rows do not show tooltips. Account setup lives in Preferences, not navigation.

Inbox right-click actions retain the clicked message identity. Reply/move/export wait for that message's body, and navigation cancels the pending action; late body results must never open an editor for another message. Escape/outside click dismisses the menu; Shift+F10 opens it for the selected message, with arrow/Enter navigation. Only icon controls show tooltips, optionally including the primary remapped key. Preferences can disable all tooltips or only shortcut hints. Flagged buttons use the theme's red outline in both inbox and reader.

Contacts has its own Preferences tab; image policy and exceptions remain under Privacy. The Calendar view omits the redundant breadcrumb/footer and uses a refresh icon. Native calendar tests must follow its actual new event/agenda positions.

The native functional suite includes combined folder selection, account collapse, long labels, sidebar/window resizing, Contacts/save toast and inbox context actions in light/dark/compact layouts. The harness accepts `hover`, native `resize`, and `click`/`double_click` with `button` and optional modifiers; release held modifiers even if input fails. Schema and implementation must agree. Window persistence additionally has a real SQLite reopen test, and UI tests cover close-before-debounce, failed saves, stale acknowledgments and a full persistence queue.

The active request backlog in `docs/COMPLETION.md` includes folder trees/context mutations, collapsible/deletable drafts, an inline composer with multiple drafts, palette editing, encrypted/streamed large mail and multiple backup destinations. Those are not complete merely because this increment passes.


## Current shortcuts, reader selection and menu regressions

`shortcuts::Keymap` stores versioned primary/secondary maps and reads the old map format. Ctrl+D (Command+D on macOS) moves to Trash; Backspace and Delete archive. Only Archive starts with a secondary binding. I goes to Inbox only with sidebar focus; its primary binding can be disabled from Shortcuts. Keep conflict checks across both slots and preserve custom keys during migration, leaving a new conflicting default unbound. Captured text-editing shortcuts must not mutate mail.

Native `ContextArea` snapshots modifiers while processing the mouse event. Do not read a later global modifier value to interpret Ctrl-click. Background mail Changed notifications must preserve an open context menu; refreshed metadata updates its owned target. Normal outside clicks/Escape still dismiss. Preview sync emits the same refresh event so the saved native regression can detect accidental dismissal.

Read-only editor buffers for mail text are prepared off-thread with one blocking preparation permit; cancelled/stale results never replace another message. Only selection/cursor/copy operations are accepted. The native suite copies text from preview/full reader and checks ordinary shortcuts still work. HTML has its own worker and visible-text selection path, described below; Ctrl+F uses the displayed-text find path described below.

Preferences search indexes actual editable sections in `ui/settings_search.rs`; update that index when adding settings. Search results open the matching controls. Preserve icon-only tooltip behavior and both tooltip preferences. Unread Inbox counts cover the cache independent of the current query. The Move chooser shows the server's INBOX as Inbox and marks its first result with the Enter hint.

`move_imap_session` treats the server MOVE acknowledgment as committed even if logout fails. The local protocol test covers a spaced source folder returning to INBOX; it is not evidence of a live personal-account move. The harness starts Xvfb with `-noreset`, checks display readiness and stores `xvfb.log` inside the run artifacts.

## Immediate mail actions and command acknowledgments

`ui/mail_actions.rs` overlays small message metadata while flag/read operations are pending. Coalesce each field per message, retain newer edits after an older acknowledgment/error, and never clone body or attachment buffers on the UI thread. Pending moves hide their source row immediately; hold a move behind that message's outstanding flag changes, and restore rejected actions with a visible error. Background pages must retain pending overlays. Typed request IDs reject obsolete completions; a full command queue must not hide a message or leave a false flag. Window close waits for accepted mail changes, and a failure cancels the pending close.

Use `run_command_and_check_ok` for IMAP UID STORE and UID EXPUNGE: async-imap 0.11.3's streamed helpers discard the tagged completion status. Loopback tests must cover NO as well as OK, both read/unread directions, flag preservation and rejected transfer cleanup. Acknowledged MOVE/STORE remains committed if logout fails; folder refresh errors after MOVE are separate from rejection. Read changes write only Seen and flag changes write only Flagged.

Native `desktop.start(mail_actions="slow" | "fail")` exercises delayed success and rejection against isolated fixture storage; it never touches a real account. Saved E2E scenarios check immediate read/flag/archive feedback, coalescing during refresh, rollback and continued navigation. This is correctness testing with controlled delays, not a performance measurement. Each shortcut slot has its own ×, including primary; empty bindings remain disabled after serialization/reload.

Move search Enter resolves the latest form text in `Message::MoveFirst`; do not capture the first destination in the prior frame's text-input submit message. Keep the native fast typing/Enter regression and the no-redraw state test.


Client Find text contracts live in `shared/mail-content/src/find.rs` and `shared/find-cases.json`: retain literal Unicode case folding, Rust whitespace normalization and original UTF-16 offsets. Native search runs with a separate blocking permit; browser search uses a preloaded Rust WASM worker so cached Find works offline. Only displayed quoted text enters the search, and both models coalesce pending work and reject stale source/query/case/error results. Keep native text selection/Copy intact. Saved controls live in `flutter/test/support/message_find_scenario.dart` (host plus Android incoming integration) and `web/e2e/message-find-flow.mjs` (preview plus actual Rust HTTPS), with remapping/layout checks in `message-find.spec.ts`. Inspect the active match in the scroll viewport and review captures; do not substitute direct model actions for UI input. HTML, bounded large-text layout/search and Apple execution remain explicit parity gaps.
## Frequent background mail checks

`engine/mail_sync.rs` starts a check after cached Ready and uses a saved seconds-based interval, default 15 and configurable from 5–3600. `sync_minutes` remains the separate calendar cadence for compatibility. Do not restore a minutes-long mail default. Settings changes update a watch channel only after persistence; unrelated saves do not reset the schedule. A single scheduling loop prevents overlapping cycles and retains one requested follow-up during background/manual work. Manual clicks update the refresh control immediately; automatic checks use a separate busy key. A completed short account check flushes its cached changes without waiting for slower accounts.

The sync worker shares the provider semaphore and cannot consume foreground-read or persistence workers. Timeout/failure releases the current cycle and permits retry. Typed `MailSyncFinished` results clear only the earlier sync error after recovery; successful explicit settings saves clear only their settings error. Preserve unrelated errors. Keep optimistic mail overlays and open context menus through background refreshes.

Virtual-time Rust tests exercise the actual scheduler with object-scoped cycle implementations: immediate startup, repeated checks, interval changes, queued/coalesced refresh, failure and timeout recovery. These are correctness checks, not performance measurements. Native `background_sync: true` uses a delayed fictional arrival; `sync_failure_once: true` fails its first check. Both require test-support and never contact a real mail server. Production startup remains automatic; ordinary native fixtures keep automatic checks off for reproducibility.


## Search relevance and library matching

`fuzzy.rs` uses RapidFuzz 0.5 (OSA edits, LCS subsequences and ratio) instead of the handwritten edit matrix. Exact folder/leaf names, prefixes, words, typos and abbreviations have deterministic ordering; Enter still resolves the latest field text. Normalize Latin accents while preserving Japanese marks and recomposing Hangul. Keep Unicode regression coverage when changing token normalization.

New inbox searches select Best match; explicit search sorting is temporary. Clearing search or using Mail to return to Inbox restores the saved browsing sort. SQLite performs ranked selection and paging off-thread. Separate exact-term BM25 from expanded-term BM25 so a rare typo does not inflate an exact hit. Sender weight is lower than subject/body. A short whole-body equality check takes priority over keyword repetition; guard it with `octet_length` metadata and CASE before reading text. The guard allows normal surrounding line endings; longer bodies remain eligible through indexed ranking. Stable timestamp/ID ties, filtered counts, folder scopes and stale-page rejection must remain correct.

Vocabulary expansion retains indexed one-edit lookup and bounded two-edit candidates from a shared prefix, verified/ranked with the library. Numeric tokens remain exact. Input is tokenized and bound as data, never interpreted as raw FTS/SQL syntax. A nonempty query without indexable tokens returns no results. The existing performance benchmark now exercises Relevance; keep its measurement deferred until the final idle-host run.

Native `search_mail: true` provides an old exact-body message, newer weak/repeated/typo matches and a Café folder. Keep the saved search/sort/sync and accent/fast-Enter move scenarios. Observe `sort`, `mail_rows`, `selected`, `move_enter_destination` and destination-folder contents through real controls. This fixture contains no personal account data.

References: [RapidFuzz](https://docs.rs/rapidfuzz/latest/rapidfuzz/), [SQLite FTS5 ranking](https://www.sqlite.org/fts5.html#the_bm25_function), [SQLite octet_length](https://www.sqlite.org/lang_corefunc.html#octet_length).

Message actions resolve the current reader ID against matching body metadata, the expanded conversation page or the inbox page, in that order. Move/read/flag must work while a body is loading and must never target a stale body from another message. Clear native-focus observation when Move closes; a stale folder-search focus value cannot prove a later dialog is ready. Native tests must assert that Move opened before checking field focus and typing.

## Draft navigation and permanent discard

Drafts form a counted, collapsible sidebar group; `Preferences.collapsed_drafts` preserves the choice. Draft rows use the native `ContextArea` right-click path, retain their owned target through mail refreshes, and support mouse controls plus Up/Down/Enter/Escape. Discard from the editor's bin or context menu opens a review with the subject and attachment count. Cancel/Escape/N preserves unsaved fields; Enter/Y confirms. Do not resume editing or close the window while a confirmed discard is pending. A storage error keeps the review and original editor available for retry/cancellation.

`Store::delete_draft` atomically retires the identity with the maximum supported revision in `draft_sent`, removes its text/attachment blobs, and advances the draft snapshot revision. The permanent tombstone must survive restart and later send cleanup. It rejects saves/file imports captured before discard, even with a newer edit revision. Submitting, uncertain and accepted outgoing records require Outbox review before discard; known rejections can be discarded directly and their wire/envelope record is removed atomically. Check this in the same SQLite transaction so a concurrent submission cannot bypass it. Send commit already checks the retirement record. Route DeleteDraft through the independent FIFO persistence worker and return a typed DraftDeleted snapshot; old snapshots cannot restore deleted rows or close another editor.

`tests/composing.rs`, `tests/outgoing.rs` and UI ordering tests cover reopen, late saves/files, transaction rollback, send/discard exclusion, cancellation, failed save/retry and stale acknowledgments. The saturated-provider dispatcher test also discards a real cached draft while every network worker remains occupied. Native scenarios cover group collapse, menu survival during refresh, bin/keyboard review, attachments and light/dark/compact layouts. `desktop.start(discard_failure_once=true)` injects one fixture storage failure through test-support; it never changes production storage or contacts a server. Performance measurements remain deferred.

## Read after deliberate selection

`ui/read_tracking.rs` distinguishes deliberate inbox selection/arrow navigation/full-window opening from the programmatic first selection, hover and prefetch. Selecting an unread message arms a small metadata candidate; leaving for another message, folder, search, composer, tab or window marks it read through the existing optimistic flag path. Do not require loading a body or wait for a provider before the visual change. Explicit read/unread controls clear the candidate, so leaving cannot undo a user's mark-unread action. Refresh events must never finish a read by themselves.

Archive/move finishes an armed read first and waits for its flag acknowledgment before using the source UID. Keep the expected UI removal immediate. After success or failure of the read write, pass the confirmed flags into the queued move. Failed read saves restore their old indicator without selecting the old message/folder. A full provider queue must not close the window while discarding a newly requested read change. Native scenarios cover deliberate vs startup selection, slow success, explicit unread, failure after folder navigation and arrows in an Unread filter. Measurements remain deferred.

Archive/delete/move toast regressions: create feedback in the same update as the optimistic row change, even while waiting behind a read/flag save. Count archive and delete across accounts in the unified inbox; custom folders group by destination account/folder. Failures remove only their correlated count and retain the error. Successful completion must not recreate an expired/dismissed toast or replace a newer action. The current display lifetime is six seconds, refreshed by each action. Cross-account moves have typed completions and wait for source flags; keep that ordering when adding Undo and persistent action recovery.

A focused iced text input may leave an unhandled modified key uncaptured (for example Ctrl+D). While processing the native key event, inspect search/Find focus with a widget operation before dispatching mail-target shortcuts; do not infer editing focus from capture status or the harness focus observation alone. Test both mouse/shortcut search focus and remapped destructive keys, then verify the action still works outside search. Do not restore asynchronous key focus-check replies; they can revive cancelled intent or observe a later clicked field. Inline composition must extend this guard to its editable controls when implemented.


## Undo for optimistic mail moves

`ui/mail_actions/undo.rs` retains only the original metadata, query membership and an acknowledged `MoveReceipt`. Toast Undo carries a snapshot of counted tokens. Restore the source rows and feedback immediately; cancel a move still waiting behind flags, or wait for the accepted forward command before dispatching its inverse. Retry a full bounded provider queue on Tick without blocking. Window close counts queued/in-flight Undo. Keep navigation usable and preserve selection of unrelated mail; a restoring placeholder must never send flags, detail or conversation requests using an obsolete UID.

IMAP MOVE/APPEND receipts parse COPYUID/APPENDUID and the final tagged status explicitly. Missing mapping after tagged OK remains acknowledged success. Cache relocation changes identity inside a SQLite transaction, preserving original bytes, flags and indexes. Undo validates frozen incoming connection identity and, for real IMAP, verifies exact raw bytes at the destination before moving back. When a mapping is absent or UIDVALIDITY changed, search by size/Message-ID and accept only one exact SHA-256 match; ambiguous copies require choosing the copy in the destination folder. POP3 reversal stays local. Cross-account Undo reverses the already authorized transfer even if its preference is subsequently disabled and retains the transfer upload journal on retry.

Failed Undo removes only its optimistic restored row/count and keeps a persistent Retry Undo/Dismiss card. Correlate the error notice so a successful retry clears its own error without dismissing an unrelated one. Do not recreate dismissed/expired feedback on acknowledgment. The history is scoped to the running session; durable pending-intent recovery remains R50/R60 work.


## Native HTML reading

`email_content.rs` selects MIME alternatives before rendering and recognizes complete mislabeled/escaped XHTML documents. Explicit HTML attachments remain attachments; related Content-ID resources belong to their selected representation. `HtmlBody` holds the source, scoped inline bytes, a content signature and cache weight, prepared in backend work. Preserve raw original exports and the explicit Plain text option. Plain preview truncation and large-message streaming remain R23 work.

`html_render.rs` owns litehtml DOM/font/layout/image state on one worker. Bounded channels pass immutable viewport frames to iced. Coalesce geometry/hover work without losing selection Down/Up/Copy boundaries; ignore obsolete generations and retain the current document through metadata-only flag refresh. Never parse or render HTML in an iced handler. The native canvas participates in the outer reader scroll and must explicitly use a renderer clip layer even when the document is shorter than the viewport: the software image backend alone does not enforce the supplied image clip rectangle.

Owned visible-text geometry supports selection/copy without raw DOM pointers, excluding collapsed quotations. While the HTML body has native focus, arrows/Page Up/Page Down/Home/End scroll its outer reader and cannot navigate the inbox. Clicking outside returns keyboard focus to the surrounding UI; keep the native focus-transition regression. The Formatted/Plain text controls preserve the MIME alternative; HTML quoted-history controls respect the existing reply preference. Rebuild the renderer when image permission is revoked so previously loaded pixels cannot remain visible. HTML sources/inline bytes count toward the existing body-prefetch budget.

No email JavaScript, CSS imports, filesystem or automatic network loader is installed in the renderer. CID/data images decode off-thread to WebP; remote resources use the existing per-message/sender/domain/Contacts policy and public-address/redirect validation. Preserve natural dimensions when converting small images. External HTTP(S) links open through a background system-browser task; mailto opens a draft and cannot inject hidden headers or attachments.

`vendor/shep-html-pixbuf` is the MIT-licensed upstream 0.2.6 drawing adapter with corrected image sizing/position/repetition/device scale. The layout engine is version-pinned with a focused table-layout patch in `vendor/litehtml-sys`; its original BSD-3-Clause litehtml, Apache-2.0 Gumbo and MIT wrapper licenses stay in release archives. Keep its license/provenance in release archives. Worker pixel tests prove scaled image contents and repeated backgrounds; native screenshots prove clipping and controls remain visible. Static email HTML is supported; this is not a JavaScript browser or full support for every advanced browser CSS feature.


## Find within the open message

`Action::Find` defaults to Mod+F and uses the same primary/secondary remapping, conflict migration and disable rules as other shortcuts. The reader toolbar offers a Find icon. Its bar searches only the expanded message's displayed body and visible quoted sections; it does not change the inbox query. Enter/Shift+Enter and mouse arrows wrap through matches; Aa toggles case matching. Escape closes Find before the full-window reader. Preserve tooltip preferences and show only primary shortcut hints for the Find action.

`message_find::TextIndex` normalizes whitespace while mapping results back to original byte offsets. Queries are escaped literal text with Unicode simple case folding from `regex`; they are never evaluated as user regex syntax. HTML matching uses owned visible text geometry on the existing layout worker. Plain matching uses a separate cosmic-text font system and bundled Noto Sans, never iced's UI font lock. Bounded reader channels, coalescing, a short input debounce, revision checks and cancellation keep old results from replacing a new message/query. Plain search shaping and result geometry stay off the UI thread. Highlight rectangles are indexed by block/y so drawing visits the visible region; nearby text runs merge into one phrase highlight. Preserve original message text and native copy selection.

Find reveal scrolls the actual parent reader and, when needed, a wide HTML table horizontally. Width/font/quote changes rebuild the relevant geometry. Native Enter handling must carry the key event's Shift state and find revision through focus inspection: a later global modifier value can already reflect key release. Close/navigation clear pending focus observations; tests must wait for a newly opened field, not a stale focus label. The compact toolbar leaves room for Find and Export.

The saved five `test_find_*` native flows cover formatted/plain long mail, case matching, mouse/keyboard next/previous, delete isolation, new-message replacement, quoted history, dark/compact/full-window wide tables, remapping both slots and disabling Find. At the bottom of the standard shortcut list, Forward is near y=780, Find y=720, Inbox y=660 and Delete y=600. Keep the existing Inbox/Delete remapping regressions on their actual rows. Performance measurements remain deferred; these are functional checks.

Matching references: [RegexBuilder Unicode case folding](https://docs.rs/regex/latest/regex/struct.RegexBuilder.html#method.unicode), [literal escaping](https://docs.rs/regex/latest/regex/fn.escape.html).


## Forward drafts

`compose/forwarding.rs` prepares forwards from complete cached MIME on the persistence worker, independently of network-job capacity. Copy the selected MIME representation, full text, styles, original body attributes, scoped CID images and ordinary attachments. Do not download external images. Exclude Bcc and transport headers from the quoted header block. Start a new draft identity/thread with empty To/Cc/Bcc; retain the source account and add Fwd only when absent. Forward is the preview footer arrow, with default remappable F and the same input-focus guards as other mail actions.

`Draft.forward` preserves the original text/HTML. A note prepended to the original keeps its HTML alternative; editing the quoted original deliberately sends the edited plain text, with former inline images retained as ordinary attachments. Keep this behavior explicit in user docs. Never silently send stale HTML after the user edits the quote. `draft_inline` holds Content-ID metadata alongside independent attachment blobs and cascades on file deletion; ordinary text autosaves cannot rewrite file associations. Forward preparation commits text/files in one transaction and rejects a missing source, retired/duplicate draft ID or attachment-limit violation without saving a partial draft. Current sending/attachment ceilings remain R23 work.

Typed results match the request/source/reader generation. A late result after navigation or opening another composer stays in Drafts and must not replace the current editor. Repeated Forward while preparation is pending is coalesced. Show Preparing immediately, keep navigation available, wait before window close and clear only the matching failed-preparation notice after retry. `mail_actions="slow"` delays fixture preparation; `"fail"` rejects the first fixture forward and allows retry. These never touch a personal account. Rust tests cover complete source, MIME/header/attachment roundtrip, restart, rollback, native editor roundtrip, stale results and saturated-provider dispatch. Preserve saved native forwarding scenarios with WebP evidence.


## Printing from the reader

Print is the footer printer icon or remappable Mod+P, including both binding slots and disable behavior. It snapshots the expanded physical message and current Formatted/Plain choice. Preparation has its own bounded capacity-two command channel and worker, independent of provider jobs and foreground reads. MIME parsing and raster conversion run off-thread using complete cached raw mail, including quoted history and attachment names. Only CID resources and already cached images permitted for this message are embedded, as WebP; printing never fetches remote resources. The current R23 download/cache limits still apply.

`printing::Service` serves a self-contained document from memory on a random IPv4 loopback port and unguessable single-use path. Validate Host/method/path, bound headers/connections/timeouts, send no-store/no-referrer/CSP and stop after serving, cancellation or five-minute expiry. At most two unconsumed previews retain documents. The default browser owns printer/PDF selection; opening it is not evidence that the user printed. Retain the preview handle while its browser load may still be pending; dropping it stops an unconsumed server. Launcher and preparation failures remain visible and retry clears only the matching error. Printing does not mutate messages or block native navigation.

The trusted parent prints its sandboxed srcdoc child, so long messages paginate. The child has allow-same-origin/allow-modals, never allow-scripts, forms or popups; a stricter child CSP blocks scripts and all network resources. Remove active elements/attributes and unsafe links before serialization; the parent prevents message navigation. Escape srcdoc and header data separately and substitute template markers once, never recursively into user text. Header insertion must wait for the actual srcdoc document, not the initial about:blank document. Review actual PDFs, not just serialized markup.

The saved native Print flows start `desktop.start(print_browser="pdf" | "dialog" | "fail")`. An isolated Chrome/Chromium profile lives under the run artifacts and **must pass --ozone-platform=x11** so it stays on the harness-owned Xvfb display even on a Wayland host. The fixture-only launcher never falls back to the personal browser. PDF mode uses the actual browser print path with Save as PDF and kiosk printing, never a physical printer. Batch `print_output` asserts PDF text/pages and captures a first-page WebP; `browser_screenshot` captures the owned display, `cancel_print` uses native Escape, and `focus_app` raises/refocuses only the fixture window. Stop the owned browser process group before Xvfb. All scenarios remain in scripts/e2e.py. Linux Chromium evidence does not establish Firefox/Safari/macOS/Windows printing.

Adding Print moves bottom-scrolled shortcut rows: Print y=780, Forward y=720, Find y=660, Inbox y=600 and Delete y=540 at 1440×920 without a bottom notice. Verify the actual presentation before adjusting coordinate tests. Performance measurements remain deferred; controlled preparation delays are correctness checks.

References: [Window.print](https://developer.mozilla.org/en-US/docs/Web/API/Window/print), [iframe sandbox](https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/iframe), [srcdoc isolation](https://developer.mozilla.org/en-US/docs/Web/API/HTMLIFrameElement/srcdoc).


## HTML frame preparation and geometry

Discover remote image references while preparing `HtmlBody` on the backend,
including CSS backgrounds and base-relative URLs. The blocked-image control
must exist before the first frame; rendering must never insert that control
above an already displayed body. Discovery is metadata only: fetch only resources
actually requested by the renderer and permitted by the existing image policy.

Horizontal panning belongs inside the visible HTML canvas. Reserve its small
bottom band from the first layout, clip text selection/Find highlights above the
track, and update the thumb immediately while the worker prepares pixels. Reject
superseded pan frames. Horizontal arrows and Shift+wheel act only on the focused
body and must not capture editing keys from Find. Keep compact attachments in
two columns, with navigation sharing the action row so the body remains readable.
Retain at most one set of current-query Find results that arrives before its
matching layout frame. A rejected resize paint followed by a pan-only paint
must not lose those results; reject older query/document revisions as usual.

The fixture-only `html_delay_ms` MCP start option (0–2000) pauses the renderer
worker, never iced, and must be paired with `--demo`. Use it for readiness/layout
correctness checks, not performance claims. The native observations
`html_body_bounds` and `html_body_visible` are in the parent's content coordinates;
reset parent scroll before using them for native clicks. `html_pan_target` is
desired horizontal position. These are observations, never an action API. Preserve the
saved CSS-background delayed-render and compact attachment/body-space scenarios.

The first HTML Load waits for the native canvas viewport, including when the body
is below the visible portion of the reader. Keep Find and input behind that Load.
Reject obsolete generation/viewport/scroll frames before presenting them, while
retaining their current-document resource discovery. Draw old frames only at
compatible width/scale; never stretch a bitmap from different geometry. Loading
indicators belong inside the body allocation rather than a temporary extra row.

The interactive HTML worker retains its own font discovery across documents;
never share iced's font lock. A separate speculative worker has a replaceable
mailbox of at most two neighboring cached messages. Its first-frame cache holds
at most eight frames / 32 MiB (including retained image bytes), keyed by body
signature, message identity, geometry, font, quote policy and image permission.
Frames acknowledge their actual decoded image inputs; a download invalidates only
frames using that URL, never unrelated mail. Retain valid partial/failed-image
layouts too, but never label pixels with an image that has not been applied.
Visited frames seed their exact images on reopening even after shared cache
eviction. The shared WebP byte cache is bounded to 128 entries / 16 MiB. It performs no
network requests and cannot use the interactive renderer's capacity. Seed only
permitted cached WebP bytes, decoded lazily when that document references them.
Keep document font handles, glyphs and decoded resources isolated. Same-size
repaints clear/reuse the viewport allocation. Touch body-cache recency when a
message is opened. Completed remote WebP bytes must not be decoded/re-encoded
before the renderer decodes them.

Root document background colors are observed on the renderer worker. The
reader surround uses that color and readable native controls, retaining the same
widget tree while frames arrive. Ordinary conversation refreshes must not
reschedule its initial scroll position; explicit new-page navigation still may.

The saved native preparation flow checks cache use, rapid selection, End/Home,
pane drag and compact resize through actual input; observe html_view_current,
html_cache_ids, html_cache_hits and html_cache_bytes. These are correctness
observations, not latency measurements. Preserve existing selection, Find,
quote/image-policy and delayed-action scenarios and review their WebP captures.

## HTML image reflow and recovery

When an image changes layout above a scrolled reading position, retain a visible
text-node anchor on the renderer worker. Use its DOM traversal identity and text
fingerprint, including identities for zero-size text nodes; visible-run indices
alone are not stable identities. The new viewport pixels account for the anchor's
displacement. Image arrivals may decode into the existing document resources, but
coalesce additional layouts until the native scroller acknowledges the first
adjustment. This adds no deferred image queue and must not block Copy, navigation
or a replacement Load. Images below the reading position do not move it; a reader
at the start stays at the start.

`ui/html_reader/anchor.rs` applies an absolute native scroll only when the target,
view version, current offset and viewport geometry still match the snapshot.
Newer user navigation wins. An acknowledgement updates the renderer, never the
UI's newer observed viewport. Temporary image/selection positioning bridges the
adjustment; Find overlays wait for it to settle. Ignore older document/layout
acknowledgements. Preserve the pixel-equality, coalescing, below-viewport and
stale-navigation regressions when changing this protocol.

Recoverable render failures offer Retry formatted message and retain Plain text.
Retry creates a fresh generation for the same selected message, waits for real
canvas geometry and rejects old errors. A stopped worker keeps its explicit
reopen instruction instead of offering an ineffective Retry button.

The isolated harness accepts `image_delay_ms` (0–5000) and
`html_failure_once` (boolean), honored only by demo/test-support code. The HTML
fixture's Trash contains Delayed illustrated report, with two undimensioned
images above the text. `remote_image_pending` and `remote_image_cached` are
read-only observations. Save light/compact-dark/Find, navigate-before-arrival and
native Retry scenarios in the automated suite. These controlled waits establish
correctness, not performance measurements.

## Unread launcher badges

`desktop_badge` owns native badge publication separately from mail/provider work.
The Linux adapter maintains a session-bus connection and publishes the Unity
LauncherEntry Update/Query protocol for `application://so.shep.Shep.desktop`,
matching the installed desktop entry and iced application ID. A watch channel
retains only the newest count. Zero hides the badge; disconnection retries off
thread; a new `com.canonical.Unity` owner receives the current value. Keep IPC
bounded and never invoke a shell command from an iced handler. Windows/macOS
adapters are integrated in `1595fb3` (desktop R70); their actual Windows/macOS execution remains open, so show the preference only on implemented platforms.

The badge counts unread Inbox messages across all connected mail accounts,
independent of the open folder, filter, search or unified-inbox setting. Removing
an account excludes it immediately. Preferences → General → Mail & performance
has the persisted toggle, also searchable as badge/dock/taskbar. Publish only
changed values; badges do not depend on whether a body has loaded.

`MailQuery.observe` requests small pending-message membership records in the
same SQLite read transaction as page rows and global counts. Never retrieve raw
mail for this. `ui/mail_actions/counts.rs` projects intent independently of visible
rows, distinguishes a missing observed identity from an unobserved one, and
reconciles acknowledgements without double-counting a completed cache write.
Invalidate older page/prefetch generations when an intent starts. Preserve tests
for filtered pages, read rollback, Inbox moves, cross-account rekeying and Undo.
Ambiguous provider receipts and durable pending-action recovery remain R50/R60.

The MCP `desktop.start(desktop_badges=true)` option starts an owned private
`dbus-daemon` and a `busctl` observer before the fixture app. Its explicit bus
configuration has no service directories or activatable portal/keyring services,
and it owns a private runtime directory. It never uses the personal session bus. `desktop_badge` in harness state comes from actual protocol
messages, with count/visible/URI/sender and recent count history; it is not an app
self-report or action API. Stop only owned processes. Backend tests also use a
private bus for Query, zero visibility, owner changes, bus loss and reconnect.
Native tests operate the real preference, mail controls and Undo. Protocol/native
input evidence does not establish an actual GNOME/KDE/Windows/macOS dock render.

Protocol references: [Unity Launcher API](https://wiki.ubuntu.com/Unity/LauncherAPI)
and [Dash to Dock's receiver](https://github.com/micheleg/dash-to-dock/blob/master/launcherAPI.js).

Native selection tests wait for `mail_selection.mode` and the test-only
`mail_selection.drawn` observation before clicking new checkboxes. The row
wrapper records its actual draw epoch; merely acknowledging Select in the
controller is insufficient. Keep native mouse input and screenshot review.
This observation is not evidence of display scanout or a performance result.

The root `ContextArea` preserves motion-event cursor positions before dispatch
into scrollable coordinates. iced 0.14 supplies the final pointer position for an
input batch; using that for every queued click can toggle the same checkbox
twice. Preserve overlay exclusion and retain the captured position through redraws.
`ui/pointer.rs` shares tracking with captured dropdown/nested-overlay events so
closing a popup cannot leave a stale base position. Clear on pointer leave,
window blur or interface-scale changes; never clear just because a frame drew.
The native bulk flows intentionally click different rows consecutively, without
inserting sleeps between clicks; the widget regression submits both clicks in
one event batch, including redraws between motion and press. Do not mask this
regression by slowing down native input.

## Mobile and browser printing

`shared/mail-content::printing` prepares client documents from complete cached MIME using the shared resource sanitizer. The fixed runtime reconstructs bounded inline WebP blobs and reports readiness only after resources/fonts decode. The browser gateway must include both the reader and print runtime CSP hashes. Its print iframe allows scripts/modals with an opaque origin; only the owned hash may execute. Android's owned WebView denies network/file/content loads and survives until `PrintDocumentAdapter.onFinish`; UIKit uses ephemeral WKWebView and native print controls. Keep platform preparation independent of provider/reader work, capture the original source/mode, and never turn dialog launch/cancellation into a print receipt. Preserve `printing.spec.ts`, the native cached-alias/host contracts and `printing_android_test.dart` with `android_print_fixture.py`; review actual paginated PDFs and native captures. Apple/other-engine execution, remote-image policies, native shortcut and large-mail/lifecycle parity remain explicit TODO gaps. The full Android wrapper includes the saved Print scenario. Browser runners additionally require Xvfb and Poppler.


Flutter reader actions stay outside the scrolling body in a responsive safe-area footer. Use standard footer visual density so Flutter desktop targets remain at least 44 pixels. Keep the footer and touch targets stable when cached detail, formatted preparation or attachments arrive; pending Forward/Print keep their visible labels and announce preparation through semantics. Counted feedback and Undo sit above the actions. Preserve theme shapes, scaled-text wrapping and the mounted formatted WebView. `flutter/test/support/reader_actions_scenario.dart` is shared by host and Android native controls: hold a real touch through delayed preparation, reject a second pending Print, recover Forward errors, scroll and Reply. Keep the Android printer/PDF and formatted Playwright/Appium regressions; fixture printer observations are not real printing receipts or Apple verification.

Desktop relevance queries in `store/mail_query.rs` materialize literal FTS rowids/ranks once, then join that SQLite relation. Do not restore a virtual-table LEFT JOIN that repeats FTS filtering/ranking for every candidate. Preserve exact short-body priority, literal-versus-fuzzy scoring, combined folder parameter order and the shared list/selection plan. Keep the query-plan guard, `tests/search.rs`, `tests/selections.rs`, bulk projection tests and native search/sort/selection flows. Run the unchanged 100,000-message responsiveness benchmark after planner or SQLite changes; an interrupted run is not timing evidence.

`mail_inbox_badge_counts(folder,unread,account)` covers the global unread-account count requested with each mail page. Preserve its existing-cache creation and `tests/unread_counts.rs` plan/reopen regression; a narrower index requires a message-row lookup per unread item plus sorting. Pending bulk projections still use the visible-mail source. Keep value/projection and native badge tests alongside the unchanged backend budget.

## Desktop selection ownership, History and process recovery

Active selections retain their normalized query in a temporary SQLite row.
Explicit clicks on arrivals and Shift ranges rebase query order while retaining
previously selected identities, including unavailable ones. New rows stay
unselected until an explicit gesture chooses them. Frozen reviews never rebase.
Validate range endpoints against the current query before appending retained
choices outside it. Keep all membership/ranking work in SQL; send at most one
metadata page to iced. A rejected gesture preserves confirmed membership, and
a failed passive observation retries without clearing it.

Individual Move/Transfer/Undo/read/flag paths check group ownership inside their
account mutation lock. Group paths validate the running item phase and atomically
claim a resolved Undo identity without replacing another item’s ownership.
Keep old receipt ownership until completion; a finished or cancelled phase
cannot acquire new claims. Row/context controls reflect pending ownership, while
other messages remain usable. This does not establish cross-process serialization
of individual provider writes; that remains in the provider/platform audit.

Disabled nested flag buttons must still absorb their mouse press; wrap them in
`opaque` so the inbox row does not open or start read-on-leave tracking instead.
The saved pending-group native flow verifies the reader stays on its original
message when that disabled control is clicked.


The MCP harness supports explicitly persistent fixture workspaces via
`desktop.start(persistent=true)`. `tests/support/workspace.rs` marks new fixture
SQLite files with an application ID and rejects unmarked existing databases
before migrations. Demo seeding runs once, preserving native test changes across
processes. Production builds exclude this opener and all fixture seeds.
`desktop.close` sends WM_DELETE_WINDOW on the owned Xvfb display (including
hosts whose xdotool predates windowquit), waits for normal exit, and refuses a
replacement after timeout. `desktop.restart` retains the fixture cache/display;
its explicit `crash=true` option kills only the owned app. Restart is batchable.
Keep the graceful-close/crash native tests and harness ownership/timeout tests.
See [xdotool's close/quit distinction](https://github.com/jordansissel/xdotool/blob/main/xdotool.pod).

Continue must clear a group's persisted pause before waking the coalesced worker.
It must not replay completed receipts or bypass the worker's execution lease.
History fixture seeds are setup data, never an action interface; pagination,
Continue, Undo/retry and uncertainty acceptance use real native input.

History keeps its visible 20-job page separate from active progress tracking.
Worker updates replace matching displayed entries in place; they must not jump
an older page or forget running work. Pagination resets the modal scroll. Close
asks an initialized engine to stop even when the visible History page has no
running jobs, while flushing pending pane sizes before waiting.

HTML and neighbor-preparation subscriptions own cancellation guards that wake
blocking renderer receives when iced tears down, including while UI state still
retains its senders. Do not rely on those senders dropping before Tokio shutdown.
Preserve renderer cancellation unit tests and the actual formatted-reader native
close/restart regression; a window disappearing does not prove process exit.

Unconfirmed group results require explicit review. Acceptance retires only those
steps without replaying provider work or inventing receipts, replaces the stale
recovery instruction with an accepted-state note, and preserves Undo for the
other acknowledged messages. Test mouse acceptance, Y/Enter and N/Escape, and
reopening History without a stale confirmation.

## Dragging messages into folders

`ui/drag_mail.rs` validates cached account/folder rules; its widget module owns the
pointer gesture. Reuse `context_menu::ContextArea` and the root pointer tracker so
batched native motions, redraws and scroll coordinates keep their actual targets.
Move only after six logical pixels of motion. Flag/checkbox presses cannot start
a drag. Escape, focus/cursor loss and right-click cancel; swallow the subsequent
release so it cannot select a row, open a reader or run a sidebar action. Ordinary
clicks and release-based double-click reading retain their existing behavior.

Payloads contain one message's metadata or the acknowledged selection snapshot,
never all selected messages or MIME. Passive snapshot observations do not disable
dragging; pending membership edits require their acknowledgment. At drop, recheck
the source identity/current selection revision and destination. Inbox, Archive and Trash use each source account; the combined Sent/Flagged
views are not destinations. Explicit destinations honor the cross-account
preference and require two IMAP accounts. Provider operations still validate
actual server capabilities. Only Inbox is a case-insensitive folder alias.
Single drops use optimistic mail actions and Undo. Groups freeze through the
existing review/journal path, including selections spanning other pages.

Hover opens collapsed accounts/Inbox after 600 ms; it never toggles a group shut.
Sidebar scrolling remains available while holding. Pointer motion requests local
redraws; target transitions and completed gestures publish bounded app messages.
Draw the floating label in its own renderer layer above pane clips. Preserve the
shadow damage/clip regression; full repaint is not an acceptable substitute.

The MCP batch actions `mouse_down` / `mouse_up` hold/release the left button on the
owned fixture display, allowing hover, wheel input, assertions, short waits and
screenshots during a drag. Duplicate presses/releases fail; cleanup releases a
held button before stopping the owned display. `pop3_account=true` changes only
the fictional personal account for local/cross-account destination checks.
`mail_drag` is observation-only. Preserve all ten `test_drag_*` native scenarios,
controller/widget/selection tests and harness ownership checks. The scrolled
Unicode-folder scenario also checks saved WebP pixels for a stale shadow trail.

## Mailbox trees and folder navigation

Preserve IMAP LIST names, per-name hierarchy delimiters (including NIL),
selectability and encoding in `folder_catalogs`. Never infer a slash hierarchy
from a legacy name. A nonselectable or NonExistent entry may be a visible
container, but must never enter SELECT, Move or drop destinations; cached mail
cannot make its canonical/trailing-delimiter name selectable again. Keep cached
originals accessible to recovery instead of deleting them during LIST refresh.

`folders::Tree` builds missing ancestors and decoded display paths on the storage
worker; Workspace shares trees through Arc. Only metadata crosses UI channels.
Preserve exact wire names in queries, operations and receipts. Decode modified
UTF-7 only for an IMAP session using that encoding, including ampersands and
encoded delimiter characters; UTF-8 names with the same spelling remain literal.
Display lookup is account-specific. Move search ranks readable labels but returns
wire names. Review, history and toast labels must use the same display mapping.

Selectable parent labels open mail; their chevrons only expand. Nonselectable
parents expand without selecting. Groups default collapsed and persist per-account
expansion; closing a parent retains its descendants' remembered state. Common
unified-folder shortcuts must not hide real children under the account tree.
Left/Right navigates hierarchy; Up/Down moves focus without toggling containers.
Keyboard targets scroll into view using native widget bounds, not guessed row
heights. Retry only missing new layout rows, and reject superseded targets.
Mouse wheel position remains alone unless keyboard navigation requests a reveal.
Drag hover opens a closed folder group after the existing dwell without changing
the reading selection or turning a container into a drop target.

The MCP `nested_folders=true` fixture includes slash/dot/NIL hierarchies,
nonselectable/trailing-delimiter containers, selectable parents and modified-UTF-7
Japanese mailboxes. Preserve its five saved native scenarios: mouse/restart,
keyboard/delimiters, hover/drop/Undo, Unicode Move/review/Ctrl-selection, and
compact dark/120% keyboard reveal with saved window size. Backend tests use actual
loopback IMAP protocol and reopened isolated SQLite files. This is not live
personal-server evidence. Folder delete/move controls and their remaining lifecycle work are tracked under R30.

MCP `paste` writes only the owned Xvfb clipboard and sends native Ctrl+V after
verifying exact bytes. It shares the existing owned clipboard cleanup, preserves
Unicode/whitespace and refuses operation without a live fixture display. A failed
clipboard setup must never paste stale content. Use it for Unicode tests; the
initial Japanese xdotool typing scenario intermittently delivered no text even
after native input focus was acknowledged. Keep ASCII typing and real keyboard
shortcut coverage; paste is not direct application-state injection.

Harness startup waits for the metadata page, not the message body. Use
`selected_id` and its `mail_rows` entry when remembering an action's source.
The reader's `selected` subject may still be null; waiting for it would hide the
requirement that metadata actions remain available while bodies load.

## Folder mutation work in progress

`ui/folder_controls` supplies native right-click/Shift+F10 folder menus, fuzzy
parent search, explicit move/delete review and bounded recovery history.
`engine/folders` routes options/reviews/history through local read workers and
staging/retry/stop through the selection FIFO. The coalesced bulk wake worker
executes durable folder jobs serially with the existing provider/account locks
and close barrier. R30 remains open for its wider lifecycle and scope audit.

Project reviewed folder changes immediately; projected moved paths browse the
original cache until commit. Rollback must retain later navigation, even when a
user leaves and returns to the same Inbox. Job revisions reject stale history
and progress; history-fetch state must not unlock a separate recovery action.
Unconfirmed results cannot retry or stop without the appropriate explicit review.
Accepting uncertainty stops remaining work, retains cached originals and says the
result is unconfirmed; it is never a successful server acknowledgment. Keep
source/destination labels decoded on the storage worker after the source leaves
LIST. Never perform MIME or mailbox encoding work in iced handlers.
An older Workspace snapshot must retain the newer folder trees together with
account-folder names at its connection revision. A closing folder job waiting
for provider capacity or its account lock stays queued and releases that wait;
only an already-started command needs to persist its receipt before exit.

IMAP RENAME moves descendants; DELETE does not. Delete reviewed descendants
deepest first and protect Inbox. Preserve NoInferiors/NonExistent metadata and
exact wire names. A complete, tagged-OK LIST is required for preflight: async-imap's
streamed name helper can hide a final NO. Recheck the remaining subtree before
each destructive step; a new descendant or recreated completed folder requires
another review. An absent nonselectable container needs only cache cleanup.

Persist Running before a provider command, then Acknowledged before cache work.
Only tagged rejection is retryable as a rejected command; lost acknowledgments
remain Uncertain and require explicit review. A later LIST failure cannot repeat
an acknowledged RENAME. The runner observes database commits to completion and
can stop between durable receipts. Local POP3 work can resume after interruption
because its cache update and Done state commit atomically. Workspace preparation
initializes local POP3 catalogs and discovers later imported local folders. Keep
literal slash names flat and never recreate a deleted empty folder. Do not change
an IMAP namespace or an unfinished folder review during this preparation.

The owned per-job filesystem lease excludes another executor/process. A shared
in-process claim also protects memory stores without creating disk lock files;
release it on errors, cancellation and drop. This does not establish cross-process serialization of every ordinary provider write;
that remains R01/R06. Pending folder changes gate account writes/sync and group
staging. Account removal reviews include the journal and delete its records after
confirmation. Cache moves copy MIME inside SQLite, rekey IMAP metadata, preserve
local POP3 identities, conversation/restored markers, Sent mappings and relevant
Undo receipts. Deleting a referenced folder retires only affected history items.
Retain the protocol and `tests/folder_actions.rs` recovery/collision regressions.

HTML opening measurements use the owned X11 pixel sampler in
`scripts/native_pixels.py` and the saved MCP equivalent `scripts/html_latency.py`.
See the repository E2E skill for timing boundaries and fixture limitations. Keep
20 samples per case and the 100 ms cold-document / 50 ms cached-document p95
limits in `performance-budgets.json`. `--html-only` checks only this explicitly
authorized work while other final performance measurements remain deferred.
Bounded document-local text/glyph caches and visited initial-frame reuse must
preserve font, content, viewport, image-policy and generation identity. The
software renderer coalesces overlapping damage and paints only visible solid
panel interiors; keep full/partial pixel and fractional-scale regressions.


Deeply nested HTML tables must not redo the same subtree layout exponentially.
`vendor/litehtml-sys` retains one table layout per complete containing-block
constraint within the current normal-flow document render. Compare typed width,
height, min/max, context index and sizing mode; a parent-adjusted box width is
not proof of an identical layout. Caption displacement applies to the cells exactly once; do not accumulate a
second offset on their row parents. Disable reuse in positioned layout and never
reuse across render calls, resize or image updates. Preserve paired uncached/
cached pixel, selection, span/caption/float/position and reflow tests, plus the
exact inline-offset and caption-height expectations shared by both modes. The
`shep-test-support` dependency feature exposes only thread-scoped test controls;
normal and native test-support application builds do not enable it. Never edit
the Cargo registry source. The cold/visited deep-table MCP pixel gates use a
fictional 16-level template with 1,182 utility CSS rules; simplified letters alone
failed to reproduce the user's 1–2 second delay. Read-only personal diagnostics
remain explicitly ignored and cannot add mail content to public fixtures or logs.

While selection mode is active, row clicks toggle that one message and preserve
all other choices, including other pages. Shift ranges add to that selection.
Only Clear/Done/Escape or a scope change clears the group deliberately. Native
checkbox/row/modifier inputs must not count as reading. Preserve the additive
row/range/cross-page bulk-review test and the separate double-click reader flow.

Editor viewport bounds do not include partial glyph extents. The software
renderer must always intersect editor text with its local viewport and damage
mask, even if the editor box lies wholly inside that damaged area. Preserve the
partially visible final-line renderer test and long-reply native typing captures.

Inline fragment elements retain only their relative offset; `line_box.cpp` resets it before each application. This prevents wrapped fragments and repeated table measurements from accumulating a superscript/span offset. Keep the exact 5px/2px selection-geometry and repeated-layout regressions.


## New-mail notifications

`Preferences.notifications` independently controls popups, sound and sender/subject details; all default on. Native delivery runs on a separate coalescing watch worker, never on iced or the sync worker. A fixed 150 ms burst window groups arrivals; one delivery runs at a time, with bounded observation output. Turning both outputs off consumes arrivals without replaying them when re-enabled. Test requests must finish even if muted before delivery. Desktop rejection stays visible with a recovery instruction; an acknowledgment is not proof that the OS displayed pixels or played sound under Do Not Disturb.

Arrival identity and initial-import readiness are persisted in `store/notifications.rs`. Initial Inbox import and IMAP UIDVALIDITY changes stay quiet until that Inbox completes; later unread Inbox arrivals notify at most once per account/content identity. Imports/restores/moved copies remember identity without alerting. A crash between cache commit and desktop delivery can lose that alert, but restarting must not replay old alerts. Read toggles and unread badge changes are not arrival sources. Remove the notification ledger when removing an account.

Sync SEARCH/FETCH helpers in `providers/mail/sync_queries.rs` require matching tagged OK, including after partial data. The upstream async-imap collection helpers discard completion status; do not restore those helpers in this path. A failed command must not produce reconciliation or mark a baseline complete. Preserve loopback failure, partial-data, disconnect and logout cases. Other collection helpers remain part of the provider audit.

Linux uses `org.freedesktop.Notifications` with Shep's desktop identity, escaped body markup and explicit sound/suppress-sound hints. Sound-only mode uses `canberra-gtk-play`; Windows uses a per-user Shep AUMID and WinRT toast, with the system mail sound; macOS uses the Shep bundle identity, initialized once, and native notification delivery. Actual Windows/macOS delivery and bundle/install integration remain open platform work. Never silently borrow another application's identity.

The native MCP fixture bypasses all real popup/audio delivery. `desktop.start` accepts `notification_delivery: "slow" | "fail-once"` for isolated delayed/error/retry scenarios. Save every interaction in `scripts/e2e.py`; the notification flows cover defaults, independent outputs/privacy, persistence, arrival/restart deduplication, compact dark layout and navigation during a delayed failure. Real Linux protocol tests start their own private bus and never touch the user's notification service. Keep logs/screenshots in ignored artifacts.

## Mail move recovery

The a81d767 recovery checkpoint uses `mail_actions/journal.rs`, `runner.rs` and
`store/move_journal.rs`. IMAP preflight finishes before durable preparation;
Started/Copied/Committed/Located/Kept records retain the source MIME and actual
acknowledgments. Do not repeat an unconfirmed MOVE/APPEND. In particular, tagged
MOVE NO may have partial effects (RFC 6851 §3.3); only atomic APPEND rejection is
classified as not applied. Keep matching-tag and disconnect protocol tests.

Provider commands have individual timeouts. Never cancel observing a SQLite
receipt commit, or let waiting for LOGOUT/closed UI output negate a confirmed
write. Cross-account recovery verifies the destination's exact raw bytes and
canonical identity before source cleanup, preserving any original APPENDUID.
The legacy transfer tuple migrates atomically; an uploading tuple remains
unconfirmed. Account identity checks use the original incoming connections.

Protected cached originals remain readable through restart and reconciliation.
Destination queries carry bounded recovery metadata and clear provider UIDs;
selection capture and older selections must exclude these protected identities
from available provider targets. Detail reads can follow a completed cache alias
when rekeying overtakes a pending read. Keep the late-read error-toast regression.
The runner requires the existing account locks, sorted for two-account work;
this does not establish independent-process coordination of all mail operations.

Automatic recovery considers at most three committed records per pass and
rate-limits attempts. Manual recovery/review and confirmed Keep local copy controls
now have storage/runner/controller and native tests. Kept copies receive local
identities, never an obsolete provider UID; retiring their old Undo avoids a
false server reversal. Active recovery must save its receipt before app close,
and a failure cancels close. This checkpoint is installed/pushed with all 167
native functional scenarios passing. Complete adapter wire/journal, broader
Undo/history/alias integration and live-account verification remain in TODO;
do not close R73 based on fixture happy-path coverage.
Native `move_recovery=true` uses only a protected fictional cache and a fixture
Refresh acknowledgment; see the repository MCP skill and saved automated flow.


## Search across folders

Interactive message search sets `MailQuery.search_all_folders` while retaining
its browsing folder and account selection. `MailQuery::search_scope` is shared
by the SQLite query plan and optimistic UI membership; selection captures use
that same plan. Search spans folders in the selected accounts, preserves explicit
read/flag/attachment filters, and keeps an empty account selection empty. Clearing
search returns to the browsing scope. Do not restore the old Inbox-only search.
Folder labels in result rows must remain readable at compact sizes. Preserve the
storage scope/ranking/paging/selection tests and all three `test_search_*` native
cross-folder scenarios. Search and move-dialog folder matching are distinct.


## Default reading layout

MIME preparation computes `HtmlBody.reading_column` off the UI thread. Plain
letters and HTML with typography/color styling get a centered column with
comfortable padding. HTML tables, explicit dimensions/positioning and authored
layout CSS retain sender geometry; do not apply the simple-column CSS to them.
Font-size changes scale the column. Keep rendered Find/selection geometry and
native full/compact pixel checks alongside any default CSS changes.

Expanded conversation cards share the active document's opaque background and
choose light/dark control colors for that surface. Their themer/container tree
stays identical while rendering discovers a background, so theme updates cannot
reset the scroller or native text selection. Preserve contrasting-message,
cached-switching and refresh/scroll tests, as well as the standalone reader tests.


## Manual refresh animation

Refresh defaults to Mod+R with F5 as its secondary binding. The v2 keymap migration
adds F5 only for an absent secondary slot with a nonempty primary and no conflict.
An explicit clear stays clear. Record an empty migration decision when F5 is
already owned or Sync is disabled, so freeing F5 later does not silently bind it.

`ui/refresh.rs` owns manual animation phase. Accepted input starts feedback before
the worker acknowledgment; repeated/coalesced requests retain phase. Only the
scheduler's manual Busy(false) ends it, including failures. Background Busy and
MailSyncFinished cannot cancel a queued manual refresh. Run the 16 ms animation
subscription only while the mail header is visible. Frame updates change the SVG
only, bypassing mail scheduling/body preparation and interaction timing samples.
Use floating rotation to preserve the click target and layout.

The patched SVG pipeline caches an unrotated raster by physical size, applies the
complete translation/rotation transform when painting, and honors its local
viewport plus damage/layer clips. Matrix diagonals are not rotation-independent
scale values. Preserve the direct center/fractional-scale/partial-paint tests and
native light/dark/compact/120% refresh captures. Native tests compare actual icon
pixels, remap/clear/restart F5, queue manual work during background checks, and
navigate through failure/retry. These are functional tests, not latency evidence.


## Native keyboard ordering

The root `ContextArea` publishes keyboard presses after its native child handles
that event, into the same message stream as mouse controls. Do not route presses
through the asynchronous event subscription: an earlier Escape can otherwise
arrive after a later Review click and close its new dialog. Nested areas do not
publish duplicate keys. Captured popup events belong to their native overlay.

`ui/native_input.rs::Focus` observes actual search/Find focus in that widget event;
its immutable snapshot protects uncaptured modified chords such as Ctrl+D even
if a later mouse event changes focus. The controller retains pane/action scope
and ordinary captured-key handling. Find Enter uses this focus and the key's own
modifiers directly. Remove rather than revive old KeyFocusChecked/GuardedKey
round trips. This does not replace the layout-aware focus tasks that open fields.

The MCP batch action `key_sequence` accepts 1–32 separate key chords, each at most
80 characters without whitespace, and sends them with the existing native 1 ms
xdotool delay. It changes no observation state. Keep its ownership/validation
Python test and the three `test_native_*` ordering/isolation scenarios. The rapid
Move/Escape reproduction fails on the previous installed executable. These flows
intentionally omit waits between earlier keys and already-visible later controls;
independent scenarios may still await focus/layout to isolate their own behavior.


## Inline composer ownership

`ui/composing::Session` owns recipient/account/subject metadata, the native editor,
its pending save revision and UI key. `ui/composing/sessions.rs` parks sessions by
draft ID when navigating. Generic dialog fields must never hold composer text.
Keep one save in flight per session, coalesce newer edits and use the bounded
persistence channel. A manual Save applies to the current session even when its
previous autosave is pending. Navigation must never wait for its acknowledgment.

`Draft.reply_context` keeps original quoted text separate from the editable reply
and persists its association. MIME submission uses `delivery_body()` and the
Include original choice. Legacy drafts still deserialize without this context.
Do not replace forward HTML handling with the reply quoting path.

Window close must observe every dirty/pending session, then save newer revisions.
A failed save cancels close and retains text. Background reader results cannot
restore a composer while close or list selection is pending. Account-removal
review waits for that account's owned draft saves/file imports; successful removal
retires only its sessions. Discard must wait for an attachment import to finish.
Late file/send/save results must preserve unrelated editors and newer revisions.

The draft-keyed reader scroller separates different editors and keeps the same
editor tree through asynchronous HTML rendering. Find and HTML reflow target
`compose-reader` while replying. Native focus guards protect To/Cc/Bcc/Subject
and the editor from mail shortcuts, preserve text Ctrl+A and ordinary Tab, and
clear hidden focus when collapsing. Preserve the saved inline switching/restart,
selection, Find, send-preparation and compact typing scenarios, alongside all
existing forwarding, recipients, file picker, discard and Outbox flows.


## Complete database export

`transfer.rs` copies SQLite pages from a separate read-only connection with a
pinned read transaction. Never copy the live cache with ordinary filesystem reads
or hold the mail-cache worker for the full transfer. Keep the private temporary
file in the selected destination directory, bounded page steps/progress and atomic
publication. Cancellation before commit must preserve the previous file; a
post-commit warning must still report a saved copy. Protect active cache, WAL/SHM,
backup-journal and operation-lock paths, including aliases. On Unix, compare
metadata/inodes without opening and closing an ordinary descriptor to a live
SQLite file, which can release process advisory locks.

`engine/database_transfers.rs` owns one active job through a capacity-one command
channel, independent of provider capacity and mail reads/saves. Progress uses a
latest-value watch channel; cancellation and completion use one-shot channels.
The UI waits for its exact preferences acknowledgment and all current/parked
draft saves/file imports before starting. Keep stale-result rejection, failed-save
recovery, cancellation under backpressure and close waiting for cleanup.

Preserve `transfer::tests`, UI ordering tests, the dispatcher saturation test and
all three `test_database_export_*` native flows. The native held-copy fixture
requires both test-support demo mode and `--hold-database-export`; it waits for
ordinary Cancel/window-close input, never a state-file mutation. Export keeps all
database-backed records, including pending-operation history, but excludes OS
credentials. The import path below consumes this format; encrypted backup Restore
is a separate operation.

## Database import and local profiles

`transfer/import.rs` stages a private copy before review. Derive the accepted
schema from an independent Store, validate supported v2/v3 schema, integrity,
foreign keys and credential-owning identifiers, and reject foreign/newer schemas
or triggers. SQLite validation/copying runs off the UI/cache worker with bounded
progress and cancellation. Never add a raw-mail payload ceiling to this format.
Consume the exact reviewed copy on confirmation; never reopen its source path.

`transfer/import/fences.rs` archives changed metadata in `imported_operations`
and prevents imported pending sends, Sent uploads, bulk/folder changes and
credential cleanup from replaying. Keep original MIME and acknowledged recovery
identities. Pending work needs native review. Preserve quiet notification setup,
device-specific window/backup paths and disconnected Google/automatic-backup
state. An import is not a server acknowledgment or a successful credential import.

`transfer/import/install.rs` closes SQLite before no-overwrite file publication.
Publication is the commit boundary: later cancellation or catalog/flush errors
must preserve the copy and report saved-with-warning. Recovery adopts its marker
and same file, rather than creating a duplicate. Never recursively delete a
profile folder on cleanup. Malformed unselected profiles must not hide valid ones.

`profiles.sqlite` uses its own bounded 32-command worker and revision-checked
rename/selection. Keep lists at 50 rows. The running `profiles::Session` retains
its Store/credential scope; selection applies on next launch after normal close
saves. Imported paths/namespaces come from a fresh device-owned UUID, never data
inside the import or a shared Google profile ID. Preserve legacy paths/keys.
Export must protect the registry and every profile's cache/journal/operation files.

`credentials.rs` owns the engine's OS-keychain operations on one bounded
32-command FIFO thread. Account, SMTP, calendar, Google, backup and removal
adapters share it. Accepted writes finish even if their observer disappears;
RestoreMissing checks/writes in that FIFO. Imported profiles never fall back to
legacy/other-profile credentials. Validate portable account/CalDAV identifiers
against reserved/SMTP keys and case-insensitive collisions before accepting an
import or encrypted backup. Preserve discovered `caldav:<64 hex digits>` IDs.

Keep import validation/cancel/version tests, fences/rollback tests, installation
commit/recovery tests, catalog pagination/CAS/isolation tests, credential FIFO
tests, and the saturated-provider dispatcher test. Preserve all five
`test_database_import_*` native scenarios, all three export scenarios and affected
backup/Google/draft/restart flows. See the repository E2E skill for owned fixtures.
Live diagnostics remain explicitly ignored; `SHEP_LIVE_PROFILE_ID` optionally
selects a canonical local profile UUID alongside the required authorized
`SHEP_LIVE_ACCOUNT_ID`. Without it, diagnostics retain legacy-profile semantics.

OAuth cross-client profile implementation remains the top TODO priority. Use the
[Flutter handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md)
and shared format; do not present local database transfer as continuous sync.

## Continuous profile transport

`profile_sync` uses the pinned shared `shep-profile-core` codec; keep the exact
operation bytes and optional fields. `providers/drive_http.rs` supplies bounded
HTTP decoding to profiles and backups. Profile files use their own stable
category, names and custom properties in appDataFolder; never select them with
backup retention or serialize local Preferences/credentials wholesale. The wire
contract and live limitations are in `docs/agents/profile-drive.md`.

The profile transport journal owns a separate 32-command SQLite worker. Reserve
the remote ID and commit exact bytes before obtaining a `DurableUpload`; a retry
cannot replace its reservation. Verify the actual server checksum, or fetch the
content when omitted. A lost/conflicting reply and a failed local acknowledgment
must retain the same pending operation. Never turn file-name equality into proof
of an immutable upload. Profile transport has no overwrite/delete operation.

Persist discovery tokens/record IDs and revisions. Empty pages with another token
remain incomplete; reject loops, duplicate identities and stale or foreign pages
atomically. Only a completed scan exposes metadata pages for application, at most
50 at a time; no whole-history collection/page ceiling. Scan replacement must
preserve uploads and merge history. Keep the protocol/journal/scan regressions
and existing Drive backup tests. Run JSON/hash/SQL/network work only in backend
workers. Ongoing polling/application now has a bounded coordinator; account
linking, conflict/removal reviews and incremental pulls remain TODO.

The pre-commit hook, full check script and disabled quality workflow also run
`python3 scripts/test_profile_core.py`. Since the merge, `shep-profile-core` is a
path dependency on this repository's own `shared/profile-core` (a superset of
main's former `e3e69a4` Git pin and the client crate); keep fixture provenance
explicit when the codec/history contract changes, and never use a sibling
worktree path as a shipped dependency. Keep production Drive/history paths protected in database export/import guards alongside the other active provider journals.

`profile_sync::replica::Replica` privately owns the shared history worker. Complete
verified pulls mint revision/device/scan proofs; publishing compares core, Drive
journal and discovered identities before reserving. Preserve core-reserve →
transport-prepare → verified-upload → transport-ack → core-confirm ordering and
all restart tests. A lost observer must not choose another operation/file ID.
Keep the owner alive through upload acknowledgment; check lifecycle/category
changes between requests. Empty discovery is not permission to recreate an
existing profile. Account/profile tombstones and explicit conflict resolutions
come from the shared merge worker, never timestamp-based local replacement.

The shared Drive metadata fixture now fixes appProperties/category/file naming
across implementations; preserve exact bytes with the repository Git attributes.
The earlier bb87ac2 desktop prototype had a different unconnected wire convention.
Real same-project cross-client visibility remains unverified. The kernel still
needs account linking/reviews, remaining settings and incremental pulls. Native
continuous-update fixtures do not prove actual cross-client Google delivery.

`scripts/test_profile_core.py` checks through Cargo metadata that
`shep-profile-core` resolves to the workspace member `shared/profile-core` (a Git
pin or a sibling checkout path is rejected) and runs `cargo test -p
shep-profile-core --locked --all-features` against the root `Cargo.lock`; the
former copied-crate runner and `tests/support/profile-core.Cargo.lock` are gone.
Python 3.11+ is required. Keep the runner's source-check tests and all shared
protocol/history tests in addition to the desktop bridge tests.

Enrollment and seeds use device-local KV records on the cache owning worker.
Keep revision checks, independent offline option saves and same-transaction
Google-disconnect pausing. Database import archives/removes both records.
`profile_sync::setup` creates only from a current complete discovery review,
persists all seed UUIDs/account mappings, and checkpoints the exact expected
history revision before each edit. Retry that original request, never resnapshot
its values or assign another UUID. Keep an admitted upload owned through both
journal receipts even after stop; subsequent requests must observe newer intent.
Ongoing account application/linking and incremental polling remain open. Metadata review alone must not connect a
remote-specified server using existing credentials. See the profile reference
and TODO for supported settings and remaining behavior; preserve held-response,
restart, malformed seed, settings rollback and import-fence tests.


Profile setup now has a dedicated bounded 32-command engine owner and native
Accounts → Profiles and sync controls. Keep category saves as touched-field
patches; an old full-options snapshot must not disable a newly created profile
or re-enable a disconnected one. UI pending gestures remain separate from the
single admitted save, including repeated values after an older failure.
Read-only HTTP/provider-slot waits cancel on Stop/close; admitted uploads and
cache/history writes keep their receipts before ownership is released. Pending
setup progress must not retire the job or let shutdown skip its final receipt.

Per-workspace sync files live in `profile-sync/` beside the cache: `drive.sqlite`,
hashed-binding history databases and their sidecars/ownership files. Protect all
members and hard-link/symlink aliases during database export/import. New desktop
profiles use namespace `so.shep`; existing bindings retain their namespace. Keep
this value aligned with the participating clients and the live OAuth project.
Initial publication/import is explicitly labeled; ongoing cycles have their own
status and controls. Do not imply unfinished conflict/recovery paths are delivered.
The new native fixture modes/scenarios are documented in the E2E skill.

Profile controls remain disabled until their local enrollment snapshot loads.
Keep newer unsent choices after failed writes/status reads for explicit retry,
but do not treat failed, unadmitted intent as a shutdown dependency. An actual
admitted save/upload still drains. Preserve the controller ordering test and
`invalid-local` native fixture's disabled controls, navigation and graceful restart.


Native profile discovery uses the shared durable catalog pinned at `43cdcf0f`.
The extra shared `test-support` feature exposes only an owned loopback transport
with a fixed fake token; normal releases cannot use it. Keep nested catalog
observations and directory/hard-link aliases protected during database transfer.
Do not share a Cargo target directory between worktrees with different vendored
renderer sources: stale path-dependency artifacts can hide or invent build failures.

Existing-profile import uses a sealed discovery/history/local-revision review.
Only small summaries reach iced; acceptance reads current values on background
owners and atomically commits enrollment, supported preferences and fresh local
account IDs. Persist `profile_join_v1` shared-to-local mappings and the applied
review UUID for lost-acknowledgment retry; database import archives this source
mapping. Never reuse remote account IDs as existing OS credential slots.
`profile_reconnect_v1` keeps imported accounts out of background sync and provider
lookup until SaveAccount has persisted their device credentials. Preserve this
marker in newer workspace snapshots; remove it on explicit account removal.
Keep catalog restart/change-token/ownership, join category/stale/rollback/tombstone,
unknown-connection and native review/import/restart regressions. Joining is an
initial import; account linking, conflict/removal/endpoint reviews and credential
transfer remain TODO work. The protection choice is still
unanswered. Follow the shared handover before extending the format.

Enrollment saves `profile_replication_v1` on the cache owner. First-device setup
verifies the seed's current field values and saves their common basis before any
remote pull; retries must not replace it with newer native settings. Existing
profile import commits raw accepted fields, local/shared mappings and that basis
in the same transaction. Database import archives/removes the source checkpoint.
Never infer an old missing basis from a history that has since changed.

Local capture retains one exact operation UUID, field revision, native value and
portable value until acknowledgment. Preserve optional change/connection fields.
`Replica::admit_local` checks the actual current field after Edit (including an
idempotent retry) before minting the receipt accepted by the Store. A newer whole
history revision alone is not proof the local edit is still current. Acknowledging
admitted work must not restore old preferences, category choices or Google state.
Existing unmapped accounts stay local; local removal records suppression, not a
profile-wide tombstone. Keep restart, category pause, concurrent successor/conflict,
seed, native enrollment and import-fence tests when connecting the continuous loop.

The continuous loop connects safe polling/application; conflict controls remain TODO. Capture/admit
pending local changes before pulling remote records, and preserve per-field common
revisions when local edits race remote application. Dirty UI preferences also need
per-field merging so a later whole-form save cannot erase untouched remote changes.
Desktop uses client `184b98a`'s `initialization-v1` contract through immutable
harness distribution `43cdcf0f`. Keep start/data/completion seed operation identities
and expected revisions durable. Complete setup must be verified by the shared
worker before import or local-change admission; listing completion is not enough.
Never rewrite an admitted legacy operation to insert new ancestry. Only an empty
history with no admitted seed operations may acquire markers while retaining its
metadata/UUIDs. Previously started/finished legacy setup recovery stays in TODO.
Preserve incomplete/legacy/out-of-order and native alternative-profile tests.
Tooltips is the eighth supported portable setting; unsupported touch-only fields
stay in shared history. Preserve active sibling work; no shared target directory
across differing vendor worktrees.


After verified Google connection status, `ui/profile_sync/onboarding.rs` coalesces
one discovery request per connection/session and waits for saved preferences.
Local `Options.discover_on_login` defaults on; Not now persists off even without
an enrollment and never re-enables on reconnect. The Google grant remains local.
The background coordinator owns catalog reads and automatic joining. Only a
single complete profile in an untouched workspace can import automatically;
`Review.automatic` rechecks that condition and opt-out in the acceptance transaction.
Existing local choices or multiple profiles require the ordinary picker/review.
Keep stale Google/generation guards, reconnect-only account staging, atomic joins,
failed/held discovery and native login flows. Prompt completion must not steal
focus or switch tabs. Ongoing sync has separate receipt/application guards.


Continuous profile cycles use the existing owning coordinator and check local
intent before pulling. `profile_replication_v1.deferred` retains exact requests
when a category pauses or a field needs review, allowing unrelated fields to
progress. Only the history owner constructs `continuous::Observed`; cache
application checks current binding/consent/field values atomically. Never advance
an edited field's basis past an unseen remote change. New remote accounts get
fresh local credential identities and Reconnect markers; changed existing
endpoints/removals are retained for explicit review, not automatically applied.

Native preference writes carry typed portable edits via `preference_edits::Write`.
Use `queue_preference_write` or `persist_preferences` in UI paths, not a full
`Preferences.into()` edit. The latter is reserved for deliberate whole-value store
callers/fixtures. Per-field generations preserve reversions and queued retries;
untouched remote fields merge both on the cache owner and into a dirty UI form.
`profile_native_edits_v1` records native preference/account-name generations in
the same cache transaction as their writes. Initial reviews fence those baselines;
an acknowledgment records only its captured generation. Remote applies never
record themselves as native edits. Database import archives source generations.
Keep the race, restart, category, deferred-conflict and native continuous-update
regressions. Polling lists full history and reuses verified immutable records
from the separate profile journal. Cache hits require the exact record metadata
in a current complete scan; decode/hash/namespace checks run on its bounded owner.
Corrupt or oversized cache bytes are misses, repaired only by verified downloads.
Never turn a missing/failed listing into success because old bytes are cached.
Incremental change-token pulls and conflict/link/removal controls remain TODO.

Native tray/window lifecycle uses an iced daemon: destroying the visible native
window must leave engine subscriptions running, while final Quit uses
`iced::exit()` only after required saves/receipts acknowledge. Close-to-tray is a
local preference, off by default. With it off, pending writes can temporarily hide
behind the tray with a saving notification; failure, Open or tray loss restores
an accessible window and cancels close intent. Do not wait for optional read-only
sync or notify for an idle bulk-stop handshake. Preserve per-operation error
ownership and late-acknowledgment tests.

Tray callback actions use a capacity-one watch retaining the latest Open/Quit; Linux watcher availability
uses a separate coalescing watch value so saturated actions cannot discard host
loss. Linux uses StatusNotifierItem/DBusMenu, Windows/macOS use their native event
thread. Cross-compilation does not establish actual platform/menu-bar execution.
`desktop.start(tray="available" | "missing")` owns its own D-Bus socket, GTK host
and notification service; Python GI/GTK3 and python3-dbus are harness dependencies,
not production requirements. Never use the user's desktop session bus for these
tests. Keep normal non-tray close/restart native flows alongside tray regressions.

Ordinary tray hiding also preserves required-write error visibility. Track actual
hidden-window ownership separately from startup's absent window. A fresh error
while an ordinary-hidden write is pending reopens the work; an old error or an
optional read-only refresh does not. If Quit was already pending, use the existing
operation-owned cancellation state before reopening: a stale attachment/discard
result cannot cancel a newer close dependency. Queue-rejected draft saves keep
the visible window. Preserve `test_tray_native_ordinary_hide_reopens_when_pending_send_fails`
and its actual tray-menu Quit variant alongside the typed-result regressions.

Windows badge images are prepared on the existing count watch worker; iced's
native window callback only owns the HWND overlay and its recovery subclass.
`TaskbarButtonCreated` reapplies the latest prepared frame after Explorer restart;
`WM_NCDESTROY` releases the subclass owner. Reopening from tray reapplies the current
count. macOS Dock labels use one acknowledged main-queue operation at a time,
with a capacity-one watch signal retaining the latest count while AppKit is busy,
including when no iced window is open. Preserve held-delivery, zero/large-count,
Linux private-bus and saved native badge regressions. `scripts/check_badge_adapters.py`
checks exact platform adapter source separately from full-app compilation and OS
execution; do not report it as Windows/macOS desktop verification.


The raw Linux entry point is `scripts/install-release-linux.sh`; it fetches the
standard-library helper `scripts/install_release.py`, resolves a published native
asset, verifies `SHA256SUMS`, validates/extracts into private staging, then invokes
the archive's existing Linux installer. It defaults to the current user and offers
an interactive scope/cancel choice; sudo runs only for an explicitly chosen
all-user installation after the archive is ready. Never enable release CI, publish
an asset or install onto a developer's account merely to validate this script.
`tests/test_release_installer.py` uses an owned loopback release server, fictional
binaries, temporary prefixes/menu entries, staged failure/cancellation and a fake
bootstrap curl. Missing release/platform assets must remain explicit; a working
fixture does not prove a binary is publicly available.

The raw macOS entry point is `scripts/install-release-macos.sh`. It uses system
Bash, JXA/Foundation JSON parsing, curl/tar/shasum, plutil and sips/iconutil to stage
a native application bundle. Only exact unique regular binary/icon members stream
out of the verified archive. The destination copy has a rollback slot; an explicit
system install elevates only that final operation. Keep Gatekeeper unchanged and
never describe generated bundles as signed/notarized. `tests/test_macos_installer.py`
executes the shell/download/filesystem paths against isolated native-tool fixtures;
it is not actual macOS execution. No Python runtime is required by this installer.

The raw Windows entry point is `scripts/install-release-windows.ps1`. It targets
built-in PowerShell 5.1 and Windows tar.exe, streams binary extraction outside
PowerShell's text pipeline, creates a PNG-backed native ICO and WScript Start-menu
shortcut, and defaults to LocalAppData/Programs/Shep. Explicit all-user installation
uses a prepared script/data manifest and RunAs; encoded invocation quotes literal
paths and never evaluates release metadata. Keep checksum/member validation,
rollback and missing-asset errors. `tests/test_windows_installer.py` uses temporary
folders, loopback downloads and explicit COM/UAC/environment boundary fixtures;
set `SHEP_POWERSHELL` to an existing PowerShell binary for Linux contract execution.
Do not install a runtime or invoke real UAC on the developer's desktop for tests.
Actual Windows PowerShell 5.1/COM/UAC execution remains distinct from Linux fixtures.

Portable-setting reviews live in `profile_sync/reviews.rs` and the corresponding
store/UI modules. Keep their scalar payloads bounded by the supported setting
list and shared version limit. The history owner rereads the selected immutable
change, preserves opaque extensions and reserves one exact operation before
admission. Persist reviewed concurrent UUIDs with that request; restart must not
regenerate them. Native setting generations, enrollment/Google lifecycle and
history revisions reject stale reviews, including change-and-revert input.
Unrelated settings and newer edits after reservation must survive acknowledgments.
Native actions retain the exact displayed review identity; removing an earlier
row must never retarget an already queued button or dropdown event.
Reviewing uses cached history, without waiting for credentials or a provider slot.
A choice is locally saved before background publication; never describe that as
confirmed Google success. Preserve the `profile_setting_review_*` Rust and saved
native scenarios, including compact dropdown/mouse choices, restart and stale
native preference recovery. Account endpoint/removal/linking reviews remain
separate unfinished work.

Transparent icon sources live in `assets/shepherd-{light,dark,symbolic}.svg`;
approved original PNG references stay unchanged. Regenerate WebP/launcher PNGs
with `uv run --with cairosvg --with pillow python scripts/build_icons.py`.
The Linux installer ships `so.shep.Shep-symbolic.svg` for native system-theme
recoloring and keeps the application/desktop/StartupWMClass identity unchanged.
The StatusNotifier IconName uses that symbolic asset with transparent full-color
pixmap fallback; macOS uses the symbolic raster as a native template. Never add
an app-owned theme watcher that rewrites personal icons on every frame.
Keep alpha/interior regressions, installer update/uninstall checks and the saved
native GTK tray-theme flow. Its `tray_theme` action clicks the owned GTK host's
button; it cannot change the user's desktop theme. Distinguish GTK fixture evidence
from actual GNOME shell, Windows and macOS review.

Compact mail rows share `ui/mail_list::ROW_HEIGHT` (60 logical pixels). Native
keyboard reveal must use actual scrollable layout bounds and reject stale
scope/selection results; do not restore estimated window-height offsets. Tests
use `mail_row_y` and await the layout observation before comparing scroll after
deleting the last visible row. The isolated Xvfb harness uses GTK's Cairo picker
renderer; clipboard ownership and exact path validation remain mandatory. This
setting does not alter production rendering.

## Color palette persistence

Preferences → Colors has independent light/dark RGB roles. `appearance::edits`
tracks each role with fixed-size edit generations; `preference_edits` merges those
intents with current stored preferences before saving. Preserve untouched roles,
explicit reversions and newer edits through old acknowledgments. The editor
projects staged changes onto current palettes, and the UI owns its small iced
theme cache without a shared lock. Keep native palette invalid-input, light/dark
restart, compact Reset/Undo, low-contrast recovery and held-provider save flows.
Custom palettes are exported with the local database; cross-client color sync
still needs a shared-codec key and interoperability tests.


Shared-profile import can explicitly link an exactly matching native account.
Compare the complete portable incoming/SMTP connection, preserve its local ID,
mail, keychain slot, local-name intent and any existing reconnect marker. Never
send an existing credential to a changed downloaded endpoint. Reviews render
eight accounts per page and prohibit reusing one local account twice. Both the
live control and durable join receipt retain the exact chosen links; stale events
and a lost-acknowledgment retry with other choices are rejected. Preserve
`profile_join_*` storage/controller tests and all `test_profile_join_link_native_*`
scenarios. This does not implement post-enrollment linking or password transfer.

Native account connection edits have their own durable profile field generation,
separate from name changes. Keep incoming/SMTP/security/auth/sent-copy reversions
through pulls and restart. Rename/no-op saves must not create connection intent.
Do not turn these generations into permission to retarget saved credentials.

SFTP session/channel setup has explicit deadlines in addition to transport
inactivity and per-request SFTP timeouts. Keep the real held-channel regression: it
advances virtual time only after the SSH handshake, verifies visible timeout and
recovery, and must not write a backup file before setup succeeds. Keep host-key
verification before authentication and never use keepalive activity as proof that
a requested operation is progressing.


Shared connection reviews live in `profile_sync/account_reviews.rs` and
`store/profile_sync/account_reviews.rs`. Keep native account identities separate
when adopting changed endpoints: preserve the previous account/mail/credentials
as local-only and add a fresh account requiring reconnect. Freeze history versions,
native edit generations and Google/consent revisions; reserve native identity and
pending shared operation in one transaction before history admission. Preserve
restart/lost-reply, stale native/history/Google/removal, UID collision and real
native Keep/Add/compact review regressions. Remote removal reviews are separate;
never revive a tombstoned account through an old connection review.


Shared remote account removals never delete local mail automatically. A reviewed
Keep choice suppresses that shared identity durably without moving credentials
or publishing another connection. The ordinary local-data removal dialog retains
its current draft/receipt guards and clears stale shared review cards after a
successful local removal. Keep exact history and local lifecycle validation,
`profile_account_review` Rust tests and the saved `profile_account_removal`
native flows when changing these paths.
