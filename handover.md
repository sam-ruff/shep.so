# Shep handover — 2026-09-09

**The full application goal remains unfinished.** Read [TODO.md](TODO.md) at the
start of each turn. [AGENTS.md](AGENTS.md) owns operational instructions;
[the request audit](docs/REQUEST_AUDIT.md) preserves the conversation scope and
[the completion log](docs/COMPLETION.md) records shipped behavior and evidence.
Historical implementation notes belong there, rather than becoming new TODOs.

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
the earlier installed build. Palette/S3/account-import links are being integrated.

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
5. The password-transfer protection question remains **unanswered**. Do not assume
   Google-only unlocking or put account passwords in metadata/SQLite. The current
   shared format contains account definitions and selected settings, without secrets.
6. Verify real same-project Google appDataFolder visibility and OAuth across the
   participating clients/platforms. Fixture success is not live interoperability.

The sibling `../shep-clients` is an independent active client worktree; preserve
its changes. Shared initialization/harness support was published through the isolated
`codex/profile-initialization-harness` branch. The shared source is copied from
immutable client `184b98a`; its active uncommitted enrollment work was not used. Consume immutable reviewed revisions;
do not merge unrelated client changes into desktop. Do not share a Cargo target
directory between worktrees with different vendored renderer sources.

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
`artifacts/`; use `login:false` for shell tools. No subagents are authorized.

Performance measurement remains deferred until the final idle-host phase.
Quality/release workflows stay disabled until the self-hosted runners are ready
and Sam requests re-enablement. Documentation CI/Pages publishing is enabled.
