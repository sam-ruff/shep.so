# Shep handover

## Current immediate-action continuation

Flutter CalDAV consolidated text is integrated from
`artifacts/caldav22-folder23-consolidated.patch` (SHA-256
`eca7ce4434a799d5ebc591a9210d4b6acd6b6e93e162f914f5e4f06fd266b60c`).
Root regeneration and Rust checks use `actions-caldav23-root-*` logs.
Credential read/cleanup exceptions need the lane's sanitisation follow-up before
shipping. Preserve schema23 and the existing folder and exact-observation tests.
The v3 credential follow-up is now integrated. Root adds cleanup restart retry,
owned failure clearing, and structured observation for every uncertain Calendar
inspection, including a matching provider state. Reads never fabricate receipts.
Final logs use `actions-caldav23-root-final-*` and
`actions-caldav23-root-cleanup-restart-controls.log`. The first full rerun found a
test harness missing the production workspace listener; its isolated card is now
wrapped in ListenableBuilder and the recovery control must disappear after Retry.
Final full Flutter passes260, Rust185, analysis/Clippy/strict docs pass. Browser
passes266 units, build and three guarded Undo controls. Shipping hooks are next.

Root has uncommitted browser acknowledged Calendar Undo in calendar_actions.ts,
calendar_repository.ts and ui.ts. Optional undoOf/undoAction fields link an atomic
normal queued inverse to its original receipt. Exact cache/latest ownership and
permissions are rechecked; local failure rolls back both records. Nine new units,
three real held-provider create/edit/delete Undo controls and build pass
(`actions-browser-calendar-undo-*`); root reviewed calendar-undo-pending-light.png.
Before shipping this Undo work, complete deleted-event restore fidelity: the
portable event currently omits Google attendee/recurrence and other writable
metadata, so recreating only represented fields is not a complete inverse.
Unknown-result deferred Undo also remains open. bulk_trace knows the new optional
fields and must carry CalDAV provider/connection binding into inverse admissions.

Checkpoint `8044f22bb2f158818f2e70c352f98ba1296646b2` is pushed to main.
Normal hooks pass 1,597 Rust executions, 15 ignored. Browser final units pass257;
backend passes68 with one configured integration prerequisite ignored.
bulk_trace is consolidating Flutter CalDAV controls v2 against this exact commit,
then owns browser CalDAV schema19. review_ordering continues native Activity
schema11 and action_scope checked Flutter folder changes schema24. The full goal
remains active; do not stop at this shipping checkpoint.

Flutter folder23 v2 is integrated from the `action-flutter-folders` artifact
`folder23-v2.apply.patch` (SHA
`b7d307efee60d114ddf4354d9c4365f6023797a238745f9f92f5f9e790fd7a1c`).
Root gates use `actions-folder23-*`; all 178 combined Rust tests and seven folder
controls pass. Generated goldens match the frozen hashes and root reviewed the
light form/sidebar and compact dark recovery images. Root native Clippy and strict
docs pass. All nine ordinary folder controls/actual FFI tests and 13 native
folder lifecycle tests pass. Root also routes typed plan rejection through the
gateway/browser while keeping failed discovery Waiting. Six backend folder tests,
13 browser units, two actual planning controls and production build pass
(`actions-folder23-{backend-plan,browser-plan,browser-plan-controls,browser-build}.log`).
The combined root full Flutter suite and backend all-target Clippy also pass
(`actions-folder23-root-all-flutter.log`, `actions-folder23-backend-clippy.log`).
Preserve existing CalDAV root corrections. CalDAV controls
v1 remains unapplied while bulk_trace fixes checked observations and credential
lifecycle races. action_scope continues checked folder changes with schema24
reserved in its isolated lane.

Native calendar scheduler now uses an explicit positive status index seek.
Both 100,000-history query tests pass, including scheduler retry/dependency and
cache-repair eligibility (`actions-native-calendar-scheduler-bound.log`).

Browser Calendar18 v2 is now integrated from
`action-calendar-durable/artifacts/mail-handoff/browser-calendar-ui-v2-apply.patch`
(SHA `4094d585d5b469de74b5f15ad7ef8fda7d85ae837ee2893528896e169bffb75a`).
Root passes 250 units, production build, 20 Calendar/folder/Preferences Chromium
flows and 13 final Calendar/storage flows. Root additionally returns the exact
delete admission status and retains the editor after terminal refusal on a lost
reply; its saved actual-control regression and light capture pass. Preserve
`artifacts/web/calendar-delete-refused-light.png` and the pending light/checked
compact dark captures. Browser CalDAV, acknowledged Calendar Undo, timed creation,
recurrence editing and final performance remain active gaps. review_ordering now
owns native Activity in a fresh detached worktree `action-native-activity`, with
native schema11 reserved for bounded domain-attention indices.

Root also fixes all-day dates west of UTC in the browser grid/editor. All eight
Calendar controls and the production build pass (`actions-browser-calendar18-date-*`).
The 13 Calendar journal tests also pass, including two root regressions proving
checked changed/absent state survives restart and adoption never writes to the
provider or converts an unknown outcome into success
(`actions-browser-calendar18-recovery-restart.log`).
Root now fences browser server inspections with the calendar revision captured
before the provider read. A newer intent, sync or permission change rejects the
late result before adoption is enabled. The 20 Calendar journal/repository tests,
production build and actual lost-reply/check/adopt Chromium flow pass; logs use
`actions-browser-calendar18-inspection-*`.
CalDAV controls v1 is not integrated: its checked-state adoption used an error
string and discarded intent without applying the observed server state.
bulk_trace is correcting this with a durable structured observation, freshness
checks and atomic adoption; preserve the root nonmatching-inspection regression.
Native/browser offline folder planning already stays Waiting with regression
coverage. action_scope is correcting the equivalent Flutter classification.

Root native calendar queries now use indexed active reads and two one-row
predecessor seeks. The 100,000-history regression, seven lifecycle integration
tests and all-target/all-feature Clippy pass; logs use
`actions-native-calendar-history-*`. This has no input-to-pixel timing claim.

Checkpoint `3f7cb60` is pushed; documentation CI passed and desktop quality is
running. Root now has the uncommitted CalDAV22 v2 prerequisite integrated from
`artifacts/caldav22-native-journal-v2-apply.patch`. The root provider review adds
exact collection/propstat discovery, strong ETags and recurring write guards,
UID decoding, desktop-equivalent duration/default ends, and rejection of partial
reports. Default-feature shared tests, 159 Flutter Rust tests and strict docs
pass. Shared HTTP/parser tests and Clippy logs use `actions-caldav22-root-*`.
The native actions v3 delta is also integrated, with 163 full Rust tests passing:
connection revision fences, durable replaced/cancelled-slot cleanup, pending-intent
reconnect checks, provider dispatch/inspection and Google/CalDAV source separation.
Nonmatching inspection now retains uncertainty; root added a direct regression
for missing and changed server events. Do not ship this as complete parity:
bulk_trace still owns Dart secure-store routing, controls and checked recovery.
Its next handoff must be a delta from actions v3 and preserve root shared provider
changes and the new root inspection regression. action_scope continues
Flutter folders23 on the CalDAV22 prerequisite; its final handoff is pending.

Implementation checkpoint `8db87f1` is verified for shipping with this evidence
update. Normal hooks pass 1,586 Rust executions, ten ignored; owned calendar HTTP
tests run explicitly. Native removal (schema 10) passes 1,204 combined
library executions, 47 integration tests and ten distinct native controls. The
three original pending-read/delete/Undo regressions also pass. Normal root hooks
pass. Preserve the root folder-history query fix and Preferences readiness waits.
Browser schema 17 checked folder changes pass 235 units, 57 backend tests and 22
Chromium controls, with clean Clippy and a passing production build. Flutter
schema 21 passes 154 Rust tests and Clippy: subject-bound coherent snapshots,
safe queued edits, exact admission requests and read-only lost-reply lookup.
Dart admission reconciliation passes 29 focused controls/FFI tests; the full
Flutter suite passes 234. Preserve the root local pending bound, closed-editor
recovery, unbound-owner guard, held-sync test and capacity test's explicit poll.
Shared Google HTTP passes eight tests and replaces the Flutter copy (153 Rust
tests pass); the Calendar gateway and common Drive/Calendar grant renewal pass
67 backend tests with one configured browser skip. Root added current-session
validation even when the cached token needs no renewal. CalDAV22/shared provider
and browser calendar18 continue isolated until this checkpoint ships.
Native pixel v1 failed review timing because the optional full StoreTruth oracle
contended on the same worker. Direct review is 2,600 SQLite VM steps; oracle work
is 4,903,500. Root v2 correctness with the oracle passes (`010a375f4c3c`). Final
20-cycle pixel timing passes review p95 32.31 ms and confirmation p95 22.14 ms;
report `artifacts/performance/actions.json`, reviewed capture `3f6da406de91`.
Preserve the first failed report. Browser's three 100,000-message regressions
also pass in a quiet window. Common performance-gate wiring and its tests pass.
The evidence update ships alongside the implementation; broader client/domain/
platform work remains unfinished. Next integrate CalDAV22, then Flutter folders23,
and the independent browser calendar18 lane, preserving their explicit schema
dependencies and existing Google request recovery.

Shipped checkpoint `d7f06b7` after `48cd150` integrates native schema 8 checked
credential slots/account admission, bounded candidate seeks, group failure
cooldown and preference Retry; Flutter fair resumption, retained draft sessions
and preference Retry; and browser retained draft/file recovery. Combined library
checks pass 1,169 native and 132 Flutter Rust executions. Flutter passes 196 tests,
browser passes 51 focused unit and 11 control scenarios, and Python passes 168
tests with nine skips. Strict docs and both client analysis/build checks pass.
The combined native run passed 26 of 27 flows. Rejected synced credentials exposed
missing journal cleanup and a demo shortcut bypassing the injected keychain.
Both are corrected; four rebuilt password/backup flows pass, covering the final
failure and restart, with reviewed capture `a9b0b41a59b3`. Normal hooks pass 1,547
Rust executions with four ignored; strict docs pass and remote main is verified.

Shipped checkpoint `2605ff5` integrates browser draft conflict review (schema 15),
owned preference Retry and Flutter account lifecycle (schema 18). Flutter passes
134 Rust and 203 controls with clean analysis/Clippy, two additional reviewed
recovery goldens and 26 final FFI/control tests after the duplicate-reply fix.
Browser passes 113 focused tests and 37 distinct controls across reruns. The
100,000-message case passed in 57.5 seconds in a quiet window after loaded-run
timeouts; no budget changed. Keep failing evidence. Normal hooks pass 1,547 Rust
executions with four ignored; strict docs pass and remote main is verified.
Root has integrated durable desktop Send preparation and folder creation in
schema 9, including the combined legacy-import compatibility. Library gates pass
1,189 tests with two ignored; the final combined native binary is building.
Browser schema 16 creation, its real gateway and scheduler are integrated:
78 distinct focused units, 12 Chromium controls and 49 backend tests pass (one ignored).
Per-candidate fairness and receipt provenance corrections are integrated and tested.
The combined native build passes 14 control scenarios with reviewed folder/Outbox
captures; native and backend Clippy and the real pinned TLS folder test pass.
These changes are uncommitted. Flutter Google calendar schema 19 is integrated:
141 Rust and 210 Flutter tests pass, plus eight final policy tests and clean
analysis/Clippy. Review then found Delete decoding, source-scoped identities,
empty-cache and newer-intent projection bugs; the lane is correcting these before
shipping. Release stamping now includes calendar-core and uses manifest suffixes
instead of hard-coded list offsets; all 46 release tests pass. Native schema 10 removal, browser
checked folder changes and Flutter CalDAV schema 20 continue in isolated lanes.
The Google v1 UI correction is not integrated: newest-visible-action sorting still
lets an old rejected action override a newer completed result. The lane is replacing
that with durable intent ownership, adding stale-sync and capacity-free cache-repair
tests, and preserving editor ETags/remote identity so an edit cannot become CREATE.
Google correction v2 is now integrated. Ten Rust calendar and eleven Flutter
FFI/control regressions pass, including Delete decoding, owner-based projection,
retained editor metadata, exact receipt cache versions and stale UI replies.
Analysis and Clippy pass; the corrected compact dark empty state is reviewed.
The v2 full runs pass 144 Rust and 216 Flutter tests; all three explicit HTTP
contracts pass again. Logs are `artifacts/logs/actions-google-v2-*`.
The lane is adding durable Google subject binding, coherent snapshot transactions,
sync/cache-repair revision fences and provider-capacity fairness. Do not ship the
adapter before those gates pass. FFI reopen is another repository handle, not a
process restart. Root owns the Dart refresh-generation edits and HTTP tests.
Root added and explicitly ran three ignored loopback HTTP contracts; all pass,
with clean Flutter Rust Clippy. Preserve `calendar/http_tests.rs` and its module
declaration when applying the next correction. The 100,000-row native folder query
correction and all 20 import regressions now pass.
Preserve `handover-fable.md` without reading or staging it.
TODO remains authoritative and the full action architecture is unfinished.

## Immediate actions checkpoint, 20 September

Checkpoint `7689ef3`, pushed to `main`, adds durable calendar admission/Waiting,
native flag receipt repair, browser individual Activity/offline cancellation/Undo,
Flutter durable individual admission, and native prepared-message queued Send.
The integrated browser gate passes 90 unit tests and 18 Chromium controls;
Flutter's actual admission FFI and 21 workspace tests pass. Native bulk/calendar/
rapid scenarios pass 40 tests plus the authentication-wait scenario, with one
live mailbox skip. The latest library run passes 1,110 tests and queued Outbox
storage passes 13. Native Send queue/restart/cancellation controls pass. Normal
hooks pass 1,479 Rust test executions; browser v3 passes 94 unit and 31
control/storage/profile scenarios. Strict docs pass.

Implementation `e84f66b`, pushed to `main`, adds native individual observed admission,
field ownership and a shared bounded background scheduler. Flutter Activity,
conditional rollback, checked Undo and queued Send pass 117 Rust and 175 Flutter
tests, including input-lineage and Outbox retry/refresh. Normal hooks pass 1,509
Rust executions, with four ignored. Final native gates pass 29 scenarios, with
one live-mailbox skip; strict docs pass. The release-build observation method
also passes the no-default-features Clippy gate.
Frozen lane handoffs and logs live under ignored `artifacts/`.
Preserve the user-owned untracked `handover-fable.md`. Full domain adoption and measured mailbox-scale
latency remain open; TODO is authoritative.

`a1049ab` implements the first native mail/calendar migration requested by Sam.
Review and confirmation no longer wait for prior provider writes; calendar
save/delete uses shared projection outcomes and exact read-only recovery.
The completion log records 1,365 passing Rust tests and 38 native scenarios.
Normal hooks pass, with reviewed light and compact dark captures.

Implementation `48cd150`, pushed to main, adds Flutter schema 17 receipt-before-cache repair and
accepted-field ownership (131 Rust and 175 Flutter tests), native within-group
account concurrency (21 scheduler, 26 store and 24 native scenarios, one skip),
browser connection Activity (42 unit and 14 Chromium scenarios) and native
draft-owned errors/Retry plus a focused editor-shortcut regression. Final compact
visual review passes; normal hooks pass 1,516 Rust executions with four ignored,
both Clippy configurations and strict docs. The personal installation is unchanged.

Next: checked native credential-slot activation, indexed bounded native scheduler
claims and fair Flutter resumption, then remaining domain adapters. Keep one durable
owner per action, preserve uncertain receipts and newer intent, and retain
native 100 ms mailbox-scale timing and release/install verification as open
work. TODO remains authoritative; this checkpoint does not complete the app-wide
request or the earlier product requests below.

## Current delivery, 13 September

Mail recovery, standard HTML/title selection, ordinary-cache large receipt and
Spam are pushed as `12e6e2d`, with the production build installed. Completion
records 1,305 normal-hook Rust executions, 51 distinct native scenarios, final
selection reruns and the binary hash. Existing processes need normal Quit/reopen.
The next requested step is discussion of cross-device email/folder state, before
implementing a new sync design. Encrypted large staging, full streaming/search,
Spam aliases beyond Junk, live-provider confirmation and Flutter/browser parity
remain explicit gaps. Preserve the unrelated Google client configuration request
and the wider authoritative TODO.

## Current decision, 12 September

Moving-mail source is committed on main as `b614b8d`, with 1,263 hook executions
and nineteen relevant native scenarios passing. Missing destinations are created
before mail submission; exact proof enables automatic recovery and direct Retry.
Sidebar New folder is pushed as `842c570`, with thirteen native scenarios and
1,277 normal-hook executions passing. Text menus are pushed as `bc9ca09`, with
27 distinct native scenarios and 1,287 hook executions passing. README installer
fixes are pushed as `c3296ce`; the verified production update and approved icons
are installed. Existing processes require normal Quit/reopen. This requested
desktop sequence is delivered; continue the wider authoritative TODO, retaining
platform and client gaps. Keep uncertain outcomes protected and preserve the
retarget identity, Undo and late-page count regressions. Completion records
the native hash and reviewed evidence; isolated UI lanes started at `0abec9f`.

Sam selected Abbreviations (`b00c784`) after the live profile comparison and
requested deletion of Precise and Tolerant branches. The selected implementation
is pushed on main as `86d5ca1`; the other branch references are deleted locally
and remotely. Preserve its parser/control/client parity gaps.
The older unselected-demo notes below are comparison history.

Duplicate taskbar launches and GNOME dock/minimise restoration are fixed and
pushed as `7ef9d74`, with the verified production build and current launcher/tray
icons installed. Existing old copies require a normal Quit; do not force-kill
personal instances or discard pending saves.

The GNOME incoming-mail notification fix is committed as `ae85450` and installed.
The worker retains its D-Bus sender; integrated native notification and launcher
tests pass. Preserve initial-sync suppression, content preferences, bounded
delivery and saving-notice cancellation. Completion records exact evidence.

The shared CREATE provider now checks exact folder identity before and after
creation. Sidebar New folder follows as a separate feature shipment. Its lane retains frozen
account/parent/name requests across failure and restart. Client creation
controls remain an explicit parity gap; Sent creation is a separate review.

## Search demo delivery, 12 September 2026

The latest requested sequence is delivered: formatted HTML selection,
newest-first conversations, pinned reader actions and icon fixes are on main;
three independently verified search alternatives are pushed for comparison:

- `demo/search-precise`: `4b14a39`, worktree `.claude/worktrees/search-precise`.
- `demo/search-tolerant`: `1c43ec1`, worktree `.claude/worktrees/search-tolerant`.
- `demo/search-abbreviations`: `b00c784`, worktree `.claude/worktrees/reader-actions`.

Each passes twelve final native scenarios, normal hooks, parity and strict docs.
The 100,000-message multiword p95 values are 33.04/32.07/31.25 ms against the
unchanged 50 ms budget. [Completion](docs/COMPLETION.md) records exact commits,
native executable hashes and evidence. The [guide](docs/agents/search-demos.md)
opens fresh fictional workspaces without replacing the personal installation.

At this earlier checkpoint no search alternative was selected for main.
R18/R44/R99 still track parser/identifier semantics,
deeper settings controls and Flutter/browser parity; other TODO work remains
active. Keep each worktree's target separate and use the normal hooks. Quality
and release workflows remain disabled until the runners are ready.

The older client/profile session notes below are historical. Current AGENTS.md
and TODO.md take precedence over their restart order and branch instructions.

**The full product goal is unfinished.** `TODO.md` is the authoritative active
request list; implementation and shipping evidence is in [the completion log](docs/COMPLETION.md)
and [the request audit](docs/REQUEST_AUDIT.md). On 2026-09-09 `main` (`c414227`)
was merged into `feat/mobile-web-clients`; from now on `main` is the single
integration branch for the desktop, mobile and website sessions, desktop changes
reach the client branch through merges from `main` rather than ports, and root
desktop code follows `main` where the two disagreed (main's `src/profile_sync`
replaced the client branch's own desktop profile implementation; the shared
profile-core crate is the superset of both). Request numbers R67 to R80 exist on
both sides; the audit states the collision once. The two sessions' notes follow,
each as it stood before the merge.

# Client session (feat/mobile-web-clients)

Development resumed after the credit-limited checkpoint.

## Working constraints

Work in `shep-clients` on `feat/mobile-web-clients`. Push verified checkpoints
promptly with the configured owner identity and Conventional Commits; never skip
hooks. Do not replace the installed desktop from client-branch builds. Since the
9 September merge, `main` is the single integration branch: the client and
website worktrees keep their branch names only because git cannot check out
`main` twice, and push verified checkpoints with `git fetch origin && git
rebase origin/main` then `git push origin HEAD:main`. Delegated promo
source/assets are already integrated; do not recopy the older `shep-website`
README. Read `AGENTS.md`, `TODO.md`, the parity matrix and latest completion entry
before continuing.

The normal app was previously installed on the owner's Android phone without
clearing data. Do not reinstall, reset or automate that phone. Its package/build
and locked-screen observations remain in ignored `artifacts/flutter/phone-install/`
and `artifacts/logs/phone-*`; never commit pairing details or personal screenshots.
Use a dedicated `shep-e2e` emulator and the isolated preview package for tests.

## Product decisions

One monorepo: Rust + iced desktop at the root, Android/iOS in `flutter/`, a
**separate desktop-style browser client** in `web/`, and promo in `website/`.
The initial separate Flutter repository and hosted Flutter layout are superseded.
Preserve the restrained shadcn theme, approved Shepherd, configurable K-9-style
mail rows/previews/swipes and clickable equivalents.

The browser beta uses server-verified Google login and an explicit owner
allowlist, expandable later. `backend/` supplies Rust verification and authenticated
IMAP/POP3/SMTP transport; ordinary browsers cannot use those TCP protocols directly.
No persistent server-side mail/password storage is authorized. Browser credentials
currently remain in tab memory; secure remembered credentials are unfinished.

VPS installation is already authorized, but the SSH target, exact verified owner
identity and Google OAuth configuration are missing. Never infer identity from Git
contact details or ask deployment permission again. Assets are staged, **not
deployed**. Keep unpublished store/release destinations visibly unavailable.

Quality/release definitions stay `.yml.disabled`; documentation CI stays enabled.
Remind Sam to enable quality/release when trusted runners and signing are ready.
Apple execution, store publication and coordinated distribution remain open.
R75 still requires provider OAuth across clients; beta login alone does not connect
Gmail/Calendar/Drive. Keep R76's grouped scheduled automatic replies and R72's Linux
store submissions in TODO.

## Current continuation

Flutter preference receipt recovery is the current checkpoint. The platform write
now retains the original field revisions and forwards them to native enrollment;
exact retries cannot substitute later local revisions. Legacy receipts remain
explicitly unproven. Local reverted intent survives an earlier failed save, and
optimistic painting checks the UI generations captured with the review. Reopened
receipts do not flash an obsolete imported value.

All 129 Flutter host tests, 81 mobile Rust tests, static analysis and the two
Android integration scenarios pass. The saved controls leave a lost acknowledgment,
change appearance and resume while preserving the newer choice. Appium/Playwright,
visual evidence and exact shipping are recorded next in [completion](docs/COMPLETION.md).
Keep the retained failure evidence there; do not count an interrupted wrapper or
integration teardown as another passing scenario.

The client branch's desktop ongoing synchronization and paged conflict reviews
were pushed in
[`28c2884`](https://github.com/sam-ruff/shep.so/commit/28c288448842b7d09543fc28ef1f2f14bf36f142)
and superseded by main's desktop implementation at the merge. Their
shared-owner, active-grant, full-rescan, original-record, durable receipt and
newer-intent invariants still apply (see AGENTS.md "Desktop profile
application"); their 23 native flows, three final reruns, 54 profile checks and
498 mandatory hook tests remain recorded in completion as evidence for the
superseded code, not for main's.

**Next implementation:** connect Flutter's ongoing native subscription/edit ledger,
platform application requests and exact receipts, then SDK-owned scheduling and
per-field/conflict controls. The platform store currently retains one receipt;
new sync work must serialize with enrollment before another application can replace
it. Seed only from proven original revisions; publication and legacy receipts
without that proof require conservative pending state. Never acknowledge a later
local edit merely because its current value matches the imported value.

All 40 requests remain active. Automatic restoration/setup, complete fields and
categories, accounts, authenticated interchange, protected credentials and browser
reconciliation remain unfinished. The earlier native navigation result is still
154.81–162.33 ms against 150 ms, and the combined performance gate remains failing.
No new performance measurement is claimed on this shared, busy host.

## Restart order

1. **Highest priority:** continue [OAuth/shared profiles](docs/agents/PROFILE_SYNC_HANDOVER.md).
   Extend Flutter reconciliation and checked preference decisions with native and
   Playwright controls. Preserve the delivered desktop reviews, exact saved
   decisions and newer local intent protection. Continue automatic restoration/first setup, complete
   portable settings/categories, account lifecycle and authenticated interchange.
   Do not turn a missing remote original or another project's empty app-data
   space into successful setup. Preserve full rescan on owner reopen, known
   original verification, active-grant ownership and shared review serialization.
   Copy original records into independently owned device history, never clone a
   remote observation database/device UUID. Account application preserves mail and
   drafts, uses explicit mappings and requires reviewed credential activation when
   endpoints change. Keep offline conflicts/removal, setting-reset intent and local
   edit generations. An incomplete listing or different Google project's empty
   app-data space must not imply empty setup. Account passwords use Google-only
   protection (Sam, 11 September 2026); legacy backups and full database
   migration remain separate work.
2. Preserve shipped browser group Undo/recovery, alias/receipt and worker ownership
   regressions. Current browser recovery checkpoint `df4f29c` passes 137 Chromium,
   127 unit and 56 production HTTPS fixture stages. Continue abandoned review and
   staging cleanup; exact evidence/failed baselines remain in the completion log.
3. Flutter selection now drives durable group execution with Select/Done/Clear/
   Select all, review, Undo, Pause/Resume, History and recovery controls
   (integrated 11 September as `01569b5`). Remaining work is in TODO R42: live
   IMAP, cross-account moves, large-group performance, painting before the
   durable decision, keeping the selection after a declined review and Apple.
4. Continue account/calendar/backup, composition, cache/large-message, remote-image,
   keymap, lifecycle, performance and platform gaps in TODO. Desktop changes now
   arrive through merges from `main`; review each merge for new parity gaps.
5. Execute Apple/live-provider verification, deployment and distribution when the
   required environments/configuration exist. Full parity remains active throughout.

## Verification and artifacts

Run relevant saved scenarios from [client testing](docs/CLIENT_TESTING.md); never
substitute direct controller calls for real UI controls. Enrollment checks are
`python3 scripts/clients/android_e2e.py --device emulator-5554 --enrollment-only`
and `python3 scripts/clients/flutter_web_e2e.py --enrollment`. Publication checks are
`python3 scripts/clients/android_e2e.py --device emulator-5554 --creation-only` and
`python3 scripts/clients/flutter_web_e2e.py --creation`. Discovery checks are
`python3 scripts/clients/android_e2e.py --device emulator-5554 --discovery-only` and
`python3 scripts/clients/flutter_web_e2e.py --discovery`. They run explicit fixtures.
Wait for each Flutter build/test process to terminate before editing its source or
starting another build in that checkout.

Use `CARGO_TARGET_DIR=artifacts/root-target` for mandatory root hooks and
`artifacts/flutter-target` for native Rust checks. Flutter commands run inside
`flutter/`; wrapper commands run from the worktree root. Logs belong in ignored
`artifacts/logs/`. Before pushing docs run
`artifacts/docs-venv/bin/zensical build --clean --strict`; run the parity checker and
relevant unit/protocol/native controls. Keep snapshots, tokens and personal data
out of source and public documentation. A pushed handover is not product completion.

# Desktop session (main) — 2026-09-09

Read [TODO.md](TODO.md) at the start of each turn. [AGENTS.md](AGENTS.md) owns
operational instructions; [the request audit](docs/REQUEST_AUDIT.md) preserves
the conversation scope and [the completion log](docs/COMPLETION.md) records
shipped behavior and evidence. Historical implementation notes belong there,
rather than becoming new TODOs.

## Parallel work in progress

The user now explicitly requests independent agents/worktrees and primary-agent
integration into main. Active lanes live under ignored `artifacts/worktrees/`:
`folder-convergence` (combined folders and reconciliation), `multiple-backups`
(SFTP after its reviewed S3 checkpoint), and `native-tray` (encrypted storage
after its reviewed palette checkpoint). Compact rows/icons are integrated in the
primary workspace; matching-account import is ready in `codex/profile-links`. Keep each Cargo target separate and builds capped at
four jobs. Isolated correctness-only native flows can run alongside unrelated
builds; latency and renderer pixel measurements need a quiet window. Never relax
budgets to accommodate host load.

The primary workspace connects continuous profile publication/application and
per-field preference merging, shipped in `bd50c52`. Refresh lane `f111282` was
integrated/pushed as `d29ce06`. Native evidence and the intermittent GTK picker
readiness issue are in the newest completion entry;
conflict/removal/endpoint controls, account linking, incremental pulls and
credential protection still remain. Do not discard any lane's uncommitted work.

The isolated `profile-cache` / `codex/profile-links` lane adds explicit matching
account reuse during import. Tests and remaining shipping work are recorded in
COMPLETION; preserve its separate commits when integrating the inbox/icon lane.
It does not complete post-enrollment linking or protected password transfer.

## Current source checkpoint

Compact rows, actual-layout keyboard reveal, conversation-anchor refresh and
transparent icons are pushed as `15a4a3c`; R88 is complete. The 248 native paths
have passing coverage across the full run and corrected setup reruns, with 769
hook executions, 81 Python tests and strict docs passing. Production remains on
the earlier installed build. Palette/S3/account-import links are pushed in `4e75005`: 798 root hook executions,
81 Python and all 88 integrated native scenarios pass. R25 is complete; SFTP and
connection-reversion tracking are the next root integration checkpoint.

The encrypted-storage lane may consume shared initializer API
`8588c21b4785cca6951bdc335ca938580ae60f1e`, published separately on
`codex/profile-core-encrypted-open`. Existing client WIP was preserved; this
backward-compatible API does not by itself enable encrypted startup.

Portable preference reviews from `713f96e` are integrated: six targeted
regressions, 86 matching profile tests, 81 Python tests and all 25 integrated native
profile flows pass on the final control-identity guard. Source `22a3af5` is pushed
with 762 mandatory hook executions and exact remote equality verified.
Account linking/removal/endpoint reviews remain separate work.

`2b87db4` is pushed: multiple Local/Drive destinations, 756 hook executions and
21 integrated native backup/preferences/profile/import scenarios pass. The merge
preserves typed preference intent and per-destination metadata, including a new
remote-setting/upload-receipt race regression. The backup lane continues S3.
Linux raw installer is pushed as `3676dad`, with 68 Python tests and mandatory
hooks passing. macOS wrapper is pushed as `0f2af1f`, with 74 Python tests, mandatory hooks and
green docs CI. Windows is pushed as `ce4a2a6`, with 81 Python tests, mandatory hooks and green
docs CI. The agent continues the transparent themed launcher (R64). Actual published assets and platform execution
remain open.

`1595fb3` is pushed: Windows/macOS badge adapters, 751 hook executions, 12 native
Linux scenarios, full merged Windows checking and exact macOS adapter checking.
Actual Windows/macOS desktop execution remains open.

`6728931` is pushed: native tray, temporary-saving notification/auto-exit and
failure recovery pass 745 integrated hook executions and32 native scenarios.
Linux menus/compact settings were reviewed; Windows GNU and exact macOS adapter
checks do not establish actual platform runtime. Ordinary-hide pending-send failure recovery is pushed in b8eafde, with 748 hook
executions and 16 native scenarios passing. Keep R86 active for actual platform
verification. The tray agent has moved to direct-download installers.

`91ed9a9` is pushed: deletion follows the next displayed message without resetting
scroll, including page boundaries and previous/empty fallback. Combined testing
also fixed bulk Move folder labels when the reader belongs to another account.
All 59 integrated mail scenarios and 736 hook executions pass; reviewed native
evidence is in completion. R89 is complete; compact row styling remains in R88.

`2733cc6` is pushed: shutdown continuation, interruptible queued-provider waits
and draft-failure recovery, with 725 hook executions and 23 integrated native
scenarios passing. Root reviewed the merged error/retry controls; docs CI passed.
Native/temporary tray and personal-account close diagnosis remain unfinished.
`e77eabc` is pushed: verified Drive record caching passes 729 integrated hook
executions and nine native cache/profile/close scenarios. Full change-token polling and profile review controls
remain separate work. The installed personal app is unchanged.

`bd50c52` and `d29ce06` are pushed: continuous safe profile updates, durable native
edit generations and slower clockwise manual refresh. Root integrated the agent
refresh commit after reviewed native evidence. R87 is complete. The next profile
work is account linking/conflict/removal/endpoint reviews and incremental pulls;
all active worktrees must be preserved. See the newest completion entry for exact
checks and the isolated GTK picker failure/retries.

`acb4969` is pushed to main and connects post-login discovery/enrollment. A single
complete profile imports automatically into an untouched workspace; first setup,
multiple profiles and existing local data have native prompts/reviews. Not now
persists an opt-out and Preferences can re-enable discovery. Verification and
the shipping receipt are in the newest completion entry. Continuous
publication/application is still the next functional priority.

`9158b50` is pushed to main and adopts the shared `initialization-v1` barrier from
Flutter publication. Desktop creation writes stable start/data/completion records;
import requires the shared worker to verify completion. Unstarted legacy seeds
upgrade without changing their metadata/UUIDs; already-admitted legacy records
remain untouched for recovery. Eight settings now include Tooltips. Verification
and the source shipping receipt are in the newest completion entry.

`5c0e9f0` is pushed to main and adds `profile_replication_v1`: enrollment now saves the
last common field values, exact raw extensions and local/shared account mapping.
Local capture/admission APIs preserve pending UUIDs and per-field bases across
restart, category pauses and newer native edits. A sealed history receipt verifies
the field is still current before acknowledgment. These APIs are tested preparation
for the continuous loop; the loop and remote application are not connected yet.
See the newest completion entry for this continuation's verification and shipping.

Earlier source `071c6b0` added discovery of named shared profiles. Preferences can
review one and import its account definitions and supported preferences. Existing
accounts/mail stay intact. New definitions receive fresh local IDs and require
**Reconnect** before receiving or sending mail. A saved acceptance ID makes retry
after a lost acknowledgment safe; stale reviews and later local choices cannot
partially apply. The screen explicitly identifies this as an initial import.

Desktop now pins the shared Drive/catalog/history crate at `43cdcf0f`, which
copies published client `184b98a` and retains the fixed-token loopback harness. Discovery
retains its progress and change token in a separate owning catalog; enrolled
histories and local edits remain separate. Read
[the desktop contract](docs/agents/profile-drive.md) and
[the Flutter handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md)
before extending this path. OAuth implementation, referencing that handover,
remains **the first TODO item**, as requested.

Database export/import and local profiles already shipped in `93d4289`; inline
composition/parked replies in `e16590c`; first-device Drive creation/recovery in
`488a9ec` with failure/close correction `6860f50`. Preserve these features and
their regressions. Imported databases require reconnection and a profile change
on next launch; they do not hot-swap an engine or replay another device's sends.

## Next work

1. Connect ongoing local/remote profile updates, including existing populated
   workspaces and saved category choices. Keep local capture/admission ahead of
   remote pulls and implement per-field application plus dirty-UI preference merges.
   Preserve the new after-login native prompts, automatic single-profile import,
   saved opt-out/re-enable, retry/navigation/close and independent-device tests.
2. Preserve the shared initialization barrier: stable start/data/end requests,
   partial upload receipts/retry, out-of-order histories, independent device identity,
   disabled incomplete-profile reviews and usable alternatives. Already-admitted
   legacy profiles still need explicit recovery/migration; never insert new ancestry
   into immutable uploaded records. Unstarted legacy seeds can upgrade in place.
3. Connect account linking/suppression, conflict/removal reviews, offline recovery,
   incremental enrolled-history pulls, password-prompted legacy encrypted-backup
   migration and the remaining portable preferences.
   Current enrolled-history pulls still read full history. The catalog's change
   stream does not by itself implement continuous account synchronization.
4. Preserve the identity maps: first-device seeds map local IDs to shared UUIDs;
   `profile_join_v1` maps shared UUIDs to fresh local IDs. Future exports must use
   those durable mappings. `profile_reconnect_v1` blocks provider use until explicit
   device credential setup. Database import archives source-device enrollment,
   seed, join mapping and replication checkpoint, preserving pending-operation reviews.
   Capture/admit local edits before each pull. Keep a field's last common revision
   when local edits race application, and merge dirty native preferences per field.
   Older profiles without a provable basis need recovery review. Remote endpoint
   changes need explicit fresh credentials; do not let blank Reconnect fields reuse
   an old password against a new server. Conflict/removal controls remain unfinished.
5. Sam chose **Google-only** password protection on 11 September 2026; follow the
   shared handover's credential section. Never put account passwords in
   metadata/SQLite. The current shared format contains account definitions and
   selected settings, without secrets.
6. Verify real same-project Google appDataFolder visibility and OAuth across the
   participating clients/platforms. Fixture success is not live interoperability.

Shared initialization/harness support was published through the isolated
`codex/profile-initialization-harness` branch, copied from immutable client
`184b98a`. Since the 2026-09-09 merge the shared profile-core crate lives in this
tree as a path dependency and is the superset of both sides; the client branch is
no longer a separate source to copy from. Do not share a Cargo target directory
between worktrees with different vendored renderer sources.

After the profile priority, continue every remaining TODO. In particular, native
tray/temporary saving tray (R86), slow close dependencies (R90), channel ownership
(R91), encrypted cache, large mail, multiple backup destinations, desktop/platform
integration and final usability/performance work remain open. Bulk still needs
review of provider-capacity waits after claiming a step; some other close
completion paths need automatic continuation. Preserve durable in-flight receipts.

## Verification and installation

The latest source commit's mandatory hooks passed **647 Rust + two renderer +
53 shared tests (702 executions)**; three personal diagnostics remain explicitly
ignored. **55 Python tests and 30 selected native scenarios passed**, including
six new login flows, 11 profile flows, eight database transfers, four Google flows
and tooltip preferences. Light/dark/compact WebPs were reviewed. Windows GNU cross-compilation and strict Zensical passed.
See the completion log for exact artifacts and documentation CI shipping evidence.

Native test executable SHA-256:
`1d6cbaec9815937101de0c07489eebc36e2a5a946e2021149b317aa053585611`.
This is an isolated test-support executable, not an installed production release.
The personal Linux installation remains source `3567de2`, SHA-256
`06c0cccb3d3cc6703b143f8e7fa019c1be7032533ae6d4e776a81cc6f91ef34a`.
No personal mail, credentials, cloud data or installation was changed here.

Use the [MCP skill](.agents/skills/shep-e2e/SKILL.md) for native work. Keep automated
equivalents and review actual screenshots. `--functional-only` selects the full
functional suite and currently ignores `-k`; selected runs use `-k` alone. Do not
run builds while native tests are active. Logs and fixtures belong under ignored
`artifacts/`; use `login:false` for shell tools. Subagents follow the R93
parallel-delivery rules in AGENTS.md.

Performance measurement remains deferred until the final idle-host phase.
Quality/release workflows stay disabled until the self-hosted runners are ready
and Sam requests re-enablement. Documentation CI/Pages publishing is enabled.
