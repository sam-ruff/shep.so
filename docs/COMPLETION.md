# Completion audit

This log is the union of the desktop session's log (`main`) and the mobile/web client session's log (`feat/mobile-web-clients`), merged on 2026-09-09; the merge entry is at the end of the file. The entries directly below were written on `main`, newest first, down to the 8 September handover entries. Later sections keep each branch's own order. Request numbers R67 to R80 exist on both sides; [the request audit](REQUEST_AUDIT.md) states the collision once.

## 10 September: Google lifecycle channel ownership (R91)

The audit covered every shared lock-managed state on the Google lifecycle
path. Each confirmed case now has a bounded owning worker; nothing else on the
path held a mutex.

| State | Before | After |
| --- | --- | --- |
| Google connection (status, calendar sync, event edits, Drive backups, restore, profile sync sessions; login, disconnect, cleanup) | `Arc<RwLock<()>>` on the engine | `engine/lifecycle_work.rs` lane: 32-request FIFO coordinator, shared/exclusive grants by one-shot, abandoned requests skipped, drop releases, close drains |
| Connection lifecycle (account/calendar saves, removal, cleanup retry, restore, Google activation) | `Arc<Mutex<()>>` | exclusive lane on the same coordinator type |
| Calendar setup namespace | `Arc<Mutex<()>>` | exclusive lane |
| Token vault (grants, candidate, pending login, refresh, activation pruning, clear) | `Arc<Mutex<State>>` held across HTTP and keychain calls | `providers/google/owner.rs`: one thread owns `State`, 32-job FIFO; admitted jobs drain even when the caller is cancelled or the last handle drops; a stopped owner reports an error |
| OS credential entry | already a bounded 32-request thread (`credentials.rs`) | unchanged |
| `google_lifecycle`, `google_archived`, cleanup jobs, revisions | SQLite through the cache worker | unchanged |

Contracts are preserved: disconnect keeps cached calendars and events
read-only, cleanup pending survives restart, stale reviews and snapshots are
rejected, and the Google-exclusive-before-lifecycle order and reconnect
ordering are unchanged. The Google lane is strict FIFO, so provider work queued
behind a disconnect waits for it rather than starving it. The engine field name
`google_connection_lock` is retained because the profile sync lane reads it.
The behavioural improvement is in the token owner: a sync cycle cancelled
mid-refresh no longer drops the HTTP/keychain future, so a rotated refresh
token is always persisted.

The R64 `Owned` follow-up: a new Linux regression in the shared crate opens a
Journal, reads `/proc/self/fdinfo` to assert the lock descriptor carries
`O_CLOEXEC`, execs a child while the lock is held, and the child asserts no
descriptor names the lock file, that `Journal::open` returns `Owned`, and that
it can claim the journal after the parent drops it while the child lives. The
only shared-crate change is that test plus a test-only descriptor accessor.
Descriptor inheritance across exec is therefore not the mechanism; the
fork-to-exec window was already neutralised by the explicit unlock on drop
(`e3e69a4`). Fifteen full reruns of the shared crate (70 tests each) show no
`Owned` (`artifacts/logs/journal-owned-stress.log`); the original transient
remains unreproduced and undiagnosed.

Tests: five lane coordinator tests (FIFO with shared behind exclusive,
abandoned requests, failing holder release, 32-bound with drained overflow,
close waits for the holder) and three token owner tests (cancelled refresh
observer still persists in order, last handle drains the running save before
exit then restarts from the vault, locked keychain fails every queued caller
without stopping the owner); the removals test now probes lane occupancy. On
the lane, `cargo test --all-features google` passes 57, `lifecycle` 14 and
`journal` 33 executions; `cargo test -p shep-profile-core --all-features` 70.
The commit `f4575b2` passed the full pre-commit hook with **1101** test
executions (three personal diagnostics ignored, `artifacts/logs/hooks-google-owner.log`). Formatting, `cargo clippy --all-targets --all-features -- -D warnings` and 96
Python tests (seven skipped) pass. Native: the six Google disconnect,
permissions and consent scenarios pass in 21 s
(`artifacts/logs/e2e-google-owner.log`); reviewed captures in `92ffc34b5a29`
(disconnect review, disconnected state, cached read-only event),
`f64efd4453a0` (compact dark disconnect and mail navigation), `41a08b14baeb`
and `5555027e3c8a` (consent light/dark). These are fixture grants, not live
Google. Limitations: independent-process Google/lifecycle coordination and
live Google verification remain open; the profile sync lane should add `?`
handling if the lane API ever becomes fallible.

## Mobile and web clients merged into main — shipped

The desktop session merged `feat/mobile-web-clients` (`d8d5c1e`, itself a
`--no-ff` merge of main `c414227`) into `main` as `ee1305c` on 10 September
2026 and pushed it; remote equality is verified. The merge also carried the
held aggregate folder account chooser (`1abc4b9`, merged as `d99b111`). The
only conflict was `TODO.md`: the client branch's combined list was taken and
the desktop receipts written after `c414227` were re-applied. The root
workspace now includes the shared crates, `shep-profile-core` is a path
dependency on `shared/profile-core`, and the pre-commit hook covers the whole
workspace. Gates on `ee1305c`: **1066 hook test executions** (zero failures,
three personal diagnostics ignored; `artifacts/logs/hooks-merge-mobile.log`),
**96 Python tests** (seven skipped), the strict documentation build and **45
native desktop scenarios** in 202 s covering folder controls, search,
conversations, badges, profile account reviews and removals, Google flows and
backup history (`artifacts/logs/e2e-main-ee1305c.log`). Flutter, browser and
backend gates are the client session's, recorded in "Main merged into the
client branch — 2026-09-09". Live provider, Windows and macOS execution remain
unverified.

## Encrypted cache publication and crash recovery: lane checkpoint

R22 gains the guarded replacement path in `cache_cipher::publication`. With the
exclusive root guard held and no open plaintext connection, publication
checkpoints the legacy WAL with TRUNCATE, switches the file to DELETE
journalling and removes only empty leftover logs (a busy checkpoint or a
non-empty log fails the step), writes `<cache>.encryption-journal` naming the
verified candidate, its versions and the plaintext fingerprint, renames the
plaintext to `<cache>.plaintext-recovery`, renames the candidate into place with
directory fsyncs, reopens the published file with the key for an authenticated
schema and version read, and only then deletes the plaintext and the journal.
Staged candidates carry a source fingerprint and are refused if the plaintext
changed after staging. Startup recovery derives the state from the journal and
the main/candidate/recovery layout plus the file header, resumes verified
candidates, restores the plaintext when the key is wrong while the recovery
file exists, keeps everything and reports when the plaintext is already gone,
and refuses layouts it did not create. Guarded orphan cleanup removes stale
`.shep-encrypted-*.partial` candidates and `.shep-cache-scratch-*` folders
without touching journalled candidates or import staging.

Verification: ten new publication tests cover full publication with
uncheckpointed WAL rows, stale candidate with retry, a held plaintext
connection with retry, reader and foreign guards, interruption before each of
the seven steps followed by resume, wrong-key rollback and refusal, lost
candidates, ambiguous layouts, a stray recovery file and orphan cleanup.
`cargo test --all-features cache_cipher` passes 28 tests and the `migration`
filter passes seven. The lane's hook run on the merged workspace (`ee1305c`)
passes 1,076 test executions with zero failures and three ignored, clippy
with `-D warnings`, fmt, 96 Python tests (seven skipped) and the Windows GNU
`cargo check --all-targets --all-features`.
Production startup is unchanged and plaintext, so no native scenario exercises
this path; none was run. Not verified: actual Windows/macOS execution of the
rename and sidecar handling, and any personal database.

Still blocking activation: bootstrap routing of recover/stage/publish on a
worker with the guard held from key creation through publication, reader-guard
retention through every store/profile worker write, exclusion of legacy Shep
processes that never take the guard, keyed import staging and catalog routing,
bounded selection-summary/catalog/recovered-view sorting, native key recovery
and platform startup checks.

## 10 September: closing blocked by saving (R90/R86)

Sam's first phase-2 fix. On `c414227` three blocked-close paths were reproduced
deterministically and corrected on lane `worktree-agent-a92937f9fa561a16d`
(merged forward onto `ee1305c`).

Reproduced causes. Tray-menu Quit and Preferences Quit Shep while a close was
already pending re-entered the same `WindowClose` wait and had no effect, so a
backup upload (untimed by design so a healthy large archive is never cut off) or
a credential-store call could keep a hidden, windowless Shep alive with no way
out; three reproduction tests fail on unmodified `c414227`
(`artifacts/logs/close-repro-head.log`). Quit from an ordinary hidden tray never
sent the saving notification. After `iced::exit()` the daemon drops its Tokio
runtime, whose drop joins every running `spawn_blocking` task, so a stalled
transfer left the process running silently after the last window closed.

Fixes. A repeated explicit Quit (`App::quit_now`: tray menu, Preferences button,
or closing the visible fallback window again when notifications are unavailable)
leaves immediately when only journaled work remains (`backup:*` upload journal,
`credential-cleanup`; both resume on the next launch), otherwise reopens the
window with the exact reason plus "Shep will quit as soon as this is saved" and
keeps close intent so the pending acknowledgment still exits automatically. A
repeated native close event while hiding is not treated as Quit. Quit from an
ordinary hidden tray announces saving with the same notification, whose text
now names Quit Shep. `finish_exit` arms `lifecycle::bound_exit` with a
five-second deadline after the final required acknowledgment. Local saves and
unjournaled provider writes still block exit and stay bounded by their provider
timeouts; nothing fakes an acknowledgment and no queue became unbounded.

Evidence. Seven new Rust tests in `ui::tray`, `ui::closing` and `lifecycle`;
the targeted `tray`/`closing`/`lifecycle` filter passes 32. New
`backup_run="held"` fixture stalls the second upload inside a blocking task
after its reservation is journaled. New native scenarios:
`test_tray_native_quit_leaves_held_backup_upload_journaled_and_process_exits`
(first close waits in the tray with the notice, tray Quit exits with return code
0 inside the ten-second `wait_exit` bound, the journal row and first copy survive
exit and restart),
`test_tray_native_close_during_slow_backup_upload_notifies_then_exits_when_saved`
(temporary tray notice names Quit Shep, automatic exit, both copies saved,
journal empty) and
`test_tray_native_quit_during_held_readonly_sync_exits_and_keeps_cache`
(close-to-tray hide, Quit during an indefinitely held read-only sync, cache
count unchanged after restart) and
`test_tray_native_repeated_quit_with_pending_send_shows_reason_and_keeps_reply`
(light and compact dark: Quit during a pending preview send reopens the window
with the reason, the later rejection cancels close and keeps the reply).
`wait_exit` now reports the return code. In the held-upload run the process
ended after 5,026 ms, the exit deadline, where the slow-upload and held-sync
runs ended in 1,422 ms and 16 ms (`artifacts/e2e/c786c424d98a`,
`ffa4c24fdbd3`, `41b7ee31e58c`); without the deadline the ten-second
`wait_exit` bound would have failed. Reviewed WebPs:
`0ded00daa283/tray-repeated-quit-reason-light.webp` and
`a82017111e1d/tray-repeated-quit-reason-dark-compact.webp` show the intact
reply, disabled Sending control and the bottom-bar notice "Finishing your
changes before closing. Shep will quit as soon as this is saved";
`c786c424d98a/held-upload-retained-after-restart.webp` shows both destinations
after restart. All 33 selected native close/tray/saving/quit scenarios pass
(`artifacts/logs/e2e-close.log`); the targeted Rust filter passes 32, clippy
with denied warnings passes, `cargo fmt` is clean and 96 Python tests pass
(7 skipped).
These are fictional native fixtures on Linux/Xvfb: which save blocked Sam's
personal close remains inferred rather than observed, and Windows/macOS
execution of the Quit path is unverified.

## Post-enrollment account linking and suppression (R02/R49) — lane verification

Lane `worktree-agent-adfeba145fe019aee`, branched from `64437ee` and merged
with `ee1305c`, adds the third account review kind. When the continuous loop
observes a new shared account definition and this device has an unmapped
native account with exactly the same portable connection fields or the same
address, `apply_profile_observation` holds the definition and reports a
review instead of creating a fresh reconnecting account. `account_reviews::
prepare` appends link candidates after the mapped-account reviews on the same
eight-per-page cursor (`link:<uuid>` continuation), freezing the profile,
Google, consent and connection generations, each candidate's native
connection revision, the exact history revision and the exact shared
operation. `Store::resolve_profile_account_link` commits one of three choices
in a single cache transaction: Link to existing account (exact matches only;
maps the native id to the shared UUID, records the original change with its
optional fields as the common basis, changes no native row, credential or
mail; the device name is then shared as at import), Add as a new account
(fresh reconnecting identity through the shared `add_shared_account` helper)
or Keep this device's account local (durable suppression of an unmapped UUID;
`State::validate` now allows suppressed UUIDs without a mapping). Stale
native, Google, option, history or already-decided identities are rejected
without partial application; suppressed UUIDs stop being reported. The engine
locks the chosen native account for the request; the UI adds an exact-match
picker when several native accounts match and clears link choices with the
review page.

Verification on the lane: seven targeted tests (`profile_account_link_*` in
`src/store/profile_sync/state/account_link_tests.rs` plus the UI identity test)
and 115 matching `cargo test --all-features profile_` executions pass, clippy
is clean, and `existing-link` native scenarios pass with reviewed captures:
`test_profile_account_link_native_links_existing_account_and_keeps_mail`
(`b31793960fb7`, light: Link then Keep local on the address-only card, restart
publishes only the device name), `..._adds_new_account_once_across_restart`
(`acf0c037c2e1`, light: fourth account named Studio (shared) beside Design
studio) and `..._keep_local_in_compact_dark_window` (`87339075b31c`, 900x640
dark). The twelve selected `profile_account`/`profile_join_link` scenarios pass
together (`artifacts/logs/e2e-linking.log`). Limitations: the fixture is an
owned loopback Drive, not live Google; incremental change-token pulls,
password transfer and cross-client verification remain open; conflicting new
definitions still stay in the cycle report without a link review.

## Journal ownership, removal reviews and duplicate labels — integrated verification

Three lanes were merged into `main` with `--no-ff` after each was rebased onto
`c35e2b5` in its own worktree by an integration agent. `42c69b4` integrates the
bounded backup journal owner (`d36dca8`): the journal is a named 32-slot FIFO
worker, keyed constructors and `open_encrypted` are intact, and three ownership
tests cover bounded admission after observer cancellation, last-handle close
draining and non-poisoning failed transactions. `61c6dfc` integrates remote
account-removal reviews (`ede75aa`): a shared tombstone offers Keep on this
device or Review removal, Keep persists local suppression and stale native,
Google, option or history changes reject the choice. `c0ebf3f` integrates
duplicate-address sidebar labels (`d5c7683`): each duplicate shows its saved
name with the address beneath in inbox children and account headings.

Lane verification: the journal lane passes 935 hook executions, 114 backup,
23 journal and 18 cipher tests, 84 Python tests and eight native backup
history/all/formats scenarios (evidence `bdc0ecad2916`, `554466f91d27`,
`025cc924046a`, `9e8faa7055c5`, `b9f487068789`, `0ffc69ca42f2`,
`930f3dc1d1f8`, `8ce9ef2f1139`). The removal lane passes 935 hook
executions, 104 profile tests, 84 Python tests and five native removal/review
scenarios (`3acebfccaf97`, `e5d37c008543`, `6d2961a62066`, `b8fc277cc1bc`,
`2c008d1829f2`) with reviewed Keep, Remove confirmation, kept and removed
states. The sidebar lane passes 933 hook executions, six sidebar tests, 84
Python tests and seven native scenarios (`d54ade8b16da`, `6374378f2a4e`,
`5e5dafdd2411`, `ca32be155096`) with reviewed default, 160 px and 900x640
captures. On `main`, the merge commits ran the full hook suite on the merged
trees: **938** executions for `61c6dfc` and **939** for `c0ebf3f`, zero
failures, three personal diagnostics ignored (`artifacts/logs/hooks-merge-*`).
The combined pre-push gate on `c0ebf3f` passes: **84 Python tests** (seven
skipped), the strict documentation build, and **18 native scenarios** in 86 s
covering backup history/all/formats, removal and connection reviews,
duplicate-address labels and sidebar flows (`artifacts/logs/e2e-main-c0ebf3f.log`).
Sam confirmed the push at 22:00; source `c414227` (the docs receipt commit on
top of `c0ebf3f`) is on `origin/main` with remote equality verified.
Limitations: removal rows lack light and compact-dark captures,
duplicates with identical or empty names stay indistinguishable, and
lifecycle/Google channel ownership remains under audit.

The aggregate folder account chooser lane (`1abc4b9`, integrated by an agent
as `fd67cae` with 937 hook executions, 86 folder tests, 84 Python tests and 14
native folder-control scenarios in `2f98b1c42870`, `4af813da2222`,
`41f3623adc32`, reviewed light and compact-dark pickers) is merged into `main`
as `d99b111` with **944 hook executions** passing on the merged tree. The
merged sidebar now carries both the duplicate-address labels and the
`FolderSelection` context; on `d99b111` 84 Python tests and **19 native
folder-control, duplicate-address and sidebar scenarios** pass in 106 s
(`artifacts/logs/e2e-main-d99b111.log`). The push is held while the mobile
session merges `c414227` into its branch.

## Backup history, encrypted export and ranked scratch — consolidation checkpoint

The session restart on 9 September 2026 resolved the interrupted cherry-pick on
`main` and committed `c35e2b5`, which integrates persistent per-destination
backup history (`895fe2d`), keyed raw SQLite export (`f0ff98a`) and indexed
encrypted conversation ranking (`8d45a9e`). The `store.rs` resolution keeps the
lane's version 4 bump and the encrypted initialiser's bare connection return;
the interrupted `codex/encrypted-cache` rebase was aborted because every code
file it would have produced is byte-identical to the committed index. Normal
startup remains plaintext; no personal database or installed process changed.

Verification on `c35e2b5`: the mandatory pre-commit hooks (format, Clippy,
`cargo test --all-features`, renderer cache tests and the shared profile-core
script) passed at commit time; the full execution count was not captured by
the truncated log and is re-established by the pre-push gate run recorded in
the next entry. **84 Python tests** pass (seven skipped, including actual
PowerShell execution), the pinned strict documentation build reports no issues,
and **31 selected native scenarios** pass in 189 s: backup history, formats and
Back up all, database export/import, conversation reading and ranked search.
Logs are `artifacts/logs/python-main-c35e2b5.log`, `docs-main-c35e2b5.log`
and `e2e-main-c35e2b5.log`; evidence directories are listed in that log.

Lane consolidation: three read-only classification passes compared every
`codex/*` branch with `main` by code and tests. `profile-links`,
`profile-reviews`, `profile-cache`, `profile-account-reviews`,
`bounded-folder-delete`, `aggregate-folder-choice`, `combined-folder-delete`,
`folder-convergence`, `compact-mail`, `backup-history`, `backup-formats`,
`backup-all`, `ftp-backups`, `sftp-backups`, `multiple-backups`,
`encrypted-cache`, `native-tray`, `s3-before-main-adaptation` and
`native-palette-before-rebase-1854c62` were byte-identical or strict subsets
of `main` and were deleted with their worktrees. Unique work remains on
`codex/backup-journal-owner` (`d36dca8`), `codex/aggregate-folder-choice-v2`
(`1abc4b9`), `codex/profile-account-removals` (`ede75aa`, the previously
uncommitted removal-review work, 782 hook executions passing on the lane) and
`codex/sidebar-account-labels` (`d5c7683`, the previously uncommitted
duplicate-address labels, 916 hook executions passing on the lane). This is a
local checkpoint; the push receipt follows in the next entry.

## Shared account-removal review — worktree verification

Remote account tombstones now offer Keep on this device or Review removal.
Keep preserves the original account, mail and credential identity and persists
local suppression; changed local endpoints then remain local. The other action
opens the existing local-data confirmation. Cancel preserves everything; an
explicit confirmed removal survives restart and does not import the account
again. Successful removal clears stale shared-account review controls.

Keep validates exact remote history and current binding, consent, Google,
account and native edit generations. It never revives the remote tombstone or
issues a new remote connection. Existing endpoint-choice checks also continue to
reject a review invalidated by a remote removal. The ordinary local-data dialog
retains its existing draft/mail-change/credential-cleanup contracts.

The initial eight targeted tests, all 98 matching profile tests, 84 Python tests
and three saved native Keep/Cancel/Remove/restart flows pass. Visual review then
caught the already-removed account card remaining visible; the final source
clears it and adds a controller/native assertion. All 35 final native profile
scenarios pass in 171.199 seconds, including the corrected stale-card assertion;
root reviewed the final Keep and Remove WebPs in `be426d49a6b1` and
`52457d6367e4`. Normal hooks and main integration remain pending. Final native
SHA-256:
`5530a6ebc51207a0be89efc6c48c83f440dd2760b9514ff6c5a6bff808ead354`.
Logs use `artifacts/logs/profile-account-removals-*` in the isolated worktree.
No live Google access, real account removal, password transfer or main shipping
is claimed. Global account removal and post-enrollment linking controls remain
separate work.

## 9 September: explicit account choice for common folder actions

The isolated `codex/aggregate-folder-choice-v2` checkpoint builds on `9b300af`.
Move/Delete from a common folder now offers the accounts that actually contain
it; an explicitly scoped or sole account opens its ordinary review directly.
Sent resolves each account's configured wire path. The review names the account
and lets an aggregate action change that choice before confirmation.

Changing accounts releases the old affected-count snapshot and rejects its late
reply. Keyboard focus follows account identity when the catalog changes; removing
the highlighted account requires a fresh choice. Busy accounts remain protected,
Y cannot skip account choice, and Enter reaches the same reviewed action as the
mouse. Existing optimistic counts and remaining/newer readers survive success,
failure, retry and normal restart.

Five new deterministic controller cases cover explicit choice, configured Sent
paths, stale reviews, removed focus and busy accounts. The 52 targeted folder
cases pass. Three saved native flows cover mouse account changes and deletion,
compact-dark keyboard failure/retry, nested Move and selected/aggregate Sent.
All 18 affected native scenarios pass, including the existing folder, selection,
bulk, conversation and sidebar regressions. Reviewed WebPs include
`ce59cd0cd0f6` (mouse review and remaining reader), `8ef41323f24f` (compact-dark
choice/review and failure history), and `7427e7929267` (Move/restart/Sent).
The exact native executable SHA-256 is
`4705bbc27658e9c4c80b9eff27242cb5c1f09273f057f8270c5f9cb19d8e13f5`.

Formatting, all-target Clippy, 768 hook test executions (703 application Rust,
two HTML dependency and 63 shared profile-core tests; three live diagnostics
ignored), 56 Python tests and strict Zensical pass. Logs remain under ignored
`artifacts/logs/aggregate-choice-*`. These are isolated native/provider fixtures;
no live-provider, Windows/macOS execution or performance claim is made.
Primary-agent integration/push and broader R30/R50 history/scope convergence
remain open. R15 separately tracks distinguishing duplicate-address sidebar
accounts after a reviewed profile endpoint change.

## Shared account reviews, backup formats and bounded folder deletion — integrated verification

The current integration combines `18413e7` (safe shared connection choices),
`7ef6c56` (per-destination compression/encryption), `6d7c392` + `9b300af`
(bounded combined-folder deletion), and the staged encrypted-cache foundation
`2890fe8`, keyed catalog `7485cfc` and temporary-storage correction `86d9c6c`.
Normal startup remains plaintext pending guarded migration and the remaining
sorter/recovery audit. No personal database or installed process was changed.

The integrated build passes **850 Rust tests**, with three personal diagnostics
ignored, **84 Python tests** (including actual PowerShell), the complete Windows
GNU all-target/all-feature check and the pinned strict documentation build.
Native SHA-256 is
`5275064fcbd3520fba48e3082c900ba17c68b8e8744cf60a01f16bf016f1fe1e`.
All **273 native correctness scenarios** pass across two invocations: the full
run was terminated with exit 143 after 269 passes at 20m17s; the four unfinished
tray scenarios then pass unchanged in 17.300s. No assertion failure occurred in
the interrupted run. Its termination cause remains under investigation; this is
complete scenario coverage, not one uninterrupted successful suite. **915 normal hook test executions pass**, with zero failures and three personal
diagnostics ignored. Source is pushed as `1c0fc4500b1fe568cd64bf2c6d28287bb52a013f`;
remote equality is verified. Logs use `artifacts/logs/profiles-formats-folders-main-*`.

The preceding foundation-only native run passed 266 of 267 scenarios. Its one
failure captured selection state before the keyboard row reveal completed.
The saved test now observes that actual reveal while the write remains pending,
then retains the same rollback/scroll/reader assertions. That corrected path
passes in the new full run, with reviewed evidence in `db208df12e53`.
Root also reviewed integrated backup options, unencrypted warning and compact
dark passphrase controls in `cd98d8d9cb1d`, shared account review in `14a1a3bf11e1`
and `2ce3f9442284`, and folder rollback/uncertainty in `5175f9f7afef`,
`7c92548b3036` and `9cd2124ccb6c`. This is isolated fixture evidence;
actual provider/platform execution and final idle-host performance gates remain
separate.

The previous `cf76976` push also has green documentation CI run `34365629009`.

## Back up included destinations — integrated checkpoint

`c515b80` adds persisted Include choices and Back up all, with independent
progress, Retry and Setup controls. Each destination uses its own saved key,
reserved upload and retention rules. Lost acknowledgments and restart retain the
same encrypted bytes and filename; closing waits for durable receipts. Optional
archive formats and persistent destination failure history remain open.

All **13 integrated native scenarios** pass in 69.878 seconds, including actual
isolated encrypted Local copies, two-destination progress, exclusion/restart,
first-copy setup, lost-reply retry and close. Root reviewed light progress and
compact dark WebPs in `artifacts/e2e/db729f04f50f`. Native SHA-256:
`f88a3e6f515e2c831bb4719374cfec2d63213171e931277399e1bacf87cefd69`.
All **84 Python tests**, full Windows GNU all-target/all-feature checking and
strict Zensical pass. Lane hooks passed 811 executions with three personal
checks ignored. Source **`cf76976`** is pushed to main, with 843 normal hook executions
passing and exact remote equality verified.
Logs use `artifacts/logs/backup-all-main-*`. No live cloud or actual Windows/macOS
verification is inferred, and the installed production app remains unchanged.

## FTP/FTPS and shared credential guards — shipped checkpoint

FTP/FTPS checkpoint `17de158` uses verified TLS by default, clearly selectable
plain FTP, per-destination keychain setup and owned resumable uploads/retention.
All 28 FTP/SFTP lane regressions and six lane native flows pass; normal hooks
passed 802 executions with three personal diagnostics ignored. Main protocol
checks pass. Root preserves the current symbolic launcher release asset alongside
the new libcurl licenses. Optional archive formats, Back up all, richer history,
actual platforms and live providers remain open.

Integrated verification passes: all **13 native scenarios** in 60.678 seconds,
all **81 Python tests** with actual PowerShell, full Windows GNU all-target and
all-feature checking, and strict Zensical. Root reviewed FTP’s plain-connection
warning and compact credentials WebPs in `artifacts/e2e/3baf8f15714b`.
Final native SHA-256:
`ccb2b2672ded47898d260cd93104546f09df834fbfe48816024f6065c9201d29`.
Logs use `artifacts/logs/ftp-credentials-main-*`. Source **`13f9c36`** is pushed
to main with exact remote equality verified. All **834 normal hook executions**
pass (three personal diagnostics ignored). Documentation CI **34362323848**
is green. This source checkpoint is not a new production installation.

## Shared account connection reviews — implementation checkpoint

The native review offers keeping the current device connection or adding the
reviewed shared setup. Changed endpoints get a fresh native UUID requiring
reconnection. The previous account, cached mail, credentials and server IDs remain
at their original endpoints as a local-only “previous setup”. Keeping local
publishes that reviewed connection while preserving unrelated field edits.

Six targeted storage/history/controller tests pass, including same remote IDs on
old/new accounts, lost admission reply/restart, preserved unrelated preferences,
stale native/history/Google/consent changes, remote tombstones and replaced UI
rows. All **32 profile-related native scenarios** pass in 130.488 seconds, including
Add/restart/sync and compact Keep/version selection. All **96 matching profile
Rust tests** and **84 Python tests** pass. Final native SHA-256:
`4df84fef2a8c8aaa7d506d5940c7223213c42957446fa34e00fe64fec6fefc29`.
Reviewed final WebPs include `8b79718ab48e`, `16a1186fee12` and `31e57cf69c34`
under the lane artifacts. The compact test reproduced
iced's Escape picker-dismissal gap; it is retained in R15/R63 and its corrected
mouse selection follows the actual overlay positions. Strict Zensical passes. Normal hooks, root integration and publication remain
required. Logs use `artifacts/logs/profile-account-reviews-*`. Remote removal decisions,
post-enrollment links, protected credential transfer and live interoperability
stay in TODO.

## First-profile native fixture synchronization

The first-device scenario now waits for a loopback upload to be held, navigates
the real Mail controls while it stays pending, then releases the response. It
keeps the existing completion deadline instead of accumulating one-second delays
for every setup record. The held mode and release action control only the owned
fictional server; shutdown also releases pending responses. Three actual HTTP /
batch isolation / cleanup tests pass. All **eight integrated native scenarios**
pass in 43.079 seconds, covering held navigation, interrupted setup/restart and
post-login profile choices. All **84 Python tests** pass, including actual
PowerShell execution. Root reviewed the saved/reopened profile WebP in
`artifacts/e2e/7299297b2b38`. The native binary remains the verified
`ccb2b2672ded47898d260cd93104546f09df834fbfe48816024f6065c9201d29`.
Logs use `artifacts/logs/profile-held-upload-main-*`. Source **`91c59a4`** is
pushed with 834 normal hook executions passing and exact remote equality
verified. Documentation CI **34363018089** is green. The broader functionality
audit stays open.

## Shared account reconnection credential guard

Connection tests and saves refuse old keychain credentials while an imported or
shared account requires reconnection. Fresh explicit credentials remain usable;
SMTP with authentication disabled needs no separate password. Saving resolves all
required secrets before writing any, and only a successful account commit clears
the reconnect marker. Tests alone never clear it. Connection tests now use the
existing account coordinator, so an endpoint review can share that ordering.

Four targeted tests pass through actual engine commands and an owned fake
credential worker, covering incoming/shared/separate SMTP, failed writes and
SQLite restart, preserved cached mail, explicit probes, and ordinary saved-secret
reuse. No network or OS credential access is used in these fixtures. All **773 lane hook executions** pass (three personal diagnostics ignored).
Integrated source is pushed as `13f9c36`; endpoint/removal review UI
is still open. Logs: `artifacts/logs/profile-account-credential-guards.log`.



## 9 September: bounded combined-folder deletion counts

The `codex/bounded-folder-delete` correction replaces the held `6d7c392`
all-folder count map with one optional affected total/unread scalar. Exact
reviewed folder membership and the original query stay in indexed encrypted
scratch, using the worker-owned 2 MiB scratch cache from the R22 foundation.
The scalar and bounded 50-row preview share one cache snapshot; confirmation
refreshes stale reviews, including intervening mail receipts. No folder/search
limit or truncated count is introduced.

One unsubmitted review replaces abandoned reviews without letting an older read
remove a newer snapshot. At most 32 admitted operations retain projection
contexts. Binding is atomic with the durable folder job; completion consumes its
scalar observation and releases scratch even if the UI disappears or the count
read fails. Cancellation and removed accounts release snapshots too; reopening
a cache creates empty scratch. Provider acknowledgments and accepted uncertainty
retain their existing distinct meanings.

All 47 targeted folder/cache/controller tests pass, including 4,096 unrelated
folders, exact filtered counts, partial deletion, review/receipt ordering,
capacity release, ciphertext and restart cleanup. Eight source-view query plans
verify that the added affected-count query needs no temporary grouping/sorting
B-tree or materialization. Existing workspace/group queries and automatic indexes
in recovered views remain a separate encrypted-startup activation audit; these
checks do not declare every existing cache query bounded.

All 15 selected native flows pass on the final executable in 94.817 seconds:
11 folder-control scenarios plus selection, bulk Undo, conversation and sidebar
regressions. All 763 hook test executions pass (698 application Rust, two HTML
dependency and 63 shared profile-core tests; three live diagnostics ignored),
alongside formatting, all-target Clippy, 56 Python tests and strict docs.
Reviewed WebPs
show pending/success (`cbfc961d624a`), light rollback (`d5e01ad44257`), dark newer
selection/reader (`18f6644b7e28`), accepted uncertainty with cached mail after
restart (`caecf27b443f`), and compact dark confirmation (`76a68ae559ce`).
The executable SHA-256 is
`88fb775f31f121cfcfe2a494011b81d0169936757b0fe8a7a8108451b45aa182`.
Logs are under ignored `artifacts/logs/bounded-folder-*`.

This correction depends on the R22 scratch foundation (original `2890fe8`,
isolated cherry-pick `0aeedbe`) and the held deletion commit. Primary-agent
integration/push, aggregate account choice and broader history/scope convergence
remain open. No live-provider, Windows/macOS execution, installed-release or
latency claim is made. Performance gates remain for the final quiet host window.

## 9 September: optimistic deletion in combined folders

The original isolated `6d7c392` checkpoint below passed functional checks but was
held from integration: its all-folder count map and grouped count query were
unbounded. The bounded replacement is recorded separately; this original commit
is not shipping evidence.

R30/R50 deletion immediately removes the reviewed account's affected folders,
rows and query-wide total/unread contributions from combined views. The original SQLite implementation
returned folder counts with the same snapshot as the bounded 50-row page;
selected membership uses the same explicit exclusions. This also preserves
other accounts in aggregate choices and all-folder search. Cache pages arriving
before the operation receipt cannot apply deletion twice.

Rejected or unconfirmed steps restore only their affected membership. Confirmed
steps stay removed after a partial failure. Later folder choices, surviving
readers and unrelated mail moves remain current. A surviving reader opened after
refill can remain outside the restored first page; its cache identity is observed
on refresh and ordinary scope navigation releases that retention. Acknowledged
mail flags/moves/Undo keep the group counts aligned before the next page arrives.
Accepting uncertainty retains cached originals and the explicit unconfirmed
status; it does not acknowledge a server deletion.

Eight deterministic controller/cache cases cover counts beyond one page,
selection capture, stale generations, receipt ordering, success/rejection,
partial success, accepted uncertainty, aggregate account boundaries, search,
newer choices/readers and unrelated acknowledged mail writes.
The three saved native scenarios pass in 30.556 seconds, including light rollback
and dark newer navigation. Their normal controls delete four fictional folders,
retain the Inbox reader, restart after success/rejection/accepted uncertainty,
and inspect only the owned fixture database read-only after graceful close.
Reviewed WebPs include pending deletion (`e9bab5e2cf8d`), restored selection
(`bf11a8c3b976`), newer dark selection (`e9eef084db5a`) and uncertainty review,
retained cache and restart (`4ea06fbca9f4`). Logs are under ignored
`artifacts/logs/combined-delete-*`.

The complete selected folder run passes all 11 native scenarios in 86.180 seconds.
Five related selection/Undo/conversation/sidebar checks pass in 36.328 seconds,
including an explicit assertion that the newer reader is opened while deletion
is still pending. These are selected reruns, not a full native-suite claim.
All 668 Rust tests pass (three authorized-live tests remain ignored), as do
Clippy, 56 Python tests and the strict pinned Zensical build. Normal commit hooks
also run the HTML dependency and shared profile-core contracts. The reviewed
native binary SHA-256 is
`7aa7439f5aba3e61d75642cdd29673d79d003bd844a4781c70ef47ddec6ec03b`.

This is the isolated `codex/combined-folder-delete` checkpoint based on 66e6cb8;
primary-agent integration/push remains required. Aggregate common-folder account
choice and broader mail/history/provider convergence remain in TODO. No live
provider, other-platform or latency claim is made. Grouped cached-query and
renderer performance gates remain for the final quiet host window.

## 9 September: combined folders during pending changes

R30 Ctrl-click now uses the same retained cache path as a plain click while a
folder rename is pending. Selected-folder highlighting follows that identity.
Removing and restoring a folder in a combined selection keeps its actual mail
and counts; committed renames update the selected paths, rejected renames retain
the sources, and newer unrelated folder choices survive either result.

R50/R60 removes an excluded row's unread contribution from the filtered header
immediately. This covers unflagging unread mail in Flagged and marking a Read
result unread. Global Inbox unread badges remain independent of that filter.
The test-support `page_unread` observation reports the actual header value.

Two new controller tests pass: production SQLite rename/rejection with combined
selection, and eight filtered success/failure/cache-before-receipt cases with
an overlapping aggregate/account selection. The first rename regression failed
against the previous code as expected. Both new native scenarios pass in
12.199 seconds, covering slow rename success/rejection, real Ctrl-clicks,
filtered headers and newer choices while flag writes are pending. The first
filter setup used the Read menu row; inspection corrected it to the actual
Flagged row without changing assertions or timeouts.

Reviewed WebPs under `artifacts/e2e/` include pending filtered counts
(`c660b422247c`), dark rollback (`9136758288da`), renamed combined selection
(`1e6c3bdabd36`) and rejected rename (`c9ca6a0ada10`). Logs remain under
`artifacts/logs/combined-folder-{before,unit,native-corrected}.log`.
The first broader native run passed 11 of 13 scenarios. Its POP3 drag still used
an inherited 104-pixel row coordinate; this lane now copies main's existing
`mail_row_y(2)` correction. The badge fixture failed before app launch because
the deep worktree exceeded the Unix socket path limit. Its socket now uses an
owned private short directory, cleaned after the bus and observer stop; logs stay
in artifacts. The Python harness regression exercises a long artifact path,
actual bus access, private permissions and process/directory cleanup.

The corrected complete selected run passes all **13 native scenarios** in
90.117 seconds (`artifacts/logs/combined-folder-native-final.log`), including
folder review/retry/uncertainty/close/POP3 restart, combined choices, filters,
read-on-leave and badge failure/recovery. All **56 Python tests** pass
(`artifacts/logs/combined-folder-python.log`), including 44 harness tests.
These are selected scenarios, not a full native-suite result.

Native test executable SHA-256:
`ede4d68a35d6fbfe4f2c09123ab9cf6eb606f3e2133d46299539c283efdcd771`.
This lane checkpoint awaits primary-agent integration and push. Combined-folder
delete projection, broader page/scope reconciliation, aggregate account choice,
uncertainty/history lifecycle and live-provider/platform verification remain
open. No latency or performance measurements were run; quality/release workflows
remain disabled.

Root integration preserves main’s already-tested short private socket alias and
its actual bus/cleanup regression; the lane’s alternative socket directory is
not needed. Main’s owned GTK Cairo fixture setting is retained. Integrated
verification passes: all **19 selected native scenarios** in 104.376 seconds,
all **81 Python tests** (including actual PowerShell), full Windows GNU checking
and strict Zensical. Root reviewed dark filtered rollback (`2203398171a1`) and
committed combined-folder selection (`cf775ebcb960`). Final native SHA-256:
`e0462c9fbec8bec58798cf2122a3fa1c6a5c222df3a3bc931c5673b0fc2295a0`.
Root logs use `artifacts/logs/folder-counts-main-*`. Source **`97c9a9a`** is
pushed to main with exact remote equality verified; **818 normal hook executions**
pass (three personal diagnostics ignored). The broader TODO items remain open.

## SFTP and connection-intent integration — shipped checkpoint

Root integrates SFTP checkpoint `f60dac0` and account connection-reversion
checkpoint `85b2fd0`. SFTP uses verified SHA-256 host keys before password
authentication, bounded packet framing, exact staged upload recovery and owned
retention/restore. Its dependency requires Rust 1.89; the manifest and installation
docs record that minimum. Password authentication is supported; private-key/agent
authentication, live providers and actual Windows/macOS execution remain open.

Review found an unbounded authenticated session-channel confirmation wait in the
SSH library. The new real loopback test reaches that held stage, advances virtual
time and fails on the original implementation. Bounded channel/subsystem setup
now releases it with an actionable error; retry succeeds without any backup
file write. All 16 SFTP tests pass, with the original failure retained in
`artifacts/logs/sftp-channel-before-fix.log` and corrected provider tests in
`sftp-channel-and-provider-tests.log`. No production timeout or performance
budget was shortened for this test.

Source **`ff03b02`** is pushed to main with exact remote equality verified.
All **816 normal hook test executions** pass (three personal diagnostics ignored),
all **81 Python tests** pass with actual PowerShell execution, full Windows GNU
all-target/all-feature checking and strict Zensical pass. All **18 integrated
native scenarios** pass in 80.717 seconds, covering SFTP host verification/retry,
setup/restart, backup controls, account links, palette preservation and compact
layouts. Root reviewed the changed-host and compact-dark credentials WebPs in
`artifacts/e2e/06f30faf0d64`. Native executable SHA-256:
`17eaed92862af43fcc45ff4fd9a93ac487dfa06e801a92603f8e1a8f97c978f9`.
Logs use `artifacts/logs/sftp-reversions-main-*`. This is a source checkpoint;
production installation and genuine provider/platform verification remain open.

## Palette, S3 and matching-account links — shipped integration

Source **`4e75005`** is pushed to main with exact remote equality verified.
It integrates palette commits `af76f15`/`4727fb7`/`a6dc0c7`, S3 commits
`6d1857c`/`878e9f9` and matching-account import `59c6cf0`. All **88 combined
native scenarios** pass in 320.232 seconds, including palette invalid-input and
contrast recovery, held-provider saves, shared-profile creation/import/conflict
reviews, 12-account paging, backup setup, database transfer, preferences search
and compact layouts. The S3 setup scenario also saves a custom palette through
the global header, then verifies it survives backup settings, connection failure
and restart. No live cloud credentials are used.

Normal hooks pass **798 test executions** (three personal diagnostics ignored),
all **81 Python tests** pass, full Windows GNU all-target/all-feature checking
passes, and strict Zensical passes. Final native SHA-256:
`d0199f2ccbdad14ed0c559b2815b1bcb7384ba0a887e8929f8afdc06cf4d708c`.
Root reviewed compact palette (`516fc5061095`), contrast reset (`0795c44983c5`),
invalid header save (`620e629566fe`), compact account links (`76c52f0613d2`),
account paging (`94ef7df4e703`) and S3 setup/recovery/compact credentials
(`64a7956f0d2a`). Logs use `artifacts/logs/palette-s3-links-main-*`.
R25 is removed after this publication. Palette Drive interoperability remains
R49; further backup providers/options remain R32; endpoint/removal reviews,
post-enrollment linking and credential transfer remain R02/R49. Production
installation, live Google/S3 and actual Windows/macOS execution remain unverified.

The preceding compact/icon source `15a4a3c` also has green documentation CI
**34355986987**. Quality and release CI remain disabled.


## 9 September: compact inbox rows and unread hierarchy

R88 replaces the 104-pixel avatar rows with 60-pixel conversation rows. Wide lists
show sender, subject, snippet and time in one horizontal line; narrower lists use
two bounded lines. Unread messages have a subtle surface tint, a dot and bold
subject. Selected and hovered surfaces remain distinct in both themes. Existing
measured Ellipsis widgets keep long labels inside their actual space. Flag and
checkbox controls retain 40×44-pixel targets, with a scrollbar gutter protecting
the flag outline. Navigation, deletion reveal and virtualization share ROW_HEIGHT.

All-target/all-feature checking and Clippy pass. The unread-surface regression
passes. Eight selected native scenarios pass: light/dark/wide/900×640 layouts,
flag/read/additive checkbox and row selection, double-click reading, all five
adjacent-deletion scenarios, and filtering/sorting/paging. Twenty further saved
selection/context/move/drag scenarios pass after migrating their intended row
coordinates to `mail_row_y`, which accepts observed scroll and interface scale.
No existing assertion or timeout was weakened. Earlier runs found seven stale
coordinate failures; one rerun still had the old script loaded before the actual
migration. The final corrected group passes all 20 in 54.803 seconds.

Logs: `artifacts/logs/compact-mail-refined-native.log` and
`artifacts/logs/compact-mail-migrated-controls.log`. Reviewed WebPs in
`623901bc29bf` show clean ellipsis, the inset flag border, compact unread styling
and wide horizontal rows; `2f8cd446e903` records actual row controls.
Native binary SHA-256:
`03417e33e3cade88ad6108144f74946ca9dfee051a20816cc3759c046f3be854`.
These are selected Linux fixture checks, not full-suite, live-provider or latency
claims. Primary-agent integration and publication are recorded below.

### Integrated compact inbox, icons and conversation refresh

Main integrates the compact rows and transparent icons with the selected-account
Move changes and current profile/backup controls. Keyboard reveal now uses the
actual native viewport, rejects superseded scope/selection results, and keeps
whole rows visible at 120% and 900×640. The controller and layout-operation tests
cover delayed results, visible rows, clamping and previous-page selection.

Refreshing a thread after moving an expanded older reply now retains its surviving
selected Inbox anchor. Explicit conversation paging still opens the new page's
first row; newer expanded focus and body revisions remain protected. Five
controller tests and the saved slow-success/failure/newer-focus native matrix
cover this correction (agent checkpoint `279a72a`). Mail control coordinates in
34 existing scenarios and all three HTML timing scripts now share compact row
geometry. Deliberately rapid keyboard sequences remain rapid.

The complete **248 functional native scenarios** ran on SHA-256
`e038afe69e484f0354b0d5086a118fd973ac12c4f48357391be9560b89f1d49c`:
246 passed and two test setup failures remained. The cross-page deletion test had
captured an earlier key's scroll offset before the last-row layout operation; it
now awaits actual last-row visibility before keeping the unchanged post-delete
scroll assertion. Its corrected native run passes. The other failure was the
known unpainted GTK file picker. The isolated Xvfb harness now selects GTK's
[Cairo renderer](https://docs.gtk.org/gtk4/running.html), keeping production
rendering, path confinement, clipboard ownership proof and deadlines unchanged.
All **19 affected picker, database, attachment, print and clipboard scenarios**
pass in 94.308 seconds. This removes a GPU startup dependency in the fixture; it
does not prove the cause of every earlier GTK startup failure. All 248 paths have
passing coverage across the full run and these corrected reruns; this is not
reported as a clean single full-suite run.

All **81 Python tests**, including actual isolated PowerShell execution, pass.
Full Windows GNU all-target/all-feature checking passes. Reviewed WebPs include
compact scaled reveal (`3fb5b229091e`), deletion across a page boundary
(`570985938292`), and the rendered native picker plus import/restart
(`a6de9837219d`). Logs: `artifacts/logs/compact-icons-final-native.log`,
`compact-final-boundary-reveal.log`, `compact-cairo-picker-native.log`,
`compact-final-python.log`, `compact-icons-final-windows.log` and the targeted
navigation/reveal logs. Normal hooks pass **769 executions** (three personal diagnostics ignored),
strict Zensical passes, and source is pushed as **`15a4a3c`** with exact remote
equality verified. R88 is removed from TODO after this source publication. Production installation and actual desktop
shell/platform review remain R64/R08 work; no personal data was changed.
Performance measurements remain deferred while parallel builds run.

## Account connection reversions — review groundwork

Native incoming/SMTP connection changes now have independent durable intent
generations, alongside account-name generations. Reverting an endpoint,
authentication or sent-copy choice before the next pull remains a local edit
after restart; a rename or unchanged save cannot invent a connection change.
This uses the existing Store owner and profile checkpoint, with no added lock.
Two new SQLite/history regressions cover four connection-field reversions and
rename/no-op isolation; all 102 matching profile tests pass (one personal
diagnostic ignored). Log: `artifacts/logs/profile-account-reversions-all-tests.log`.
Mandatory hooks and root integration remain pending. Shared endpoint/removal
review controls and credential retargeting protection are separate unfinished
R02/R49 work; this checkpoint never applies a remote endpoint.


## Explicit account links during profile import — integration pending

A reviewed shared account can reuse an existing native account only when every
portable incoming/SMTP connection field matches. The choice preserves native
identity, cached mail, its credential slot, local display-name intent and any
existing reconnect requirement. Unlinked imports retain fresh local IDs and
Reconnect. One local account cannot serve two shared IDs. Reviews render eight
accounts per page, preserve choices across pages and freeze the exact choices in
the acceptance receipt. Changed/reverted connections, stale controls and a
lost-acknowledgment retry with different choices are rejected.

All 100 matching profile tests pass (one personal diagnostic ignored), including
link/restart, connection changes and reversions, reconnect preservation, bounded
pages/duplicate choices and native control-generation contracts. Clippy passes.
Four saved native paths cover reuse/restart, explicit Add new, compact dark
choices and a twelve-account paged import with the original local account kept.
Reviewed WebPs include `1d578884f9f6`, `aac242c963f8`, `aa23fde24ad1`.
Native binary SHA-256:
`c6f19a26b45797e821203d90885bc3d8b005400990abb7b4af4c55b0f5c02ef6`.
Logs use `artifacts/logs/profile-links-*` in the isolated profile-cache worktree.
All 30 profile native paths were exercised: 29 passed together; the existing
first-device slow-upload completion timed out once and passed unchanged on rerun
(`75ec92a70a21` / `a3349d5457bf`). No assertion, timeout or artificial server delay
was weakened. This fixture readiness follow-up remains R63. Mandatory checks and
root integration/shipping are pending.
Post-enrollment linking/suppression, endpoint/removal reviews, credential transfer
and actual cross-client Google verification remain R02/R49/R92 work.

## Conversation scratch ranking — R22 ongoing

Conversation metadata no longer passes through whole-thread ROW_NUMBER windows.
The owning encrypted scratch database picks one copy per logical message and
indexes chronological order; Rust receives at most 20 metadata rows. Selected
mailbox UID, expanded-message focus, timestamp/ID ties and paging are preserved.

Scratch now uses DELETE/OFF with normal transaction rollback and no crash
survival requirement. Main cache/receipt durability remains FULL. The earlier
unqualified WAL initializer also changed scratch to WAL; qualifying the main
pragma makes the intended scratch policy effective. Restart creates new scratch
and ignores crash leftovers. Orphan cleanup remains unfinished.

All five existing conversation contracts, three ranking tests and three scratch
tests pass. The encrypted fixture compares 512 logical messages across three
mailbox copies, focus and timestamp ties against the prior query contract. Query
plans reject TEMP sorting, materialization and automatic indexes. Normal rollback
is tested beyond the scratch page-cache target, and restart ignores an encrypted
orphan while preserving main FULL durability.

Eight of nine native conversation/forward/scroll/selection/bulk-close flows pass.
The remaining reader scenario exposes the lane's old move-focus fallback, already
fixed on main in 15a4a3c; its failure receipt remains preserved, and the combined
main scenario must pass before shipping. Reviewed compact/card/page/review WebPs
are under `artifacts/e2e/`; binary SHA-256 is
`5a675577419d4e16442059dba69fb509799c61198bc4ef560834471fbf2ff7c1`.
Final hooks, combined main native checks and root shipping remain pending. No
performance result or production encryption activation is claimed.

## R22 foundation temporary-storage correction (awaiting shipping)

The SQLCipher foundation's global TEMP_STORE=3 policy would have moved existing
plaintext sorts into memory. This correction preserves FILE for every ordinary
connection and enforces MEMORY for keyed main databases through the existing
native codec lookup. Encrypted scratch attachments do not change a plaintext
main's policy. FILE/DEFAULT resets fail after keying, including after authorizer
replacement/removal. A fixture VFS observes actual temp opens alongside a plain
positive control. Commit `86d9c6c` passes all three targeted regressions, 811 normal
hook executions, 81 Python tests and strict docs; root shipping remains pending. The scratch drain fixture now observes removal of the
entire owned directory, avoiding the interval between file and directory deletion
without relaxing its required outcome or deadline.

Selection account/folder summaries, workspace DISTINCT/Inbox GROUP BY and
recovered-view automatic indexes remain explicit encrypted-startup gates. No
personal cache migration is activated and no timing result is claimed.

## Keyed raw database export — R22 ongoing

An encrypted Store exports through an independent, URI-readonly snapshot and
SQLCipher logical conversion. The existing output remains ordinary SQLite,
excluding account passwords and Google credentials. Its private atomic candidate
is in the explicitly chosen export folder and is part of that intentional raw
output. Implicit cache/import/migration candidates still require encryption.
Cancellation and dropped observers share a bounded watch signal with SQL progress
handlers and prepublication checks; admitted cache saves continue independently.

The conversion audit verified indexed mail rowids and external-content FTS, then
reproduced renumbering of an unindexed rowid 991 to 1. A minimal export-only SQLite
flag patch selects its existing rowid-preserving transfer path. It restores the
caller's flags and does not alter cipher algorithms; source hashes/provenance and
release licenses accompany the SQLCipher patches. Tests preserve arbitrary
extension data, drafts/attachments, an unindexed extension rowid 4444, the exact
snapshot during concurrent saves, and existing destination files after Cancel or
a dropped observer. Seven export tests, 18 cipher tests and all eight saved native
import/export/profile/restart/cancellation flows pass. Reviewed light/compact-dark
WebPs show usable review, error and inline-editor controls; native binary SHA-256
is `ae760ffcea8a0af6f0dda5c426ee4282d4c034a7742c71a0518f43a1dab2f5b7`.
These native fixtures still use plain startup; encrypted disk conversion has the
separate Rust fixtures. Full Windows GNU all-target/all-feature checking and
strict docs pass; actual Windows/macOS execution is unverified. Normal hooks
and root shipping remain pending.
Encrypted import staging, guarded migration/recovery and native key recovery are
still unfinished; production bootstrap remains unchanged.

## Keyed local profile routing — R22 ongoing

The local profile catalog has an explicit keyed constructor. Catalog reopen,
active-profile opening, imported-marker validation, registration, activation and
orphan recovery propagate the same device key. A wrong key or unexpected plain
profile fails without fallback or replacement; one damaged orphan does not hide
other recoverable profiles. Separate catalog owners keep revision checks.

All 11 catalog tests pass, including three new encrypted disk/WAL/restart,
wrong-key/plaintext recovery and competing-owner regressions. These exercise the
real Store/catalog workers on isolated temporary files. Bootstrap still calls the
plain constructor; production migration, portable-copy conversion and native key
recovery remain unfinished. Existing import UI flows therefore remain unchanged;
this is backend fixture evidence, not an encrypted native startup claim. Final
hooks and root shipping are recorded with the checkpoint commit.

## Encrypted cache foundation — R22 ongoing

The isolated `codex/encrypted-cache` lane adds SQLCipher 4.19 / SQLite 3.53.4,
zeroizing raw keys, key creation/read-back through the bounded credential actor,
keyed Store/backup/profile paths and a read-only encrypted candidate migration.
Selection snapshots and frozen reviews use worker-owned encrypted scratch;
ordering builds an on-disk index instead of an unbounded sort/window. Shared
ancestry uses indexed database scratch and a 128-ID frontier. Normal startup
still awaits guarded publication, recovery, remaining sorter/temp-data auditing
and portable transfer routing. The [storage boundary](agents/CACHE_ENCRYPTION.md)
inventories remaining paths.

Actual temporary SQLite/WAL fixtures verify ciphertext, corruption/wrong-key
failure, worker reopen, search/settings, upload-session preservation, cancellation
without changing the source, and scratch cleanup after admitted writes drain.
All nine selection integration tests preserve page/search/range/frozen-review
semantics. Shared initializer, ancestry and owned-lock APIs are published and
pinned as `e3e69a4`, with 63 isolated shared tests/all-target Clippy and 692 normal
hook executions passing (three personal diagnostics ignored).

Full desktop checks caught SQLCipher automatic process-exit cleanup racing a
remaining cache worker. A deterministic subprocess reproduced SIGSEGV in
`sqlite3Codec` after eviction required a disk read. The documented lifecycle
patch disables automatic global cleanup, while explicit shutdown/reinitialization
still works. All eight direct cipher tests and 805 complete hook executions pass
(three personal diagnostics ignored), alongside 81 Python tests and strict docs.
All 19 selected native selection/bulk/drag/restart flows pass through the saved
MCP equivalents. Reviewed light, compact-dark, Undo and cross-folder search WebPs
show readable controls and preserved selection/recovery behavior. Native binary
SHA-256: `26df656af1ff9222746d3f4823dc137a41ab10f97d7c519995ca6c1868adf52f`.
Full Windows GNU all-target/all-feature checking passes; actual Windows/macOS
execution and macOS compilation remain unverified. Logs and source evidence are
under `artifacts/logs/r22-*`; screenshots include runs `40496105fe02`,
`04c0fb32b928`, `1062fd6439ae`, `f530cc327265` and `6e0b34ee08dc`.

These native fixtures validate existing UI/storage behavior with the new SQLite
build; keyed disk/migration behavior has separate Rust fixtures. Normal native
startup/recovery is still unfinished. The root's newer static-libcurl dependency reproduced an earlier OpenSSL
initialization in curl's pre-main constructor. A minimal vendored curl cfg patch
now invokes its existing Rust OpenSSL initializer before libcurl, preserving the
same process-lifetime policy without constructor-order assumptions or changes to
certificate/FTP behavior. The exact curl 0.4.50 / curl-sys 0.4.90+curl-8.21.0
subprocess fails before and passes after this patch. Existing CA-path probing is
unchanged. Final combined hooks/platform checks, root's FTP protocol regressions
and root shipping remain the checkpoint boundary.
No personal cache, live credentials or installed application was changed.

## Portable preference reviews — verified integration

A local history review can compare this device's preference with current shared
versions. Choosing a value reserves one durable operation and the exact concurrent
version set before history admission; the existing sync worker publishes it later.
Opaque shared extensions remain in history and survive the selected resolution.
The UI receives scalar values and IDs for at most eight supported settings and
256 versions per field, not complete operation payloads.

Acceptance checks enrollment/Google lifecycle, current field values, native edit
generations and the reviewed history revision. Changes or reversions while a
review is open require refreshing; unrelated local preferences remain intact.
Reset-to-default requests retain their original wire action while using the
native default for comparison. Admitted receipts are acknowledged through close
and newer native edits remain pending for subsequent publication.

Six targeted store/history/controller regressions pass, covering deferred
local/shared choices, concurrent causal records, stale local/remote/consent
reviews, restart after lost acknowledgment, newer native intent and stale control
events after a row disappears. All 86 matching profile tests pass (one personal
diagnostic ignored). The lane also passes 59 Python tests and all-target,
all-feature Clippy. All 25 selected native profile flows pass, including three
saved resolution/restart/stale-choice scenarios; reviewed light/compact-dark WebPs
include visible error feedback when an old choice is rejected. Logs are under
`artifacts/logs/profile-reviews-*`; error feedback evidence is `8e903b30229c`.
Root integrated source `713f96e` and repeated all 25 native profile flows on the
final control-identity guard: all pass in 90.568 seconds. All 81 integrated Python
tests and Clippy pass. Light/shared-choice, compact-dark and stale-choice error
WebPs were reviewed in `ae0d1e6a4ee3`, `8df28384ff98` and `d9bce3c1db21`.
Native SHA-256: `d85786600d99d32e2e18633e48a294350ded4780bae938acea8756172c31850b`.
Both lane and root normal hooks pass 762 executions (three personal diagnostics
ignored). Source is pushed as `22a3af5`, with exact remote equality verified.
Installed production is unchanged. Integrated logs use `artifacts/logs/profile-reviews-main-*`.
This does not complete account linking, endpoint/removal reviews, credential
transfer or live Google verification.


## Multiple Local/Drive backups — shipped checkpoint

Agent checkpoint `3f00292` adds named Local/Drive destinations, independent
schedules, retention/passphrases and history, switching and reviewed removal.
Existing single-target settings migrate when another is added; imported database
profiles clear device-local schedules. Duplicate targets and local aliases are
rejected. The remaining S3/FTP/FTPS/SFTP adapters, optional compression/encryption,
combined manual backup and richer result history remain R32 work.

The lane passes mandatory hooks (706 executions, three personal diagnostics
ignored), 37 backup tests, nine preferences tests and 55 Python tests. Its final
three native scenarios pass in 11.220 seconds, covering existing first-copy setup,
compact layout and the new migration/switch/removal/restart workflow. Root reviewed
light, red removal and compact dark WebPs under `3ee3c19c5507`; final equivalent
run is `c023ba64588f`. Root integration preserves main's typed portable preference
intent and native edit generations; a new regression combines a stale backup form,
a remote setting change and another destination's completed upload. Merged checks
pass: ten preferences tests, 756 mandatory hook executions (three personal
diagnostics ignored), 59 Python tests and strict Zensical. All 21 integrated
backup/preferences/profile/import native scenarios pass in 95.559 seconds. Root
reviewed removal and compact dark WebPs in `ffa204429d56`. Native SHA-256:
`73617db5e8a89119e7f055623e061fb55ee16e7e6b6bf72ad135b36403ac14bb`.
Source [`2b87db4`](https://github.com/sam-ruff/shep.so/commit/2b87db4) is pushed,
with exact remote equality verified.
[Documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34342193801)
passed. Logs use `artifacts/logs/multiple-backups-*`. No live cloud or installed
production update is claimed.

## Windows/macOS badges — shipped integration

Root integrated `66222f6` as
[`1595fb3`](https://github.com/sam-ruff/shep.so/commit/1595fb3) and pushed to main,
verifying exact remote equality. Mandatory hooks pass 751 executions, three
personal diagnostics ignored; Python: 59 and strict Zensical pass. Full merged
Windows GNU all-target/all-feature checking and the exact macOS adapter check
pass. All 12 integrated Linux badge/tray scenarios pass in 56.460 seconds, including
ordinary-hide failure recovery. Root reviewed compact preferences (`d197a0b27609`)
and failed-archive recovery (`96a237e88cf1`) WebPs. Native SHA-256:
`aa9145b440fa98d69b9d69fa93d95fe92882c7e19374961e8df38d6755ae9b22`.
Logs: `artifacts/logs/badge-main-*`. Actual Windows/macOS rendering and shell
execution remain R70 verification work; production installation is unchanged.
## Configurable color palettes — R25 lane checkpoint

The model now stores independent light/dark RGB palettes for fourteen semantic
roles. The searchable Colors editor supports hex input, swatches, a sample,
Apply, Undo changes and per-theme Reset. Invalid values stay out of preferences;
low-contrast combinations show a warning while the editor retains readable
controls. Applying is immediate and uses the existing ordered preference save.
The UI owns a small theme cache and retains untouched colors if preferences
change during an edit. Existing installations get the original default palettes.

The lane rebased onto main `22a3af5`, preserving its profile reviews, tray safety
and multiple-backup work. Typed preference saves now carry fixed-size per-theme,
per-role color intent, preserving unrelated colors through queued writes, old
acknowledgments, explicit reversions and stale-save retries. All twelve targeted
palette model/controller/store tests pass, including semantic mapping of every
role and System appearance. All 81 Python tests pass.

All ten selected native scenarios pass on the final binary in 42.659 seconds:
the five palette flows plus default appearance/calendar, persisted resizing,
filtered Preferences clipping, tooltip/search and continuous-profile receipt.
The compact footer keeps Apply clear of the save toast. Both Apply colors and
the global Save changes button consume staged colors; invalid input retains the
previous saved value, explains the Colors error and clears old success feedback.
Reviewed WebPs are under `artifacts/e2e/1181ed2a3a3f`, `bb598d186559`,
`3208afaa547a`, `c9978f1dfd9c`, `7fe3cefac699` and `d40509ed976a` in the
isolated lane. Native binary SHA-256:
`66f740bde3fc4bc39e1604de7ee833077cc893065f4851fb13fde73877c9122b`.
Logs use `artifacts/logs/r25-*`. Full Windows GNU all-target/all-feature checking
and strict pinned Zensical pass. The typed-save commit `4727fb7` passes 774 normal-hook executions (three personal
diagnostics ignored); the final header-save follow-up is gated by the same hooks.
Root integration/shipping remains the authorized next step.

Palette values are included in local preferences and database transfer. The
current cross-client codec has no palette setting key, so custom-color Drive
replication remains part of R02/R49 and is not claimed by this checkpoint. No
personal installation, live-provider or Windows/macOS runtime test was performed.
Root integration/shipping is pending; R25 stays in TODO.

## Transparent native Shepherd icons — R64 checkpoint

The approved source PNGs remain unchanged. Built-in imagegen background extraction
was applied, and the ragged dark result was rejected. The user-authorized Vectorizer
service then traced the light extraction and approved dark reference into clean
editable vectors; its keys stayed only in the original tooling configuration and
request memory. Light interior opacity was corrected after the regression test
caught the trace's slight translucency. Prompts, provenance and export instructions
are in `assets/README.md`. The shipped light/dark WebP now has true exterior alpha,
fully opaque flat-color interiors, and the approved Shepherd contour/details.
The compatibility launcher PNG is transparent too.

Linux installs the traced symbolic SVG alongside the full-color PNG, retaining
`so.shep.Shep.desktop` and StartupWMClass. Its symbolic foreground follows native
GTK/system styling without an app-owned theme watcher or filesystem writes during
theme changes. The StatusNotifier adapter supplies the same symbolic name with a
transparent full-color pixmap fallback. macOS requests native template treatment
for its symbolic WebP mask; Windows retains a transparent full-color tray icon.
The development-only export script uses CairoSVG/Pillow and makes no network call.

All three targeted tray Rust tests pass, including real alpha/opaque-interior/clean
margin checks. All 81 Python tests pass, including actual isolated installer
update/uninstall and raw Linux/macOS/Windows package paths. Full Windows GNU
all-target/all-feature checking and exact macOS native tray adapter checking pass.
Nine saved native MCP scenarios pass on binary SHA-256
`6ec92da765e6f99f13b9fd3c1794b8151784148df8c79bded5433ea0b3d16a68`:
hidden-app icon light/dark, tray lifecycle/host loss/background mail, appearance,
and all four unread badge scenarios. The new flow observes real X11 WM_CLASS and
actual SNI IconName, then clicks the owned GTK host's theme button while Shep is
hidden. It never changes the personal desktop theme.

Reviewed WebPs include `artifacts/e2e/c7d083fec4d5/symbolic-tray-light.webp`,
`symbolic-tray-dark.webp`, `transparent-logo-restored.webp`, the compact dark tray
preferences in `fee7caa42dcf/`, and dark Calendar in `f2c81dda84df/`. The contour is
clean and the symbolic foreground changes visibly. These are GTK/native-protocol
fixtures, not an actual GNOME Shell session or Windows/macOS runtime review.
Strict docs, mandatory hooks and root source integration are recorded with the
checkpoint; personal installation and live shell/panel review remain open in R64.
No installed application, personal icon, desktop favorite or secret was changed.

The first mandatory hook hit an unrelated shared-core history reopen failure:
`reserved_upload_identity_and_exact_bytes_survive_lost_replies_and_reopening`
returned `Owned` at history.rs:482 after dropping its Journal. The exact unchanged
pinned binary test and all 11 history tests passed on rerun. The retained File lock
and another parallel test's child-process spawn suggest transient fork/exec
inheritance, but that cause is not proven. No dependency checkout/assertion changed;
the full mandatory hook is rerun normally. Evidence is retained in
`r64-commit.log` and `r64-profile-ownership-rerun.log`; follow-up remains in R91.

## Native PowerShell Windows raw installer — R61 checkpoint

README and the install guide now include the GitHub raw PowerShell command.
The script requires built-in PowerShell 5.1 and Windows 10/11 tar.exe, defaults to
LocalAppData/Programs/Shep, and stages a native ICO and Start-menu shortcut.
Archive selection/checksums reject missing or ambiguous platform assets; only
exact unique regular binary/icon members stream out through binary process stdout.
An application marker prevents overwriting an unrelated installation. Directory
replacement and the shortcut commit preserve/restore the previous application
when a step fails. Updates never kill an open app or change mail/configuration.

Explicit all-user installation uses a data manifest, prepared fixed script and
quoted encoded invocation for [native RunAs elevation](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/start-process).
Only that installer process permits script execution; no system execution policy
changes. Cancellation happens before installed files change. The shortcut follows
[Microsoft's WScript contract](https://learn.microsoft.com/en-us/powershell/scripting/samples/creating-.net-and-com-objects--new-object-).
The PNG-backed ICO contains the unchanged approved 128px launcher image.

Seven isolated PowerShell execution tests pass: binary-safe install/update and
native metadata/icon contract, invalid checksums and links, failed shortcut commit
rollback, cancelled and successful staged elevation with spaces/apostrophes,
user/default/custom scope, pre-download cancellation, HTTPS-only transport,
invalid version, missing platform/release, duplicate members and unrelated apps.
All 81 Python tests pass. PowerShell 7.6.6 was downloaded from Microsoft's release
into ignored artifacts and verified against its published checksum; no runtime
was installed globally. Tests use actual PowerShell, tar, hashing and filesystem
operations against temporary destinations and an owned loopback release server,
with explicit environment, COM, transport and UAC boundary fixtures. These do not
claim actual Windows PowerShell 5.1, COM, UAC or Start-menu rendering verification.

Root integrated and pushed Windows source as `ce4a2a6`, with 81 Python tests,
756 mandatory hook executions (three personal diagnostics ignored), strict docs
and [green documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34344920136).
Exact remote equality was verified. Actual Windows/macOS execution, platform
distribution and published binary assets remain
open in R61. No developer installation or public release was changed; quality and
release CI remain disabled.

## Native-tool macOS raw installer — R61 checkpoint

The raw macOS entry point prepares `~/Applications/Shep.app` with the approved
launcher icon and native application identity using built-in macOS tools only.
The [bundle structure and property keys](https://developer.apple.com/library/archive/documentation/CoreFoundation/Conceptual/CFBundles/BundleTypes.html)
follow Apple's application-bundle contract. The script selects a matching
published architecture, verifies SHA-256, streams only exact unique regular install
members, stages an application bundle and replaces it with rollback. Explicit
all-user installs elevate only the prepared final copy; cancellation leaves the
old application intact. Gatekeeper, running processes and user data are preserved.

Six isolated shell contract tests pass, covering install/update/identity/icon,
checksum and link rejection, failed replacement rollback, missing architecture
and release, refusal to replace another application, conflicting scope flags,
terminal cancellation before network work and cancelled administrator elevation.
They run actual Bash, tar, checksums and temporary filesystem operations with
loopback release downloads and test doubles for JXA/Foundation, plutil, icon tools
and sudo. The embedded JSON selector executes in Node; a poisoned `python3` on
PATH proves that the installer does not invoke a Python runtime. This is Linux
fixture evidence, not actual macOS execution or icon/Launchpad verification.
All 74 Python tests pass; Bash syntax, strict documentation and mandatory hooks
are recorded with this checkpoint. Windows PowerShell implementation, actual
macOS desktop execution, notarization/distribution and published release assets
remain open. No installed user application was changed and no release was created.

Root integrated and pushed macOS source as `0f2af1f`, verifying exact remote
equality. Mandatory hooks pass 756 executions (three personal diagnostics
ignored), all 74 Python tests and strict Zensical pass.
[Documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34343891835)
passed. Linux source `3676dad` also has
[green documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34342832313).


## Raw Linux release installer — R61 checkpoint

README's first install option and the installation guide now show the standalone
GitHub raw Bash entry point. Its standard-library helper resolves a published
Linux archive and checksum, stages/validates files before installation and delegates
to the existing atomic binary/native-menu installer. Per-user installation remains
the default; interactive scope offers all users or cancel, and only an explicitly
selected system install invokes sudo. Updates leave already-open executables and
user data alone. Version selection, prompt-free user installs, custom paths and
optional GNOME pinning are supported.

Nine isolated tests exercise actual loopback metadata/archive/checksum downloads,
installation and atomic update in temporary prefixes, native launcher identity,
checksum/asset/archive rejection, scope/sudo cancellation, interrupted downloads,
unsafe paths/links, raw-wrapper argument quoting and staging cleanup. All 68 Python
tests pass; Bash syntax and strict pinned documentation checking pass. No native
UI changed. Mandatory hook results and source shipping are recorded during root
integration. Root merged and pushed Linux source as `3676dad`, with exact remote
equality verified, 756 hook executions (three personal diagnostics ignored),
68 Python tests, Bash syntax and strict Zensical passing. No release was published and no developer installation was changed.

A read-only public release API check returned no published releases. The README
and installer explain that state without claiming a working public binary download;
quality/release CI stays disabled. Windows/macOS scripts, platform execution and
actual published-archive installation remain in R61 for the next checkpoints.

## Windows/macOS unread badges — integration checkpoint

The existing default-on unread preference and aggregate optimistic Inbox count
now feed all three native adapters. Windows prepares a transparent red count
image on its independent watch worker, displays 99+ above 99, and keeps the exact
count in the accessible description. Its HWND-owned subclass reapplies the current
image after taskbar recreation and releases its resources with the window;
tray reopening uses the newest retained frame. macOS uses the system Dock badge
label on AppKit's main queue, including when the window is hidden. It waits for
one native acknowledgment before admitting another and retains only the latest
pending count. Zero or disabling the preference clears the badge.

The platform contracts follow [Microsoft's overlay API](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nf-shobjidl_core-itaskbarlist3-setoverlayicon)
and [Apple's Dock badge property](https://developer.apple.com/documentation/appkit/nsdocktile/badgelabel).
Windows overlays require large taskbar icons. Five targeted Rust tests pass,
including held native delivery, an occupied output bridge, zero/large-count
raster validation and unchanged Linux bus/reconnect regressions. Prepared 32px
WebPs for 1, 10 and 99+ were reviewed under `artifacts/e2e/taskbar-raster`;
these images are not screenshots of Windows. Full Windows GNU all-target/all-feature
checking, Windows Clippy with warnings denied, and exact macOS adapter checking
pass. The Windows check also caught an existing cfg-dependent unused index in a
profile path-protection test; separating its Unix alias loop preserves that test
coverage on both targets. The reproducible adapter check is
`scripts/check_badge_adapters.py`.

All 11 selected native badge/tray scenarios pass, and all 59 Python tests pass.
Reviewed current native WebPs include `a21f7f3fe231/badge-preference-compact-dark.webp`
and `de3ee2be6d28/badge-failed-archive.webp`. The native binary SHA-256 is
`408b7732dca0ff00e62e4651fcf53045b82332cca978a7fb5071422d35a7fc0e`.
The first badge run exposed the existing fixture's AF_UNIX path limit in a long
worktree, before application actions; an owned short alias and actual private-bus
cleanup regression now cover it. Native Linux badge behavior is unchanged.
Strict documentation and mandatory hook results are recorded during integration. Actual Windows/macOS rendering, Explorer restart, hidden AppKit delivery,
full macOS app checking, production installation and root integration/shipping
remain separate unfinished work. Linux adapter behavior is unchanged. R70 stays
in TODO for platform/runtime and remaining ambiguous-provider/restart count work.


## Native tray and window lifecycle — integration checkpoint

Preferences → System tray controls close-to-tray, off by default, with an
accessible Quit Shep button. The native tray offers Open Shep and Quit Shep.
Closing to tray keeps engine subscriptions and background arrivals alive; opening
recreates the window with its current size, theme, reader and drafts. An iced
daemon owns the process so a closed native window is separate from actual Quit.

Actual Quit retains the durable-close barriers. With ordinary close-to-tray off,
required saves temporarily hide the window and request a native saving notice;
completion exits, while failed saves or Open cancel close and restore the window.
Missing/lost tray support keeps or restores an accessible window. A native file
chooser stays visible; once selection finishes its attachment write can hide and
recover normally. Notification-service failure falls back to visible saving.
Read-only synchronization is not a shutdown dependency; the idle bulk-stop
handshake alone does not create a saving notification.

Linux uses StatusNotifierItem/DBusMenu with independent capacity-one coalescing
action and availability channels. Native callbacks retain the latest Open/Quit
intent when the UI is busy; a deterministic full-signal test covers this.
Windows/macOS create native icons/menus on iced's window event thread. The approved logo loads once
on a background worker. Native fixtures own their D-Bus/GTK tray and notification
services on Xvfb; they never access the user's session bus, mail, keychain or Drive.

Nine targeted lifecycle/controller regressions and 58 Python tests pass.
**All 26 selected native correctness scenarios pass**, including seven tray flows
plus existing draft, bulk/folder receipts, profile, database transfer, held-sync
and preferences flows. Reviewed WebPs cover compact dark preferences
(`a605cc916d14`), native menu (`0b42e6ef0f7d`), missing host (`29c90843c818`),
send failure (`fed756dada64`) and attachment-picker failure (`c92068817f79`).
These retain clear controls, recoverable errors and draft contents. Native binary
SHA-256: `4fb723a05a5573147627fb58297b92ec065ea93f3710b0fc4cc379802f2fc087`.
Mandatory hook results and accepted commit/shipping are recorded during root integration. Full Windows GNU all-target/all-feature checking passes; the exact
macOS tray adapter compiles for aarch64-apple-darwin using the reproducible
`scripts/check_tray_adapters.py`. That macOS check does not compile/link the full
app. Neither cross-check proves actual Windows/macOS execution. Actual desktop
shell review, personal-server diagnosis, root integration/shipping and a production
installation remain distinct unfinished work; no performance claim is made.



Root integrated the native tray checkpoint as
[`6728931`](https://github.com/sam-ruff/shep.so/commit/6728931) and pushed it to main,
verifying exact remote equality. Mandatory hooks pass 745 executions with three
personal diagnostics ignored; Python58 and strict Zensical pass. All 32 selected
integrated native scenarios pass in170.625 seconds, including the newer adjacent
selection and profile-cache restart regressions. Binary SHA-256:
`cfd6e424e619821cde28b4f041c6a06470dab67769474189aa178d804d89c65d`.
Log: `artifacts/logs/tray-main-native.log`. Root reviewed the integrated compact
preferences (`62acdb7a7cee`) and failed-send recovery (`e410af37a3bd`) WebPs.

A follow-up saved native scenario reproduced an additional ordinary-close case:
with close-to-tray enabled, a failed pending send remained hidden. Reproduction
log: `artifacts/logs/tray-ordinary-hide-reproduction.log`. This is separate from
temporary-saving mode, whose failure recovery already passes. The root is adding
fresh-error recovery for pending writes and visible queue-admission failure;
old errors and routine read-only refreshes must not reopen the window.
The first full hook run caught two older-operation ownership regressions in the
initial correction. Explicit hidden-window state and pre-event close ownership
now distinguish ordinary hide, startup, and a pending Quit; an obsolete attachment
failure cannot cancel a newer required save. The failing hooks were retained and
corrected, without bypassing them. Final verification/shipping remains pending.

The corrected ordinary-hide follow-up is pushed as
[`b8eafde`](https://github.com/sam-ruff/shep.so/commit/b8eafde), with exact remote
equality verified. All 12 tray/controller tests and 748 hook executions pass
(three personal diagnostics ignored), as do all16 selected native scenarios in
78.974 seconds and strict Zensical. The new native scenario covers both ordinary
hide and subsequent actual tray-menu Quit. Root reviewed the retained reply and
visible error in `df09111f194d/tray-ordinary-hide-write-failure.webp`.
Native SHA-256: `811d628d18dc80f74f7e83ac991f7bc92f749e3388a022c8999d914c01f1b5dc`.
Logs use `artifacts/logs/tray-ordinary-hide-final-*`. Remaining actual platform
execution and personal-server diagnosis stay active; this is a source update.

## 9 September: adjacent selection after deleting mail

R89 now selects the following displayed message immediately after a move/delete,
keeps the list scroll position, falls back to the previous row at the end, and
clears the reader when empty. This also applies to same-account and cross-account
moves. Cross-folder search retains the current reader when the moved row remains
in the result. Page-boundary refill uses the foreground query channel; a short
projected page retains its pending successor until refill arrives or its total
proves no successor remains. Explicit newer selection cancels that follow-up.

Six controller regressions pass, covering repeated removal, sorted/filtered order,
empty/previous-page fallback, rollback with newer navigation and short-page then
full-refill ordering with cancellation. Five saved native scenarios pass against
the final binary: scrolled mouse/keyboard deletion while saves are pending,
failure without losing newer navigation, unread/oldest filtering and an empty
reader, next-page refill, and deleting the final page back to the previous last
row. Final native log: `artifacts/logs/delete-navigation-final-native.log`;
25.623 seconds, five passed. Native SHA-256:
`0f87954d654b735751cb9598b0f7d0f0701ea5ecaa1da003841b35a88679258c`.
Reviewed WebPs show adjacent selection and retained scroll; final runs include
`4ccfdb9160d6` (pending repeated deletes) and `8e52d2ba69fc` (previous-page fallback).

The initial native run exposed test setup issues: read-on-leave generated several
serialized slow writes, a page assertion preceded completion of injected keys,
and mouse Delete preceded presentation of the selected reader. Focused tests now
start on already-read rows and await the intended selection/presentation; the
unread-filter scenario preserves read-on-leave/deletion coverage. Existing rapid
input scenarios and all timeouts remain unchanged. These isolated fixtures do not
establish live IMAP behavior or performance percentiles. Primary-agent integration
and push are pending; TODO remains open until that shipping step.

Root integration `5a85ac3` passes mandatory hooks (735 executions, three personal
diagnostics ignored), Python57 and strict Zensical. The wider native run passed
58 of59 scenarios and exposed a real group-Move labeling bug: the new neighboring
reader could belong to another account, while the group consisted only of the
original account's messages. Folder labeling used that reader's catalog, hiding
its Japanese destination from fuzzy search. Root changed labeling to use selected
account membership, with explicit destination choice taking precedence. A new
real-selection-snapshot regression passes. The existing native Unicode
move/Undo/group scenario is retained unchanged; final rerun/shipping is pending.
Failure evidence `d338c6f4c4ce` is retained under ignored artifacts.

Final corrected source [`91ed9a9`](https://github.com/sam-ruff/shep.so/commit/91ed9a9)
is pushed to main with exact remote equality verified. Mandatory hooks pass 736
executions, zero failures and three personal diagnostics ignored. All 59 selected
native mail scenarios pass in 258.950 seconds; strict Zensical passes. The final
native binary SHA-256 is
`9f542c54cde33fe9f7bc92b17092f7a0ed3f160f6f7512ca0b11e4dc5fe33884`.
Log: `artifacts/logs/delete-navigation-final-main-native.log`. Root reviewed
`aa1c77b3323d/nested-unicode-bulk-review.webp`,
`93a9f3fbf44a/delete-scrolled-neighbors-saved.webp` and
`eed86f7250b8/delete-final-page-previous-last.webp` under `artifacts/e2e/`.
R89 is complete and removed from the active TODO. Compact row styling remains
separate R88 work; production installation and final performance gates remain open.

## 9 September: verified shared-profile record reuse

R02/R49 now retains verified immutable Drive records in the separate bounded
profile journal. Every poll still completes and validates a fresh listing; only
records with matching identity, namespace, operation, size and digest reuse bytes.
Corrupt/oversized cache entries are repaired through verified downloads. Missing,
changed or incomplete listings cannot be hidden by cached history. No wire-format
or credential-storage changes are involved.

The profile-filtered Rust suite passes 80 tests with one personal diagnostic
ignored. New protocol checks prove unchanged polls and restarts use one listing
request with no repeated metadata/body downloads; adding a record downloads only
that record. Journal tests cover restart, stale scans, replacement identities,
corruption and bounded allocation. All 55 Python tests and six selected native
scenarios pass. The new saved native Sync now/restart scenario observes actual
owned HTTP request counters, with reviewed Preferences evidence in `16ce5ad1b34f`.
It runs alongside automatic account receipt, local publication, offline retry,
partial failure and enrollment. These are correctness/request-count checks, not
latency or live Google claims. Worktree commit `654a90e` passes mandatory hooks
(717 executions, three personal diagnostics ignored), Clippy and strict Zensical.
Native binary SHA-256:
`5873fc7eb7093c8538416941e9243253f3d225a89190f1d508b4fcf109060a66`.
Root integrated this as [`e77eabc`](https://github.com/sam-ruff/shep.so/commit/e77eabc)
and pushed to main with exact remote equality verified. All 729 integrated hook
executions pass (three personal diagnostics ignored), Python passes 57 and strict
Zensical passes. All nine integrated native cache/profile/close scenarios pass;
merged binary SHA-256:
`22285ff1b50ce5f124234f9200ffa15d859419b57600988980481fa1fc6f9933`.
Change-token incremental polling and account/conflict review controls remain active
TODOs.

## 9 September: shutdown failure ownership

Attachment storage, draft discard and forward preparation cancel automatic close
only when their failed result owns the current pending operation. Late results
for another draft/request keep newer shutdown dependencies intact. Errors stay
visible; discard retains the review and original draft for retry. Three production
`App::update` regressions cover current/old IDs and late stop acknowledgments.
The close-filter Rust run passes 32 tests; all 57 Python tests and nine selected
native scenarios pass, including the three new close/failure/retry flows. Reviewed
WebPs: `artifacts/e2e/effa8bf75b5b/close-attachment-failure.webp`,
`artifacts/e2e/563bb8045bcf/close-discard-failure.webp` and
`artifacts/e2e/73046dc11efc/close-forward-failure.webp`. They retain the composer,
red discard control and visible recovery errors. Native binary SHA-256:
`20d1be5a8a8df061b8ef92afb39cf281d23397be82ba05d4f901ca76f079c0c8`.
These are fixture correctness checks, not live server or timing measurements.
Agent commits `9c2e84c` and `e30c174` pass all mandatory hooks (711 and 714
executions respectively, with three personal diagnostics ignored). The root
reviewed the failure WebPs and integrated both checkpoints together as
[`2733cc6`](https://github.com/sam-ruff/shep.so/commit/2733cc6), pushed to main
with exact remote equality verified. Integrated mandatory hooks pass 725 executions
(three personal diagnostics ignored), Python passes 57, strict Zensical passes,
and all 23 selected native scenarios pass. The combined suite includes ongoing
profile sync, database transfers, group/folder close, composition and failure
recovery. Root reviewed the merged discard-error screenshot `83ea39e4a99e`.
Integrated native SHA-256:
`dbe6de7e2e80a196697559b9accf703c6f131e2386365d59bff4dbb84d419783`.
All injected delays and failures require the isolated preview feature.

## 9 September: close continuation and interruptible provider waits

The R90/R91 shutdown checkpoint keeps close intent through account/calendar,
Google, outgoing, attachment and draft saves, then continues automatically once
all required acknowledgments arrive. Account and calendar writes record their
busy dependency when admitted, before provider capacity becomes available.
Errors reach the UI before that dependency is released; current failures cancel
close while obsolete draft failures preserve newer saves. A failed calendar
connection remains editable with its inline error.

Bulk execution now waits for provider capacity before claiming a journal item.
A close interrupts that wait through a capacity-one lifecycle channel, preserving
unstarted work for the next launch. Already claimed work still records its actual
receipt. Folder cancellation uses the same signal instead of polling. Production
shutdown still waits for durable writes; optional read-only sync is not a close
dependency.

Targeted Rust checks pass 29 tests, and all 57 Python tests pass. **15 selected
native scenarios pass**, including two new flows: closing with all eight provider
slots held preserves queued journal steps, and closing during rejected send
preparation cancels close and retains the reply through restart. Existing group,
folder, read-on-leave, recovery, inline draft, account/calendar, profile upload
and database-transfer close scenarios also pass. Reviewed WebPs are in
`94e04def5ba4` (pending send, visible error and reopened draft) and `35aaa8f6ef81`
(optimistic group before close). Native binary SHA-256:
`8eb9fafec14de5bde0338967b6c7cdd42bfc7389feb7e3aecc07e795cd42eafe`.
These are fictional native/protocol fixtures; no personal account, credentials
or installed executable changed. Native tray integration and live personal-server
close diagnosis remain open. The integrated shipping receipt is above; this
checkpoint does not finish R86 or the full product goal.

## 9 September: ongoing profile updates and parallel feature delivery

R02/R49/R92 now runs ongoing profile checks through the bounded coordinator,
with immediate manual retry and separate account/settings toggles. It publishes
local changes and applies received preferences, account names and new account
definitions without switching tabs. A later upload failure still refreshes
already committed received changes. New definitions require Reconnect; existing
endpoint changes, removals and conflicts retain local data for review.

Exact deferred edits retain their operation UUID and causal basis. Native
preference writes merge only edited shared fields. Durable per-field generations
also protect a setting or account name that is changed and then reverted during
a pull or acknowledgment; database imports archive source-device generations.
The targeted profile suite passes 76 tests with one personal diagnostic ignored.
Required hook checks pass 711 executions with three personal diagnostics ignored;
Python passes 55. No performance budgets were measured or changed.

The ongoing-update UI and protocol checkpoint passed 34 native scenarios,
including four new automatic-receipt, publication/restart, offline-retry and
partial-upload-failure paths. Reviewed WebPs include `8a68015b56c6` (received
accounts while Mail remains open) and `184fead95e69` (received accounts alongside
an explicit upload error). After generation tracking and refresh integration, the combined run passed 40
of 41 scenarios. The remaining database-export setup failed because the native
GTK picker's location field did not accept its path (unpainted picker evidence
`ee7f4ccad7e2`); three unchanged, fresh reruns passed (`5301b548bd0c`,
`5a38e38e1f9c`, `5bd787f589d4`). This intermittent picker readiness issue remains
in R63; no timeout or performance gate was weakened. Final reviewed native
screens include `4feda00dae1e` and compact dark refresh `aeec46d6a1df`.
Integrated binary SHA-256:
`92e957553a9bc375dc81651c120d379ece51cdbcbd955a6c1edb431bfb7e5d48`.
The integrated hooks pass **713 executions**, three personal diagnostics ignored;
Windows GNU all-target/all-feature checking and strict Zensical also pass.
Source [`bd50c52`](https://github.com/sam-ruff/shep.so/commit/bd50c52)
and [`d29ce06`](https://github.com/sam-ruff/shep.so/commit/d29ce06)
are pushed to main; exact remote equality was verified.

R93 assigns three isolated worktree lanes to compact mail/deletion/refresh,
multiple backups and shutdown/tray, with the primary agent integrating tested
commits. Agent refresh checkpoint `f111282` was integrated as `d29ce06`. R87 is
complete: the icon turns clockwise every 2.4 seconds, matching its arrowheads,
only during manual refresh. Four animation tests, 14 renderer tests and seven
native refresh/background scenarios pass, including light, compact dark and
scaled rendering, F5 remapping/restart, failure/retry and navigation. Root reviewed
its visual evidence before integration. R87 leaves TODO; remaining requests stay
tracked. Incremental history pulls, account linking, conflict/removal/endpoint
reviews, remaining portable settings, credential transfer and real cross-client
Google access remain unfinished. Personal mail, OS credentials and the installed
production app were untouched. Quality/release CI stays disabled.

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

Four new Rust regressions cover persisted opt-out through the actual store
disconnect/cleanup/grant-activation lifecycle, settings/draft
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
passes. Strict Zensical and formatting/Clippy passed. Mandatory hooks passed
**647 root Rust + two renderer + 53 shared tests (702 executions)**, with three
personal diagnostics explicitly ignored. Source
[`acb4969`](https://github.com/sam-ruff/shep.so/commit/acb4969154bb6a882a9688cf6e340d27a977a32a)
is pushed to main; exact remote equality was verified. [Source documentation CI](https://github.com/sam-ruff/shep.so/actions/runs/34323561457)
passed the strict build and Pages deployment. The first native attempt identified
a missing preview status-event
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

## 8 September: OAuth/profile implementation handover

Documentation checkpoint [`02c4b32`](https://github.com/sam-ruff/shep.so/commit/02c4b32a6b380c1d5312c20bf187584c683ff10f) is pushed to `feat/mobile-web-clients` and remote equality was verified. [The handover](agents/PROFILE_SYNC_HANDOVER.md) maps existing desktop/Flutter storage and credential lifecycle code, first/new/existing-device flows, configurable profiles, the proposed versioned format, conflicts/removal, outstanding credential protection and required interoperability tests. OAuth implementation now comes first in TODO and the restart notes, as requested. Continuous profile sync and full database transfer remain unimplemented; no live Google or client UI behavior is claimed by this documentation change.

The pinned strict Zensical build passed, all 31 shared scenario entries passed structural validation, and normal hooks passed formatting, Clippy and 381 Rust tests. Logs are under ignored `artifacts/logs/profile-sync-client-*`. This checkpoint changes documentation and an explicitly open scenario contract only; it does not update the phone, desktop installation or deployed service. Main remains separate.

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

The following records describe the independent desktop main worktree at their cited commits. They do not establish verification of this merged client branch. Desktop request IDs R67–R71 here are main's numbers (written as `desktop-main:RNN` in older client notes); the client branch's own R67 to R80 are different requests, as stated once in [the request audit](REQUEST_AUDIT.md).

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

SFTP checkpoint `ff03b02` has green documentation CI **34359867454**.

Folder checkpoint `97c9a9a` has green documentation CI **34360832298**.

## Client branch history from the desktop integration onward

The following entries were written on `feat/mobile-web-clients`, in their original order, and end at the merge entry. Their desktop discovery/publication/enrollment/reconciliation sections describe the client branch's own desktop implementation, which main's implementation replaced at the merge.

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


Final review found a mixed-version writer gap: a schema-six tab could save an acknowledged mail change without the new atomic cache-applied record. Mail schema 7 now closes older database connections before this executor can use the cache and prevents older-version reopening. A real Chromium upgrade scenario preserves the old clock, status and Sent roles, verifies the old connection cannot write, and confirms no cache acknowledgment is invented from an earlier acknowledgment-only status. The executor implementation was committed locally in `3467937` but held from pushing until this compatibility fix and its full regression passed. Updated final evidence follows.


The final schema-seven revision passes **107 browser unit tests**, all **87 Playwright scenarios** (including 14 executor/storage scenarios), the **55-stage real Rust HTTPS test**, TypeScript and the rebuilt production browser. Python's 39 tests and 30 parity contracts remain passing; the final change affects only browser storage/version tests and documentation. Production fixture exclusion, licenses and unchanged shared WASM artifacts are verified. Strict documentation and required hooks precede the combined review-branch push. Full group UI/optimistic-query/account-cleanup integration and the wider parity/deployment backlog remain active.


### Browser durable executor shipping record

The executor [`3467937`](https://github.com/sam-ruff/shep.so/commit/346793738fda85502a8395c5578f86cc691c875a) and older-writer compatibility fix [`302bac2`](https://github.com/sam-ruff/shep.so/commit/302bac26e742c55883af5cf528c7f4fa8ec72d42) are pushed to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests** for each commit, with two personal-account diagnostics intentionally ignored. Final evidence includes **107 browser unit tests**, **87 Playwright scenarios**, **55 real Rust HTTPS beta/mail stages**, **39 Python tests**, **30 parity contracts**, TypeScript, production bundle inspection, reviewed layout regressions and pinned strict documentation validation.

R77's prompt push is fulfilled for this increment. The executor API is verified but not activated by application controls. Visible group review/History/Undo, optimistic query effects, group account cleanup, explicit ambiguous-result resolution and native equivalents remain active, alongside full client parity, R75/R76 and missing VPS/owner/OAuth configuration. Main and the personal installation were untouched; artifacts remain ignored and owned browser test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Browser group-aware account removal

Preferences now includes the account's group-history entries and unfinished changes in its removal review. Changed group decisions require fresh counts and explicit discard remains required for queued, failed, uncertain, inverse and pending-cache work. Actual light/dark controls verify this at 900×640; reviewed captures show readable counts, recovery instructions and reachable actions.

Removal retains group ownership around the existing draft/account/cache locks. The committed mail-removal token is authoritative across the two databases: failures before it preserve local data/history, a lost committed reply is recognized, and cleanup failure afterward keeps the account removed with a visible instruction. Every fresh journal owner reconciles committed removals before claiming provider work. Cleanup deletes at most 50 source/destination-owned entries per strict transaction, adjusts surviving job counts, preserves unrelated receipts and records content-free completed fences against stale frozen captures. Tab loss and a failed second cleanup page resume without replaying removed work.

Journal schema 4 migrates account ownership indexes and preserves receipts; read-only inspection waits for an older receipt owner before upgrading its database. Mail schema 8 closes older account-removal code that lacks group coordination while preserving existing cache/intent metadata. Review counting retains one item/job; the global group revision deliberately invalidates an open review if another group changes. General bounded cache reads remain separate work.

Ten targeted Chromium scenarios pass with real browser storage and locks, including three actual Preferences flows. They cover stale/discard decisions, failed/lost mail replies, page rollback, cross-tab ownership, tab loss, completed/uncertain/inverse/missing entries, source/destination history migration, old-owner upgrade protection and post-commit cleanup feedback/reopen. The first control run caught an incorrect plural, corrected before the passing rerun; its traces remain under ignored `artifacts/browser-group-removal-failures/`. Full regression, production/HTTPS, strict documentation, required hooks and prompt review-branch shipping follow.

This increment does not activate group execution from the application UI. Visible group review/History/Undo, optimistic query effects, explicit ambiguous-result resolution and native equivalents remain active alongside the full parity/deployment backlog, R75/R76 and missing VPS/owner/OAuth configuration. Root Rust/native Flutter code is unchanged; Android/Appium, Apple, root native UI and final idle-host performance are not rerun for this browser increment. Main and the personal installation remain untouched; quality/release workflows remain disabled.


The first complete browser run passed 94/97 scenarios. Startup cleanup unnecessarily created group storage in profiles without removals, preempting the legacy migration fixture; it now consults mail removal records first and opens the group journal only when needed. The occupied-account fixture now obtains the complete production removal review before testing the held account lock. A separate Print click completed without a popup while the formatted iframe received focus; its unchanged focused rerun passes, and the reflow/scroll hit-testing investigation remains explicitly active in R63. No click bypass or timeout relaxation was added. All three traces remain under ignored `artifacts/browser-group-removal-failures/full-first/`. The 31 affected executor/removal/storage/printing regressions pass together. The final unchanged-source full run follows.


Final verification passes **107 browser unit tests**, all **97 Playwright scenarios** in an unchanged-source run, **55 real Rust HTTPS beta/mail stages**, **39 Python tests** and **30 parity contracts**. TypeScript, pinned Prettier checks and the production browser build pass. Production inspection excludes the preview entry and checked fixture markers, includes bundled licenses and preserves the recorded SQLite/shared MIME WASM hashes. The three new Preferences captures are reviewed and saved as lossless WebP under ignored artifacts. Strict documentation validation and required hooks precede prompt review-branch shipping. The retained intermittent Print input observation remains an active R63 investigation despite the passing final suite; Apple/live-provider/native parity and the wider product backlog remain open.


### Browser group account-removal shipping record

Committed and pushed as [`92452c3`](https://github.com/sam-ruff/shep.so/commit/92452c35523deb357ec4ee921fe887ee476487f2) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final verification includes **107 browser unit tests**, **97 Playwright scenarios**, **55 real Rust HTTPS stages**, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, production fixture/license/WASM inspection and pinned strict documentation validation. Reviewed light/dark removal and cleanup-warning captures are retained as ignored WebP artifacts.

R77's prompt push is fulfilled for this increment. Group-aware account review/cleanup and restart protection are verified; visible group execution/review/History/Undo, optimistic query effects, explicit ambiguity resolution and native equivalents remain active. R63 retains the intermittent Print click/reflow observation despite its passing focused and final full reruns. Full parity, R75/R76 and missing VPS/owner/OAuth configuration remain open. Main and the personal installation were untouched; owned test processes are stopped and the worktree contains no root log files. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


## Stable browser reader actions and native activation

The retained Print trace is now reproduced deterministically: attachment and formatted-body preparation moved Print after the pointer had been positioned, eventually placing it below the window. Reply/Reply all/Forward/Print now stay in a responsive footer outside body scrolling. Preparation keeps visible labels and column sizing stable while displaying an accessible busy state. Light/dark pointer scenarios exercise the original target through delayed preparation without forcing a hit test.

A second regression holds the mouse button while preparation completes. Reusing a disconnected/reinserted button still cancelled native activation; `reader_actions.ts` now retains the same-message footer and its ancestor branch continuously while replacing surrounding controls and rebinding current callbacks. The actual press then opens exactly one preview on release. The formatted sandbox retains its separate lifetime, text selection and scroll behavior.

Native Enter/Space now activates focused action controls before mail accelerators run. Row Flag activation is isolated from the reader shortcut, and opening a message uses the focused row and configured reader binding, including remapping/disable. Saved keyboard input verifies focus through preparation, Enter printing, scrolling with a reachable footer and Reply's explicit missing-cache-header recovery. That print-only fixture does not establish Reply success; the broader existing Reply tests remain separate.

The ten focused printing/input scenarios pass, including five new regressions. Retained failure evidence under ignored `artifacts/reader-click-investigation/` includes the original geometry mismatch, cancelled activation after reinsertion, intercepted Enter and the corrected Reply-fixture expectation. No timeout, native input or hit-testing guard was weakened. Full regression, production/HTTPS, strict documentation, hooks and prompt review-branch shipping follow.

This fixes the identified browser reader-action defects, not the whole R63/native/list-input audit. Full browser/native group controls, optimistic bounded queries, explicit ambiguity resolution, full client parity, R75/R76 and VPS/owner/OAuth deployment configuration remain active. Root Rust and native Flutter are unchanged; Android/Appium, Apple, root native UI and final idle-host performance are not rerun for this browser increment. Main and the personal installation remain untouched; quality/release workflows remain disabled.


Source review also confirms Flutter still places Reply/Forward/Print/Move in its scrolling reader body. The corresponding native footer/reachability work is explicitly retained in R67 and the parity matrix; the browser verification does not establish Android or Apple behavior. Reviewed light 1280×720 and dark 900×640 footer captures show stable four-column/two-column actions and readable content; WebP evidence is retained under ignored artifacts.


Final verification passes **107 browser unit tests**, all **102 Playwright scenarios** in an unchanged-source run, **55 real Rust HTTPS beta/mail stages**, **39 Python tests** and **30 parity contracts**. TypeScript, pinned formatting and the production browser build pass. Production inspection excludes the preview entry and checked fixture markers, includes bundled licenses and preserves the recorded SQLite/shared MIME WASM hashes. Light/dark footer and existing layout captures are reviewed; lossless WebP evidence is ignored. Required hooks and strict documentation validation precede prompt review-branch shipping. The identified browser reader input defects are fixed; native footer parity, the wider input audit and full product/deployment backlog remain active.


### Browser reader-action shipping record

Committed and pushed as [`cebcbe2`](https://github.com/sam-ruff/shep.so/commit/cebcbe24d990dbe7d889b71a4234f6395aa0e58a) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final verification includes **107 browser unit tests**, **102 Playwright scenarios**, **55 real Rust HTTPS stages**, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, reviewed light/dark footer and layout captures, production fixture/license/WASM inspection and pinned strict documentation validation. The reproducible failed baselines remain ignored and are described above.

R77's prompt push is fulfilled for this increment. The identified browser reader-action movement, cancelled press and native activation defects are fixed. Native footer/touch parity, the wider R63 input audit, browser/native group controls and optimistic queries, full client parity, R75/R76 and missing VPS/owner/OAuth configuration remain active. Main and the personal installation were untouched; owned test processes are stopped and root logs are absent. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Flutter reader-action continuation

The original Flutter reader placed Reply/Reply all/Forward/Print/Move after scrolling content. The new saved host regression reproduces their movement from the visible reader to thousands of pixels below the viewport when delayed body preparation completes; the failing baseline is retained in `artifacts/logs/flutter-reader-footer-before.log`. Actions now occupy a responsive bottom safe-area footer, with counted feedback/Undo above it. Forward and Print preserve their visible labels, button size and native element while preparation runs; semantic labels expose the busy state. Existing action handlers, read-on-leave, independent print preparation and persistent formatted WebView remain in use.

The shared host/Android scenario uses actual touches: press Print before delayed detail/files and formatted text arrive, release afterward, verify exactly one launch, refuse a duplicate while pending, recover a Forward preparation error, scroll and Reply. The first fixture iterations corrected an exact-type finder, observed actual route-animation completion and disposed the test workspace before the timer invariant. Compact 150% text also exposed an existing Inbox filter-chip overflow; the filters now wrap while Sort remains visible. The overflow evidence stays in `artifacts/logs/flutter-reader-footer-scaled.log`.

Flutter analysis and all **74 host tests** pass, including the three new reader scenarios and existing FFI/cache tests. Native Android, printer/PDF, formatted controls, production inspection and shipping checks follow below. Apple execution, native keymap/large-mail/lifecycle parity and the full original client/deployment backlog remain active. R75/R76 and the pending VPS/owner/OAuth configuration are unchanged. Main and the personal installation remain untouched; quality/release workflows stay disabled.


The first Android Forward regression stopped at its deliberate preparation barrier because the old helper called `pumpAndSettle` while the new busy icon intentionally kept animating. The captured screen shows usable reader controls and the pending Forward. The owned driver was stopped; the helper now waits for that specific transport barrier without waiting for all animations, then presses the real Back control and continues the independent-editor assertions. No timeout or action guard was weakened. Logs remain in `artifacts/flutter-reader-footer-failures/` and the screen in `artifacts/flutter/native/reader-footer-forward-investigation.png`; final rerun evidence follows.


The wider Android Outbox run exposed an obsolete test expecting read-on-open after reopening a locally stored IMAP Sent message. Current read-on-leave correctly retains unread while that reader stays open. The saved scenario now checks the visible Mark read control, uses Back, waits for the actual mutation and verifies both cached/native persisted read state with no credential access. Its earlier failure remains in `artifacts/flutter-reader-footer-failures/outbox/`. This updates the assertion to the already-approved behavior; it does not change mail-action production code.


All **16 targeted Android integration scenarios** now pass: eleven native mail/cache/account/control scenarios plus incoming files, complete Forward, complete Print/PDF, formatted WebView and Outbox. Both **six-stage Appium suites** pass. The existing general preview/composer integration targets were not rerun; their Android gestures and native mail paths remain covered as described above. Print again cancels/retries, saves exact formatted content and a **10-page** complete long-message PDF. Reviewed captures include native footer light/dark, the dark formatted Appium footer and the formatted PDF. The Forward barrier and Outbox assertion reruns pass with their original deadlines. Production action code was unchanged by those test corrections. The dedicated emulator and owned native test drivers are stopped.


The Flutter browser preflight rejected a stale frozen formatted fixture: it still carried older default body colors despite the current shared renderer's authored-color preservation. Regeneration changes only that CSS in the test document; MIME text, signature and display runtime/CSP hash remain unchanged. Android Appium now runs the same fixture-freshness guard before building its formatted preview. The old fixture and preflight error remain under `artifacts/flutter-reader-footer-failures/`; both formatted previews are rerun against current generated source. The production renderer is unchanged.


The new Flutter Playwright geometry assertion then found desktop adaptive visual density shrinking a nominal 44-pixel footer button to 36 pixels. The footer now explicitly uses standard visual density, retaining the same touch size across Flutter platforms. The assertion remains at 44 pixels; its screenshot/DOM/log are saved in `artifacts/flutter-reader-footer-failures/desktop-density/`. Android already used standard density. Final host/native/browser checks are rerun with this explicit setting.


The next formatted browser run observed replacement text being appended after switching to Plain text: the capture contains `AlphaPlain alternative`, so the intended replacement was appended. The saved helper now observes the actual focused input and native selection range before typing, retaining pointer clicks and keyboard Select all. No controller/value assignment or timeout increase replaces those inputs. Failure evidence is in `artifacts/flutter-reader-footer-failures/find-focus/`; the wider rapid-input lifecycle audit remains R63 work.


The corrected formatted browser scenario subsequently completed Move and Reply; its final generic text oracle could not see prefilled values in inactive Flutter editing proxies. The captured composer visibly contains the correct subject and recipient. The saved test now focuses those real fields before observing their native input values. That assertion failure is retained under `artifacts/flutter-reader-footer-failures/reply-observation/`; the actual Reply action and application source are unchanged.


### Flutter reader footer — final verification

The final application passes Flutter analysis/formatting, **74 host tests** and the **eleven native mail/cache/account/control scenarios** rerun with explicit standard footer density. This increment also passed the five native incoming-file/Forward/Print/formatted/Outbox scenarios and **twelve Appium stages** recorded above. Both Flutter browser paths pass: **six general scenarios** and **ten formatted-reader scenarios**, including actual 44-pixel bounds, stable scrolling, Move and prefilled Reply. The last JavaScript-only observation corrections ran on the unchanged final Flutter build; the owned server stopped afterward. Final native and browser captures were reviewed. **39 Python tests**, **30 parity contracts**, pinned formatting and generated-fixture freshness checks pass.

The production-flavor debug APK contains all three native libraries, Internet permission and no checked fixture markers, including the new reader fixtures. SHA-256: `2a0626c2aee2b14b089fabc55ade94f09c4041dd9e43b22f12e93cec3da1b002`. This is a development build; distribution signing remains separate. Logs use `artifacts/logs/flutter-reader-footer-*` plus the saved Android/Flutter browser runner logs. Failure evidence remains under the ignored paths above. Required root hooks, pinned strict documentation validation and prompt review-branch shipping follow.

The root desktop UI, separate hosted browser implementation and native bridge source are unchanged. Root native UI, Apple, live providers and final idle-host performance were not rerun. Full client parity, native keymaps and lifecycle, browser/native group controls and bounded queries, R75/R76 and VPS/owner/OAuth configuration remain active. Main and the personal desktop installation were untouched. Owned emulators/browser servers/Appium drivers are stopped; quality/release workflows remain disabled.


### Flutter reader-action shipping record

Committed and pushed as [`48c6c18`](https://github.com/sam-ruff/shep.so/commit/48c6c18cf5e841c3e168720bbf2a01e69eb45ea0) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Verification includes **74 Flutter host tests**, the targeted **16 Android integration scenarios**, **12 Appium stages**, both Flutter browser previews (**16 scenarios**), **39 Python tests**, **30 parity contracts**, production APK isolation, final visual review and pinned strict documentation validation. The final density setting reruns eleven native scenarios and both browser paths; the preceding Android printer/attachment/Outbox/formatted evidence and test corrections are detailed above.

R77's prompt push is fulfilled for this increment. Flutter reader actions remain reachable through loading and scrolling, retain native held touches and keep 44-pixel targets. The full original goal remains active: native keymaps/lifecycle, group controls and bounded queries, remaining mail/calendar/Google/backup parity, Apple execution, distribution and VPS deployment are unfinished. R75/R76 remain tracked TODOs, and exact VPS/owner/OAuth configuration is still pending. Main and the personal installation were untouched; artifacts stay ignored and owned test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser persistent mailbox query prerequisite

The current browser application still loads the complete cached mailbox and bodies before filtering/displaying 50 rows. The new, not-yet-connected query worker instead derives a per-profile SQLite/OPFS index from authoritative IndexedDB. Mail schema 9 assigns a fresh cache incarnation while preserving revision/floor, messages, intents and acknowledged Sent roles. Each request coordinates VFS installation/acquisition through a Web Lock and closes/pauses handles before releasing it. The worker returns at most 50 metadata rows and supports separate single-message body requests. Current substring/filter/folder/Sent semantics, pending individual fields and bounded alias observations share the source snapshot. Full desktop fuzzy ranking remains separate work.

The first two Chromium storage scenarios pass, covering pages/search/projection, aliases/current Sent roles, separate bodies, cooperating tabs and a recreated source with the same revision. The initial 100,000-message recovery scenario exceeded its unchanged 180-second deadline; its trace/log remain under ignored `artifacts/browser-mailbox-failures/initial/`. Index construction now reuses its prepared writer and builds FTS after a complete cursor pass, inside the same rollback transaction. Final verification follows below; no latency/performance success is inferred from cardinality tests.

Application paging, the bounded retained-reader/body lifecycle, stale-result and optimistic rollback/count integration, removed-account cleanup of derived storage, independent body capacity during indexing, provider sync/Outbox bounds, visible bulk review/History/Undo and actual controls remain active. The existing UI and native clients are unchanged; root native UI, Android/Apple, live providers and final idle-host performance are not rerun for this storage prerequisite. R75/R76 and exact VPS/owner/OAuth configuration remain open. Quality/release definitions stay disabled; main and the personal installation remain untouched.


All **six targeted Chromium storage scenarios** now pass. The optimized large-cache case completes within its original 180-second deadline, including the independent draft commit, failed incremental update rollback, full replay-window rebuild and account exclusion. Additional scenarios verify the 32-request bound under a held Web Lock, abrupt worker interruption/recovery, punctuation/combining-character/NUL search against the existing list predicate, and schema-eight migration preserving clocks/intents/Sent roles while assigning one stable incarnation. These establish storage correctness; the worker is not yet connected to application list/reader controls or included through the production entry point. Full browser regression, production/HTTPS, strict documentation and required-hook shipping checks follow.


The complete browser run passed **106/108 scenarios** on the final application source. The two failures are older schema-four/five migration assertions that require the previous exact state shape; the received revision/floor and protected data are correct, with schema nine's new UUID. Both assertions now require a valid UUID in addition to their unchanged exact revision/floor and data checks. Their traces are retained under ignored `artifacts/browser-mailbox-failures/migration-expectations/`. The two corrected migration scenarios pass in a targeted rerun. Final query API review also adds explicit refusal of a damaged non-text cached body, with a focused storage regression; the application UI path is unchanged. Shipping evidence follows.


Final verification includes **107 browser unit tests**, **108 Playwright scenarios verified across the full run and the two corrected migration reruns**, the focused damaged-body regression, **55 real Rust HTTPS beta/mail stages**, **39 Python tests** and **30 parity contracts**. The complete run itself remains recorded as 106/108, with the two exact schema-expectation failures described above; there was no blanket rerun after those test-only corrections. TypeScript, pinned formatting and the production browser build pass. Production inspection excludes the preview entry and checked fixture markers, includes licenses and preserves the recorded SQLite/shared MIME WASM hashes. The unused query worker has dedicated real-browser storage evidence; its production-entry/UI/HTTPS activation remains unfinished. Required hooks and pinned strict documentation validation precede prompt review-branch shipping.


### Browser mailbox query shipping record

Committed and pushed as [`3dd0837`](https://github.com/sam-ruff/shep.so/commit/3dd08372ad0ebcd9383a1611142906209948cea0) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Verification includes **107 browser unit tests**, the **108 Playwright scenarios covered by the 106/108 full run plus two corrected migration reruns**, the focused damaged-body regression, **55 real Rust HTTPS stages**, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, production fixture/license/WASM inspection and pinned strict documentation validation. The full-run failures and initial large-cache timeout remain recorded in the completion log and ignored artifacts; they are not described as a clean single full run.

R77's prompt push is fulfilled for this increment. Persistent query storage is a tested prerequisite; application list/reader activation, optimistic counts/rollback, independent body capacity during rebuilding, derived account-removal cleanup, bulk controls and full feature parity remain open. R75/R76 stay tracked TODOs; exact VPS/owner/OAuth configuration and Apple/distribution verification remain pending. Main and the personal installation were untouched. Root logs are absent, test artifacts stay ignored and owned browser/HTTPS processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser application paging — R42/R50/R60/R63/R67/R69/R71/R73/R74/R77

The pending integration connects the persistent query worker to the production browser Gateway/Workspace. Startup and list reloads no longer materialize every cached body. Coalesced queries return at most 50 metadata rows; two independent foreground readers, separate scan/speculative capacity and an eight-body/32 MiB cache preserve navigation and the active reader. Schema 10 adds account/folder/server-identity indexes without resetting schema 9's source incarnation or protected clocks. Sync scans return metadata pages; each flag-cache write retains one body.

Initial verification exposed a real input regression: body arrival detached a row Flag button during a held press. The saved scenario fails before the control-retention fix; its trace remains under ignored `artifacts/browser-paging-failures/initial/`. The same run's search test used a nonexistent searchbox role; its corrected target is the actual Search conversations textbox. Controlled unit checks also exposed a stale pending page overwriting immediate rollback after a synchronous reservation failure. Failed projections now retire before that result can publish. Shutdown cannot recreate read workers, removed-account projections are discarded, and evicted-metadata Undo reports missing source mail for review. Final regression and shipping evidence follow after validation.

The full product goal remains active. Bulk controls/projections, whole-client Outbox/draft/removal-snapshot bounds, provider known-ID/reconciliation metadata, physical derived-index cleanup retries, fuzzy relevance, large-message preparation, performance, Apple/live-provider execution and VPS deployment remain unfinished. R75/R76 remain tracked TODOs; exact VPS/owner/OAuth configuration is still pending. This browser-only change does not rerun native root/Flutter UI suites. Main and the personal installation remain untouched; quality/release definitions stay disabled.


The complete Chromium run passes **113/113 scenarios**, including both schema-eight/nine migration paths, 100,000-message cardinality/recovery, actual paging/selection/Undo, retained row/footer presses, account removal and reader layouts. Final review adds explicit Gateway shutdown and cache-incarnation guards: late callers cannot recreate workers, and old mutation results cannot paint reused IDs in a replacement cache. All **120 browser unit tests** pass; **31 affected production-control scenarios** pass again after those guards. The full 113-scenario run preceded those final guards; the unchanged storage/formatting/printing coverage is not described as a second full run.

TypeScript, pinned formatting, **39 Python tests**, **30 parity contracts** and pinned strict documentation validation pass. Reviewed light/dark/compact captures retain readable rows, reachable reader actions and explicit Retry. Production build inspection confirms the active query/read workers, required licenses, excluded preview fixture markers and unchanged recorded SQLite/shared-MIME WASM hashes. Real Rust HTTPS verification and required-hook shipping evidence follow.


The built application passes **55 real Rust HTTPS beta/mail stages** with the paged query path active. These use the production service and isolated scripted providers, not personal/live mail. Final shipping runs the mandatory formatting, Clippy and root/shared Rust hooks; the exact commit and verified remote branch are recorded next. Quality/release definitions remain disabled.


### Browser application paging shipping record

Committed and pushed as [`2cb6a46`](https://github.com/sam-ruff/shep.so/commit/2cb6a46404aaabf03995984e0a5abc602bac1703) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Verification includes **120 browser unit tests**, a **113/113 Playwright run** followed by **31 affected control reruns** after the final shutdown/incarnation guards, **55 real Rust HTTPS beta/mail stages**, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, production fixture/license/WASM inspection, reviewed light/dark/compact WebP evidence and pinned strict documentation validation. Earlier failed held-input and test-selector evidence remains recorded above and under ignored artifacts.

The production inbox now uses coalesced metadata pages with independent body reads, retained-reader/body budgets, current pending fields, explicit Retry and preserved counted Undo. R77's prompt push is fulfilled for this increment; the combined review branch also contains the prior Flutter, Rust beta backend and delegated promo website work. Full client parity remains active: bulk projection/review/History/Undo, whole-client Outbox/draft/removal bounds, physical derived-index cleanup retries, full search ranking, large-message preparation, Apple/live-provider execution, distribution and VPS deployment are unfinished. R75/R76 remain tracked TODOs; exact VPS/owner/OAuth configuration is still pending. Main and the personal installation were untouched. Test artifacts remain ignored, root logs are absent and owned browser/HTTPS processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser group controls — R42/R50/R60/R63/R67/R69/R71/R73/R74/R77

The browser now connects captured selection to frozen action reviews, approved full-membership execution, worker query/selection projections, bounded History, Pause/Resume, Undo and explicit failed/uncertain recovery. Mail schema 11 invalidates derived intent state; journal schema 5 tracks changed items and its own incarnation. Source-cache replacement guards protect dispatch and cache-only repair. Accepting an uncertain result retires only its unresolved local intent and never infers a provider outcome.

Targeted real controls verify mixed-account cancellation, all 125 messages, 50-item History pages, pending/acknowledged Undo, retained native presses through receipts, explicit failure/review and conservative startup recovery. The first control run exposed observational lock contention that stranded Undo; current-schema readers now observe without taking execution ownership. Its failure trace is retained under ignored `artifacts/browser-bulk-failures/controls-initial/`. A later recovery test used an incorrect appearance selector; the saved test now uses the actual Theme combobox. Both corrected scenarios pass. Shared projection checks cover off-page counts, newer individual choices and incremental journal transfer. Final regression, visual review and shipping follow.

Full parity remains active. Synchronous Undo rows/counts while local decision persistence is held, startup recovery notifications, abandoned-review cleanup, staging/alias/overlapping-group lifecycle, cross-account transport, large-group performance and native equivalents remain open. Broader mail/calendar/Google/backup parity, Apple/live-provider execution, distribution and VPS deployment remain open too. R75/R76 remain tracked TODOs; exact VPS/owner/OAuth configuration is pending. This browser-only increment does not rerun unchanged root/Flutter native UI suites. Main and the personal installation remain untouched; quality/release workflows stay disabled.


The initial full regression run passes 117/120 scenarios. Its failures expose a real concurrency regression and two outdated assertions: adding `removedAccounts` to the long projection snapshot blocked draft commits; the schema assertion expected 10 instead of 11; selection-summary assertions expected raw flags/folders instead of the newly shared optimistic values. Projections now derive valid owners from accounts in the same mail snapshot, leaving draft-fence storage outside the long read. Physical export remains separate from displayed values. Failed evidence stays under ignored `artifacts/browser-bulk-failures/full-initial/`; final reruns follow. History now identifies messages by subject/sender and original folder using a bounded metadata page.


The focused concurrency rerun passes all storage contracts, including draft saving during the 100,000-message capture. Its History test exposed repeated subject lookups slowing provider progress; History now retains one displayed metadata page until navigation or explicit Refresh. The same unchanged completion assertion passes with this fix; the timeout was not relaxed. That intermediate evidence remains under ignored `artifacts/browser-bulk-failures/history-metadata/`.


Final Chromium regression passes **120/120 scenarios**, and all **121 browser unit tests** pass. TypeScript, pinned formatting, **35 Rust backend tests** (the separate browser test is explicitly ignored here), backend Clippy, **39 Python tests**, **30 parity contracts** and pinned strict documentation validation pass. Reviewed light/dark/compact WebP captures show readable reviews, subject-labelled History and reachable recovery controls. The production build contains the query/read/selection workers and licenses, excludes fixture markers, and retains the recorded SQLite/shared-MIME WASM SHA-256 values. The public promo and protected app assets are staged for review, not deployed. Production Rust HTTPS and mandatory-hook shipping evidence follow.


The built application passes **56 real Rust HTTPS beta/mail stages**, including actual captured Flag/Undo controls and exact per-message transport calls. These use the production service with isolated identity/mail fixtures, not live personal providers. The promo build and separated site staging pass. Required formatting/Clippy/root Rust hooks run during the prompt review-branch commit; the verified commit and remote follow.


### Browser group controls shipping record

Committed and pushed as [`74082c7`](https://github.com/sam-ruff/shep.so/commit/74082c71458970f32dae1d7b252d23478bbffa5c) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Mandatory hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final verification passes **120/120 Chromium scenarios**, **121 browser unit tests**, **35 backend tests**, **56 real Rust HTTPS beta/mail stages**, backend Clippy, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, strict documentation, production fixture/license/WASM checks and promo build/site staging. Reviewed synthetic WebP evidence is retained in ignored `artifacts/browser-bulk-visuals/`. The initial failures and their fixes remain recorded above; no timeout or performance threshold was relaxed.

R77's prompt push is fulfilled for this increment. The combined review branch includes Flutter, the browser client, Rust beta backend and the delegated promo website. Full parity stays active: synchronous Undo before local persistence, startup/review cleanup, staging/alias/overlapping-group lifecycle, cross-account transport, native group controls, broader account/calendar/Google/backups, performance, Apple/live-provider execution, distribution and VPS deployment remain open. R75/R76 remain tracked TODOs. Exact VPS/owner/OAuth configuration is pending; assets are staged, not deployed. Main and the personal installation were untouched, the client worktree was clean after the code push, root logs are absent and owned browser/HTTPS processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser group identity continuity

R42/R50/R60/R67/R73/R77 continuation in the client review worktree. A real 125-message control scenario reproduced the previous failure: a Flag review captured during an earlier Archive group changed one row and rejected the other 124 after their physical folder changed. Mail metadata now retains an origin only through an acknowledged local/provider transition or validated alias adoption. Frozen review metadata stays distinct from the physical dispatch and Undo receipts. Journal version 6 and mail version 12 fence older writers; worker projections apply the same origin rule. Unproven source changes remain rejected, and ambiguous moves are not automatically replayed.

Targeted protocol/storage cases cover individual moves with acknowledged or recovered destination UIDs, canonical aliases, rollback without changing caller inputs, flattened merges, lost cache checkpoints, inverse receipts and stale replacement projections. The control flow also exposed History retaining the previous group's details during a new read; selecting another group now clears its old actions/details immediately and late decisions cannot replace a newer selection.

Initial failure evidence is retained under ignored `artifacts/browser-lineage-failures/`. The first diagnostic assertion combined two sequential 125-row jobs in one wait; the saved test now observes each job through the existing five-second completion assertion. That exposed stale History text allowing an assertion to pass against the previous job; the held-read regression checks the actual newly selected heading and empty loading state. A new projection fixture initially queried a nonexistent physical Flagged folder and now uses the production Inbox/Flagged filter contract. No responsiveness threshold was relaxed. Final validation and shipping follow below; full parity remains active.


Final validation passes **125/125 Chromium scenarios**, **121 browser unit tests**, **56 real Rust HTTPS beta/mail stages**, TypeScript, formatting of changed browser files, **39 Python tests**, **30 parity contracts**, pinned strict documentation, production build/fixture/license/WASM inspection and promo build/separated site staging. The SQLite/shared-MIME WASM hashes match the previous checkpoint. Reviewed light/dark/compact synthetic WebP captures are in ignored `artifacts/browser-lineage-visuals/`; logs use `artifacts/logs/browser-lineage-*`.

The full protocol/control rerun initially found one obsolete regression expecting rejection after an acknowledged individual move; its stale-source test now performs an unverified physical replacement, while two separate cases verify acknowledged/recovered moves. The first production HTTPS run also expected the old two-field alias shape. It now verifies the original provider lineage and canonical target lineage against their respective metadata. Both failed runs remain in `artifacts/browser-lineage-failures/`; final runs above pass. The broad formatter inspection included generated WASM glue and an unchanged pre-existing test with style differences; changed-file formatting passes, without modifying those unrelated files.

This increment does not rerun unchanged desktop/Flutter native suites or claim live-provider, Apple or performance verification. Full parity remains active: synchronous Undo before local storage, remaining startup/review/staging lifecycle, native group controls, cross-account transport, broader account/calendar/Google/backup behavior, distribution and VPS deployment stay in TODO. R75/R76 remain tracked. Exact VPS/owner/OAuth configuration is pending; the site is staged, not deployed. Quality/release workflows remain disabled and main/the personal installation remain untouched. Mandatory commit-hook and prompt shipping evidence follow.


### Browser identity-continuity shipping record

Committed and pushed as [`396b806`](https://github.com/sam-ruff/shep.so/commit/396b806156e2fe91b74334b9d40a0b42c860ff88) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final evidence is **125/125 Chromium scenarios**, **121 browser unit tests**, **56 real Rust HTTPS beta/mail stages**, TypeScript, changed-file formatting, **39 Python tests**, **30 parity contracts**, pinned strict documentation, production fixture/license/WASM checks and promo/site staging. Light/dark/compact and production HTTPS WebP captures were reviewed in ignored `artifacts/browser-lineage-visuals/`; earlier failures remain recorded above. No responsiveness budget was relaxed.

R77's prompt push is fulfilled for this increment. The combined branch contains Flutter, the separate browser client, Rust beta backend and delegated promo website. Main and the personal installation remain untouched; owned browser/HTTPS processes have stopped, and root log files are absent. Full parity and VPS deployment remain active with the explicit TODO limitations above. Quality/release workflows stay disabled; re-enable them when the trusted runners and release prerequisites are ready.


### Browser immediate group Undo — R42/R50/R60/R63/R67/R69/R71/R73/R74/R77

The pending increment prepares one additional metadata page in worker SQLite and restores the expected rows/counts synchronously when Undo is clicked, before decision persistence or a subsequent query. Durable jobs and provider execution remain authoritative. Rejected decisions restore the display while preserving newer flag intent and the open reader body; their recovery error survives closing History. Restored rows retain their physical baseline so a follow-up action can run before another page arrives. Counterfactual queries never modify durable source data, and an in-flight successive move uses its actual dispatch folder rather than obsolete frozen-review metadata.

Initial held-storage controls reproduced delayed first paint. Subsequent runs exposed query contention and History progress waiting for repeated preview preparation. Query-only calculations now run after releasing the mail snapshot; History renders observed progress independently while its first Undo preview prepares. Receipt projection uses per-field applied revisions when journal acknowledgment is ahead of cache. No assertion deadline or performance budget was relaxed. Earlier failures remain under ignored `artifacts/browser-undo-failures/`, including `push-targeted/`. A protocol fixture also incorrectly resolved every move to the last unrelated message; its object-scoped provider now retains exact per-message identities and content proofs. New controller checks caught missing source metadata on restored rows and verified the fix; the stale-scope fake was corrected to hold the original request instead of returning an Inbox response for Trash.

All 123 browser unit tests pass, as do 39 Python tests and 30 parity contracts. Final Chromium, production HTTPS, visual review, documentation and mandatory-hook shipping evidence follow. These browser-only changes do not require rerunning unchanged Android/desktop UI suites and do not establish Apple or live-provider execution. Full client parity, native group controls, the wider Undo/lifecycle/performance audit, distribution and VPS deployment remain active. R75/R76 retain Google OAuth and grouped scheduled automatic replies as TODOs. Exact VPS/owner/OAuth configuration remains pending. Quality/release workflows stay disabled.


The full Chromium run passes **128/129** scenarios; the unchanged Find-remapping file then passes **9/9** over three repetitions. The isolated failure remains open under R63 with its trace in `artifacts/browser-undo-failures/full-first/`; this is not a clean full-suite pass. All 28 targeted bulk controls/executor scenarios and 123 unit tests pass. Light/dark/compact WebP captures are reviewed, and production build, changed-file formatting, Python/parity and strict documentation checks pass.

R78 requests a credit-limited handover and final push. Root `handover.md` records the worktrees, superseded choices, implementation, actual verification, remaining requirements and commands. TODO history was consolidated while preserving all 40 active request entries and their order; unfinished features stay open. The user requested stopping feature development after this checkpoint. Independent main work remains untouched. Final production HTTPS, hooks and shipping evidence follow.


### Immediate Undo and credit-limited handover shipping

Committed and pushed as [`d4da04c`](https://github.com/sam-ruff/shep.so/commit/d4da04c4ebcf39358f9227fb6190c822f8094a8c) to `feat/mobile-web-clients`; the exact remote SHA is verified. Mandatory hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final production HTTPS passes **56 fixture stages**. Other evidence: **123 browser unit tests**, **28 targeted bulk scenarios**, **128/129 full Chromium scenarios followed by 9/9 unchanged Find reruns**, **39 Python tests**, **30 parity contracts**, TypeScript, changed-file formatting, strict pinned documentation, production fixture/license/WASM inspection, promo build and site staging. The intermittent Find failure remains explicitly open; no deadline was weakened. Reviewed synthetic WebP captures and failed traces remain in ignored artifacts.

R78 is complete: `handover.md` and the consolidated TODO list are shipped, preserving unfinished product requests. Only the completed handover entry is removed from active TODO. R77's prompt push is fulfilled for this checkpoint. The combined branch includes all client/backend/promo work; the independent main worktree and personal installation are untouched. Assets are staged, not deployed; full parity, Apple/live-provider execution, performance and VPS configuration remain open. Quality/release workflows stay disabled; re-enable only when requested and trusted runners are ready. **Feature development stops here at the user's request.**


### Android phone installation — R79

On 2026-09-08, paired the owner-authorized phone over wireless ADB and installed Shep **0.1.0 (1)** for its active Android user. The normal `production` flavor uses `lib/main.dart` and the packaged native Rust library; no preview fixtures were installed. Flutter 3.44.2 / Dart 3.12.2 built the ARM64 release APK successfully. A separate copy was signed with the existing development key, verified with `apksigner`, and installed successfully without clearing data. This is a development-signed installation, not a store release. Its unchanged Flutter/shared source is from [`67544dc`](https://github.com/sam-ruff/shep.so/commit/67544dc28a04b137d4c689b9f2d2fa9d1eaf0d80).

Android reported a successful cold activity launch for `so.shep.shep_mobile/.MainActivity`; package/version checks passed and the process remained running. The phone was locked during the final UI observation, so rendered screen contents were not verified. No personal accounts, messages or credentials were used by automation. Pairing details and device identifiers are excluded from repository records.

The local installable APK is in ignored `artifacts/flutter/phone-install/shep-0.1.0-arm64.apk`, SHA-256 `cbe6780f67b922dfc4a31e19bd12ec5ffd0552d3aef52b824ee96957acd2fe83`. Build/signature logs are under `artifacts/logs/phone-*`. R79 installation is fulfilled; documentation shipping follows with the resumed browser checkpoint. Full parity and platform/distribution testing remain open.

### Browser shortcut capture during background completion — R63/R44/R67/R69/R73/R77

After the user explicitly resumed work, two real-input regressions reproduced the saved Find-remapping failure: a provider completion could discard active capture/focus or detach a button held between mouse-down and mouse-up. Shortcut capture now belongs to the mounted UI, with retained buttons/ancestors and callbacks rebound to current preferences. Remapping checks current conflicts, saves current settings and cancels on Escape or focus departure. Clear remains clickable during capture.

Before-fix failures are retained in ignored `artifacts/browser-capture-failures/reproduce/`. The first corrected targeted run passed 15 scenarios and all 123 browser unit tests passed. TypeScript subsequently caught optional-state narrowing; that compile error is corrected before final validation. An additional saved control scenario checks switching captures, theme changes and leaving/reopening Preferences. Final full controls, production HTTPS, screenshot review, mandatory hooks and shipping evidence follow. This browser change does not claim complete native keymap parity, Apple execution, live-provider verification or performance results.


The first resumed full Chromium run passed **131/132** scenarios, including all three new shortcut controls. A successive-group History observation stayed at 5 changed after its durable Archive job had completed all 125 writes; the following Flag job had already progressed to 102. The trace established a stale display, not a failed server/cache move. Holding both the initial History observation and Undo preview reproduced this deterministically with the unchanged five-second assertion. The first preview-only hold had passed because a separate observation could win the refresh race; both runs are retained in `artifacts/logs/browser-history-preview-reproduce*`, and the failing controls in `artifacts/browser-capture-failures/history-preview-held/`.

History now starts preview preparation separately from its progress refresh, with an independent request token that rejects late preparation after changing groups. A preview failure leaves progress and other actions usable, keeps Undo unavailable until prepared and offers a persistent Refresh-history recovery. The saved successive-group test holds the relevant observation/provider/preview boundaries; another control scenario retries a rejected preview and completes Undo. No timeout was relaxed. Initial shortcut screenshot review also caught missing fictional service capabilities and a saved light capture that did not scroll to Find; those fixture/evidence issues are corrected before final review.


Final Chromium regression passes **133/133 scenarios**. All **123 browser unit tests**, TypeScript, changed-file formatting, **39 Python tests** and **30 parity contracts** pass; all 15 focused History/Find/Preferences controls pass too. Reviewed WebP captures in ignored `artifacts/browser-capture-visuals/` show persisted light/dark bindings, completed History progress with preparation held, and visible failed-preview recovery. Earlier failures remain recorded above. Production build/HTTPS, strict documentation and mandatory-hook shipping evidence follow. Unchanged desktop/Flutter UI suites were not rerun for this browser-only increment; the separately requested phone build/install evidence is R79 above.


### Shortcut/History and phone-installation shipping record

Committed and pushed as [`1ea12a6`](https://github.com/sam-ruff/shep.so/commit/1ea12a677829dcd71c4246b87cae46327f9c749f) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA is verified. Mandatory hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final evidence passes **133/133 Chromium scenarios**, **123 browser unit tests**, **15 targeted History/Find/Preferences scenarios**, **56 real Rust HTTPS fixture stages**, **39 Python tests**, **30 parity contracts**, TypeScript, changed-file formatting and pinned strict documentation. Production fixture/license/WASM checks, promo build and separate site staging pass. Reviewed synthetic WebP captures and the earlier failed traces remain in ignored artifacts. No assertion deadline or performance threshold was relaxed.

R79 is complete: the owner-authorized phone has the normal Shep 0.1.0 ARM64 application installed and Android confirmed its launch, with the locked-screen observation limit recorded above. Its development-signed APK remains local, not published as a store release. Only R79 leaves the active TODO. R77 prompt shipping is fulfilled for this increment. The resumed handover now starts with remaining group/lifecycle/native parity work; the prior credit-limited stop remains superseded.

Full client parity, R75 provider Google OAuth, R76 grouped scheduled automatic replies, Apple/live-provider execution, performance, distribution and VPS deployment remain active. Exact VPS/owner/OAuth configuration is still pending. The combined branch includes Flutter, browser, backend and delegated promo work; assets are staged, not deployed. Main and the installed desktop remain untouched. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready. Final documentation shipping records this checkpoint without claiming completion of the full product goal.


### Browser saved group recovery at startup — R42/R63/R67/R69/R71/R73/R74/R77

The resumed parity increment reads failed, unconfirmed, interrupted-review and pending cache-receipt state across all History pages using existing IndexedDB indexes. It returns at most four recovery targets and no MIME or membership list. Startup recovery publishes the observed status before unrelated queued provider work finishes; the notice opens the affected group directly even when it is older than the first 20 History entries. Observational reads never recover another tab’s running step. Active receipt/cache handshakes are not presented as failed cache repairs.

Status checks coalesce to one read and one replacement, retain known targets when inspection fails, and stop publishing after client disposal. A new ordering test reproduced an obsolete cache-repair notice after execution completed. Execution revisions now reject a status read crossing that boundary; the failed unit baseline remains in `artifacts/logs/browser-recovery-units-first.log`. All 127 units pass after the fix. The initial light/dark startup controls pass with 52 failed messages, one abandoned result, one interrupted review, 25 newer reviews and unrelated provider work held open. Broader retry/cache/tab-loss controls, visual review, production checks and shipping follow.

This is a browser parity increment. Native captured-group controls/execution and their recovery notices remain open, as do abandoned-review/staging cleanup, remaining Undo/lifecycle/performance work, the broader client/account/calendar/Google/backup goal, Apple/live-provider execution and deployment/distribution. No personal phone or account is used by these fixtures.


All **137 Chromium scenarios**, **127 browser unit tests**, **13 focused group/recovery controls**, **39 Python tests**, **31 parity contracts**, TypeScript and changed-file formatting pass. The full run includes light/dark accessibility checks for both compact and expanded recovery notices. Reviewed synthetic WebP evidence is in ignored `artifacts/browser-recovery-visuals/`: the default summary leaves room for the inbox, while explicit Review opens the detailed targets. The original expanded-first layout was readable but crowded the compact inbox and was replaced before shipping. Cache retry completes the saved receipt without repeating its acknowledged mail action; another tab observes live ownership without recovering it, then reports uncertainty only after the owner’s lock is gone. Production HTTPS/build/docs and mandatory-hook shipping evidence follow.

During this increment, concurrent documentation commits `02c4b32` and `59578f3` added the owner-requested highest-priority OAuth/profile handover and a 31st parity contract. These changes are preserved. After shipping the already-started recovery increment, resume with OAuth and cross-device profiles from `docs/agents/PROFILE_SYNC_HANDOVER.md`, preserving the outstanding credential-protection choice and separate database-transfer requirements.


Production build and all **56 real Rust HTTPS fixture stages** pass for saved-group recovery. Production output excludes the preview entry and injected failure data, retains the SQLite/font licenses and expected shared WASM, and stages with the promo site in ignored `artifacts/site-recovery-20260908/`. The pinned strict documentation build passes. Logs are under `artifacts/logs/browser-recovery-*`; mandatory hooks and verified review-branch shipping follow.


### Saved-group recovery shipping record

Committed and pushed as [`df4f29c`](https://github.com/sam-ruff/shep.so/commit/df4f29c2e12d21fc71353920696dd4958cde363a) to `feat/mobile-web-clients`; the exact remote SHA is verified. Mandatory hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Relevant browser evidence passes **137 Chromium scenarios**, **127 unit tests**, **13 focused recovery/group controls**, **56 real Rust HTTPS fixture stages**, **39 Python tests**, **31 parity contracts**, TypeScript, formatting, production build/staging and strict documentation. Synthetic light/dark/compact WebP captures were reviewed; the stale cache-observation failed baseline is retained. No deadline or performance threshold was relaxed.

Only browser startup saved-group discovery/review is delivered in this increment. R42 remains active for remaining Undo lifecycle, review/staging cleanup, cross-account transport, performance and native captured-group execution/controls. Full client parity, Google OAuth/profiles, automatic replies, Apple/live-provider verification and deployment/distribution remain open. Main, the installed desktop and the personal phone remain untouched. Quality/release CI remains disabled; re-enable it when the trusted runners are ready. OAuth/shared profiles are the next priority.


### Shared profile metadata prerequisite — desktop-main:R92/R75/R02/R49/R67/R73/R77

The next-priority increment adds a pure shared Rust operation codec with explicit account connection/name/settings/deletion records, stable identities and causal parents. It preserves optional data, requires explicit security/capabilities, rejects ambiguous duplicate fields/targets and bounds parsing/output per record. Shared desktop/Flutter account mappings resolve legacy SMTP defaults and produce review candidates without importing data or credentials. The native validation request has separate bounded background capacity; a standalone WASM ABI uses the same golden fixtures. See `docs/agents/PROFILE_FORMAT.md` for the implemented subset and limits.

Initial tests caught a mistaken fixture field-count expectation and, more substantially, two flattened Serde deserializers retaining known action fields as extensions: re-encoding created duplicate keys. Explicit known-field consumption fixes the latter while retaining unknown fields. Failed evidence is retained in `artifacts/logs/profile-codec-tests*.log`, `profile-codec-native.log` and `profile-codec-wasm.log`; subsequent corrected fixtures pass. Final native/Dart/WASM/regression and shipping evidence follow.

This is not complete OAuth or profile sync. Platform consent/verified identity, live same-project app-data visibility, discovery/enrollment, durable causal merge/application, remaining portable settings/categories, protected credential transfer and full database migration remain active. No production profile files are uploaded. The pending password-protection choice is preserved; main, installed applications and personal accounts are untouched.


Final profile-codec checks pass **9 codec/account-mapping Rust tests**, **67 Flutter native Rust tests**, **14 actual Dart FFI tests**, **23 common WASM cases plus duplicate/size/depth checks**, **35 backend tests** (the explicit production HTTPS test is ignored in that default run), **39 Python tests** and **31 parity contracts**. Root/native Clippy, formatting and pinned strict documentation pass. The native test holds all provider capacity while validating fixtures; the Dart FFI verifies existing accounts and untouched credential access. Earlier duplicate-extension failures remain recorded. The existing browser UI and Android/Apple control suites are unchanged and were not rerun for this metadata-only feature; no new control, live-provider or performance claim follows from these tests. Flutter analysis and mandatory-hook shipping evidence follow.


### Shared profile codec shipping record

Committed and pushed as [`80979d2`](https://github.com/sam-ruff/shep.so/commit/80979d26db8d44e2f9caaed4f02e535807f9b826) to `feat/mobile-web-clients`; the exact remote SHA is verified. Mandatory hooks pass formatting, Clippy and **390 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Additional verification passes **67 mobile native Rust tests**, **14 actual Dart FFI tests**, **23 shared WASM cases plus malformed-record checks**, **35 backend tests**, **39 Python tests**, **31 parity contracts**, Flutter analysis, native Clippy and strict pinned documentation. Shared release stamping includes the crate and its path lockfile entries; the new WASM gate is recorded only in the deliberately disabled quality workflow.

The codec/account review adapters and common fixtures are delivered prerequisites. Full OAuth/provider consent, verified cross-client Google access, discovery/enrollment, durable causal merge/application, remaining settings/category mappings, protected credentials and complete database transfer remain unfinished. The next turn continues this highest-priority work from the handover; the browser recovery increment was separately pushed as `df4f29c`. All work remains on the review branch, with main, installed applications and personal accounts untouched. Quality/release CI stays disabled until the trusted runners are ready.


### Desktop feature-scoped Google consent — desktop-main:R92/R75/R02/R49/R63/R67/R73/R77

Preferences now separates the next sign-in's Drive/Calendar permission choices from the active grant. Fresh setup selects no services implicitly; legacy known grants supply initial choices until an explicit selection is saved. Drive uses app-private storage; Calendar can be off, read-only or editable with its required list scope. Google token responses omitting scope inherit only the exact requested set. Pending candidate metadata is bound to that set across failed keychain saves/restart; refresh retains the original selection, and broader returned scope cannot activate unselected services. Changed preferences fence queued sign-in, its saved-form acknowledgment and final activation, retaining the prior grant/cache.

All 46 selected Google protocol/lifecycle/UI-ordering tests pass, including exact URL/PKCE parameters, partial and broader grants, scope fallback/refresh, candidate replay prevention, changed selection and opt-in persistence. Existing denial/callback/rotation tests remain. Native control/visual/regression and shipping evidence follow; this desktop change does not implement mobile/browser provider consent, verified cross-client profile identity, enrollment or continuous sync.

Implementation follows [Google installed-app authorization](https://developers.google.com/identity/protocols/oauth2/native-app), [Calendar scopes](https://developers.google.com/workspace/calendar/api/auth) and [OAuth token-response scope](https://www.rfc-editor.org/rfc/rfc6749.html#section-5.1). It keeps the installed-app system-browser/PKCE flow and does not assume incremental authorization support. Real Google client registration and live cross-platform access remain unverified.


Initial native verification retained two fixture-coordinate failures: the taller Google card required scrolling before clicking Disconnect, and a compact test tried to click a large-layout agenda target outside its owned 900-pixel display after resizing the window. The saved tests now reach the visible Disconnect and compact agenda controls directly. Deadlines are unchanged. Light saved/refused-sign-in controls and compact dark requested-versus-active permissions pass; the compact event remains read-only after requesting future editing access. Reviewed WebP captures show both permission states clearly. Full functional native regression and shipping follow; logs remain under `artifacts/logs/google-consent-*`.


Final desktop consent verification passes **118/118 native functional scenarios**, **398 root/shared Rust tests** (two personal-account diagnostics intentionally ignored), **46 selected Google tests**, **39 Python tests** and **31 parity contracts**. Root Clippy/formatting and pinned strict documentation pass. Light 1440×920 and dark 900×640 captures were reviewed, including requested editing versus an active read-only event. Earlier coordinate failures remain in the ignored logs; assertion deadlines were unchanged. This was a functional run on the development host, with no new latency, live Google or Apple claim. Mandatory-hook and remote shipping evidence follow. Full mobile/browser provider authorization and continuous profiles remain open.


### Desktop consent shipping record

Committed and pushed as [`3e1181b`](https://github.com/sam-ruff/shep.so/commit/3e1181ba68692b63104cec4f926f3391ecfab470) to `feat/mobile-web-clients`; the exact remote SHA is verified. Mandatory hooks pass formatting, Clippy and **398 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. All **118 native functional scenarios**, **39 Python tests**, **31 parity contracts** and pinned strict documentation pass; synthetic light/dark/compact captures are reviewed.

Desktop feature-scoped consent is delivered. Full R75 OAuth parity and R02/R49/profile sync remain unfinished: mobile/browser provider grants, verified cross-client identity/app-data visibility, discovery/enrollment, causal merge/application, remaining portable categories and protected credentials are next. No request is removed for this partial delivery. The personal phone, installed desktop and independent main worktree remain untouched. Quality/release CI stays disabled; re-enable when requested and trusted runners are ready.


### Flutter native Google consent continuation — R75/R02/R49/desktop-main:R92/R63/R67/R69/R73/R77

Flutter's production entry now supplies the supported native Google SDK adapter and secure connection metadata to Preferences. Identity comes from SDK authentication and is bound to the configured application; access tokens remain in the SDK. Requested Drive/Calendar permissions stay separate from committed access. One SDK operation and a coalescing preference writer preserve ordering; changed choices, denial, different identity/application and failed storage cannot activate a replacement. Disconnection commits before local-only sign-out, with retryable cleanup after restart. Background token requests use only an already authenticated session and enabled scopes.

All nine lifecycle tests, two actual compact/held-consent widget scenarios, the configured SDK-adapter contract and secure-store lost-reply regression pass. The host regression passed 87 tests before the additional secure-store test. Initial control failures exposed lazy-list test navigation and a real narrow Calendar dropdown overflow; saved controls now scroll the actual settings list, and the expanded dropdown fits its field. The first emulator launch used an unsupported GPU mode and terminated; the owned dedicated AVD booted with its supported SwiftShader mode. No personal device was used. Full native/Appium, final Playwright capture, strict documentation and shipping evidence follow.

This increment is not full provider or profile parity. Real Google configuration/callbacks/refresh, safe seamless account switching, automatic session restoration, Calendar/Drive use, discovery/enrollment/merge, credentials and Apple execution remain open. The SDK has one active account on some platforms; reconnect therefore keeps the saved identity and never implicitly signs it out just to show a picker. Configured builds currently require explicit local disconnection to select another account. See `docs/agents/GOOGLE_MOBILE.md`.


The first Android control run reached the saved light connection but stalled in the new capture helper. Comparing the working native capture helper showed the missing frame pump between surface conversion and screenshot. That owned driver was explicitly interrupted and is not counted as a pass; its log and separate ADB observation are retained. The runner now also requires both scenario completion markers in a fresh report, because an interrupted `flutter drive` can exit zero. A follow-up test build caught a misplaced import in the capture change; it was corrected before the next run. Final native evidence follows.


Android's corrected capture run passes both named Google scenarios and saves four screenshots; compact dark and changed-consent captures are reviewed. Final browser capture review exposed insufficient real scrolling in the new script, which is being corrected using the existing dropdown pointer helper. Dependency review also found that the combined Google plugin automatically loads its web SDK during registration. The adapter now uses the maintained native platform packages directly through their pinned interface, and the preview Playwright flow rejects external requests. This changes dependency registration; final SDK/Android/browser checks must use the native-only package set.


Further storage review separated an unconfirmed write/readback from a definite rejected write. Google operations now pause until an explicit successful read reconciles the committed record, preserving newer unsaved choices. A lost disconnection acknowledgment cannot lead to a new sign-in before cleanup is recovered. Actual secure-store method-channel and controller regressions pass, alongside the existing compact controls; native controls also retain a saved unconfirmed-storage/read-retry path.


The real-pointer browser run reproduced merged accessibility semantics: the Calendar dropdown inherited the whole Google card's label/bounds and a click could hit the Drive switch. A separate explicit child-semantics boundary fixes the product issue; the browser/Appium assertions now require Drive to remain off when only Calendar is selected. Before-fix DOM/captures are retained under `artifacts/flutter/web/google-semantics-before-*`. The preview's new external-request gate additionally caught CanvasKit's unconditional fallback Roboto download. The same SDK-provided Roboto font and its license are now bundled locally; Shep's Noto Sans theme stays configured. Final offline/native regression follows.


The first complete Appium run passed six existing flows, then its generic label helper selected the non-clickable “Preferences saved” status instead of the Preferences tab. The saved native tree distinguishes those controls. The helper now requires clickable elements and waits for actual Preferences content before scrolling; the failure screen/tree/log are retained in `artifacts/flutter/google-appium-first-failure/`. This is a test-navigation correction, not a successful Google flow. Final execution follows.


Final Flutter source verification passes **89 host tests**, the separately configured SDK-boundary test, **two named Android Google control scenarios**, **seven offline Flutter Playwright flows**, clean analysis, **40 Python tests** and **31 parity contracts**. Five Android Google screenshots were saved and the compact light/dark, held-consent and unknown-storage states reviewed. The production ARM64 release APK builds with the Rust bridge and local Roboto/Noto font/license assets; checked preview-only identity/token/message markers are absent. Native Google plugin registration was exercised by Android platform initialization, without account authentication. This development-signed build has no registered Google client configuration and was not installed on the personal phone. Final Appium and shipping follow.

The second Appium attempt reached Preferences but the new high-level scroll command left the list stationary. Its saved tree/screen are retained under `artifacts/flutter/google-appium-scroll-failure/`. The scenario now injects an actual W3C touch swipe and requires a changed visible tree before continuing, keeping the existing assertion deadlines. The final Appium corrections change test navigation only.


**Final Appium verification passes all seven flows**, including scoped consent, cancellation retaining Drive-off/Calendar-read-only access, retry and reviewed local disconnection. The last intermediate run had already saved the correct connection but its added account details pushed the confirmation below the viewport; its capture/tree remain under `artifacts/flutter/google-appium-notice-failure/`. The saved scenario now scrolls to newly inserted confirmation/error text through actual touch input. The final cancellation capture is reviewed and retained as WebP. Final logs are `artifacts/logs/mobile-google-appium-complete.log` and `android-appium-e2e.log`. The final Appium corrections change test navigation only. Flutter formatting, configured SDK fixture, Python/parity and pinned strict documentation pass. Mandatory hooks and remote shipping follow; live Google, Apple, providers and continuous profiles remain open.


### Flutter scoped consent shipping record

Committed and pushed as [`6c4bb65`](https://github.com/sam-ruff/shep.so/commit/6c4bb65fbc02c96dd258ed9942e64e6282bca181) to `feat/mobile-web-clients`; the exact remote SHA is verified. Mandatory hooks pass formatting, Clippy and **398 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final source checks pass **89 Flutter host tests**, the configured SDK contract, **two Android Google scenarios**, **seven Appium flows**, **seven offline Flutter Playwright flows**, **40 Python tests**, **31 parity contracts**, clean Flutter analysis/formatting, the production ARM64 release build and pinned strict documentation. Synthetic compact light/dark and cancellation/storage-recovery captures are reviewed. Intermediate failures remain explicitly recorded above; none counts as a passing run.

This delivers the native consent/permission/local-cleanup increment. Full R75/profile parity remains active: registered live Google clients, safe switching/automatic restoration, actual Calendar/Drive use, verified profile discovery/enrollment/merge, protected credentials and Apple execution are unfinished. The test emulator and inspection server were stopped. Main, installed desktop and personal phone were untouched; quality/release CI remains disabled until requested with trusted runners ready. The next restart continues OAuth/shared profiles from the handover.

## 2026-09-09 — Native causal profile history checkpoint

R75/R02/R49/desktop-main:R92 adds durable metadata history in
`shared/profile-core`, a bounded owning worker and the production Flutter native
bridge. Two local stores retain exact operations, merge independent fields,
preserve concurrent versions for review, reject stale resolutions and retain
account/profile tombstones through offline edits and restart. Local edits are
idempotent after lost replies; reserved upload IDs and byte digests persist.
Accepted worker writes outlive cancelled observers. SQL failure rolls back record,
ancestry, field versions and counters together. See [the API and integration
boundaries](agents/PROFILE_HISTORY.md).

The history uses its own database and worker, independent of occupied mail
provider capacity. Native binding changes drain the previous journal; a stale
close cannot close another binding. Flutter encodes/decodes profile metadata off
its UI isolate. Root/native rusqlite moves to 0.40.2 / SQLite 3.53.2, deliberately
porting the dependency fix inspected in desktop `db8c82a`; that commit's entire
mail-cache owning-worker migration is not included here.

Verification before shipping: **17 shared profile tests** (10 history, one worker,
six codec), **68 native Rust tests**, **90 Flutter host tests**, clean native Rust
Clippy/Flutter analysis, **41 Python tests**, **31 parity contracts**, strict pinned
documentation and **23 common WASM cases** plus duplicate/size/depth rejection
pass. The shared actual Dart FFI scenario also passes on the isolated Android
emulator: two stores exchange synthetic account/settings records, retain/resolve
a conflict and reopen queued work without accessing credentials. Its completion
marker is required by the wrapper; it is backend integration, not a new Settings
control E2E. Production ARM64 APK builds successfully with the native library;
this new artifact is unsigned. The earlier installed phone checkpoint was
development-signed. Final root hooks, performance and shipping evidence
follow below.

Retained failed-before evidence under ignored `artifacts/logs/profile-history-*`:
initial SQL integer-conversion compile errors; symlink lock alias failure; duplicate
tombstones counted as conflicts; the planner choosing a competing index; two
Clippy style findings; the first Dart fixture's missing import; and two Android
runs failing temporary-directory setup before opening history. Corrected ownership,
tombstone/index logic and fixture setup pass the corresponding unchanged contracts.
The full Android setup failure log is retained separately. No interrupted or failed
run counts as a pass. The history-only code did not change product UI. The subsequent required storage
benchmark exposed the search query issue below and extended desktop verification.
Unchanged Appium/Playwright control suites were not rerun for the metadata bridge;
earlier control evidence remains separately recorded.

**Remaining:** authenticated Google identity/owned-file transport, complete
creation/discovery/enrollment checkpoints, category/profile switches, actual
account/preferences application, complete portable settings and protected
credentials. A supplied binding is not verified Google identity; matching a local
reserved ID/digest is not proof of a cloud upload. Zero missing ancestry is not a
complete cloud listing. Large-history/compaction, device-ID rebinding during full
transfer, encrypted local SQLite, Apple and genuine cross-client Google access
remain open. The full product goal stays active; personal phone/main/installed
desktop were untouched and quality/release CI stays disabled.


## 2026-09-09 — Search plan correction during storage verification

The required 100,000-message benchmark passed page budgets but spent several
minutes in search. It was stopped for diagnosis; that incomplete run is not a
search p95 or a passing benchmark. The original plan used a virtual-table LEFT
JOIN, repeating literal FTS filtering/ranking for each fuzzy candidate. A preserved
synthetic database and the pinned SQLite plan comparison reproduce that expensive
plan; an attempted debugger attachment was unavailable and provides no profiling
evidence. Logs stay under ignored `artifacts/logs/profile-history-*`.

`store/mail_query.rs` now materializes exact-match rowids/ranks once in SQLite and
joins the indexed relation. It preserves short exact-body priority, exact-versus-
fuzzy scores, Unicode, filters and shared list/selection ordering. The new planner
regression rejects a virtual-table LEFT JOIN for both ordinary and combined folder
scopes. Existing search/selection/bulk tests pass (21 tests), plus the plan guard.
Mobile/browser fuzzy-ranking parity remains explicitly open in TODO and the shared
scenario matrix. Final native controls and unchanged performance budgets are
verified below before shipping.


The first complete benchmark after materializing exact ranks measured search at
68.94 ms against the unchanged 50 ms gate (Inbox 33.38 ms, account page 30.47 ms).
It failed before body/navigation measurements, and remains preserved as
`artifacts/logs/profile-history-backend-benchmark-final.log`. A separate pinned
SQLite diagnostic isolated the per-page unread badge query: the original plan
looked up message rows and sorted account groups. A covering
`(folder, unread, account)` index removes both steps. On that synthetic database,
the diagnostic component measured 45.204 ms before and 4.668 ms after; those are
diagnostic component timings, not final release benchmark results.

The cache now creates the covering index on open, including existing caches. A
reopen/plan regression protects that path; existing unread projection, search,
selection and bulk tests retain their behavioral assertions. Full native
verification before this additional index passed all 118 functional scenarios;
final targeted controls and fresh performance evidence follow below. No query
predicate, result ordering or budget was weakened.


The first 14-scenario native index rerun during compilation passed 12 and failed
two existing input flows: compact bulk selection stayed at zero after consecutive
checkbox clicks, and the badge preference expected four while the underlying mail
state had already changed to three unread messages. Screenshots/state were reviewed
(`68972d6b03cb` and `c51b45348c43`); these are retained input-under-load evidence for
R63, not proof of an index-counting error or a fixed input race. The unchanged suite
is rerun after compilation, without forced clicks, added delays or relaxed checks.
The earlier full 118-scenario run passed. See final results below.


## 2026-09-09 — Final profile-history and storage verification

Code checkpoints: [`e9115f9`](https://github.com/sam-ruff/shep.so/commit/e9115f978d6753fb66400178927ffb5b4bd61c97)
(profile history), [`e568c84`](https://github.com/sam-ruff/shep.so/commit/e568c84)
(materialized search) and [`da6f2e8`](https://github.com/sam-ruff/shep.so/commit/da6f2e803fbddf1e485418b925923eeeb08afd1e)
(covering unread counts). Mandatory hooks pass formatting, Clippy and **411
root/shared Rust tests**, with two personal-account diagnostics intentionally
ignored. Native Rust passes **68**, Flutter host **90**, Python **41**, shared WASM
**23 cases** plus malformed-record checks, and parity **32 contracts**. Native
Clippy, Flutter analysis, the backend's locked dependency check and strict pinned
documentation pass. No provider/platform claim is inferred from these fixtures.

The actual profile-history FFI scenario passes on Android. Its wrapper verifies a
fresh named completion marker. The final ARM64 production build contains the Rust
bridge, configured NotoSans/Roboto fonts, license notices and checked production
identity, with checked fixture markers absent. This single-ABI artifact is
**unsigned**, confirmed by `apksigner`; it is not the earlier development-signed
phone installation or the complete distribution gate. An initial ad-hoc package
check assumed the wrong font family; the corrected inspection checks the configured
files. No phone installation was repeated.

All **118 native functional scenarios** pass before the final index; all **14
relevant index/badge/search/selection/bulk scenarios** pass unchanged afterward
with compilation stopped. The earlier 12/14 run's input failures remain R63.
Reviewed synthetic exact-body-first/after-refresh, compact dark selection and
light page-two captures are copied to ignored `artifacts/profile-history-reviewed/`.
The original run directories and failed screenshots are retained.

The final backend benchmark passes: Inbox **6.030 ms**, account **3.106 ms**, search
**35.537 ms**, body **0.025 ms** p95. Native navigation remains over budget:
**154.81, 162.33, 154.88 and 155.14 ms** in four 30-transition runs against **150 ms**.
The combined gate was executed and **fails on navigation**; its handler measurement
passes at 0.010 ms. A previous cached worktree test executable also failed three
runs at 159.31–168.24 ms. This supports retaining the existing responsiveness
investigation, not attributing it to the new index or claiming it is fixed. The
current executable and report were restored after comparison and verified. See
[measurement scope](PERFORMANCE.md); no timing deadline, click flow or threshold
was changed to produce a pass.

Logs are under ignored `artifacts/logs/profile-history-*`; comparison hashes and
reports under `artifacts/profile-navigation-comparison/`. Final source checks and
code shipping are recorded here separately from full product completion. R75
provider identity/transport/discovery/enrollment/category controls, real
account/preferences application, protected credentials, browser history, Apple,
VPS configuration and the broader parity/performance work remain in TODO. Main,
installed desktop and personal phone were untouched. Quality/release workflows
remain disabled; re-enable only when requested with trusted runners ready.


The three code checkpoints are **pushed to `feat/mobile-web-clients`**, with exact
remote head `da6f2e803fbddf1e485418b925923eeeb08afd1e` verified. This fulfills prompt
review-branch shipping for the profile-history/storage increment, with the native
navigation failure explicitly retained. Full goal completion and release readiness
are not claimed. Final handover/evidence documentation follows this code head.


## 2026-09-09 — Shared Google profile transport

The optional native `drive` feature in `shared/profile-core` now verifies the
Drive principal, validates owned app-data operation metadata and exact media,
returns bounded 50-file pages and connects immutable uploads/imports to the
existing owning journal. Reserved IDs persist before POST; a lost response or
restart reconciles the same file, and only matching remote bytes acknowledge it.
Conflicting files cannot be overwritten or deleted. The wire format and remaining
integration are in [Google profile files](agents/PROFILE_DRIVE.md).

The shared metadata/media fixture and **34 profile core tests** pass. The scripted
production-transport tests cover identity and namespace refusal, access failures
and redirects, invalid/partial/oversized pages, byte/metadata mismatches, separate
real journals, missing ancestry, persisted reservation before POST, response loss,
409 verification, exact-ID retry, cancellation and restart. The first durability
fixture queried a nonexistent table; its failed log is retained, and the corrected
assertion reads the real operations table through an independent connection. No
production schema or safeguard was weakened to fix the fixture.

Compatibility checks pass **68 mobile Rust tests**, **23 shared WASM fixtures**
plus malformed-record checks, **41 Python tests**, **33 parity contracts**, and
the backend's locked dependency check. Mandatory hooks, strict documentation and
exact shipping confirmation are recorded with the code commit below. Logs remain
in ignored `artifacts/logs/profile-drive-*`. No personal data or credentials are
used by these tests.

This transport is optional infrastructure, not yet enabled/called by desktop or
Flutter Settings. It has no durable discovery catalog, enrollment/category UI or
real account/preferences application. Its bounded page API cannot prove an atomic
cloud snapshot; a configured namespace cannot prove cross-client Google-project
visibility. Live registered Google, browser transport, Android/Apple provider
execution, protected credentials and full sync remain active TODOs. Existing
Android/UI evidence belongs to earlier increments, not this HTTP fixture suite.
No UI/E2E, APK or timing rerun is claimed for this provider-only change. The prior
combined performance gate still fails native navigation. Main, installed desktop
and personal phone were untouched. Quality/release CI remains disabled; re-enable
when trusted runners are ready and the user requests it.


Profile transport code [`557f8d5`](https://github.com/sam-ruff/shep.so/commit/557f8d5d1dd01ed9d2eee2decbd2a5235f036cac)
and fixture portability [`9289f53`](https://github.com/sam-ruff/shep.so/commit/9289f5327b71bb6aaff463ee965eab48c13df85a)
are **pushed to `feat/mobile-web-clients`**, with the exact remote `9289f53` head
verified. Mandatory hooks pass formatting, Clippy and **428 root/shared Rust
tests**, including the 34 profile core tests; two personal-account diagnostics
remain intentionally ignored. Strict pinned Zensical passes. A real Git checkout
with `core.autocrlf=true` confirms that the two wire fixtures preserve their exact
bytes, size and digest under the explicit LF attributes. This is a checkout
conversion check on Linux, not Windows or Apple execution.

Final shipping logs are `profile-drive-commit.log`,
`profile-drive-portability-commit.log`, `profile-drive-line-endings.log`,
`profile-drive-docs-final.log`, `profile-drive-docs-portability.log` and
`profile-drive-push.log` under ignored `artifacts/logs/`. Updated TODO/handover
evidence follows these code checkpoints. The full product goal remains active;
durable catalog/enrollment, real client application, live access and the previous
native performance failure are not completed by this push.

Final review also shares the HTTP client policy between production and loopback tests, so redirect rejection is verified through the same builder instead of a duplicated fixture policy. Production keeps its fixed HTTPS endpoint; the fixture changes only local connection settings and its deadline. The final mandatory-hook result and push cover this refinement with the existing redirect cases.


## 2026-09-09 — Durable remote profile discovery

R75/R02/R49/desktop-main:R92/R67/R69/R73/R77 continuation adds a per-principal,
application-scoped catalog under the optional shared native Drive feature. Saved
metadata pages, file identities and change replay preserve progress across
interruption. Prepared file identities commit before remote history import, so a
failed later receipt cannot hide a file disappearing during a full rescan.
Missing/reclassified files and pagination conflicts retain explicit errors and
cached observations. Profile summaries preserve missing ancestry, concurrent names,
setting reset intents and tombstones. Remote observation journals remain separate
from enrolled local history and offline edits. See [the contract](agents/PROFILE_DISCOVERY.md).

**49 shared core tests pass**: 32 unit/protocol/worker, 6 codec and 11 history.
New scenarios cover held success/error results after a rescan, retry/reopen,
arrivals and repeated changes, long cycles, duplicate IDs, 52-profile paging,
conflicts, missing parents, removal and a real SQLite receipt failure after history
commits. Queue saturation, cancelled observations, canonical aliases and an
independent child process verify ownership. An overview test initially treated an
explicit reset as no setting; the final test verifies the retained reset intent
and zero visible settings only after a profile tombstone. Failed compile/fixture
logs remain in ignored `artifacts/logs/profile-discovery-*`; no production
history semantics or test budget was weakened.

Compatibility checks pass **68 mobile Rust tests**, **23 WASM fixtures** plus
malformed-record checks, **41 Python tests**, **34 parity contracts**, the backend
locked dependency check and strict pinned Zensical. Core Clippy passes. Final
mandatory-hook results and the code commit are recorded in the shipping entry.
Logs use the `profile-discovery-` prefix, including `final-core`, `native`, `wasm`,
`python`, `backend`, `clippy` and `docs` under ignored `artifacts/logs/`.

This optional catalog has no client Settings caller or live Google grant. Actual
creation/enrollment, platform lifecycle binding, own-upload identity integration,
first-setup publication completeness, account/preferences application, category
controls and credential protection remain open. A caught-up scan is not proof of
complete ancestry, successful enrollment or shared Google-project visibility.
Browser/Apple/Android discovery and catalog performance are unverified. No new UI,
E2E or APK result is claimed. The earlier native navigation/combined timing gate
still fails. Main, installed desktop and personal phone are untouched; quality
and release workflows remain disabled. The full product goal remains active.

The unchanged 100,000-message storage benchmark also passes: inbox p95
6.101 ms, account 3.007 ms, search 36.017 ms and cached body 0.074 ms, with
60 query samples. The optimized build finished before measurement; the desktop
session and other applications remained open, so this is not an idle-host claim.
This verifies mail storage budgets, not catalog latency or native presentation.
The prior native navigation failure remains unchanged.


Discovery code [`3c9b98d`](https://github.com/sam-ruff/shep.so/commit/3c9b98d514bf667064f5cd92a22d4dda84998de7) is **pushed to `feat/mobile-web-clients`**; the exact remote
head was verified. Mandatory hooks pass formatting, Clippy and **443 root/shared
Rust tests**, with two personal-account diagnostics intentionally ignored. The
49 profile tests and all compatibility/strict-docs/storage results above cover
this code. No client control, live Google or full parity completion is implied.
Shipping logs are `profile-discovery-commit.log`, `profile-discovery-push.log` and
`profile-discovery-docs-final.log` under ignored `artifacts/logs/`; the saved
benchmark is `artifacts/profile-discovery-backend.json`. The final documentation
checkpoint follows this code in branch history.


## 2026-09-09 — Flutter Google profile discovery controls

R75/R02/R49/desktop-main:R92/R67/R69/R73/R77 continuation connects Flutter's saved
Google grant to the shared Drive catalog. Preferences now offers discovery,
retry/rescan, pause/resume and 50-summary paging with explicit missing/conflicting
history. The verified principal commits to device secure metadata before profile
contents appear. Same-account re-consent retains it; stale grants, failed writes
and unconfirmed storage cannot silently replace the binding. The native session
owns accepted work through close, and both old data and errors are fenced from
replacement sessions. See [the client contract](agents/PROFILE_MOBILE.md).

Verification passes **71 mobile Rust tests** and native Clippy, **104 Flutter host
tests**, clean analysis and the separately configured SDK-boundary test. Android
executes two discovery control scenarios (retry, 52-profile pagination, appearance,
pause while reading mail, resume), the existing two Google consent scenarios and
the real native two-store history/conflict/restart scenario. The same four saved
flows pass with **UiAutomator2/Appium and Flutter Playwright**: failed discovery and
retry, dark appearance, local disconnect and return to mail. UI providers are
isolated fixtures; Rust session/catalog tests exercise actual native ownership
separately. These results do not establish live Google or separate browser-client
profile integration. **41 Python tests** and **35 parity contracts** pass.

Final synthetic WebP captures are under ignored
`artifacts/profile-client-reviewed/final/`: browser and Android light/dark profile
rows, persistent errors, disconnected recovery, pagination and paused discovery
were reviewed. Singular/plural labels and row spacing were corrected. The first
Android Appium run was obstructed by a System UI unresponsive dialog; its capture
and diagnostics are retained, with no claimed root cause. After choosing the OS
Wait control, later tests ran without that dialog. Further failures exposed a tap
on Theme's trailing padding and a Back-transition accessibility race. The saved
flow now targets the painted dropdown and waits for the actual clickable control
under its existing deadline. No forced clicks, hidden ANR handler or relaxed
functional/performance thresholds were added. Failed web accessible-name matching,
widget scrolling and the initially misplaced native async-trait dependency are
also retained under `artifacts/profile-client-failures/` and
`artifacts/logs/profile-client-*`.

Result logs include `profile-client-native-fixed`, `native-clippy`, `analyze-final`,
`host-final`, `sdk-final`, `android-history-final`, `android-google-final`,
`web-final`, `appium-final`, `python-final` and `parity-final` under
`artifacts/logs/` (each has the `profile-client-` prefix). Named Android reports are
under `artifacts/flutter/native/`; the Appium/browser results are in
`artifacts/flutter/discovery-native/` and `discovery-web/`. Production build,
mandatory hooks, strict documentation and shipping are recorded below when verified.

Initialized first-profile publication, own-upload receipts, reviewed enrollment,
real account/preferences application, complete category mappings, credentials,
legacy migration and desktop/browser/Apple/live Google integration remain active.
The unanswered password-protection choice is preserved. The prior native timing
failure remains; no new latency claim is made. Main, installed desktop and personal
phone are untouched. The new flows are wired into the **disabled** quality
workflow; documentation publishing remains enabled. The full product goal is open.

The production ARM64 release APK builds successfully with the Rust and Flutter
libraries, bundled fonts/licenses, production package identity and Internet
permission. The inspected fixture markers are absent, including the new discovery
entrypoint/provider markers. It is **unsigned**, has no registered Google project
configuration and was not installed on the personal phone. This ARM64-only check
does not satisfy the all-architecture distribution gate. Inspection and SHA-256
are in ignored `artifacts/profile-client-apk.json`; the build/inspection logs have
`profile-client-apk-` names. Dart formatting, strict pinned Zensical and diff checks
pass. TODO cleanup preserves all **40 active request entries** in their original
order; older completed prerequisite prose remains traceable in this log.


Flutter discovery code [`438682e`](https://github.com/sam-ruff/shep.so/commit/438682e277c93832a95168034b9940afe8de0cc0) is **pushed to `feat/mobile-web-clients`** with
exact remote verification. Mandatory hooks pass formatting, Clippy and **443
root/shared Rust tests**, with two personal-account diagnostics intentionally
ignored. The final mobile/Android/browser/APK and strict-docs evidence above
covers this code. Shipping logs are `profile-client-commit.log` and
`profile-client-push.log`; the remote verification record is ignored
`artifacts/profile-client-shipping.json`. A final documentation checkpoint follows
in branch history. Creation/enrollment, real account/preferences application and
all remaining parity work stay active.

## 2026-09-09 — Reviewed Flutter profile publication

Flutter **Profiles and sync → Create profile** now prepares a named account/settings
review and publishes it through the production native Drive transport. The review
shows all eight current Flutter preferences and 50 account rows at a time, with
selectable connection details. Changed local accounts/preferences invalidate approval.
No passwords, OAuth grants, mail or drafts enter the portable records. This is
publication; enrollment, real account/preferences application and continuous sync
remain active. See [the publication contract](agents/PROFILE_PUBLICATION.md).

Native schema 10 retains frozen reviews, explicit account mappings, exact history
requests and publication progress. Lost staging/approval receipts retry the original
identities. Shared `initialization-v1` history requires a preparing root and causal
completion before a profile becomes initialized. Tracked upload records the verified
owned file in discovery before confirming the local queue; a failed catalog receipt
keeps the same remote reservation for retry. Pause/close finishes accepted work,
and grant/session generations prevent late results reaching a replacement account.

Verification passes **52 shared profile tests**, **74 mobile Rust tests**, native
Clippy, **111 Flutter host tests**, clean analysis and the configured SDK-boundary
fixture. The **28 shared codec cases** pass native Rust, actual Dart FFI and standalone
Rust WASM, including additional duplicate/size/depth rejection. Native tests exercise
75 accounts/78 operations, 50-row reviews, legacy IDs, stale account/settings reviews,
lost cross-database receipts and close/reopen while an upload and mail capacity are
held. They preserve cached accounts and credential slots. **41 Python tests** and
**36 parity contracts** pass. Final Android regressions, strict docs, production APK
and mandatory shipping hooks are recorded below.

The two saved publication scenarios pass on Android, and four matching publication
flows pass through **Appium/UiAutomator2 and Flutter Playwright**. They cover entered
names, frozen values, account details/pages, failed upload and Resume, light/dark
appearance, Pause and actual mail reading while a step is held. Providers in these
controls are isolated fixtures; shared/native protocol tests separately cover the
real transport and persistence. No live Google or Apple success is claimed.

Synthetic final captures under ignored `artifacts/profile-publication-reviewed/final/`
were reviewed for readable settings/account details, paged controls, persistent errors,
paused progress, mail navigation and light/dark completion. Browser account details
were initially visible but absent from the accessibility tree; replacing their
`SelectableText` with `SelectionArea(Text)` preserves selection and exposes the
content. The same saved Playwright assertion now passes. Native Appium initially
met a System UI unresponsive dialog; its screenshot/tree and logs are retained,
and only the dedicated emulator's OS dialog was dismissed. No root cause is claimed.
A missed initial Preferences tap and unfocused name entry are also retained; the
saved flow waits for Appearance and focuses the real field before typing/asserting
the entered name. No forced clicks or relaxed deadlines were added. The broader
native input-under-load audit remains active.

Failures are retained under `artifacts/profile-publication-failures/` and
`artifacts/logs/profile-publish-*`, including initial SQL integer conversions,
old schema expectations, Dart lint/control-route checks and native Clippy. Final
core/native/host/SDK/Android/web result logs carry the same prefix; named Android
reports live under `artifacts/flutter/native/`. Fixture captures contain no personal
mail. The website, main worktree, installed desktop and personal phone are unchanged.
The new flows are wired into the deliberately **disabled** quality definition;
documentation CI remains enabled.

All **40 active requests** remain in TODO. Reviewed enrollment/application,
desktop/browser publication, full portable categories, credential protection,
legacy migration, automatic native SDK restoration, Apple/live Google and complete
product parity remain unfinished. The prior native navigation gate still fails
154.81–162.33 ms against 150 ms; this increment makes no new latency claim. The
password-protection choice remains unanswered.

Final Android regression executes **seven named scenarios**: two publication,
two discovery, two Google consent and one actual two-store native history/restart
flow. Publication and discovery each pass four Appium and four Flutter Playwright
flows, **eight flows per automation surface**. The shared harness retains actual
click/touch input and both earlier discovery recovery/dark/disconnect flows.
Final named reports and WebP captures are retained under the paths above. Final
analysis and all **111 host tests** pass after the account-detail accessibility fix.

The production ARM64 release APK builds and passes scoped inspection for the Rust
and Flutter libraries, absent fixture markers, bundled fonts/licenses, production
package and Internet permission. SHA-256 and details are in ignored
`artifacts/profile-publication-apk.json`. `apksigner` confirms it is **unsigned**;
no registered Google project is configured and no phone installation occurred.
All-architecture distribution, signed releases and Apple verification remain open.
The dedicated emulator and preview servers have been stopped after testing.

The unchanged 100,000-message storage benchmark passes: Inbox p95 **6.28 ms**,
account page **3.10 ms**, FTS search **36.10 ms** (each under 50 ms), cached body
**0.02 ms** (under 10 ms). Owned Android/browser builds and the emulator finished
before measurement; host observations are retained in `profile-publish-bench-host.log`,
without claiming an otherwise idle host. This is a storage regression check, not
native input-to-pixel evidence or a fix for the prior combined-gate failure.
Strict pinned Zensical, Rust/Dart formatting and diff checks also pass.


Publication code [`184b98a`](https://github.com/sam-ruff/shep.so/commit/184b98afafcf53bc3fd7c32a497fad04746304bf) is **pushed to `feat/mobile-web-clients`**, with exact
remote verification and a clean worktree at the code checkpoint. Mandatory hooks
pass formatting, Clippy and **446 root/shared Rust tests**, with two personal-account
diagnostics intentionally ignored. No hooks were skipped. Shipping logs are
`profile-publish-commit.log` and `profile-publish-push.log`; the machine-readable
record is ignored `artifacts/profile-publication-shipping.json`. A documentation
checkpoint follows in branch history. TODO and handover retain all 40 active
requests, with enrollment and actual account/preferences application next.


## 2026-09-09 — Reviewed Flutter profile enrollment

Flutter can now open an initialized discovered profile, review account connections
and eight portable preferences, and apply the selected changes on this device.
Review pages contain at most 50 items; connection details, category switches and
individual choices remain visible. Pause/Resume and lost-reply recovery retain the
same account IDs and settings receipt. See [the enrollment contract](agents/PROFILE_ENROLLMENT.md).
This advances actual client application; continuous synchronization and complete
cross-client parity remain unfinished.

Shared catalog export returns one original immutable operation, fenced by source
revision and scope. Native schema 11 copies those records into an independently
owned editable journal, without cloning device identity or upload queues. Approval
checks the source, local history revision and frozen account fingerprint. Each
account application atomically records its connection, independent empty credential
slot, shared mapping, Reconnect marker and receipt. Existing matching accounts keep
their credentials and cached mail; changed endpoints require an explicitly selected
separate account. Newer local changes and removed mappings remain protected.

Imported accounts refuse credential lookup/provider work until reviewed credential
activation, and remain removable before reconnecting. Preference writes retain
per-field revisions and the application receipt together. Delta saves and UI edit
generations preserve unrelated changes, including edits made while a write is held
or changed away and back. Two actual preference controls tapped before the next
repaint now merge current state; the saved regression failed before the callback
fix. Metadata refresh preserves the reader, cached messages and unsaved drafts.

Verification passes 79 mobile Rust tests, 54 shared profile tests, native Clippy,
125 initial Flutter host tests, clean analysis, the configured Google SDK fixture, 28 WASM
codec cases with malformed-record rejection, 41 Python tests and 37 parity
contracts. Native enrollment tests use original production history records and the
real SQLite application path; control providers are isolated fixtures. These layers
do not establish fully authenticated Google-to-Flutter interchange, live provider
success or Apple execution. Final control, packaging and shipping results follow
below after verification.

Retained failures under ignored `artifacts/profile-enrollment-failures/` include
old schema expectations and the removal-before-reconnect defect, widget cleanup
and independent storage ownership, offscreen/ambiguous control locators, rapid
preference overwrites and the browser completion accessibility omission. The latter
was painted but missing from the accessibility tree; explicit live-region semantics
now expose the same visible summary. A final Android rerun was canceled before
building because Flutter discovery blocked on a stale wireless debugging transport;
it is not a pass. The host transport was detached and testing resumed on the
isolated emulator. No phone installation or personal-data automation occurred.

All 40 active requests remain in TODO, including provider OAuth, grouped scheduled
automatic replies and Linux store submissions. Desktop/browser publication and
enrollment, ongoing reconciliation, all portable categories, credentials, automatic
SDK restoration, same-project interchange, Apple/live-provider evidence, VPS
configuration/deployment and full feature parity remain open. The prior native
navigation gate still fails at 154.81–162.33 ms against 150 ms; this increment does
not resolve it. Quality/release definitions remain disabled, documentation CI
remains enabled, and main and the installed desktop remain separate.


Android enrollment passes both named scenarios and four Appium flows; publication
regressions also pass both named scenarios and four Appium flows. The rapid preference regression failed before the fix and passes through
real host and Android controls. Review then added a device-inset test: existing
footer spacing passed a small inset but failed a 60-logical-pixel inset. The new
SafeArea uses the actual device inset. All 126 final host tests pass, including the larger-inset regression. Its initial
run also exposed a saved test scroll direction that could not reach an earlier row
after paging; the helper now scrolls toward that row through real input. Native
and browser verification follow below. This is not a claim that every navigation
mode has been run.


Final Android verification passes **five named scenarios**: two enrollment, two
publication and one actual two-store native history/restart scenario. Enrollment
and publication each pass four Appium flows, **eight total**, with no recorded
external requests or runtime errors. The final enrollment controls also assert
footer clearance and both saved rapid preference changes. Android reports are in
ignored `artifacts/flutter/native/integration-{enrollment,creation,profiles}-result.json`;
Appium reports are under `artifacts/flutter/{enrollment,creation}-native/`.
The dedicated emulator was stopped after these runs.


Final Flutter Playwright verification passes **eight flows**: four enrollment and
four publication, with empty runtime-error and external-request reports. All
preview servers stopped normally. These are Flutter automation surfaces, not a
claim about the separate hosted browser client's provider parity. Reviewed WebP
captures under ignored `artifacts/profile-enrollment-reviewed/final/` cover
light/dark reviews, current/profile values, details, paging/footer controls,
application retry, paused progress, completion, reconnect status and preserved mail.


The production ARM64 release APK builds and passes scoped inspection for the Rust
and Flutter libraries, bundled fonts/licenses, production package, Internet
permission and absence of fixture markers. Its digest and checks are in ignored
`artifacts/profile-enrollment-apk.json`. It is **unsigned**, has no registered Google
project configuration and was not installed on the phone. All-architecture signed
distribution and Apple execution remain open. Final Dart/Rust formatting, clean
Flutter analysis and pinned strict Zensical pass; later shipping documentation is
built again before pushing.


New storage timing is **deferred** because unrelated compilations saturated the
host during finalization, as required by AGENTS.md. Host observations are saved in
ignored `artifacts/logs/profile-enroll-bench-host-before.json`. Only benchmark
compilation was requested; it is not timing evidence. Run the unchanged storage
benchmark after these jobs settle, and keep the prior native/combined performance
gate failure active. Functional/protocol/control results above do not imply a new
latency result or full performance completion.


Enrollment code [`9d6a6c6`](https://github.com/sam-ruff/shep.so/commit/9d6a6c6df644d340c1192ba13215c298ce8ac8b0) is **pushed to `feat/mobile-web-clients`**, with exact
remote verification and a clean worktree at the code checkpoint. Mandatory hooks
pass formatting, Clippy and **448 root/shared Rust tests**, with two personal-account
diagnostics intentionally ignored. No hooks were skipped. Shipping logs are
`profile-enroll-commit.log` and `profile-enroll-push.log`; the verification record is
ignored `artifacts/profile-enrollment-shipping.json`. Benchmark compilation also
finished successfully; timing remains deferred because unrelated compiler jobs are
still active. A final documentation checkpoint follows in branch history. Full
parity, continuous synchronization and all 40 active requests remain unfinished.


## 9 September: desktop Google profile discovery

Preferences → Profiles and sync now uses the saved active Google grant and the
shared Drive catalog. Verified principal, OAuth client/lifecycle revision and local
request generations fence old data and errors. The separate bounded queue keeps
cached mail and ordered local saves independent. Pause stops subsequent steps;
retry, reopening and rescan retain original history and saved progress. Reopening
a completed catalog checks changes again. Profile pages contain at most 50 rows.

The initial compact layout test exposed a wrapped tab row that shifted existing
controls. A horizontal strip now retains their positions and brings the selected
tab into view. The native controls cover light/compact dark discovery, failed scan,
pause/browse/resume, completion, close/reopen, incremental/full refresh and refusal
without active Drive permission. The full **121-flow native functional suite passes**
(`artifacts/logs/desktop-profile-native-full.log`, 474.582 s). Reviewed captures are
in `artifacts/e2e/54d226133fd6`, `767783b22836` and `ad5d2e748927`; the initial compact
failure remains in `dd3e9c0180a8/failure-8.webp`.

Five targeted desktop/controller tests pass, including real shared HTTP/SQLite
recovery across 51 profiles and a new catalog owner, invalid-page retention,
foreign-principal refusal, grant replacement and namespace edits. The fixture
transport is available only with nondefault test-support and cannot target remote
hosts or accept real tokens. **41 Python checks**, **37 parity contracts** and the
pinned strict Zensical build pass. The mandatory commit hook runs formatting,
Clippy and the full Rust suite; its result is recorded with the shipping receipt.

This is read-only desktop discovery. The eight desktop preference mappings are
preparation for reviewed application. Desktop/browser publication and enrollment,
native large-page controls, automatic restoration, continuous reconciliation,
remaining portable categories/settings, protected credentials, Apple/live Google
and fully authenticated cross-client interchange remain open. Stale parity notes
that still called all Flutter enrollment unfinished were corrected. All 40 active
requests remain in TODO. Native performance measurements were omitted on the busy
host; the earlier 150 ms navigation gate still fails. No phone installation, main
merge, VPS deployment or quality/release CI enablement is part of this checkpoint.


Desktop discovery code [`9e666a5`](https://github.com/sam-ruff/shep.so/commit/9e666a55582746fa59a1849ecc2d870a4c9d4b3c) is pushed to `feat/mobile-web-clients`; exact remote
verification matched that commit. Normal hooks pass formatting, Clippy and **454
Rust tests**, with only two opt-in personal-account diagnostics ignored. The
production configuration also passes `cargo check --lib` without test-support.
Shipping and gate logs are under `artifacts/logs/desktop-profile-*`. This receipt
delivers the discovery increment, not full profile sync or product parity.


## Desktop reviewed profile publication

Preferences → Profiles and sync now publishes a named selection of saved account
connections and eight supported portable preferences. The review freezes source
values, pages account metadata in groups of 50 and shows connection details.
Changing accounts or selected preferences rejects approval. Displayed preferences
must finish saving before preparation or approval enters the profile queue.

The mail owner stores exact planned operations and explicit account mappings.
Each history request is durable before crossing into an independent journal;
tracked Drive uploads retain reserved IDs and exact media before confirmation.
Pause/navigation stops subsequent steps, and reopening retains the review and
receipts. The active grant fences each accepted step; changed OAuth clients force
full discovery while retaining known files. Publication never copies mail,
passwords, tokens or device-only settings.

Ten targeted Rust tests pass, including 75-account preparation/paging, changed
reviews, lost staging receipts, and a full mail-database/session reopen after a
failed Drive confirmation. The latter publishes five records with five distinct
POSTs and verifies the initialized result through another catalog. The save-order
controller test covers mismatched acknowledgments, pause, save failure and changed
grants. Native controls pass both saved `test_desktop_profile_publication_*` flows:
selection/name input, details, pause/browse, failed confirmation/retry, reopened
receipts, compact dark review, changed preferences/cancel and account exclusion.

Reviewed targeted captures are in `artifacts/e2e/ce56a55b4f59`, `77a412dc2c44` and
`10259a2a8af5`; final-suite publication captures in `e9fe87150127` and
`30226a395d57` were also reviewed. Earlier evidence caught a moved Retry control (`53e08622ea48`) and
an upload error below account details (`ee445bb4cbc3`); the final layout preserves
Retry and places publication errors beside recovery controls. A test initially
used the compact Dark coordinate in a wide window (`5be7c1a7416a`); the corrected
scenario targets the actual wide control before resizing.

The full **123-flow native functional suite passes** in 483.337 seconds
(`artifacts/logs/desktop-publication-native-full.log`). **41 Python checks**,
**37 parity contracts** and the pinned strict Zensical build also pass. The final
commit-hook and shipping receipt follows below. All 40 active requests remain.
Desktop enrollment, native large-page controls, browser publication/application,
ongoing reconciliation, complete portable categories/settings, protected passwords,
authenticated cross-client interchange, Apple/live Google and full product parity
remain open. Fixture HTTP/SQLite/native success does not establish live Google.
Performance measurements remain deferred on the busy host; the earlier navigation
gate still fails. Quality/release CI remains disabled. No phone reinstall, main
merge or VPS deployment accompanies this worktree increment.


Desktop publication code [`35f11ba`](https://github.com/sam-ruff/shep.so/commit/35f11ba0627621624659455dfebf6f341ea18893) is pushed to
`feat/mobile-web-clients`; the exact remote SHA matches the source commit and the
worktree was clean after shipping. Mandatory hooks pass formatting, Clippy and
**459 Rust tests**, with only the two opt-in personal-account diagnostics ignored.
`cargo check --lib` also passes without test-support. The 123 native functional
flows, 41 Python checks, 37 parity contracts and strict docs build are recorded
above. Shipping evidence is `artifacts/desktop-publication-shipping.json`; logs
remain under `artifacts/logs/desktop-publication-*`. This completes the publication
increment, not desktop enrollment, continuous sync, live Google or full parity.


## Desktop reviewed enrollment continuation — 2026-09-09

Preferences → Profiles and sync now prepares and applies an existing shared
profile. Original records enter independently owned local history without taking
the catalog's device identity or upload queue. Source/history revisions fence
approval. Reviews page 50 items, show account connection details and offer category
and individual choices. Matching identities preserve local metadata, mail, drafts
and credentials; different connections require explicit selection as a separate
account. Unsupported fields stay unavailable and retain their original history.

Account metadata, mappings, reconnect guards and receipts commit atomically.
Imported accounts cannot use saved password entries or server operations until
explicit reconnect succeeds. Incoming and separate SMTP writes must both succeed;
partial failure, changed metadata and removal retain the guard. Backups skip those
password entries, and restore cannot silently activate guarded metadata. Local
cached actions remain available. Profile preferences use per-field revisions and
explicit GUI edit masks, preserving newer/reverted intent and unrelated settings.
Accepted values update inbox/reader behavior without another preference save.

The source passes **470 Rust tests**, with the two opt-in personal live diagnostics
ignored; Clippy and production compilation without test-support pass. **41 Python
checks**, **37 parity contracts** and all **seven targeted native profile flows**
pass. Tests include lost copy/application receipts and database reopen, matching
mail/draft preservation, differing connections, removal after approval, an
independent offline journal, 78 items in 50/28-row pages, unsupported settings,
partial keychain failure, protected backup/restore and exact preference-save
ordering. Google/layout controller tests now assert the actual preference-patch
command while retaining their ordering and queue-saturation assertions.

Native evidence includes final targeted runs `d9ba56c8abe3` and `e5d8cef42318`:
review/details, application, Reconnect required, incoming setup with an empty
password, independent mail navigation, saved enrollment reopen, compact dark
cancellation and a newer Light preference kept during application. Reviewed
captures also include `eae4d48e8e4c`, `5ec88fd015d2`, `03000c59bea5`,
`c662aa276e45` and `0015523d36da`. Initial failures `68c8715e7d2e` and
`5bf106e8feb5` caught Retry moving after removal of obsolete explanatory text;
`ea3479414d09` caught a profile row whose click area was too narrow. Final rows
span the available width and saved controls target the actual Retry position.
All artifacts/logs stay ignored under `artifacts/desktop-enrollment-*`,
`artifacts/logs/desktop-enrollment-*` and `artifacts/e2e/`.

The full **125-flow native functional suite passes** in 477.553 seconds
(`artifacts/logs/desktop-enrollment-native-full.log`); the strict Zensical build
also passes. Final-suite captures `bb572a9ac551` and `8df5119fca63` were reviewed.
The mandatory-hook and shipping receipt follows below. All 40 active requests
remain. Automatic setup, ongoing reconciliation, full categories/settings,
credential protection/transfer, browser application, native large-page controls,
Apple and live Google/cross-client verification remain open. The earlier
performance gate still fails; new timing is deferred on the busy host. Quality
and release CI remain disabled. No main merge, phone reinstall or VPS deployment
accompanies this worktree increment.


Desktop enrollment code [`8f969cd`](https://github.com/sam-ruff/shep.so/commit/8f969cd91d45ac2e4a821c927c360a86e64569cd) is pushed to
`feat/mobile-web-clients`; its exact remote SHA matches. Mandatory hooks pass
formatting, Clippy and **470 Rust tests**, with only the two opt-in personal live
diagnostics ignored. The source worktree was clean after shipping. All **125
native functional flows**, 41 Python checks, 37 parity contracts, production
compilation and strict docs pass as recorded above. The shipping receipt is
`artifacts/desktop-enrollment-shipping.json`. This completes reviewed initial
desktop enrollment; the full product goal and all 40 active requests remain open.


## Desktop profile pages — 2026-09-09

Changing an enrollment choice on a later page no longer returns the review to
page one. Recoverable failures retain the page for the same review; opening a new
review starts at the first page. The 78-row protocol/storage regression failed
before this fix and passes afterward (`artifacts/logs/profile-pages-before.log`
and `profile-pages-rust.log`). Skipped new connections say **Not imported**;
publication guidance now describes the delivered reviewed import.

Three saved native scenarios use a bounded loopback fixture with 51 profiles,
75 local accounts and 75 offered accounts. Discovery exercises 50/1 profiles in
light and compact dark. Publication reviews 50/25 accounts, connection details,
First/More and cancellation without uploading. Enrollment exercises 50/26 rows,
repeated second-page choices, details and page revisits, then applies 74 chosen
accounts while excluding one. All 74 imports retain reconnect guards; the chosen
Dark appearance applies. Original local accounts remain intact.

The full native run passed **127 of 128 flows** in 570.506 seconds. Its only failure
was the existing badge preference test expecting its initial count of four after
read-on-leave navigation correctly changed the count to three. The private-bus
trace confirms 0 → 4 → 0 → 3; the saved screen and mailbox agree. The test now
checks the current global unread total before re-enabling badges, retaining
zero/hidden, preference persistence and account-scope assertions. Focused rerun
and final Rust/shipping results follow below. Logs are
`artifacts/logs/profile-pages-native-full.log` and `profile-pages-badge-rerun.log`.

Reviewed full-run captures are `0c03527d1547` (discovery), `b35109af89ab`
(enrollment) and `c5d1ee4520df` (publication); targeted captures include
`08470e12e865`, `dadd38d66249` and `b63beeeb0ac0`. Retained failed-before captures
include `5e0f787d867f` (General retained scroll), `5994de8400de` and `d95d3d518d6e`
(test cursor assumptions), `b63beeeb0ac0` (cancelled reviews remain durable) and
`b2ed904d168a` (the badge count). No control was forced or deadline relaxed.

The fixture/targeted Rust checks, 42 Python checks, 37 parity contracts and strict
Zensical build pass. Flutter already retains its selected enrollment page; its
existing mobile controls remain the counterpart. Separate browser enrollment,
continuous reconciliation, automatic setup, complete settings/categories,
protected credentials and live Google/Apple remain open. All 40 active requests
remain. Performance is deferred on the busy host and its earlier failure remains
tracked. Quality/release CI stays disabled; no phone reinstall, main merge or VPS
deployment accompanies this checkpoint.


All four badge flows pass in the focused rerun (26.535 seconds), including the
corrected current-count assertion. Its compact capture `d34cbac780b9` is reviewed.
Thus all **128 native functional scenarios pass across the full run and focused
rerun**; this is not a claim that the initial full invocation had no failure.
The other 127 scenarios already passed on the same production source; only that
test's stale baseline/navigation checks changed before the rerun. Final mandatory
Rust hooks and remote shipping are recorded below.


Profile page code [`0b0ffcb`](https://github.com/sam-ruff/shep.so/commit/0b0ffcbf4bdb4e6501cf3d098d7691d4c9eef497) is pushed to `feat/mobile-web-clients`; the exact
remote SHA matches and the source worktree was clean after shipping. Mandatory
hooks pass formatting, Clippy and **471 Rust tests**, with only the two opt-in
personal live diagnostics ignored. Production compilation without test-support,
42 Python checks, 37 parity contracts and strict Zensical pass. Native results
are the 127-pass full run plus the corrected four-flow badge rerun described
above, covering all 128 scenarios. The receipt is
`artifacts/profile-pages-shipping.json`. All 40 active requests remain; next is
ongoing Flutter/desktop reconciliation and automatic first setup/restoration.

## Desktop reconciliation engine — 2026-09-09

A durable local edit ledger and bounded runner now exchange later preference
changes through independent enrolled histories and the shared Drive provider.
Exact requests survive lost acknowledgments and newer local edits. Remote
application and its receipt commit atomically, preserve device-only settings and
do not echo. Concurrent values remain available for review. Copy cursors are
bound to the observation history identity and reset after rebuilding that cache.
Missing, replaced or rolled-back enrolled histories fail explicitly. Pausing
retains requests/upload identities and cannot authorize a workspace switch.

**This is an engine prerequisite.** Automatic scheduling, reviewed subscription
creation, active-grant ownership, sync/conflict controls, account/category
reconciliation and Flutter/browser equivalents remain unfinished. Existing
publication/enrollment does not silently enable it. The full product goal and all
40 active requests remain open. The [engine contract](agents/PROFILE_RECONCILIATION.md)
and handover record the next concrete integration steps.

Twelve new regressions comprise seven Store and five runner tests. The runner
uses actual reviewed enrollment, independent device identities and loopback Drive
HTTP. It verifies two-device changes and concurrent versions, original-record
replay after a lost receipt, cache rebuild, a delayed local receipt followed by
another device's value, missing/replaced/rolled-back history, and a committed
upload with a lost reply across Pause/restart without another upload. The complete
profile-focused run passes **41 checks**; all five runner regressions pass after
the final rollback guard. Initial fixture failures (missing synthetic grant
identity and Drive change type) remain in
`artifacts/logs/profile-sync-runner-fixture-before.log` and
`profile-sync-runner-change-feed-before.log`.

The updated synthetic Drive fixture also passes all **10 saved native profile
flows** in 131.276 seconds: discovery/permissions, publication/retry, enrollment,
page retention and light/dark/compact layouts. Reviewed captures include
`bacfd0e3f8f7` and `b451ae5e1283` (discovery), `630f3bb5ec11` (74 guarded imports,
page exclusion and compact footer), and `334ed6552462` / `fd1141808e3f`
(publication and connection details). The subsequent rollback guard changes only
the unconnected runner and has its separate Rust regression. This native run
checks the existing controls; it cannot prove an ongoing-sync UI exists.

Forty-two Python checks, 37 parity contracts, Clippy and strict Zensical pass.
Production compilation passes before the final guard; final production and
mandatory-hook results are recorded with shipping below. Logs use
`artifacts/logs/profile-sync-*`. Performance remains deferred on the shared host;
the earlier native/combined performance failure remains open. No live Google,
Apple, authenticated Flutter interchange, deployment or full-parity claim is
made. Quality/release definitions remain disabled. Phone data, the installed
desktop and the independent main worktree remain untouched.


Reconciliation engine code [`ecd98c5`](https://github.com/sam-ruff/shep.so/commit/ecd98c59254d59b270ec0ef521aa6e70fff29bd6) is pushed to
`feat/mobile-web-clients`; its exact remote SHA matches and the source worktree
was clean after shipping. The normal hooks pass formatting, Clippy and **483 Rust
tests**, with only the two opt-in live diagnostics ignored. Final production
compilation and strict documentation pass. The 42 Python checks, 37 parity
contracts and 10 native profile flows described above also pass. No full native
suite or performance remeasurement is claimed for this engine-only checkpoint.
The shipping receipt is `artifacts/profile-sync-shipping.json`.

Before exposing automatic sync, extend recovery coverage to partial catalog loss
(retained observation histories) and missing remote ancestry after a rebuild.
The current cache-rebuild test removes the complete observation directory while
all original remote files remain available. It does not prove that broader
recovery case. Then connect authenticated scheduling, reviewed subscriptions and
sync/conflict controls, followed by Flutter/account/category reconciliation.
All 40 active requests and the full product goal remain open.


## Connected desktop preference sync — 2026-09-09

Completed publication/enrollment reviews now offer **Sync these preferences**.
Setup derives original selected values and preference revisions from the backend
review and starts paused. Master and per-preference switches preserve newer
choices behind one revision-checked request. A bounded background owner runs
outside Preferences, yields to occupied provider/lifecycle capacity and suspends
for frozen reviews. Local Pause remains available during Google connection work.
Canonical snapshots preserve pending native edits; status distinguishes unchecked
uploads, queued operations and failures. Status rows keep switch positions stable.

Recovery verifies acknowledged original records against the provider inventory,
including when only catalog metadata is lost and observation history survives.
Every new/reconnected owner performs a full inventory scan. A regression first
reproduced upload into another project's empty space using cached proof; it now
stops before upload. Restoring a missing original and rescanning reuses saved
operations. Shared export uses a partial index and excludes unsent records.

The profile-focused run passes 46 checks, including atomic reverted-intent setup,
real reviewed publication/enrollment, source ownership, lost receipts, rapid UI
choice ordering, provider saturation, Google lifecycle changes and wrong-project
recovery. Sixteen relevant Flutter Rust bridge tests and 43 Python checks pass;
37 parity contracts remain valid. Shared full-suite results are included in the
mandatory gates below.

All **22 native profile/Google/Preferences flows pass** in 221.987 seconds. The
ongoing flow searches Mail through a remote Light appearance change, edits Dark
with the field paused, resumes, retains a lost-upload error/count and recovers. Protocol regressions separately
verify retry without another upload or operation. It also operates the compact field switch. Publication
controls preserve seven selected preferences and review light/compact pages.
Reviewed runs include `bfe6e5d12db4` (ongoing) and `26437ba96d3f` (publication).
Final wording-only screenshots and mandatory shipping results follow below.

Retained failures include `profile-sync-ancestry-before.log`,
`profile-sync-project-before.log`, `profile-sync-status-before.log` and
`profile-sync-native-footers-before.log` under `artifacts/logs/`. The first proves
the partial-catalog gap, the second the Google-project gap, and the third a status
update hiding the frozen-review wait. The native run passed 19/22 before the
footer correction; grouped sync controls restore the paging/retry footer layout.
Earlier native authoring failures retain search-focus and batch-validation logs;
the saved scenario now waits for actual search focus and uses bounded batches.
The existing no-force-input and timeout requirements are unchanged.

The full goal and all 40 active requests remain unfinished. Checked conflict
resolution, complete portable settings/categories/accounts, Flutter/browser
ongoing sync, first setup/restoration and live authenticated interchange remain.
No live Google, Apple, deployment, complete parity or performance claim is made.
The host had other active work; the earlier navigation/combined performance
failure remains open. Quality/release CI is disabled. Phone data, the installed
desktop and the independent main worktree are unchanged.


The two new sync control flows also pass after final wording changes (39.962
seconds). Reviewed final captures are `92c482b10061` (queued-error recovery,
compact field controls and Mail search) and `7604433c3ceb` (seven-preference
publication and compact pages). Counters use clear labels, unchecked uploads
remain distinct from zero, and values reuse the existing human-readable formatter.
Mandatory hooks, production compilation, strict docs and exact remote shipping
are recorded next; this is not yet a shipping receipt.


Connected desktop sync code [`13e056d`](https://github.com/sam-ruff/shep.so/commit/13e056dcc836fbf41e418de220085540fe174dbc) is pushed to
`feat/mobile-web-clients`, and its exact remote SHA matches. Normal hooks pass
formatting, Clippy and **490 Rust tests**, with two opt-in personal live diagnostics
ignored. Production compilation without test-support and pinned strict Zensical
pass. The 43 Python checks, 37 parity contracts, 16 Flutter Rust bridge checks,
22 native regressions and two final wording/control flows above also pass.
The source worktree was clean after shipping. The final receipt is
`artifacts/profile-sync-connected-shipping.json`. No full native suite or new
performance measurement is claimed for this increment. All 40 active requests
remain; next is checked conflict resolution and ongoing Flutter integration,
followed by the remaining profile/product scope.


## Checked desktop preference decisions — 2026-09-09

Desktop preference conflicts now open a durable review with 50-version pages,
local/shared choices, explicit cancellation and exact saved-decision retry.
Every version page must be opened before saving. The backend rechecks the local
field generation, subscription/device/workspace binding and history versions;
newer or reverted local edits remain pending. A staged decision survives restart
and cannot be discarded while its result is uncertain. Preferences and the
completed receipt commit atomically. Cached browsing/cancellation still work
after Google disconnection; new decisions require the active grant.

The saved native flow passes in **26.282 seconds**. It reviews 51 versions,
clicks the disabled premature save, cancels/reopens, chooses the last-page Light
value, reviews 900×640 dark controls, recovers a failed local receipt, then retries
a committed cloud upload with a lost reply while searching Mail. Reviewed WebP
captures are in `artifacts/e2e/3804d69ea8da/`; earlier successful save/retry captures
are in `e5c8e8e23d7d/`. Native fixtures do not access personal Google or mail.

Retained authoring failures: `profile-resolution-native-first.log` used an invalid
comparison operator; `profile-resolution-native-save.log` exceeded the harness's
30-wheel-action bound. The corrected saved flow uses existing `gte` and bounded
real scrolling. `profile-resolution-native-complete.log` clicked a row after
reopening retained scroll position; its failure capture is
`16c3948ceb12/failure-23.webp`. The corrected flow scrolls to the actual page
controls. No forced inputs, state mutations or relaxed deadlines were introduced.
`profile-resolution-controls-rust.log` retains the initial macro-import compile
error; `profile-resolution-ui-ordering.log` retains a test wired to the profile
channel instead of the existing separate preference-save channel. Both fixes pass.

Final surrounding regression checks and verified shipping are recorded below
when complete. Ongoing Flutter/browser reconciliation, complete settings,
accounts/categories, automatic setup/restoration, secure credential portability,
live Google/Apple execution and the full product goal remain open. All 40 active
requests remain in TODO. No performance budget or CI enablement changed.


Final decision checks pass: **54 profile Rust checks**, **44 Python checks** and
**37 parity contracts**, plus production compilation and strict Zensical. The
independent-device runner regression now resolves a conflict through the review,
uploads it via the loopback Drive transport and verifies the other enrolled
device converges. The surrounding **23 native flows pass in 219.995 seconds**.

A new ordering regression reproduced Back being undone by a late review-page
reply (`profile-resolution-navigation-before.log`). Review visibility now follows
explicit navigation; replies update cached review data without reopening it.
The regression passes. All **three final affected native flows pass in 64.050
seconds**: conflict decisions, reviewed publication and ongoing background sync.
Final conflict Back/reopen, compact footer and lost-receipt captures in
`artifacts/e2e/b5255ffe7d09/` were reviewed. Normal commit hooks and remote shipping
are the remaining checkpoint steps. The full native suite and performance gates
were not rerun; their earlier limits remain active.


Decision code [`28c2884`](https://github.com/sam-ruff/shep.so/commit/28c288448842b7d09543fc28ef1f2f14bf36f142) is pushed to `feat/mobile-web-clients`, with exact remote SHA
verification. Normal hooks pass formatting, Clippy with warnings denied and
**498 Rust tests**; two opt-in personal live diagnostics remain ignored. Strict
documentation and final production checks pass. The shipping receipt is
`artifacts/profile-resolution-shipping.json`. Source/UI files, tests, request
tracking, parity notes and handover are included; the main worktree and installed
phone/desktop were not changed. Quality/release CI remains disabled. Next continue
Flutter ongoing reconciliation and its native/Playwright conflict controls.


## Flutter original preference receipts — 2026-09-09

Platform profile applications now save the original eight field revisions alongside
their values and receipt. Native enrollment validates and durably acknowledges
that exact map. A retry still returns current preferences for display, while newer
local revisions cannot replace the original proof. Legacy receipts omit the map
and cannot acquire fabricated current revisions. Explicit same-value local intent
advances its field generation after a failed intermediate save.

The UI now checks the generations captured with the review before optimistic
painting. A changed-and-reverted preference, an already applied receipt or a
reopened application cannot briefly repaint obsolete imported values. Unchanged
reviewed fields still project immediately. Navigation and local editing stay
available while a reopened receipt is checked.

All **129 Flutter host tests** and **81 mobile Rust tests** pass, with formatting,
Flutter analysis and Clippy clean. The **two named Android integration scenarios**
pass (43 seconds reported by the integration suite; teardown is not a third flow).
New Rust checks cover failed native receipt transactions, original revisions after
restart, malformed/missing/older revisions, exact retry and legacy proof. Saved
controls cover losing the native acknowledgment, leaving to change appearance,
resuming and preserving the newer Light theme. Its original revision map is
asserted separately from current display preferences.

Retained evidence: `mobile-sync-optimism-before.log` reproduces the obsolete
optimistic repaint; the fixed host suite passes. `mobile-sync-receipt-controls-host.log`
records a test using the pending label after completion; it now opens **Profile
applied on this device**. `artifacts/mobile-sync-appium-system-ui-failure/` retains
Android's System UI ANR over the initial mailbox. The existing dedicated-emulator
Wait helper recovered it without changing deadlines. The next Appium run retained
an off-screen Appearance expectation after returning to scrolled Preferences in
`artifacts/mobile-sync-appium-scroll-failure/`; the flow now waits for Preferences
and uses its existing real-scroll Theme helper.

Final verification on the resumed session, 9 September evening: the Flutter web
enrollment flow first failed at **Resume profile review and application** because
Flutter web merges a list tile's title and subtitle into one clickable node and the
Playwright harness anchored its name match at the start; `artifacts/flutter/enrollment-web/failure.txt`
retains that tree. The shared harness now matches a label at the start of a name or
after whitespace, matching Android's contains-selectors. After the fix all **5 web
enrollment flows**, **5 native Appium flows** and the **two Android integration
scenarios** pass, and the harness-sharing web discovery and creation flows pass
again. Root hooks (formatting, Clippy, tests) pass with `artifacts/root-target`;
44 Python tests and 37 parity contracts pass. Lane `codex/profile-catalog-harness`
(`33d222d`) was superseded by the stricter fixture transport in `9e666a5`; its
positive-path regression is ported as a shared `test-support` test, the local branch
and desktop-side worktree are removed, and the origin branch waits for Sam.

This increment is a prerequisite for ongoing Flutter reconciliation, not its
completion. The native sync ledger/runner, SDK scheduler, field/conflict controls,
all settings/categories, accounts, automatic setup/restoration, authenticated
interchange, live Google and Apple execution remain open. All 40 active requests
remain; no performance budget or CI enablement changed.
## Main merged into the client branch — 2026-09-09

`main` at `c414227` was merged into `feat/mobile-web-clients` at `724f764` with `git merge --no-commit --no-ff`, producing the first tree that contains the desktop, mobile, browser and website sessions together. From this point `main` is the single integration branch for all three sessions; desktop changes are no longer ported into the client branch separately, and the client TODO/AGENTS porting instructions were removed.

Resolution rules applied:

- **Root code: main wins.** Main's desktop profile sync implementation (`src/profile_sync`, `src/engine/profile_sync.rs`, `src/store/profile_sync` and the related desktop UI) replaced this branch's own desktop implementation (`src/profiles/discovery.rs`, `src/profiles/enrollment` and related files). The client branch's desktop discovery/publication/enrollment/reconciliation entries above remain as history of that superseded implementation; the desktop behaviour that ships is main's.
- **Shared profile core: superset.** The shared profile-core crate became a superset of both sides, and main's git pin (`e3e69a4`) became a path dependency on the crate in this tree.
- **Provider changes ported.** Main's `a81d767`/`5eabb52` provider changes (destination recovery, folder mutation plans and checked provider commands) were ported into `shared/mail-core` at this branch's paths so the shared clients and the desktop use the same transport behaviour.
- **Hooks: union.** `.githooks/pre-commit` runs every gate either branch required: formatting, Clippy with `-D warnings`, `cargo test --all-features`, the `shep-html-pixbuf` tests and `scripts/test_profile_core.py` (main's hook was already a superset of the client hook; the client branch's `scripts/check.sh` added nothing main lacked). The commit-msg hook is unchanged. `core.hooksPath` must be the relative `.githooks` at the repository level so each worktree runs its own checkout's hooks rather than the main checkout's.
- **Tracking files: union.** TODO.md, this log, the request audit, AGENTS.md, README.md, the docs navigation and PERFORMANCE.md keep both histories. Request numbers R67 to R80 exist on both sides and are not renumbered; [the request audit](REQUEST_AUDIT.md) states the collision once and marks the client rows.

No request is closed by the merge. Performance figures from either side were not re-measured on the merged tree; the client branch's failing native navigation gate stands until rechecked on an idle host. Quality and release workflows remain `.yml.disabled`; documentation CI stays enabled.

Kept from the client branch in root code, each with its earlier completion record:
desktop feature-scoped Google consent (`3e1181b`), the materialised exact-match
search relation (`e568c84`), the covering unread-account badge index (`da6f2e8`),
shared MIME parsing in store/print/reader tests (`4226f57`) and one divider
settle wait in the native harness. Everything else in `src/`, `tests/` and
`scripts/` follows main; this branch's own desktop profile implementation under
`src/profiles`, `src/store/profile_*` and `src/ui/profiles` was deleted.
`Drive::connect_fixture` keeps main's `Option<&str>` signature and fixed token
with the union of both loopback checks. `scripts/test_profile_core.py` now
verifies the workspace-member path dependency and tests it in place; the copied
`tests/support/profile-core.Cargo.lock` is removed. `vendor/shep-html-pixbuf`
requests litehtml's `vendored` feature so its own test target resolves inside
the explicit workspace. `flutter/rust` and `backend` carry an empty
`[workspace]` table so a nested checkout never joins a parent workspace. Main's
shared `MailSyncItem` folder hierarchy and inbox lifecycle events reach the
clients, which still cache flat selectable names (recorded under R30).

Gates on the merged tree, logs under `artifacts/logs/merge-*.log`:

| Gate | Result |
| --- | --- |
| Desktop fmt, Clippy `-D warnings`, `cargo test --all-features` | 990 passed, 3 ignored |
| `cargo test -p shep-html-pixbuf`, `scripts/test_profile_core.py` | 2 passed; 47+6+13+3 passed |
| Python `unittest discover` | 96 ran, 7 skipped (Windows installer) |
| Selected native flows (`scripts/e2e.py`, 23 flows: Google consent, divider, search, forward, HTML, badges, folders, moves, preferences, read-on-leave, reply, backups, profile sync, print, move recovery) | 23/23 |
| Shared crates: profile-core all-features/test-support/history/default; mail-core; mail-content | 69, 69, 29, 6; 41; 25 passed |
| Flutter `analyze`, `flutter test` | clean; 129 passed |
| Mobile Rust, backend, real browser beta gate | 81; 35 passed, 1 ignored; 1 passed |
| Browser `npm test`, `npm run build` | 127 passed; built |
| Parity checker, strict Zensical | 37 contracts; no issues |

Not rerun on the merged tree: the full 282-flow desktop functional set, the
latency benchmark, the full browser Playwright suite (web/ and the WASM inputs
are unchanged by the merge) and the Android scenarios (mobile code unchanged
apart from folder-name caching; the host suite covers it).
