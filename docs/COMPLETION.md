# Completion audit

## 9 September: profile discovery and enrollment after Google sign-in

R02/R49/R92 now connects verified Google connection status to background profile
discovery. No existing profiles produces an optional setup prompt; multiple
profiles or existing local data/settings use the picker and review. A single
complete profile imports automatically into an untouched workspace. Imported
account definitions use fresh local IDs and require Reconnect; the completion
message includes the profile and applied counts without switching tabs.

Not now persists discovery opt-out before any enrollment, across restart and
reconnection. Preferences can enable it again. Discovery waits for pending
preferences, coalesces repeated status events and preserves close cancellation.
The acceptance transaction rechecks opt-out and newly created local data; stale
reviews cannot apply. Errors show a recovery prompt, never an empty-cloud success.
Existing profile publication, initialization barriers and retry receipts remain.

Four new Rust regressions cover persisted opt-out/reconnection, settings/draft
eligibility, automatic acceptance with late local changes and idempotent retry,
and UI status coalescing/save/error/decline/close ordering. The targeted profile
run passes 69 tests, with one personal diagnostic explicitly ignored. **30 native
scenarios pass**: six new login flows, 11 existing profile flows, eight database
transfers, four Google-connection flows and tooltip preferences. Actual buttons
exercise setup, profile choice, Not now and re-enable. Reviewed final WebPs include
`147f41edd4cf` (compact dark prompt), `a83dba5f2152` (automatic import/notice),
and `28d56de408ca` (light setup after re-enable). Native executable SHA-256:
`1d6cbaec9815937101de0c07489eebc36e2a5a946e2021149b317aa053585611`.

Python passes 55 tests and Windows GNU all-target/all-feature cross-compilation
passes. Strict Zensical passed; mandatory hook and shipping receipts follow the
source commit. The first native attempt identified a missing preview status-event
fixture; it was connected to the normal command path. A checkbox-edge coordinate
was corrected before the complete passing run. No test or budget was weakened.

The owned login fixture supplies a synthetic committed grant and uses real
catalog/history HTTP and native controls. It does not prove actual browser OAuth,
Google cross-client visibility or password transfer. Continuous local/remote
updates, account linking/conflict/removal controls, legacy recovery/migration,
remaining portable settings and credential protection remain in TODO. No personal
mail, keychain, cloud data or production installation changed. Performance remains
deferred; quality/release CI remains disabled and documentation publishing enabled.
The full goal and OAuth handover priority remain active.

## 9 September: shared setup completion before native import

R02/R49/R92 now uses Flutter publication's `initialization-v1` protocol. The pinned
[`43cdcf0`](https://github.com/sam-ruff/shep.so/commit/43cdcf0f70f7dbff2f80b7828eb7e570d025b09a)
distribution copies the shared source/fixtures from client `184b98a` and retains
the owned loopback harness seam. The active client worktree was preserved. The
independent shared suite passes 53 tests; its Clippy check also passed.

Desktop creation persists a start operation, metadata chunks and completion
operation with stable IDs/revisions. An interrupted upload retains its receipt
while later records stay queued. Native discovery/import and local-edit admission
require the shared worker's initialized state. Complete listings, a visible name
and populated settings cannot authorize importing a partial setup. Out-of-order
complete histories import successfully with an independent device identity.

Unstarted legacy seeds can acquire markers without changing metadata or operation
IDs. Already-admitted legacy records stay untouched for recovery; their ancestry
is never rewritten. Desktop now applies the portable Tooltips choice, bringing
supported settings to eight. Validated touch-only settings remain in history.

The targeted profile suite passes 62 tests, including three new boundary/recovery
regressions and updated multi-record upload contracts. **17 native scenarios pass**:
11 profile flows, five database imports and tooltip preferences. Two new native
flows reject unfinished/legacy Home while allowing complete Work. Reviewed WebPs:
`1f665e379cfa` (incomplete/light), `f99c7cb81fad` (compact dark review) and
`ab3e7ba843e3` (import/reconnect). Native executable SHA-256:
`709eff8bba2e270f5cb51152989e8cc4890f8d887226f2cd9a5ce266f8e1c6c4`.
Python passes 54 tests; Windows GNU all-target/all-feature cross-compilation
passes. Mandatory formatting/Clippy hooks passed **643 root Rust + two renderer +
53 shared tests (698 executions)**; three personal diagnostics remain explicitly
ignored. Strict Zensical passed. Source
[`9158b50`](https://github.com/sam-ruff/shep.so/commit/9158b502bddb7ae6cae9937db9d0965ad938cd8f)
is pushed to main; exact remote equality was verified. [Source documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34320559268)
passed its strict build and Pages deployment. The initial hook's redundant test
closure was corrected before this successful commit; no hook or gate was bypassed.

Automatic login discovery/enrollment, continuous local/remote updates,
conflict/removal controls, admitted legacy migration and protected credentials
remain open. These are isolated protocol/native tests, not live Google or actual
Flutter-to-desktop cloud verification. No personal data, credentials, cloud state
or production installation changed. Performance stays deferred and quality/release
CI stays disabled; documentation publishing remains enabled. The full goal and
OAuth handover priority remain active in TODO.

## 9 September: durable common values for later profile synchronization

R02/R49/R92 enrollment now saves its original common field values and shared/local
account identities. Initial creation establishes this checkpoint before pulling
later records; import commits it with the accepted accounts/settings. Setup and
acceptance retries preserve later native edits. Database import archives the
source checkpoint without replaying its pending profile changes.

The new local capture/admission APIs reserve an exact operation UUID and the last
common revision of its field. They preserve optional shared fields, paused
categories and newer local values. History admission verifies the current field
before issuing a receipt: an idempotent Edit response containing a newer whole
history revision cannot silently acknowledge an unseen successor or conflict.
Existing unmapped accounts stay local; local-only removals record suppression.

Nine new backend regressions cover restart/lost acknowledgments, edits before
initial upload, category pause/re-enable, disconnect, concurrent remote changes,
stable account mapping, suppression and invalid field bases. Existing join and
database-import tests now check atomic common values and source fencing.
**14 native scenarios passed** (nine profile and five database import), including
read-only SQLite assertions after native enrollment and graceful close. Reviewed
WebPs: `f5adc93c24d1` first-device/light, `76913bfe3666` import/reconnect/dark and
`cec12e9c521b` compact dark review. Native executable SHA-256:
`be29388aaf1579dc24d60d8789c6e5e35e726d05972465b9df8777cddeba8097`.
The 54 Python tests and Windows GNU all-target/all-feature cross-check passed.
Mandatory formatting/Clippy hooks and **640 root Rust + two renderer + 50 shared
tests passed (692 executions)**; three personal diagnostics remain explicitly
ignored. Strict Zensical also passed. Source
[`5c0e9f0`](https://github.com/sam-ruff/shep.so/commit/5c0e9f06e935b3e5425ad23155ed17ea5c68e854)
is pushed to main and exact remote equality was verified; targeted profile tests
passed 59/59. [Source documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34317935845)
passed both its strict build and Pages deployment.

The periodic publication/application loop, conflict/removal controls and automatic
login/enrollment remain unfinished. The newly published Flutter `184b98a` adds an
initialization barrier that desktop's current `33d222d7` pin must adopt before
claiming current first-device interoperability. No live Google, personal account,
keychain or installed production data was changed. Performance remains deferred;
quality/release CI stays disabled. OAuth with its Flutter handover reference
remains the first TODO item; no full feature request was removed.

## 9 September: named profile discovery and existing-device import

R02/R49/R92 adds a native profile picker and reviewed import of account definitions
and seven supported preferences. Discovery uses the shared owning catalog and
saved Drive change tokens; the dependency is pinned to published `33d222d7`.
That commit adds the fixed-token loopback harness seam to the client catalog from
`3c9b98d`, without editing the active sibling client worktree.

Joining reads a complete conflict-free history and commits enrollment, supported
preferences and fresh local account IDs atomically. Existing accounts/mail remain.
Imported accounts show Reconnect and are excluded from background sync/provider
lookup until explicit device credential setup. A saved review UUID prevents
duplicates after a lost acknowledgment; later local choices survive retries.
The import fence archives source-device join mappings. Nested observation files,
directory aliases and hard links are protected from database export.

New backend regressions cover change-token reuse, stale discovery/history/local
reviews, held-read cancellation and ownership, category selection, restart,
credential-slot isolation, rollback, unsupported connection fields and tombstones.
The controller rejects reviews older than acknowledged category choices.
**22 selected native scenarios pass**: nine profile flows, eight database transfer,
two Google disconnect and three account setup/removal flows. These include three
new existing-profile scenarios. Reviewed WebPs are in ignored runs `b5114d0926e2`
(light/import/reconnect), `e38b0fccef38` (compact dark/settings-only/cancel/disable)
and `407fa9f5b9c7` (unsupported account/recovery). Native executable SHA-256:
`e7d39ae27f87967be4612310288b391cc5d80c92a1eb1c287ef75756004a61fb`.
The pinned shared suite passes 50 tests; Python passes 54 tests. Windows GNU
all-target/all-feature cross-compilation and strict Zensical pass. Mandatory
formatting/Clippy/Rust hooks passed **631 Rust + two renderer + 50 shared tests
(683 executions)**; three personal diagnostics remain explicitly ignored.
Source [`071c6b0`](https://github.com/sam-ruff/shep.so/commit/071c6b065cd236d1c821aa60734f635fca2541e2)
was pushed to main and exact remote equality verified. [Source documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34315151637) passed its build and Pages deployment. The final Reconnect navigation also passed through its native
button and the populated wizard was visually reviewed (`768ac1756273`).

This is an initial metadata/settings import, not continuous sync. Automatic
post-login enrollment, linking already-populated devices, local/remote changes,
conflict/removal controls, remaining portable settings and protected credentials
remain in TODO. Real Google/cross-client/Windows/macOS execution and performance
were not measured here. No personal installation, keychain or cloud data changed.
Quality/release CI stays disabled; documentation CI remains enabled. OAuth
implementation with the Flutter handover reference is still the first TODO item.

## 9 September: profile settings failure and close recovery

The final profile-control review found that an unreadable enrollment could leave
unsent checkbox intent blocking later close attempts. Controls now wait for their
local snapshot; failed, unadmitted intent is retained for explicit retry without
becoming a shutdown dependency. A failed status reload cannot silently retry a
write against the stale snapshot. Admitted saves/uploads still drain. The error
screen points to local data recovery instead of another Google login.

A controller regression covers unavailable initial settings, failed writes,
newer input, a failed reload and successful explicit retry. The owned
`invalid-local` MCP fixture covers disabled controls, visible errors, normal mail
navigation and graceful restart without replacing the opaque local record.
**18 selected native scenarios passed** after the functional correction. All
**six profile scenarios** passed again after the final recovery-copy change;
the 12 database/Google flows were unchanged by that text/empty-state adjustment.
Final invalid/reopened screenshots were reviewed (`159dcacff190`). The final
native executable SHA-256 is
`9944235f3c2b45ff2d35573fe5b21feb3c94041b1887ddf0b1b02ef3131dcbd0`.
Python tests pass 54/54; final Windows GNU cross-compilation is warning-free.
Source [`6860f50`](https://github.com/sam-ruff/shep.so/commit/6860f50230c6ab90a9cbae3ab9c96bca6fe5e6d2)
was pushed to main and exact remote equality verified. Mandatory formatting,
Clippy and full hooks passed **623 Rust + two renderer + 34 shared tests
(659 executions)**; the three personal diagnostics remain explicitly ignored.
Strict Zensical and [source documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34311219726)
passed, including Pages deployment. Quality/release workflows remain deliberately disabled.
The larger sync scope, personal installation, credential-choice and performance
limitations from the preceding setup checkpoint remain unchanged.

## 9 September: native initial-profile setup and channel ownership

R02/R49/R92 now expose reviewed first-device creation, persistent category choices,
Stop/Resume and enable/disable in Preferences. A separate bounded 32-command
coordinator processes local controls while provider work is held. Touched-field
changes and one in-flight UI save preserve rapid/newer choices and backend-owned
enrollment/disconnect changes. Intermediate enrollment progress keeps the job
owned; it is not upload success or permission to finish shutdown.

Stop and close cancel read-only HTTP/provider-slot waits. Admitted cache/history
writes and uploads remain owned through durable receipts; reopening resumes the
same saved profile. `profile-sync/` beside each workspace cache contains its Drive
and binding-specific history files. Database export protects the directory,
existing members, sidecars, ownership files and hard-link/symlink aliases.

Targeted Rust tests cover field ordering, stale progress, offline/disconnected
options, held reads versus admitted uploads and real export alias protection.
The saturated-provider dispatcher regression also saves actual profile choices
while every provider slot and its queue remain occupied. Five new saved native
MCP scenarios pass with an owned loopback Drive fixture: first creation/reopen,
retry/opt-out, category changes/held-read close, compact dark rapid gestures, and
close during upload followed by Resume and disabling/re-enabling. Reviewed WebPs:
`a26c2d013572` compact dark/review, `c883eeb38c8d` initial saved copy,
`b7c46dd88d3f` failed discovery and `c47727d6fb24` resume after close.
Source [`488a9ec`](https://github.com/sam-ruff/shep.so/commit/488a9ec4f2e8c197b8bd27299dd10961f750b2a3)
was pushed to main with exact remote equality verified. Mandatory hooks passed
**622 Rust + two renderer + 34 shared tests (658 executions)**, formatting and
all-target/all-feature Clippy. Three personal diagnostics remain explicitly
ignored. All **54 Python tests** and **17 selected native scenarios** passed
(the five new flows plus 12 affected database import/export and Google controls).
Windows GNU all-target/all-feature cross-compilation is warning-free; strict
Zensical passed. The tested native executable SHA-256 was
`5b7466cfcd418d56d487069521b67b79d093e5636f01e91d540b781c8efdbf65`.
The Windows warning correction changed only a `cfg(test)` local name after the
native run. No root runtime artifacts remain.
[Source documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34310152847)
passed build and Pages deployment.

This checkpoint provides initial setup, not continuous sync. Joining existing
profiles, actual account application/reconnection, conflicts, local edit capture,
remaining portable preferences and incremental polling stay in TODO. OAuth stays
first with its Flutter handover link. Desktop creation uses namespace `so.shep`;
same-project cross-client access remains unverified. Credential-transfer protection
has no recorded decision. No personal installation, live Google operation or
performance benchmark was performed. Logs live under ignored
`artifacts/logs/profile-controls-*`; quality/release workflows remain disabled.

## 9 September: persisted profile enrollment and first-device publication

R02/R49/R92 now have backend enrollment/category revisions and a durable initial
seed. A complete verified discovery review precedes explicit creation. The seed
freezes account mappings, values and operation IDs; each exact history request is
checkpointed before admission. Restart and lost acknowledgments reuse those IDs
and bytes. Initial publication uses the shared worker and both Drive journals.
A stop or Google disconnect during upload keeps its receipt, leaves setup pending
and reports that a copy was saved. Cache reads/edits remain available while the
fixture holds the provider response indefinitely.

Seven supported portable settings apply atomically against current local
preferences, account, Google and enrollment revisions. Invalid/conflicting pages
roll back; newer local edits and device-only fields remain intact. The explicit
account adapter maps security/authentication/Sent metadata for review only.
Database import archives source enrollment and seed data, including opaque future
records, and requires fresh device setup. A corrupt enrollment cannot prevent
Google disconnection or silently become an empty setup.

Isolated tests cover two-device settings transfer through actual HTTP and SQLite,
restart/repeated seed preparation, lost upload replies, complete/stale discovery,
stop/disconnect during a held upload, category changes, 70 legacy account mappings
across chunks, malformed seeds, atomic settings application and database-import
fences. Shared test HTTP responses now have a one-shot hold/release for causal
race checks. No production state locks were added; tests access no real cloud or
personal data.
The full Rust suite passed **617 tests** with three explicitly ignored personal
checks; 53 Python tests, Windows GNU cross-compilation and strict Zensical passed.
Seven existing native import/Google-disconnect scenarios passed against a freshly
built test executable. Reviewed WebPs include light import and restart
(`652e495620ae`), pending-action review (`86763300d459`), compact dark import
(`56996185079e`) and disconnected cached calendars (`917c9f116652`). These protect
existing controls; they are not native profile-enrollment or live Google evidence.
Final hooks and shipping are recorded below when complete.

This remains backend work. Native first/new/existing-device controls, actual
account application/re-authentication, local change capture, conflict/removal
reviews, remaining portable settings, production journal guards and incremental
polling remain in TODO. No engine/native entry point activates this setup yet.
The OAuth handover stays first in TODO. Password protection has no recorded
choice, and real cross-client Google access is unverified. No personal installation
or performance measurement was changed. Logs: ignored `artifacts/logs/profile-enrollment-*`.

Source [`6e2880b`](https://github.com/sam-ruff/shep.so/commit/6e2880bac6bd49777e41f9a8e44ccdbff4e12a1c)
was pushed to main, with exact remote equality verified. Mandatory hooks passed
**617 Rust + two renderer + 34 shared tests (653 executions)**, all formatting and
Clippy checks. Final Windows GNU cross-compilation also passed. The native test
executable was SHA-256 `9f62842bed91c601f144316736ce628efbebb764146be6625a51cd08506a09f8`;
the installed production app was not replaced. Compact Google disconnect was
also visually reviewed (`06a4c2a494e6`). No root runtime artifacts remain.
[Source documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34306619608)
passed build and deployment. Quality/release workflows remain deliberately disabled.
The full application goal and all remaining TODO entries stay open.

## 9 September: shared causal pull/publish bridge

R02/R49/R92 now connect the verified desktop Drive transport to the published
shared history worker. Complete listings feed bounded, exact-byte imports;
missing ancestry remains pending and ready work drains before publishing.
Publishing compares discovered, shared-history and transport-journal identities,
commits the original reservation, verifies the upload and acknowledges both
journals in order. A newer edit, foreign device proof or replaced scan requires
a fresh pull. Empty discovery cannot implicitly recreate an existing profile.

The desktop now uses the client branch's committed Drive file convention: private
appProperties, stable category and operation UUID filename. It consumes an
unchanged copy of the shared metadata/operation fixtures; Git attributes preserve
exact bytes on all platforms. This supersedes the earlier unconnected desktop
prototype convention. The shared dependency is pinned to published `9289f53`;
no client working changes or Cargo source cache were edited.

Eight bridge tests exercise two independent stores through production HTTP:
offline conflicts and explicit resolution, account/profile removal despite stale
edits, lost commit replies/reopening, gaps between both journals, foreign/stale
proofs, contradictory file identities, and 105 reverse-ordered ancestors across
multiple pages. The shared metadata fixture test and existing transport/journal
regressions also pass. Clippy and 53 Python tests pass. Final hooks, platform check
and shipping are recorded below when complete.

Cargo cannot directly test an external Git package with dev-dependencies. A new
runner copies the pinned shared crate/fixtures into an isolated temporary
workspace and uses a committed test lock. All **34 shared codec/history/Drive
tests** pass, including worker cancellation, independent-process ownership and
protocol failures. Hooks and disabled CI use that runner; two Python tests prove
exact revision selection and source/fixture isolation.

This is backend progress, not completed continuous sync. Persisted first/new/
existing-device enrollment, native controls/toggles, account/settings application,
local removal suppression, journal path protection and incremental pulls remain.
Live same-project Google visibility is still unverified; passwords stay outside
the metadata format pending the protection choice. No native UI, production
installation or performance measurement changed. Evidence is under ignored
`artifacts/logs/profile-replica-*`; see [the updated protocol reference](agents/profile-drive.md).
OAuth implementation remains first in TODO with the Flutter handover linked.

Source [`ace653b`](https://github.com/sam-ruff/shep.so/commit/ace653b1d466d0126a063bdda2c186644bd3a4d2)
was pushed to main; exact remote equality was verified. Mandatory hooks passed
**605 Rust tests, two renderer tests and 34 shared tests**, including the ordinary
locked isolated-runner path. Three personal-data diagnostics remain ignored.
Windows GNU cross-compilation, all 53 Python tests and strict Zensical passed.
[Documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34303450475) also passed.
This is not Windows/macOS execution, a native UI run or live Google verification.
Quality/release workflows remain deliberately disabled. No root runtime artifacts
were present after testing.

## 9 September: Drive profile transport and durable discovery

R02/R49/R92 now have a backend transport for the shared Rust/Flutter operation
format. Cargo pins the published client codec; exact JSON bytes and optional
fields survive download/upload. The connection checks the actual Drive identity
and granted scope. Profile files have a separate stable app-data category and
bounded metadata/content parsing; backup retention cannot select them.

A separate bounded SQLite worker persists discovery tokens, revisions, record
identities and exact reserved uploads. Discovery survives restart, rejects loops,
duplicates and stale/foreign pages atomically, and exposes only completed scans
in pages of 50. There is no total history page cap. Upload retries verify the same
reserved ID after lost/conflicting responses, including content verification when
Google omits its checksum. A previously acknowledged file that disappears is not
silently recreated. Accepted journal writes survive observer cancellation.

Verification uses the production HTTP implementation against a scripted loopback
server and isolated SQLite files. The shared fixture round trip, scope/identity
checks, malformed/oversized replies, interrupted upload/reopen, immutable
reservations, discovery restart and cancellation regressions pass. Existing Drive
backup protocol tests also pass after extracting the common bounded HTTP reader.
Final hooks and shipping are recorded below when complete. Earlier checks passed
51 Python tests, Windows GNU compilation and strict Zensical. No native UI changed;
no new native flow, live Google connection, performance measurement or production
installation is claimed.

**Continuous sync is still open and remains the top TODO priority**, with the
[Flutter handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md)
as the implementation reference. The shared causal-history worker, enrollment,
account/settings application, incremental polling, native controls, production
journal path protection and actual cross-client Google verification remain.
Passwords are outside this metadata format; the credential-protection choice is
still unanswered. See [the transport contract](agents/profile-drive.md). Evidence:
ignored `artifacts/logs/profile-sync-*`. Quality/release workflows stay disabled.

Source [`bb87ac2`](https://github.com/sam-ruff/shep.so/commit/bb87ac2e215bf0c7e798a76ec0acae1bdc12a9b9)
was pushed to main. Mandatory formatting/Clippy/test hooks passed **596 Rust tests,
two drawing-adapter tests and six shared-codec tests**; three personal-data
diagnostics remain explicitly ignored. The strict docs build passed, followed by
[successful documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34301648758).
The Linux app was not installed or exercised through native controls for this
backend-only checkpoint. The independently developing client transport must be
consolidated with this wire contract before any cross-client sync claim; the
codec alone is already shared, not the live Drive file protocol.

## 9 September: complete database import and local profiles

R83 adds **Backups → Database transfer → Import database** and **Accounts → Profiles**. A private staged copy is checked against supported v2/v3 schema, SQLite integrity/foreign keys and portable connection identities before account/count review. Confirmation consumes that exact copy, archives changed pending-operation metadata and publishes without overwriting the original workspace. Imports retain cached MIME/attachments, drafts, account/calendar definitions, cached events and portable settings. Pending sends, Sent uploads and bulk/folder changes become explicit review work; imported credential cleanup cannot delete local secrets. Notification setup stays quiet, and Google/automatic-backup state requires new-device setup.

The profile catalog owns a bounded 32-command SQLite worker, paged listings and revision-checked rename/selection. The current engine retains its Store/credentials until exit; the chosen profile opens on the next launch after normal saves. Publication is the import commit boundary: later cancellation/registration errors preserve the saved file, and recovery adopts its marker without duplicating it. Invalid unselected profiles report warnings without hiding valid ones. Export now also protects the catalog and every other profile's caches/journals/operation paths.

R91 gains one bounded 32-command credential thread shared by account, SMTP, CalDAV, Google, backup and removal adapters. Accepted OS writes retain ordering after observer cancellation; imported profiles receive a fresh device-owned credential namespace and never fall back to legacy secrets. Reserved keys, SMTP aliases and case-insensitive credential collisions are rejected before import/restore. This also fixes R32's discovered `caldav:<hash>` identifiers being rejected during encrypted backup restore; a complete encrypt/decrypt/engine restore test covers the calendar password.

Rust coverage includes validation/corruption/foreign schema, v2 migration, pinned-copy consistency, cancellation/drop, pending-operation fencing/rollback, publication/recovery boundaries, catalog paging/CAS/isolation, missing-profile behavior and credential worker ordering. The saturated-provider regression performs actual export/import review/cancellation while all provider capacity remains held. UI ordering tests preserve exact preference acknowledgments, review gating and stale-result/close behavior.

Verification: **580 Rust tests** (three explicitly ignored personal-data diagnostics), **51 Python tests** and **16 selected native flows** passed. Five new native imports cover invalid/reserved-ID retry, review cancellation, pending delivery acknowledgment without sending, held-copy navigation/close/draft save, compact dark catalog protection, rename and reopening both preserved workspaces. Three export and eight related backup/Google/preferences/inline draft/notification flows also pass. Light/dark/compact/review/error WebPs were reviewed. Windows GNU compilation and strict documentation pass; final hooks and shipping are recorded below when complete. This is not a full 199-functional-flow run or Windows/macOS execution.

Final native executable SHA-256: `eeb91c17a19e8542c11cb60ffd7bab19a666c6e76e8c84a38903212e8e2d0822`. Final import fixtures: `ca5906c88bf4` (cancel/invalid/retry), `da425891089c` (compact/protected catalog), `fc6f3309aa06` (held copy/close), `d593d7137d75` (pending actions) and `145ab0775c72` (review/rename/reopen both profiles).

Evidence is under ignored `artifacts/logs/database-import-*` and the native run directories. Performance measurements, live provider verification and production installation remain deferred. SQLite transfer is unencrypted, excludes keychain secrets and external backup-upload journals, and requires disk space for its private copy/WAL history. Imported accounts require reconnection. Continuous OAuth/Drive profile synchronization is still the top TODO priority, linked to the [Flutter handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md). Local profiles do not establish that protocol. Quality/release workflows remain disabled.

Source [`93d4289`](https://github.com/sam-ruff/shep.so/commit/93d42895a9cb3820efeabc710e110a5e82522db6) passed mandatory formatting, Clippy with warnings denied, **580 Rust + 2 drawing-adapter tests**, and Conventional Commit hooks. It was pushed to main and exact remote equality was verified; its [documentation build/deployment](https://github.com/sam-ruff/shep.so/actions/runs/34297303930) succeeded. No production installation changed. The database-transfer R83 entry is retired; encrypted local storage, large-mail streaming, protected credential sharing and continuous profiles remain separately tracked. The full product goal remains unfinished.

## 8 September: complete database export in Preferences

R83 now exposes **Backups → Database transfer → Export database**. The UI saves its displayed preferences and current/parked drafts before requesting a copy. A dedicated capacity-one channel owns export/cancellation independently of provider saturation and foreground cache reads/saves. SQLite online backup copies bounded page batches through a separate, pinned read transaction into a private temporary file, then publishes atomically. Existing destinations survive pre-commit failure/cancellation; a committed copy remains success if its final directory flush warns. Active cache/journal/lock paths and aliases are protected. No extra 256 MiB encrypted-backup payload ceiling applies to this complete database copy.

Five real SQLite tests cover exact original MIME, account definitions, drafts/attachments, unknown tables and pending metadata; concurrent writes retain snapshot consistency; cancelled completion observers, cancellation/drop/retry, existing destinations, path aliases and memory/missing-folder failures are covered. Three UI tests cover exact settings acknowledgments, newer draft revisions, attachment preparation, stale results, cancellation backpressure and errors cancelling close. The saturated-provider dispatcher regression now also completes an actual database export while every provider slot and its queue remain occupied.

Verification: **552 Rust + 2 drawing-adapter tests**, **50 Python tests**, and **7 selected native flows** passed. The three new export flows use real native Save/Cancel/Replace controls and inspect only owned exported fixtures. They cover a saved reply, compact dark protected-cache failure/retry, navigation/draft saves during a held real copy, cancellation cleanup and graceful close/restart. The other four preserve backups, inline reply sessions/attachments/restart, preference resizing and settings search. Light/dark/pending/error screenshots were reviewed. Formatting, Clippy with warnings denied, Windows GNU cross-compilation and strict documentation passed. This is not a full 194-flow rerun or Windows/macOS execution. Performance measurements and production installation were deferred.

Evidence: `artifacts/logs/database-export-*`. Final native executable SHA-256: `d2ca7eaeb38f300a85ba7eab0fbe8ab062eb35ae813e86838430e6b56bb4d96a`. Final export fixtures: `bd6146459f48` (saved SQLite), `28de94cea2bf` (compact/retry), `8fad0f1a664d` (held copy/cancel/close).

Source [`3927053`](https://github.com/sam-ruff/shep.so/commit/3927053f4b97f40caba85429fde1ede5fde249f4) passed the mandatory formatting/Clippy/Rust/adapter commit hooks and was pushed to main; exact remote equality was verified. No production installation was changed. A long pinned snapshot retains SQLite WAL history until completion/cancellation, so disk-space errors must remain visible; it does not retain the full database in application memory.

**R83 remains open for safe import and activation.** The SQLite file is unencrypted, excludes OS-keychain credentials and preserves pending-operation records; importing must isolate credential identities and prevent automatic replay on another device. It is not the existing encrypted backup archive. OAuth/profile implementation remains the top TODO priority with the Flutter handover linked; no working cross-client sync protocol or live Google verification is claimed. Quality/release workflows remain disabled.

## 8 September: mail-cache channel ownership

`store/worker.rs` replaces the shared connection/local-lease mutexes with one owning thread and a bounded 32-command FIFO. Accepted operations drain even after cancellation of their observer or the last Store handle; requests cancelled before admission never execute. Local leases release by closing a one-shot channel, including when the queue is full or the grant's recipient disappears. SQLite temporary selection tables and external process leases retain their existing behavior. Five deterministic worker tests cover these boundaries and failures.

The existing notification restart test reproduced a deadlock with bundled SQLite 3.51.1. The retained debugger trace shows WAL close waiting for SQLite's global Unix mutex while open waits for the inode mutex. Updating to rusqlite 0.40.2 / bundled SQLite 3.53.2 fixes the same test. This matches SQLite's documented [Unix deadlock correction](https://www.sqlite.org/releaselog/3_51_2.html); the selected version also includes the later [WAL correction](https://www.sqlite.org/releaselog/3_51_3.html). No application lock or registry-cache patch masks the fault. This is an isolated fixture reproduction, not proof of the cause of the personal account's remaining delays.

Verification: 544 Rust tests plus two drawing-adapter tests passed, with three live tests intentionally ignored; 49 Python tests passed. All 15 selected native scenarios passed, covering pending read/flag work, held sync, bulk/folder close and recovery, Undo, selection/paging, independent reply drafts/restart, preferences/resize, backups and removal. Synthetic restart/Preferences screenshots were reviewed. Windows GNU cross-compilation and strict documentation passed; this is not Windows execution or a full 191-scenario rerun. No performance measurements or production installation were performed. Quality/release workflows remain disabled.

Evidence: `artifacts/logs/store-channel-*`, including the failed-before Rust run and debugger trace, the fixed restart test and native run. Native executable SHA-256: `7900c32a5a44db8401059273bf3aa269e1543fe09ca12f3adf4e6a3cd01f6357`. Source [`db8c82a`](https://github.com/sam-ruff/shep.so/commit/db8c82aeef2c58f46c85b71c0723f072d297e2b3) passed the normal formatting/Clippy/Rust/adapter hooks and was pushed to main; exact remote equality was verified. R91 remains open for Google/lifecycle/backup-journal coordination. R83 full export/import and R02/R49/R92 continuous profile sync remain the next feature work; this worker foundation does not implement them.

## 8 September: cross-device profile handover and priority

The requested Flutter implementation handover is written in the sibling client worktree and shipped as [`02c4b32`](https://github.com/sam-ruff/shep.so/commit/02c4b32a6b380c1d5312c20bf187584c683ff10f) on `feat/mobile-web-clients`. [Read the handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md) for existing integration points, first/new/existing-device enrollment, configurable categories, the proposed shared format, conflicts/removal, credential protection and the required verification. Both TODO lists and restart notes now prioritize OAuth implementation and reference it. R83/R02/R49/R92 remain open: a document does not implement database transfer or continuous sync.

The client strict docs build, 31-entry scenario validation and mandatory formatting/Clippy/381-Rust-test hooks passed. The push was verified against the remote branch. Source credentials and personal data were not copied; actual cross-platform Google app-data access and the password-protection decision remain unresolved. No production installation was changed.

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

## R35/R77 — Inline replies and independent draft sessions (2026-09-08)

New messages, replies and forwards use the preview pane. Reply editors keep the
original conversation below, with a saved choice to include quoted text in the
outgoing message. Switching mail parks its editor; returning restores the matching
reply. Recipients, newer text and imported attachment copies survive navigation,
Preferences and a graceful restart. Collapse and close retain the draft. Explicit
Save leaves the editor open; send releases it after the durable outgoing receipt.

Each session owns its editor and one pending save revision. Background results
cannot replace another draft, cancel a pending close by reopening a reply, or
clear list selection. Window close drains every owned draft. Removal reviews wait
for related draft/file persistence, and discard cannot race an attachment import.
Find and HTML reflow address the inline scroller. Native editing focus preserves
text selection and protects text fields from destructive mail shortcuts.

Source commit: `e16590c`. Its normal hooks passed **541 Rust/adapter executions**
(539 Rust tests and two pixbuf adapter tests); **49 Python tests**, formatting,
Clippy and strict Zensical also passed. The migrated **188/188 native functional**
run passed, followed by all six inline scenarios on the final executable. The final
**191/191 native functional** run passed on the committed source, which was pushed
to `main` as `e16590c`. Logs: `artifacts/logs/inline-composer-commit.log`,
`inline-composer-python.log`, `inline-composer-native-final-full.log` and
`inline-composer-docs.log`. The native binary SHA-256 is
`d866d418c7ddc0d8125de4c6634e667fc3b082b539a85dc8830713def4d504ca`.

Reviewed evidence includes compact light/dark typing, red discard confirmation,
reply switching/restart, original-message Find, long-thread paging and a delayed
fixture send refusal that leaves another reply editable. Functional runs omit
performance gates as requested; no new latency claim or production installation
is implied.
The installed production executable remains the previous baseline. Native tests
use fictional accounts; live SMTP and Windows/macOS execution are separate work.
Temporary saving-tray behavior and the wider close-dependency audit remain R86/R90;
provider rekey/recovery integration remains R73. Message-size ceilings remain R23.

## R90/R91 — Channel-owned account scheduling (2026-09-08)

Full account sync previously held the account mutation mutex across the entire
provider download/cache cycle. A read-on-leave change could wait behind a stalled
download and keep the window from closing. Commit `34cfc71` replaces account and
calendar mutex maps with a coordinator that owns its state and receives bounded
requests. Writes interrupt read-only sync; queued writes retain order, independent
accounts proceed separately, and abandoned requests release their place.

Sync now drains already-started cache writes before releasing account ownership,
including after provider error, timeout or cancellation of the refresh owner.
This also fixes a `try_join!` path that could detach an active SQLite write and
let stale cache work finish after a newer mail change. An interrupted check is
not reported as a complete folder listing or Inbox baseline.

Verification: normal hooks passed Clippy, formatting and 528 Rust/adapter test
executions, including ten new coordinator/download regressions. Python passed
49/49. The optimized native fixture build and strict Zensical build passed. The
three new native scenarios cover read-on-leave during graceful close, flagging
while the provider is held indefinitely, and failure/rollback/retry. The fixture
rejects every retry in its failure mode; intermediate optimistic state is not a
successful save. Final WebP captures were reviewed. The complete functional native suite passed
185/185 on the isolated checkpoint binary (`artifacts/logs/sync-checkpoint-native-full.log`).

This source checkpoint does not install a new production executable or establish
live-provider/Windows/macOS runtime behavior. No performance measurements were
run. R86 temporary saving tray, remaining shutdown dependencies, personal-account
diagnosis and the wider channel-ownership audit remain in TODO. R35 inline
composer work is preserved separately and is not included in this commit.

## R85 — Credit-saving handover; R35 state foundation (2026-09-07)

The user stopped feature development and requested a handover, TODO cleanup and
push. [handover.md](https://github.com/sam-ruff/shep.so/blob/main/handover.md)
records the installed baseline, current code, remaining work and restart order.
TODO is condensed without removing unfinished requests; R84 cross-folder search
remains delivered. The complete product goal is still unfinished.

The current source separates composer metadata/editor state from other forms,
retains autosaves when another form opens, coalesces edits behind one pending save,
and waits for the newest revision on close. Late attachment and durable-send
results preserve the appropriate draft/form. Five new controller regressions and
a saved native Preferences/graceful-restart scenario cover this foundation.
The composer still uses a modal; inline presentation and multiple active draft
sessions remain R35. This checkpoint is not installed as a production release.

Verification: targeted composer Rust tests passed 14/14; Python tests passed
48/48; the strict Zensical build passed. Three existing native scenarios passed
(save/reopen, recipients/files and cross-folder search), and the new graceful
restart scenario passed its corrected rerun. Its first version incorrectly tried
to open Preferences while the existing modal was still open; the saved scenario
now closes/saves that modal first. Dark attachment/restart captures were reviewed.
The full native suite and release build/install were not rerun for this handover.
An accidentally unfiltered functional run was stopped; it is not full-suite evidence.

The checkpoint commit is `fix(drafts): isolate composer state and record handover`
(the commit introducing the root handover file). Its normal pre-commit hook
enforces formatting, all-target/all-feature Clippy, the full Rust suite and the
drawing-adapter tests. Logs stay in ignored `artifacts/logs/handover-*`.
No performance measurements or live-provider/platform-runtime checks were run.
Private local tool settings and ignored artifacts are excluded from the push.
R85 is completed by this handover push; R86 close-to-tray is tracked only, without
implementation. Quality and release workflows remain disabled.

## R30 — Native folder controls installed and pushed (2026-09-07)

Source [3567de2](https://github.com/sam-ruff/shep.so/commit/3567de29847d81d29c8882269c82a6aa7fa986d4)
is installed for the Linux user and pushed to `main`. Git hooks pass, including
formatting, Clippy, the Rust suite and two drawing-adapter tests.
Native sidebar right-click and Shift+F10 menus now open Move/Delete reviews.
Parent search ranks readable folder names and highlights the Enter target;
choosing a parent opens an explicit subtree/count review before changing it.
Delete has a red confirmation. Accepted input projects the folder tree immediately,
keeps moved mail readable from its original cache and permits navigation while
provider work continues. Rejection removes the projection and restores the prior
view only if later navigation has not superseded it.

The durable folder runner uses local read/staging queues and the existing
coalesced worker/close barrier. Local POP3 folders initialize without guessing
hierarchy for literal slash names. History retains decoded source/destination
labels after moves. Monotonic job revisions reject old history observations;
separate recovery request state prevents a late history read from unlocking an
active button. In-process leases now also exclude competing memory-store workers,
without creating root lock files. A regression reproduced an older workspace
restoring the previous folder tree after a committed move; the revision guard
now retains the new tree alongside the new folder names. Close also stops a
queued folder job while provider capacity remains occupied, preserving queued
steps without waiting for unrelated network jobs.

Seven saved native scenarios cover slow move/navigation/restart, rejected delete
and retry, unconfirmed move/explicit acceptance with retained cache, local POP3
move, compact dark review/keyboard/Inbox protection, graceful partial-delete close
and resume, and failed-close navigation/retry followed by successful close. Initial
new tests used lowercase SQLite statuses instead of JSON enum values and clicked
a resized sidebar before its layout settled; their corrected equivalents pass.
Neither correction changes application timeouts or performance budgets.
The broader context-menu scenario now focuses its message row before Shift+F10:
with the sidebar still focused after browsing, that key correctly belongs to
folder controls. The actual mail-menu mouse and keyboard assertions remain. Reviewed
WebPs show the compact red confirmation, pending moved folders, actionable history
and preserved originals. No live provider, personal cache or OS credential was used.

The full native run passed **180/181** scenarios in 702.7 seconds. Its one failure
was the focus assumption above. After the late stale-tree/queued-close fixes,
the final rebuilt binary passes **18/18** targeted native scenarios: the corrected
mail menu, all seven folder controls, all five folder-tree scenarios and five
search/ranking/focus scenarios. This is not a single clean full run; the final
focused rerun follows the earlier full run.
**511 Rust test executions**, **48 Python tests**, Clippy, Windows GNU compilation,
strict Zensical and the optimized release archive's checksum/extraction/installer
checks pass. Production compilation caught a test-only observation call missing
its feature guard; the release now compiles with fixtures disabled. Performance
measurements remain deferred. The optimized production binary is installed
atomically for the Linux user; existing windows were preserved and need reopening
to use the updated executable.

Evidence is under ignored `artifacts/logs/folder-controls-*`. The full native
binary was `f2ddff0212b6437231ca51b6eb8ce0a479461e4b739fce3016cb9ba129d430b8`;
the final native binary is
`5b7d2d81a11e01081a35c853b13fcae274ae9b726f3b88b30e900c83f480b1e0`.
Installed production SHA-256:
`06c0cccb3d3cc6703b143f8e7fa019c1be7032533ae6d4e776a81cc6f91ef34a`.
Reviewed native evidence includes `b1c2028d31b9/folder-destinations.webp`,
`c06d677b277f/folder-delete-history.webp`, `8f382ebe5ac2/folder-delete-dark-compact.webp`
and `b0451d12173f/folder-uncertain-accepted.webp`; the final search runs are
`a2e17065657d/` and `790d14014d97/`. These are fictional fixtures. No logs or
SQLite files were left at the repository root. Quality/release CI remain disabled. R30 remains open for combined-folder optimistic
scope, aggregate common-folder account choice, wider account/history pagination
coverage and cache/server convergence after an accepted unconfirmed operation.
Accepting uncertainty is explicitly recorded as stopping work, never as a
confirmed move/delete.

## R63 — Native keyboard ordering installed and pushed (2026-09-07)

Source **b45177792c84246308da78451684820aa70cef2a** moves key presses from the asynchronous event subscription to
the root native widget's message stream. Keys retain their order relative to
later mouse controls. Native widget operations snapshot search/Find focus during
that event; mail actions cannot use the focus of a later click. The old async
key focus-check messages are removed. Find Enter retains event modifiers and
ordinary text selection, pane scope and remapping remain protected.

The new MCP `key_sequence` action batches bounded native key chords. Its saved
rapid Move/Escape scenario fails on the prior installed binary by leaving Move
open. With the new input path, it and 11 other targeted native flows pass: repeated
navigation/flags, Escape then recovery Review without an intermediate wait,
search/Find Ctrl+D followed by another click, Find, remapping, selection and
context menus. Three direct native-widget tests cover ordered key/click output,
focus at each event and Find Enter/Shift. An initial test incorrectly expected
an Enter without an on-submit callback to be captured by iced; the assertion
now reflects the actual widget contract and still requires the correct focus.
Formatting, Clippy, Git hooks, **495 Rust tests plus two drawing-adapter tests**,
**47 Python tests** and Windows GNU cross-compilation pass. Visual review caught
a new test accidentally double-clicking into the full reader; alternating inbox
rows and explicitly checking the reader remains inline preserves the intended
text-field/later-click regression. The strengthened test passes.

The full native run passed **172/174** scenarios. One failed before keyboard input
because the long HTML body did not become ready within its existing deadline;
three unchanged targeted reruns pass. The other ran the earlier repeated-click
scenario, which inadvertently entered the full reader and no longer clicked inbox
rows. The corrected scenario alternates rows, checks the inbox stays open, and
separately tests Find through actual field/body clicks in the full reader. The
final rerun of all three new input scenarios plus the original Find scenario
passes **4/4**, with no changed timeouts. All **174** scenarios have passing
coverage across the full run and targeted reruns on the same native binary;
this is not a single clean full run. The HTML readiness timeout remains recorded
as intermittent evidence, without attributing it conclusively to host load.

The optimized production archive passes checksum, extraction and bundled installer
verification. Source **b45177792c84246308da78451684820aa70cef2a** is installed for
the Linux user and pushed to `main`, with the strengthened native test and this
evidence in the following audit commit. Pinned strict documentation building passes.
The full product goal and R63's final functionality-path audit remain open.

Native test binary SHA-256:
`4a3d14e0b8e437cf59fbcd4efeb6ccbc76742f6c0810e90b146fe38c5012173c`.
Installed production binary SHA-256:
`7b85b2fa131e0064933724b3022d672fd4db11d5034b84411182b58542982e79`.
Already-open windows need reopening to use the update.

Reviewed WebP evidence is under ignored `artifacts/e2e/b7c44e3152f7`,
`b144d2370089`, `47e5508a7b49`, `62aaf00e6a37`, `17144fd92471` and
`35c641d8f01f`. This includes the final inbox/full-reader isolation and light/compact
dark cross-folder search controls. Evidence under ignored `artifacts/logs/`:

- `native-input-before-fix.log`, `native-input-after-fix.log`, `native-input-full-native.log` and `native-input-final-native-rerun.log`.
- `native-input-click-isolation-targeted.log`, `native-input-full-reader-isolation.log` and `native-input-find-rerun.log`.
- `native-input-commit.log`, `native-input-final-python.log` and `native-input-windows-check.log`.
- `native-input-release.log`, `native-input-install.log`, `native-input-docs-final.log` and `native-input-push.log`.

No performance measurement or personal-provider operation was performed. Quality
and release CI stays disabled; documentation publishing remains enabled. No root
log/database artifacts remain, and unrelated untracked work is preserved.

## R74 — Manual refresh installed and pushed (2026-09-07)

Source **80f5867bcde7cb003f0d1e1b9b255974a68baf93** is installed for the Linux
user and pushed to `main`, alongside **2b4c48023863ee254e9422d9a347e91e6f57f641**
for native test synchronization. R74 is complete and removed from TODO. The full
product goal and remaining backlog stay active.

Refresh defaults to Mod+R plus F5. Its v2 migration preserves custom keys,
explicit clears, disabled actions and conflicts, including after restart. Manual
mouse/keyboard refresh animates the existing top-right icon immediately; automatic
checks stay still. Coalesced requests keep their phase until the scheduler finishes
all manual work, including failure. Hidden mail headers stop the frame timer;
frame updates bypass mail scheduling/body preparation and handler timing samples.

Native pixel checks exposed the software renderer using rotation matrix diagonals
as both raster dimensions and screen positions. The fix caches an unrotated SVG
at physical scale, applies its complete transform, and clips against its viewport,
layer and damage region. A direct regression fails before the fix. All 12 drawing
regressions pass, including fractional scaling and partial redraws without trails.
The first fractional-edge assertion incorrectly rejected a partly covered boundary
pixel; it now checks pixel/viewport intersection, retaining the pre-fix failure.

**Verification:** formatting, Clippy, both commits' hooks, **492 Rust tests plus
two drawing-adapter tests**, **46 Python tests**, Windows GNU cross-compilation
and pinned strict docs build pass. The release passes checksum, extraction and
bundled installer verification; the installed binary matches the release hash.
No performance measurements or live-provider/Windows/macOS delivery claims were
made. Quality/release CI remains disabled; documentation publishing stays enabled.

The full native run passed **170/171** scenarios. Its sole failure was the existing
recovery scenario's earlier Escape reaching the app after its subsequent Review
click and dismissing that dialog. The saved trace established the ordering. The
scenario now waits for native search focus to clear before its independent click,
and for Refresh to start and finish before restart. That corrected scenario and
three refresh scenarios pass in the final rerun. All **171** scenarios have passing
coverage across this full run and corrected rerun on the same binary; this is not
a claim of a single clean full run. The rapid-input ordering bug remains R63 work.

Native coverage includes F5/default primary/remapping/clearing/restart, actual
rotating versus background-still icon pixels, queued manual work, failure/retry,
mail navigation and tab switching while pending, light/compact dark and 120% scale.
Reviewed WebPs are under ignored `artifacts/e2e/fd965fb20685`, `e6b31317bc06`,
`b31e96ef7e45` and `c5f3e1d58496`. Logs are under ignored `artifacts/logs/`:

- `refresh-svg-before-fix.log`, `refresh-svg-after-fix.log`, `refresh-animation-native-fixed.log`.
- `refresh-animation-native-full.log`, `refresh-animation-final-native-rerun.log`.
- `refresh-animation-commit.log`, `refresh-animation-test-commit.log`, `refresh-animation-python.log`.
- `refresh-animation-windows-check.log`, `refresh-animation-docs.log`, `refresh-animation-release.log`.
- `refresh-animation-install.log`, `refresh-animation-push.log`.

Native test binary SHA-256:
`442566e90c0f4abb131b279fc2769302323f0671513618fed68aa0d5a52e2099`.
Installed production binary SHA-256:
`6f907c2897a71a6472694c3523eae34672148284c2172b14d1d6ccb524708f6f`.
Already-open windows need reopening to use the update. No root log/database
artifacts remain; unrelated untracked work is preserved.

## R81/R76 — Reading styles installed and pushed (2026-09-07)

Source **1bf6ac9cdd9127c169b74923aab9a63f5f8cf196** is installed for the Linux
user and pushed to `main`; **cc20ce03799eca61dbbe937e7e8cae28a2c4bd22** updates
the existing native copy test for the centered text position. R81 and R76 are
complete and removed from TODO. The full product goal remains active.

Plain text has a padded, centered column that follows the text-size preference.
Simple HTML gets equivalent low-priority CSS defaults. MIME preparation
classifies typography/color-only letters off-thread; tables, explicit dimensions
and layout CSS retain sender geometry. Sender CSS can override defaults.
Expanded conversation cards take the active email's opaque background and
matching control colors. A stable themer/container tree preserves the scroller
and input state as rendered backgrounds arrive or cached messages change.

**Verification:** 486 Rust tests plus two drawing-adapter tests, 46 Python tests,
formatting, Clippy, Git hooks, Windows GNU cross-compilation and pinned strict
documentation build pass. The optimized archive passes checksum, extraction and
bundled installer verification. The installed binary matches its release hash.
This does not establish Windows/macOS native execution or live-provider testing.

The full native run passed **168/169** scenarios. Its only failure was an older
text-copy drag aimed at the previous body position. After updating those mouse
coordinates, that same copy/paste/full-reader/dark scenario and both new reader
scenarios pass in `reading-column-native-final-rerun.log`. All 169 scenarios have
passing coverage across the full run and corrected rerun on the same binary;
no production code changed after the full run began. Selection, copying and
read-only shortcut assertions remain intact. No timing budget changed.

Rendered Find/selection geometry proves padding and centering at 340/1000/1600 px
and 14/22 px fonts. Native flows cover plain/HTML selection, Find, actual column
pixels, full/compact layouts, contrasting conversation backgrounds, repeated
cached switching and refresh/scroll. Two initial new-scenario setup errors
(clicking above the plain editor and appending to a retained Find query) were
corrected before the full run. The existing conversation-action, HTML image,
scrolling, resize and background regressions remain covered.

Reviewed WebP evidence under ignored `artifacts/e2e/` includes `5702d689ea4e`
(plain full width), `53782149d020` (HTML full width), `871baa7be21d`
(compact letters), `4b26fba9eb20` (compact dark conversation), and `c63c760ec908`
(corrected copy regression). Logs are under ignored `artifacts/logs/`:

- `reading-column-full-rust.log`, `reading-column-clippy.log`, `reading-column-python.log` and `reading-column-geometry.log`.
- `reading-column-native-full.log`, `reading-column-native-final-rerun.log` and `reading-column-commit.log`.
- `reading-column-windows-check.log`, `reading-column-docs.log`, `reading-column-release.log`, `reading-column-install.log` and `reading-column-push.log`.

Native test binary SHA-256:
`77e5f50ce98c63e8c8279acd689a627a7ee01312f09cda1809b47a8faeab799a`.
Installed production binary SHA-256:
`4f61267e133680817782cbc011908f378587cfd11ee2dd28b9bd3fc63cd69e39`.
Already-open windows need reopening to use this executable. Performance
measurements remain deferred; quality/release CI stays disabled. Documentation
publishing remains enabled. The reader checkpoint did not include the subsequent R74 refresh update,
whose shipping evidence is recorded above.

## R84 delivered; R73 recovery checkpoint shipped (2026-09-07)

Source commit **a81d767d0d687338755ec1b76b807cb97e5b6635** is installed for the
Linux user and pushed to `main`. The full product backlog remains active.

Interactive search now spans cached folders within the selected account scope,
including combined folder views. Results display their folder. Clearing search
restores the browsing folder and usual sort. SQLite queries, frozen selections
and optimistic move membership share the scope rule. Read/flag/attachment
filters remain explicit. Search retains a moved result that still matches, and
long preview text clips within its space without overlapping folder labels.
R84 is complete and removed from TODO.

Recovery is available from the cached reader and Preferences → Accounts. Retry
checks a confirmed destination; unconfirmed moves require explicit review before
using an existing, byte-verified copy. Keeping a local original requires
confirmation and never changes server copies. Its new local identity preserves
content/flags through restart and sync and retires the old server Undo action.
Removing a destination account retains another account's original with this
same identity protection. Closing waits for an active recovery receipt; failure
cancels that pending close.

**Verification:** all **167 native functional scenarios**, **484 Rust tests**
plus **two drawing-adapter tests**, **45 Python tests**, formatting, Clippy and
Git hooks pass. The Windows GNU cross-target check and pinned strict docs build
pass. The optimized production archive passes checksum, extraction and bundled
installer verification; the installed executable matches the release hash.
Already-open windows need reopening to use the new executable.

Final evidence under ignored `artifacts/logs/`:

- `recovery-and-search-final-native-full.log`: 167 scenarios, all passed.
- `recovery-and-search-commit.log`: formatting, Clippy, Rust/adapter tests and commit hooks.
- `recovery-and-search-final-python.log`, `recovery-and-search-final-windows-check.log` and `recovery-and-search-final-docs.log`.
- `recovery-and-search-final-release.log`, `recovery-and-search-install.log` and `recovery-and-search-push.log`.

The first native run passed 163/167: three assertions expected the old
folder-only search count; one recovery click was followed by delayed Escape.
Corrected scenarios pass individually and in the clean full rerun. The recovery
scenario waits for native search focus to clear before its independent click;
rapid input cancellation remains tracked under R63. No timing budget was changed.

Seven recovery scenarios cover explicit confirmation, delayed success and
failure/retry with navigation, local flags/copies after restart, Preferences,
compact dark layout and graceful close with read-only receipt inspection.
Three new search scenarios cover non-Inbox results, opening/moving, exact bulk
membership, clearing search and account scope. Reviewed WebP evidence includes
`66bd98906785`, `b9b00e6f29ae`, `50d3bdc7497e`, `45e02c844b68`, and
`2b9ce6702a98` under ignored `artifacts/e2e/`. Fixtures never contact personal
providers or OS credentials. No new performance measurements were run.

Native test binary SHA-256:
`a9d68d7925147b37d7004a251a852a4c11e6d17ed229c7d031c7a57e2644b5ff`.
Installed production binary SHA-256:
`badcb12f3ca055135741aa0675ebe5a7dca2022d0daa8b8d3e76e8e41e9e9fd9`.

R73 remains open for actual adapter wire/journal integration, broader
Undo/group/folder-history lifecycle checks, repeated-move aliases and the live
A. Keep report. Cross-compilation is not Windows GUI verification. R83 database
export/import remains tracked separately and is not implemented by this work.
Quality/release workflows remain disabled; documentation publishing stays enabled.

## R73 — Durable recovery work in progress (2026-09-07)

These foundation-stage notes preceded the combined a81d767 checkpoint above.
See that checkpoint for current shipping evidence; R73 and the full product goal
remain active.

The working tree connects IMAP moves/transfers to a durable journal before the
first provider write. It protects original MIME through sync/restart, retains
APPENDUID and incoming connection identities, migrates old transfer tuples
atomically and resolves confirmed destinations without repeating MOVE/APPEND.
A known UID completes the cache without another network request; a missing UID
returns the acknowledgment before background lookup. Copied transfers verify
exact destination bytes and identity before retrying source cleanup. Timed-out
or unconfirmed results keep their originals. A tagged MOVE NO can have partial
effects, whereas unsuccessful APPEND is atomic; the protocol tests preserve this
distinction ([MOVE semantics](https://www.rfc-editor.org/rfc/rfc6851.html#section-3.3),
[APPEND semantics](https://www.rfc-editor.org/rfc/rfc9051.html#section-6.3.12)).

Destination search/pages retain protected cached mail after restart; provider
IDs are withheld until resolved. Both new and older bulk selections exclude
protected identities. Folder navigation includes the pending destination.
Cache relocation and journal completion commit atomically, retaining originals
on conflicts. The reader follows acknowledged aliases when a page or body read
overtakes its completion event; a native screenshot exposed a late old-ID error
toast and the corrected detail lookup removes it.

Foundation-stage verification: **474 Rust tests**, **44 Python tests**, Clippy and formatting
pass. Ten store, eight runner and added protocol/controller tests cover original
protection, cache collisions, copied/committed restart, stale writes, changed
connections, bad lookup identity/content, legacy migration, bounded retry/pages
and provider-safe selection. Logs: `move-recovery-verified-rust.log`,
`move-recovery-verified-clippy.log`, `move-recovery-final-python.log` under ignored
`artifacts/logs/`. No performance measurements were run.

Four targeted native scenarios passed in `move-recovery-native-targeted.log`.
After correcting the toast, the saved cold/restart/refresh scenario passes again
in `move-recovery-native-verified.log`; its assertion now includes an empty
notice. Reviewed WebPs are under `artifacts/e2e/daabe132d246/` (restarted/located
reader), with the initial reproduction under `4e818eb298d6/`. These use owned
fixtures, not a live account or keychain. The whole 158-flow native suite has not
been rerun for this working tree. The corrected native test used binary SHA-256
`255ac6c42f4500aae2fd5d38c5f39aec674a12d44be32e4287600c01b6b7a763`.
The pinned strict documentation build also passes (`move-recovery-docs.log`).

At that foundation stage, remaining work included visible retry/review controls
for Copied/Started/failed lookup states and actual adapter wire/journal integration,
Undo/bulk/folder-history lifecycle review, complete native failure/recovery
coverage and release/install/push checks. Do not infer these from the fixture
lookup or the passing prior 157-flow shipped run. The new full-database export
request remains separately tracked as R83.

## R73 — Pending destination checkpoint — installed and pushed (2026-09-07)

Source [acb33c0](https://github.com/sam-ruff/shep.so/commit/acb33c0d20f547f626c2aa934190858b30cf446e)
is installed and pushed. The saved slow-provider native reproduction failed before this change: Projects
only contained the moved message after the provider acknowledgment. Per-read
SQLite projection now includes pending moves in the destination's full-text
search, sorting, filtering and paging. It leaves persisted source data intact.
The reader can reuse a cached body, blocks provider actions on a temporary ID,
and adopts the acknowledged destination identity without clearing that body.
Unified Inbox membership remains stable when moving between accounts' Inboxes.
Failure and Undo remove the projected destination; Undo can also cancel a move
waiting behind a flag save without cancelling the flag.

Six controller and four storage regressions pass. Three saved native scenarios
pass for pending reading/return to Inbox, failure/pending Undo and cross-account
dragging; light destination/failure screenshots were reviewed. Native evidence:
`e854518c111b`, `1a78176994a9`, `e693b0450662`, `14728d75d5b1`, `1a9059fdd1e2`.
All 454 Rust tests, two drawing-adapter tests and 44 Python tests, Clippy and
formatting pass. Optimized production build, archive checksum/extraction and
bundled installer checks pass. The full native run passes **157/157 functional
scenarios** in 585.869 seconds (`move-projection-native-full.log`), using binary
SHA-256 `7446f68841cf674ba44998444521d29f1e03b26d36b21ebc046b90fd4463487b`.
Windows GNU cross-target compilation passes; this is not Windows runtime evidence.
Git hooks pass. The installed production binary matches the verified release,
SHA-256 `c6ff9c86421c5800d1132ff63a708104b86f25b9ea626a5e578470e84df37d66`.
Installation was atomic; existing personal windows were left running on their
previous executable and need reopening. Logs use the `move-projection-` prefix
under ignored `artifacts/logs/`, including `commit`, `push`, `install`,
`release-verified`, `windows-check`, `all-rust`, `python` and `docs-final`.
Documentation publishing for the source commit succeeded in run `34125535804`.
Timing measurements remain deferred. Quality/release workflows remain disabled.

R73 stays open: acknowledged moves without COPYUID and cross-account retry
journals can still lose the destination identity. Durable preservation and
resolution of those acknowledged copies need their own protocol/cache/restart
checks. This checkpoint does not claim those cases or the personal-account
report are resolved.

## R82 — New-mail notification checkpoint — installed and pushed (2026-09-07)

Source [ded5aca](https://github.com/sam-ruff/shep.so/commit/ded5aca104b5fa129f5b9c8d89199f3a826e9f96)
is installed and pushed. Native delivery adapters and searchable Preferences
controls are implemented.
Popups, sound and sender/subject details default on and can be changed
independently. Test notification uses the current settings; muted or failed tests
release their pending state. Delivery runs separately from mail sync and iced,
coalescing bursts into a counted notification without retaining each body.
Failures remain visible and navigation stays available.

SQLite records per-account message identity and initial-import readiness.
First imports and UIDVALIDITY resets remain quiet until their Inbox completes;
new unread Inbox messages alert at most once. Repeated syncs, restart, read/flag
changes, restored mail and moved copies do not turn into new arrivals. Claims
are committed with cached mail: a process crash before OS delivery can lose that
alert, but does not replay old alerts. Shep must be running.

Linux uses its desktop notification protocol and system sound theme. Windows
uses a per-user Shep AUMID with WinRT, and macOS uses the Shep bundle identity
initialized once. Popup and sound-only paths remain independent. No adapter
borrows another app's identity. Desktop permissions, Do Not Disturb and sound
settings can prevent presentation despite an acknowledged request.

Protocol failure tests exposed upstream async-imap SEARCH/FETCH helpers accepting
rejected commands as empty success. The sync path now requires matching tagged
OK, including after partial data, before returning results or reconciling cache
membership. Inbox completion is independent of a later logout failure. These
checks use production sync functions against scripted IMAP/POP3 connections.

Validation includes 444 Rust tests (three explicitly ignored live/profile tests),
44 Python tests, and fmt/Clippy. Seven new SQLite tests and ten worker/UI/protocol
tests cover import/restart identity, commit rollback, burst counts, mute/privacy,
blocked delivery, private-bus sound hints and errors, and sync failure ordering.
Windows GNU `cargo check` passes with both all features and production defaults;
it is not Windows execution.
The MCP fixture never calls the host notification/audio service. Four saved native
scenarios exercise defaults, independent outputs/privacy, restart, real arrival
flows, compact dark layout and navigation during delayed failure/retry.

The complete native run exercised 154 functional scenarios: 153 passed and one
shortcut test observed a stale dialog before its close completed. That scenario
passed in isolation; it now waits for actual close/open transitions. The rapid
M/Escape asynchronous-focus ordering edge case remains tracked under R63 rather
than being claimed fixed by a test wait. This is not a clean 154-case full run.
All four notification flows passed in the full run on native binary SHA-256
`0fba6d1bb2a5a9afe5aba851e22db29d0fa55a690cd8bd3a62ff7a851ab37963`.
Reviewed WebP evidence includes popup-only/privacy (`5404d1b0741a/`), compact dark
(`7fabfa3c966f/`) and delayed error/recovery (`064b37d21d81/`), under ignored
`artifacts/e2e/`. Logs in `artifacts/logs/` include `notifications-full-rust.log`,
`notifications-final-targeted.log`, `notifications-python.log`,
`notifications-native-full.log` and `notifications-native-recovery-current.log`.

The corrected modal-transition scenario passes in `notifications-shortcut-transitions.log`.
Final Git hooks pass 444 Rust tests plus two drawing-adapter tests, fmt and Clippy.
Strict Zensical, optimized release checksum/extraction and bundled-installer
verification pass. The installed Linux binary matches the release, SHA-256
`9754ccd700b5dc60cd1ed3eb990955406d231dd591b10915e905e9023fb4cec8`.
Installation was atomic and existing personal windows were preserved; reopen
those windows to use this build. Shipping logs: `notifications-commit.log`,
`notifications-release-verified.log`, `notifications-install.log` and
`notifications-push.log`. Source and docs publishing succeeded in GitHub runs
`34121452515` and `34121562739`. A read-only desktop capability query reports
sound support (`notifications-desktop-capabilities.log`); this is not evidence
of a displayed popup or audible alert. R82 remains
open for actual Windows/macOS delivery, Mac app-bundle integration and desktop
sound/popup review. Other performance measurements remain deferred. Quality and
release workflows stay disabled; documentation CI remains enabled. The full
product goal and remaining TODO entries are still active.

## Complex HTML and reader interaction follow-up — installed and pushed

Source [ebddf54](https://github.com/sam-ruff/shep.so/commit/ebddf54c0124c30561db117c2b041ee184aa4c3c)
is installed and pushed. R72, R78, R79 and R80 are removed from TODO only after
that verified shipping step.

R72 was reopened after the earlier synthetic result failed to explain the user's
1–2 second pause. Read-only diagnostics reproduced it in the original cached
mail: deeply nested tables repeatedly measured identical subtrees. Two messages
that took 1,538–2,107 ms now render in 47–63 ms. A later paired comparison turns
only table reuse off/on and proves identical viewport pixels and heights for
both corrected documents. One message differs from pristine upstream pixels
because of the tested superscript-offset correction. No personal content,
addresses or hashes were added to public fixtures, screenshots or documentation;
temporary diagnostic copies were removed.

The vendored litehtml patch reuses complete containing-block constraints within
one normal-flow render, without crossing resize/image/media changes or positioned
layout. Inline fragments apply relative offsets once; caption displacement no
longer accumulates on row parents. Seven renderer regressions compare pixels,
selection, spans, captions, floats, positioning and reflow, plus exact 5px/2px
inline offsets and caption heights (including the border-height edge case).
The deep fixture verifies a reduction from more than 10,000 table layouts to
fewer than 200, independent of host timing. Licenses and patch provenance are
bundled in the release archive.

Visited frames retain their actually decoded image inputs within eight frames /
32 MiB; the shared WebP cache is bounded independently. Image invalidation is
URL-specific, and a failed replacement cannot acknowledge undisplayed bytes.
Actual visits also refresh the body LRU. The native pixel benchmark now has
eight cases with twenty observations each; all pass unchanged 100 ms cold / 50 ms
cached gates. The deep template measures 38.2 ms p95 cold and 22.6 ms revisited;
the image-heavy pair measures 21.0/22.8 ms. See [PERFORMANCE.md](PERFORMANCE.md)
for all results and measurement boundaries.

R78 preserves the conversation scroll offset during ordinary sync/read/flag
refreshes. R79 replaces list selection and sender-copy text buttons with native
icons, retaining generous targets and configurable icon tooltips; clipboard
regressions paste the copied address/domain into the actual native search field.
R80 makes selection-mode row clicks toggle one message and Shift ranges additive
across pages, preserving checkbox/modifier and double-click reading behavior.

R76 now matches the single-message reading surround to the document's opaque
background with readable controls and an unchanged widget tree; conversation-card
surround review remains open. R77 fixes editor glyph clipping at a partially
visible bottom line; its direct renderer test fails before the patch and passes
afterward. Native long-reply typing is visually clean in light/dark compact views.
Inline replies and the follow-up integration review remain R35/R77 in TODO.

Final source passes **427 Rust tests**, **2 drawing-adapter tests**, **2 FFI tests**,
**43 Python tests**, fmt and strict Clippy. An earlier revision passed all **150
functional native scenarios**; after the final C++/image correction, all **36
relevant native regressions** pass, including selection, Find, printing, image
policy/reflow, conversations, resizing and restart. This is not presented as a
second full 150-scenario run. The eight pixel gates used native binary SHA-256
`c6ee6d73113f54d36548ec0d50ce8b428ab7c03e8d73d29e08585c7eb188febc`.

Logs remain in ignored `artifacts/logs/`: `html-selection-native-full.log`,
`html-offset-native-regressions.log`, `html-offset-final-rust.log`,
`html-offset-final-clippy.log`, `html-verified-python.log`, and
`html-verified-pixel-latency.log`. Reviewed fictional WebP captures under
`artifacts/e2e/` include the deep template (`08f6cae1a117/`), image-complete revisits
(`44931a15d365/`), compact editor edge (`8d8a47ad6be7/`), dark-theme white surround
(`ee2f53fe4414/`), sender-copy icons (`86f9e366222f/`), cross-page selection review
(`1d3af182aeae/`) and retained thread scroll (`cf86a567ea05/`).

Strict Zensical and release checksum/extraction/bundled-installer verification
pass. The optimized Linux binary is installed atomically, SHA-256
`551f9ef3afae31f29b5b039c0ca7cee89ac4293fd01dd71e727aee35a13615d7`. Personal windows remain running on their earlier
executable until reopened. Git hooks and the direct main push passed.
Release/install logs: `html-offset-release.log` and
`html-offset-install.log`. Documentation CI runs **34115749075** and
**34115885902** completed build and deployment successfully.
R81 default reading styles and R82 configurable native notifications are recorded
in TODO/audit. Other performance measurements stay deferred. Quality/release
workflows remain disabled; documentation CI remains enabled. These tests do not
establish live provider mutation, physical monitor scanout or Windows/macOS
execution. The full product goal remains active.

## HTML opening speed and Mail focus — installed and pushed

Source [3f24823](https://github.com/sam-ruff/shep.so/commit/3f2482394d1d0aa277b500dd1a7be7165d724035)
ships R72 and R75. R71 already identified the earlier refresh-icon correction;
the four newest requests were assigned R72–R75 to preserve that audit identity.
The moved-mail destination issue and F5/manual animation remain R73/R74 in TODO.

Mail navigation now clears the previous folder's focus outline, focuses the
message list and retargets subsequent sidebar navigation to Inbox. The remappable
sidebar Inbox action retains sidebar focus. Both unified/light and per-account/
dark native cases pass; reviewed evidence includes `e4ffb201c285/` and
`98a6219a9ab8/` under ignored `artifacts/e2e/`.

HTML layout reuses bounded font-specific text widths and glyph bitmaps; visited
initial frames join the existing adjacent preparation cache. Image policy,
content, viewport, font and generation remain part of validity. The software
renderer combines overlapping damage regions and fills only visible solid panel
interiors. It keeps partial redraws and the general edge/gradient/shadow painter.
A fractional-edge discrepancy found by the new direct pixel tests was corrected
before shipping; full/partial and solid/general painter results now match.

The final native build, SHA-256
`93e7fd8a4aee65fc6477c15d164fcc118fe3e2e31a02dbf6dcaaf9659440d7f2`,
passes **all 144 functional native scenarios**, **413 Rust tests**, **2 dependency
cache tests**, **43 Python tests**, fmt, Clippy and Git hooks. The native suite
includes HTML selection, Find, policy/late images, resize, scrolling, Retry,
compact/dark views and action/Undo/restart paths. Navigation from Preferences to
Calendar passes in both themes with its ordinary timer and with the app Tick
disabled; the saved comparison needs no subsequent input or two-second wait.
Reviewed Calendar evidence is `f9a4eb3a5418/after-calendar-dark.webp`; HTML review
includes `e9da58bf7ba1/reference-styled.webp` and
`a5adb8f70141/html-current-narrower.webp`. This establishes those paint-correctness
scenarios, not a separate Calendar latency percentile.

Native XTest input through X11 body pixels, 20 samples per case, improved from
p95 **122.4 to 45.8 ms** for an unprepared 200-paragraph letter; **103.3 to 25.7 ms**
for returning to styled mail; **140.3 to 26.3 ms** for a prefetched neighbor; and
**104.8 to 37.0 ms** for reopening the long letter. All new HTML gates pass.
Reports: `artifacts/performance/html-opening-baseline.json` and `html.json`.
The final measurement has evidence `10c50ade9651/` and `ee7c4702b917/` onward.
[PERFORMANCE.md](PERFORMANCE.md) describes the boundary: font discovery is already
warm, pointer/reference/dwell setup is outside timing, external downloads and
physical monitor scanout are not measured. No builds ran during measurement;
the host was not asserted fully idle. Other final performance gates remain
explicitly deferred. Thresholds were not weakened.

Logs are under ignored `artifacts/logs/html-final-*`, with `html-release.log`,
`html-install.log`, `html-commit.log` and `html-push.log`. Strict Zensical and
release checksum/extraction/bundled-installer checks pass. The installed Linux
production binary matches the verified package, SHA-256
`43cc66577793d945088f216d3d8bc9fcd85e06a888c56ebd2500dca3c9f71cfb`.
Personal data/windows were preserved; already-open windows need reopening to use
the new executable. No root logs remain. Quality/release CI stay disabled;
the HTML pixel gate and dependency-cache tests are included in their dormant
quality definition. This is local/native fixture evidence, not new live-provider
or Windows/macOS verification. Unfinished folder-controller work was preserved
separately and excluded from this checkpoint. The full product goal remains active.

## Folder mutation backend checkpoint — R30 remains open

Source checkpoint [5eabb52](https://github.com/sam-ruff/shep.so/commit/5eabb521f1f11e4ec180bcdcb73835cb9e52db8c)
is pushed to main and installed for the Linux user. The installed binary matches
the release checksum below; existing personal windows were preserved.
Documentation build and deployment for that exact source head passed in
[run 34095669134](https://github.com/sam-ruff/shep.so/actions/runs/34095669134).

The provider-independent folder plan and runner now review whole subtrees,
protect Inbox, reject invalid nesting/collisions and preserve exact names,
NoInferiors and NonExistent metadata. The IMAP adapter checks the final LIST
response and distinguishes tagged rejection from a lost write acknowledgment.
RENAME moves a subtree; deleting one requires individual, deepest-first commands.
These rules follow [RFC 9051 mailbox operations](https://www.rfc-editor.org/rfc/rfc9051.html#section-6.3.5).

The SQLite journal records each command before execution and its acknowledgment
before cache migration. Retry skips completed deletes. An interrupted IMAP write
requires review; an acknowledged rename followed by failed LIST remains cache
recovery, never another RENAME. An owned filesystem lease excludes another
executor, including an independent process. Atomic migrations keep MIME inside
SQLite and preserve flags, conversation identities, restored markers, expansion,
Sent mappings and relevant group Undo receipts. Folder deletion retires only the
affected history items. Reviewed account removal includes unfinished folder work.

This is preparatory backend implementation, not availability of folder context
menus. Native dispatch, destination/deletion review, immediate presentation,
visible recovery, close integration and POP3 local hierarchy setup remain R30.
Independent-process coordination of ordinary provider writes remains R01/R06.
No real account or server folder was changed by these tests.

Verification: 408 Rust tests and 36 Python tests passed with formatting and
Clippy. The existing 142 native functional scenarios passed in one complete run
on the initial backend build. After final backend review-count/collision checks,
the final test executable passed 16 relevant native scenarios, including all
five folder-tree flows, mail actions/context menus and a new pixel comparison
for navigation. There are now 143 saved native functional scenarios; this
checkpoint does not claim a full 143-scenario run. Optimized Linux packaging,
checksum verification, extraction and the bundled installer passed.

Final native binary SHA-256:
`3e0884242949b67c12f3f20963309eaf5e46fa6dc3bcf8885d9026c58ad09e15`.
Release binary SHA-256:
`5cea9013d05fa718dac877feae3f43f9253e5bd3af03971f198a256848dd8b43`.
Logs are under ignored `artifacts/logs/folder-actions-*`. Settled light/dark
calendar captures in `artifacts/e2e/31c88b55c87b/` were reviewed; the refresh
icons render correctly. Compact folder-tree evidence is in
`artifacts/e2e/d49bf5893ba4/`.

Visual review also found that Calendar's state can change before its first paint
after appearance changes. The earlier captures still showed Preferences after
a 300 ms settling period. A saved native pixel check converges with a two-second
wait, and the settled calendar captures are correct. Longer test waits do not fix
that delay: first-paint scheduling and idle-host responsiveness remain explicitly
tracked as R15/R63. Performance gates remain deferred.

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


## 2026-09-07 — Selection arrivals, pointer redraws and group conflict guards

Shipped and installed source commit `2444049ae843573b952604a2c601c088fd27b61d`.
Explicit checkbox/Ctrl-click selections can now include arriving mail while
preserving earlier choices. Shift ranges use the current query order, including
intermediate arrivals. Passive refresh never selects new mail by itself. Frozen
reviews keep their original membership, and rejected gestures or transient
observation failures retain confirmed choices. Query order and membership stay
in SQLite; the UI continues receiving bounded metadata pages.

Individual move/transfer/Undo/read/flag paths check pending group ownership.
Group Undo claims a resolved identity without stealing another item's claim;
completed phases cannot reacquire ownership. Pending row flag buttons absorb
clicks without opening the message, and conflicting context actions explain how
to review the group. These guards do not establish serialization across independent
processes or finish every individual/group staging race; broader ordering and
provider ambiguity remain in TODO.

The final native run exposed a redraw between motion and press erasing the
previous queued-click fix's pointer position. The root tracker now survives
redraws and shares captured dropdown/nested-overlay motion, while retaining
runtime overlay exclusion and resetting after blur/leave/interface-scale changes.
The deterministic widget regression fails on the old behavior; an additional
overlay regression covers captured motion and popup closure. Consecutive native
checkbox clicks remain consecutive, without sleeps masking input defects.

Validation: **353 Rust tests**, **29 Python tests**, formatting and Clippy pass.
The same final native binary passes all **119 saved functional scenarios**.
The first runner was externally terminated after 21 completed cases; a detached
runner passed all 98 remaining cases, including the interrupted case. Evidence:
`artifacts/logs/selection-pointer-native-a.log` and
`selection-pointer-native-remaining.log`. No failed scenario was omitted.
Rust/Clippy/Python logs share the `selection-pointer-` prefix. Git hooks also pass.

Reviewed WebP captures include arrival checkbox/range selections in
`artifacts/e2e/5de4c05ddac0/` and `7ab94d4a3392/`, and the pending-group conflict in
`78e612e842df/`. Native Mail/Calendar refresh arrows remain clean in standard,
compact dark and 120% layouts (`2177522fb60f/`, `b18dcbe0dfe0/`). This verifies the
native icons; the original browser-chrome crop was not separately reproduced.

Strict Zensical builds and optimized release checksum/extraction/bundled-installer
checks pass. The Linux user install matches the release binary, SHA-256
`c9583cc01bb02b2d8220d7ad6a92d9044e915910556fd978653005156e89d281`.
Existing personal windows were left running. R42 multi-selection is removed from
TODO; its remaining bulk History/recovery/pagination paths stay open. The full
product goal remains active, performance measurements remain deferred, and
quality/release CI remain disabled.


## R42 — History and process recovery (2026-09-07)

Shipped in `b352d125a267e36322803549c52e2d903916876b`.

**Continue** now clears the persisted pause before waking the worker; completed
receipts are never replayed. History keeps its visible group page separate from
active progress tracking, preserves newer receipts when an older read arrives,
and returns to the top after group or message pagination. Unconfirmed-result
reviews accept mouse input and Y/Enter, cancel with N/Escape, and cannot leak
into a reopened History dialog. Acceptance replaces the stale recovery error
with an explicit accepted-state note and keeps Undo available for other
acknowledged messages.

Real window-close tests exposed renderer workers keeping the process alive
after the window disappeared. Subscription cancellation now wakes both HTML and
neighbor-preparation receivers even while UI state retains their senders. Close
flushes pending pane sizes and quiesces the bulk worker before exiting, including
when the displayed History page contains no running work. Closing before engine
readiness does not wait for a missing worker.

The MCP harness now owns optional persistent, explicitly marked fixture caches.
It refuses unmarked databases before migration. Native close/restart keeps the
owned display and cache; explicit crash mode kills only that fixture process.
A close timeout reports the failure without silently killing or replacing the
app. The graceful-close scenario reads the fixture journal while the app is
closed, proving one durable receipt and one queued step before restart.

Validation: **362 Rust tests**, **32 Python tests**, formatting, Clippy and Git
hooks pass. All **127 saved native functional scenarios** pass on the final test
binary: the complete existing 126-case run plus the newly added empty-Inbox
restart case. Logs are `artifacts/logs/bulk-history-native-full.log`,
`bulk-history-native-result.json`, `bulk-history-empty-native.log`,
`bulk-history-final-rust.log`, `bulk-history-final-python.log` and
`bulk-history-clippy.log`. No failed scenario was omitted.

Eight added native scenarios cover graceful and crash recovery, uncertainty
acceptance/cancellation, retained Undo, failed inverse retry after restart,
Continue, both pagination types, compact dark mouse review, formatted-reader
shutdown with saved appearance, and reopening an empty Inbox with all 120
archived messages retained. Reviewed final WebP evidence includes accepted
results in `artifacts/e2e/ae702538d9b8/`, compact dark review in `ce03b78c5462/`,
History pages in `f7e68df55523/` and `d9648d6faece/`, and empty Inbox/Archive in
`74fbf4c5f512/`. These fixtures establish application recovery; they do not prove
live-provider or independent-process correctness.

Native refresh arrows remain clean in compact dark and 120% Calendar captures
(`1207d242ae17/`, `734c9cbabbe5/`). The browser-chrome crop from the original
report was not separately reproduced.

Strict documentation builds and optimized release checksum, extraction and
bundled-installer checks pass. The installed Linux binary matches the release,
SHA-256 `6b2fcf3b5824437208ff74ded58a4bfa2cf3d604a2f82478bc800098f8e49cb2`.
Existing personal windows were preserved. R42's remaining native verification
is complete and removed from TODO. The full product goal remains active;
individual/group ordering, provider ambiguity, independent-process coordination
and the other TODO requests remain open. Performance measurements stay deferred,
and quality/release CI remain disabled.

## R45 — Drag messages into sidebar folders (2026-09-07)

Shipped in `ca39080b3afe3ac95685d42858cd017f76a956e3`.

Drag one inbox row or the entire selected group to a sidebar folder. Destination
outlines and a floating label show the target/count/account. Hover opens collapsed
accounts and unified Inbox; wheel scrolling works while holding. Group drops
freeze the existing cross-page selection and use the normal review, optimistic
commit, durable history and Undo. Single drops use the dragged metadata even
when a different body is open, and show immediate counted feedback and Undo.

Cached rules reject missing accounts/folders, same-folder no-ops and disabled or
POP3 cross-account transfers; local POP3 folder moves remain available. Inbox,
Archive and Trash preserve each source account. Combined Sent/Flagged views are
not drop destinations; actual account folder rows remain available. Real provider
capabilities are checked by the existing commit path. Escape, right-click,
focus/cursor loss and outside drops cancel without leaking a row/sidebar click.
Flag/checkbox presses and ordinary click jitter do not start a drag.

A compact screenshot exposed a software-renderer shadow trail. Damage bounds
omitted shadows and shadow drawing ignored the damaged layer's mask. The vendor
patch includes full shadow damage, clips buffers/drawing to visible damage, and
handles shadow-only intersections. Two renderer regressions fail on the prior
code and pass after the fix; a third covers offscreen positioning. The native
Unicode-folder scrolling scenario checks a saved WebP region before any forced
repaint. Reviewed corrected capture: `artifacts/e2e/10eef656611e/`; the final full-run
capture in `951f9763ca4d/` also passes the pixel check. Its previous failing
capture had a 95-level grayscale range in the otherwise empty footer region.

Reviewed final captures also cover hover expansion (`2abba962dd82/`), explicit
account destinations (`a7b06ef9ec65/`), red Trash review (`50434b0bdf5f/`), all-page
review (`b652be35da4b/`) and 120% controls (`92074448e407/`). Native refresh arrows
remain clean in compact dark and scaled Calendar (`bf9c15901e35/`,
`9c769d176277/`). The original browser-chrome crop was not separately reproduced.

The MCP harness now supports owned left-button mouse_down/mouse_up actions, so
hover, scroll, waits, assertions and screenshots can be batched during a drag.
Cleanup releases held input. Optional pop3_account fixture setup tests destination
rules without a real provider. Every interaction is saved in scripts/e2e.py.

Validation: **374 Rust tests**, **33 Python tests**, formatting, Clippy and Git
hooks pass. All **137 saved native functional scenarios** pass on the final
test binary, SHA-256
`bb7b3c2f1c565be6057a77e15905395cc82a1561769c712702309c7009039ec3`.
Logs: `artifacts/logs/drag-mail-final-native-full.log`,
`drag-mail-final-native-result.json`, `drag-mail-final-rust.log`,
`drag-mail-final-python.log` and `drag-mail-final-clippy.log`. No failed scenario
was omitted. Ten added native scenarios cover
source identity/Undo, group reviews and mixed accounts, cancellation/no-op,
cross-account preference, hover reveal, failed writes/navigation, POP3 local and
cross-account rules, compact dark/120% layouts, scrolling/Unicode/shadow cleanup,
and all 120 selected messages across pages. The initial complete 137-case run
passed before the final renderer change; it is not substituted for the final run.

Strict Zensical builds and optimized release checksum, extraction and bundled
installer checks pass. The installed Linux binary matches `target/release/shep`,
SHA-256 `901418794f54e89eea29b9d5de0d96719d0b64e6ba261d790602785746ce0bfe`.
Personal windows and data were preserved.
R45 is removed from TODO after this checkpoint was installed and pushed. The full product
goal remains active; folder trees/context menus, other TODO features, live-provider
and platform validation remain outstanding. Performance measurements stay deferred.
Quality/release workflows remain disabled; documentation CI remains enabled.

## R30 — Native mailbox trees (2026-09-07)

Installed and pushed source **6d520e9b26d2c1dc257255ef607761575c992521**,
with harness corrections **ec080b9** and **f1a6a63**. Folder context-menu
mutations remain a separate open R30 item; this checkpoint delivers the tree.

IMAP LIST metadata now retains each mailbox's delimiter, exact name,
selectability and session encoding. Cached trees preserve selectable parents,
nonselectable/trailing-delimiter containers and missing intermediate ancestors.
NIL-delimited names remain flat even when they contain slashes or dots. Cached
messages cannot make an explicitly nonselectable parent a destination again.
Original cached mail is retained. Legacy names remain flat until real LIST
metadata arrives. Tree construction and modified-UTF-7 decoding happen on the
storage worker; Workspace shares cached trees and labels through Arc.

Groups start collapsed and remember expansion per account. Clicking a selectable
parent opens its mail; its chevron expands without selecting. Containers only
expand. Closing an ancestor retains its descendants' expansion for later and
normal process restart. Unified shortcuts do not hide actual Inbox children.
Left/Right/Enter navigates the hierarchy, while Up/Down passes containers without
toggling them. Native layout operations reveal keyboard targets below the compact
viewport, rejecting superseded targets and retrying only missing new-layout rows.
Hovering during a message drag opens nested groups without changing the reader.
Nonselectable containers never become drop targets.

Japanese and other modified-UTF-7 names display decoded in the sidebar, title,
Move search, highlighted Enter choice, drag label, bulk review/history and toasts.
Queries, provider commands and Undo receipts retain exact wire names. Display
lookup remains account-specific; an identically spelled UTF-8 name stays literal.

**387 Rust tests**, **36 Python tests**, fmt, Clippy and Git hooks pass. The new
coverage includes delimiter/encoding domain cases, actual loopback IMAP LIST and
SELECT, reopened SQLite catalogs/preferences, stale-save ordering, keyboard
scroll bounds and action/Undo identity. Five saved native scenarios cover
mouse/restart, keyboard/dot/NIL containers, nested drag/Undo, Unicode Move/review
and Ctrl-selection, and compact dark/120% navigation with saved dimensions.

Both complete desktop runs executed all **142 functional scenarios** on test
binary SHA-256
`a0537d67f9dd8c6884d21bff1c47ddf9dfe10783f62207e40fbbf2582440cc7c`.
Each reported 141 passes and one different test failure; neither is represented
as a clean 142-case run. The first Japanese xdotool input intermittently delivered
no text after native focus acknowledgment. The batchable owned-display clipboard
paste path fixes that test; the targeted case and all five nested cases in the
second full run pass. The second run's Undo restored the message correctly, but
its final assertion used a null subject captured before the initial body load.
The three affected tests now resolve selected_id against the metadata page, and
all three pass in `folder-tree-metadata-action-native.log`. No scenario or
asserted behavior was removed. No additional full run was performed after that
last test-only correction. All 142 paths have passing coverage across these runs
and the corrected targeted reruns.

Logs remain under ignored `artifacts/logs/`: `folder-tree-final-rust.log`,
`folder-tree-final-python.log`, `folder-tree-final-clippy.log`,
`folder-tree-final-native-full.log`, `folder-tree-verified-native-full.log`,
`folder-tree-native-unicode-paste.log` and `folder-tree-metadata-action-native.log`.
Reviewed WebP evidence includes compact keyboard reveal (`69165f22266f/`),
dot-delimited folders (`915d5accece5/`), restart expansion (`92eda9ec6f96/`),
Unicode review (`b7cd9dfe3cfd/`) and large-scale dark controls (`319396c41206/`),
under `artifacts/e2e/`. Native refresh controls remain clean, including the
compact Calendar capture `631506faedc0/refresh-calendar-dark-compact.webp`.
The original browser-chrome crop was not separately reproduced.

Strict Zensical, optimized release checksum/extraction and bundled-installer
checks pass. The installed Linux binary matches the packaged release, SHA-256
`534ccfdabc4c1ab46615bdda7dee932175021f190f52745123156fbde5529e61`.
Personal windows and data were preserved. The tree TODO was removed only after
installation and the main push. Folder mutations and the other product work
remain in TODO; the full goal is active. These isolated tests do not establish
live personal-provider or Windows/macOS execution. Performance measurements
remain deferred, and quality/release workflows remain disabled.
Documentation CI run **34091152799** completed build and deploy successfully for
source/testing head **f1a6a63e18992e13f9655a61f47226b8da8944c3**.
