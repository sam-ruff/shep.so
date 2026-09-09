# Shep handover — 2026-09-09

**The full application goal remains unfinished.** Read [TODO.md](TODO.md) at the
start of each turn. [AGENTS.md](AGENTS.md) owns operational instructions;
[the request audit](docs/REQUEST_AUDIT.md) preserves the conversation scope and
[the completion log](docs/COMPLETION.md) records shipped behavior and evidence.
Historical implementation notes belong there, rather than becoming new TODOs.

## Current source checkpoint

The current continuation adds `profile_replication_v1`: enrollment now saves the
last common field values, exact raw extensions and local/shared account mapping.
Local capture/admission APIs preserve pending UUIDs and per-field bases across
restart, category pauses and newer native edits. A sealed history receipt verifies
the field is still current before acknowledgment. These APIs are tested preparation
for the continuous loop; the loop and remote application are not connected yet.
See the newest completion entry for this continuation's verification and shipping.

`071c6b0` is pushed to main. Preferences can discover named shared profiles,
review one and import its account definitions and supported preferences. Existing
accounts/mail stay intact. New definitions receive fresh local IDs and require
**Reconnect** before receiving or sending mail. A saved acceptance ID makes retry
after a lost acknowledgment safe; stale reviews and later local choices cannot
partially apply. The screen explicitly identifies this as an initial import.

Desktop now pins the shared Drive/catalog/history crate at `33d222d7`. Discovery
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

1. Review/adopt the newly published shared initialization barrier from client
   `184b98a` (audit `751b67b`), documented in
   [Flutter publication](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_PUBLICATION.md).
   Desktop still pins `33d222d7` and refuses the new required `initialization-v1`
   records. Migrate first-device creation, enrollment checks and isolated fixtures
   together; incomplete publication cannot look like a finished shared profile.
2. Add automatic post-login discovery/enrollment prompts, then ongoing local and
   remote profile updates. Support first setup from either desktop or Flutter,
   new devices and already-populated workspaces, with saved enable/category choices.
3. Connect account linking/suppression, conflict/removal reviews, offline recovery,
   incremental enrolled-history pulls and the remaining portable preferences.
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
its changes. Shared catalog harness support was published through an isolated
`codex/profile-catalog-harness` branch. Consume immutable reviewed revisions;
do not merge unrelated client changes into desktop. Do not share a Cargo target
directory between worktrees with different vendored renderer sources.

After the profile priority, continue every remaining TODO. In particular, native
tray/temporary saving tray (R86), slow close dependencies (R90), channel ownership
(R91), encrypted cache, large mail, multiple backup destinations, desktop/platform
integration and final usability/performance work remain open. Bulk still needs
review of provider-capacity waits after claiming a step; some other close
completion paths need automatic continuation. Preserve durable in-flight receipts.

## Verification and installation

The source commit's mandatory hooks passed **631 Rust + two renderer + 50 shared
tests (683 executions)**; three explicitly authorized-only personal diagnostics
remain ignored. **54 Python tests and 22 selected native scenarios passed**,
including nine profile, eight database-transfer, two Google-disconnect and three
account setup/removal flows. Light/dark/compact/error WebPs were reviewed. Windows
GNU all-target/all-feature cross-compilation and strict Zensical passed. See the
completion log for exact artifacts and documentation CI shipping evidence.

Native test executable SHA-256:
`e7d39ae27f87967be4612310288b391cc5d80c92a1eb1c287ef75756004a61fb`.
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
