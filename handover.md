# Shep handover — 2026-09-09

The user stopped feature work to conserve credits and requested this handover, a cleaned TODO and a push. **The full application goal is not complete.** Resume from [TODO.md](TODO.md), preserving the original requirements in [docs/REQUEST_AUDIT.md](docs/REQUEST_AUDIT.md). Read [AGENTS.md](AGENTS.md) for operational rules and [docs/COMPLETION.md](docs/COMPLETION.md) for evidence; do not repeat historical work based only on the latest chat message.

## 9 September continuation: native initial-profile controls

Preferences now has **Accounts → Profiles and sync** for explicit first-device
review/create, saved categories, stop/resume and enable/disable. The owning
32-command coordinator keeps local choices available during network work and
retains admitted upload receipts through shutdown. Touched-field saves preserve
newer choices and Google-disconnect state. Production per-workspace journal paths
and all existing aliases are protected from database export.

Five new native scenarios pass, including compact dark rapid choices,
failure/retry/opt-out, first creation/reopen, held-read close and closing during
upload followed by Resume. Actual WebP captures were reviewed. Relevant Rust
ordering/protocol/export tests pass; final full checks and shipping are recorded
in the newest completion entry. No personal installation or live Google test was
performed. The screen labels completion as an initial copy, because continuous
updates and joining existing profiles remain unfinished.

**Next finish existing-device discovery/application, then continuous sync.** Keep
OAuth implementation first in TODO with the Flutter handover reference, as the
user requested. Review newer committed shared-client discovery/history support
without touching that worktree's active files. Desktop new profiles currently use
namespace `so.shep`; all participating clients must agree before live parity is
claimed. Preserve saved bindings and the pinned dependency until a reviewed
migration/update. Account reconnection/suppression, portable preferences beyond
the seven mapped fields, conflict/removal reviews and incremental local/remote
change capture remain open. Password-transfer protection still has no decision.

## 9 September continuation: persisted enrollment

Source `6e2880b` is pushed. The backend saves profile/category choices and exact initial seeds, performs
reviewed first-device publication through the shared history and Drive journals,
and applies seven supported settings with local revision checks. Restart, lost
responses and disabling/disconnecting during an upload have isolated regressions.
Database import archives/removes the source-device enrollment and seed. Account
metadata conversion produces review candidates; it does not activate credentials.
Mandatory hooks passed 617 Rust, two renderer and 34 shared tests. All 53 Python
tests, seven selected native import/Google-disconnect flows, Windows GNU
cross-compilation and strict docs passed; see the newest completion entry.

**Next connect this to the engine and native Preferences.** Use an owning bounded
channel coordinator, preserve Google/profile/category lifecycle and keep uploads
owned until receipts are durable. Choose/protect production journal paths first.
Complete existing-device discovery/application, account reconnection/suppression,
local changes/conflict reviews and incremental polling. Do not expose an initial
copy as working continuous sync or assume later edits have been published.
The OAuth implementation stays at the top of TODO with the Flutter handover linked.
No native UI or personal installation changed in this checkpoint. Keep the sibling
client work intact; credential-transfer protection and live Google parity remain open.

## 9 September continuation: causal profile bridge

Source `ace653b` is pushed and uses the published shared history worker for verified
Drive pull/publish. It retains exact reservations across both journals, refuses
stale/foreign proofs and ambiguous duplicate IDs, and preserves conflicts/removal
markers. Two isolated devices exercise the real HTTP/SQLite path. The desktop
file convention now matches the client branch's committed metadata fixture.
Hooks passed 605 Rust, two renderer and 34 shared tests. Windows cross-compilation,
53 Python tests and strict docs also passed; see the completion log.

Cargo pins `9289f53`; `python3 scripts/test_profile_core.py` runs all 34 shared
codec/history/Drive tests from an isolated copy with a committed test lock.
Ordinary hooks use locked mode. After reviewing a pin/dependency change, use
`--update-lock` to refresh that isolated test lock. Python 3.11+ is required.

Next implement durable enrollment and real account/settings application, then
native controls, profile/category/lifecycle fencing and incremental polling.
Current full-history pulls are not the final continuous scheduler. Protect the
chosen production journal paths during database transfer. Actual Google
cross-client access and the credential-protection choice remain unresolved.
OAuth stays first in TODO, using the linked Flutter handover. Keep the sibling
client thread's work intact; no personal installation was changed.

## 9 September continuation: shared profile transport

Source `bb87ac2` is pushed with verified Drive profile discovery and immutable
uploads using a published shared-codec revision. A separate bounded channel
worker keeps page tokens, revisions and reserved bytes durable across restart.
Read [the transport contract](docs/agents/profile-drive.md) and the newest
completion entry for verification/shipping. Hooks passed 596 Rust, two adapter
and six shared-codec tests; documentation CI passed. No production installation
changed. The client thread is independently implementing Drive transport: reconcile
file metadata/query conventions before claiming cross-client interoperability.

OAuth/profile sync remains first in TODO, referencing the
[Flutter handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md).
Next connect the published shared causal-history worker to enrollment and account/
settings application, then polling and native controls. Keep current sibling
client work untouched. Transport tests do not prove complete sync or live Google
interoperability. Protect the eventual production journal paths during database
transfer. The credential-transfer protection question remains unanswered.

## 9 September continuation: full database import and local profiles

R83 now has native **Database transfer → Import database** with a private schema/
integrity-checked copy, account/count review, profile naming and cancellation.
Pending sends and provider changes require acknowledgment and remain review work
instead of replaying on another device. **Accounts → Profiles** lists/renames
profiles and selects one for the next launch, keeping the original workspace.
The engine's current Store and credentials never change under in-flight work.

The catalog, database-transfer controller and OS-credential worker use bounded
channels. Imported profiles receive a fresh local credential namespace and require
reconnection. Export protects every profile's cache/journals and the catalog.
Interrupted registration recovers the same published file; cancellation before
publication removes the private copy. Reserved/colliding credential IDs are
rejected, including Windows case aliases. Discovered CalDAV IDs now survive
encrypted backup and password restore.

Read [the profile reference](docs/agents/profiles.md) and the newest completion
entry for tests and shipping status. Source `93d4289` is pushed to main: mandatory
hooks passed 580 Rust + 2 adapter tests; 51 Python and 16 selected native flows,
Windows cross-compilation and strict docs passed. This is a source checkpoint; the personal
production installation remains unchanged. Database files are unencrypted and
exclude OS secrets. Profile selection requires reopening; there is no hot switch.

**OAuth account/profile sync stays the top TODO priority**, as the user requested,
with the [Flutter handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md)
as its implementation reference. Use the client branch's shared profile-core
operation format and fixtures; a desktop local UUID is not its shared profile ID.
The sibling worktree has independent active client work: do not overwrite it.
First/new/existing-device enrollment, continuous merge and live interoperability
remain open. The separate credential-protection question has no recorded answer.

The export-only and earlier foundation notes below are historical; their open
import statements are superseded by this continuation.

## 8 September continuation: database export

R83 now has a complete SQLite export in **Preferences → Backups → Database transfer**. `transfer.rs` pins a consistent read transaction on a separate connection and copies bounded page batches into a private temporary file before atomic publication. The capacity-one database command worker is independent of provider/read/persistence queues. The UI saves its settings and owned drafts first, reports progress, supports cancellation and waits for cleanup on close. See the latest completion entry for verification and shipping.

Source `3927053` is pushed to main. Verification passed 552 Rust + 2 adapter tests, 50 Python tests, seven selected native flows, Clippy, Windows cross-compilation and strict docs. The production installation remains unchanged; import and profile sync must not be inferred from this checkpoint.

**Import is still open.** Preserve the original workspace, validate schema/integrity before activation, isolate imported credential identities and prevent pending sends/moves/folder jobs from replaying on another device. Raw SQLite export deliberately retains that state; it is not the existing encrypted backup format. Account passwords and Google tokens remain in the OS keychain.

OAuth profile implementation stays at the top of TODO, referencing the [Flutter handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md). Keep the outstanding credential-protection choice open. No live Google profile protocol or full database import is claimed by this export checkpoint.

## 8 September continuation: sync/close and channels

Development resumed. The user requested temporary native tray behavior while
pending saves finish, even with ordinary close-to-tray off, automatic exit after
saving, a notification, and investigation of slow sync/close. They then explicitly
required channels rather than shared locks for state coordination. R86, R90 and
R91 in TODO preserve that scope; the temporary tray is still unimplemented.

The new source checkpoint is `34cfc71` (`fix(sync): prioritize account writes
through channels`). `engine/account_work.rs` owns account/calendar scheduling
through bounded requests and completion channels. Read-only sync yields to
writes; an owned task drains already-started cache writes before acknowledging
completion, including on timeout or cancelled refresh. See the completion log
for verification and shipping status (528 Rust/adapter, 49 Python and 185/185
native functional tests passed). The installed production binary is still
the baseline below. Remaining investigation includes the bulk worker claiming a
step before waiting for provider capacity, and close paths that ask for another
close click after credentials, send, attachment or calendar setup finishes.

## 8 September continuation: inline composition

Source commit `e16590c` adds inline new-message/reply/forward editors and independent
parked sessions. Returning to mail restores its reply; text, recipients and copied
attachments survive Preferences and owned-process restart. Reply quotes stay
outside the editor and are included in MIME only when selected. The editor can
collapse, supports normal text selection/Tab, and preserves the original below,
including Find and conversation paging.

Close observes all pending draft revisions. Removal review waits for related
saves/file imports; discard cannot race an import. Late file/save/send/detail
results retain unrelated forms and newer edits. Sending still releases the editor
after its durable outgoing acknowledgment; delayed preparation allows navigation.

The original 188-flow native run passed, then all six inline scenarios passed
against the final executable. Hooks passed 541 Rust/adapter executions; 49 Python
tests and strict docs also passed. The final **191/191** native run passed and `e16590c` was pushed to main;
see `docs/COMPLETION.md`. No production installation was performed.
The old ignored patch backup is historical and has already been integrated; do
not restore it over the current source. The foundation notes below describe the
older installed build.

**Next priority from the latest user message: R83/R02/R49/R92.** Implement full
database import/export in Settings and configurable Google OAuth/Drive account
and profile sharing. Cover first setup originating on either Rust or Flutter,
new-device discovery/enrollment and ongoing sync on existing devices. Write the
interoperability contract in `../shep-clients` for its Flutter implementation.
An asynchronous question about Google-only unlocking versus a separate sync
passphrase is pending; no answer has been recorded. Database work is independent.

The [Flutter profile sync handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md)
records the proposed interchange and first/new/existing-device flows. The user's
follow-up explicitly keeps OAuth implementation first in the client TODO and
links to that document. Document delivery does not establish working profile sync.

The sibling repository is the independent `feat/mobile-web-clients` worktree.
Preserve its client work and use that review branch for handover changes, with
parity/scenario tracking. `flutter/rust` has its own cache and credential-slot
lifecycle, so desktop SQLite is not its profile interchange format. Google
appDataFolder and native/cross-client OAuth primary docs inform the handover;
actual cross-platform project/namespace access remains a required live check.

R86/R90/R91 remain active after that priority. Read-only inspection confirms bulk
claims a durable item before waiting for provider capacity (`engine/bulk.rs`),
and several window-close branches still require another click after other saves.
Use channel-owned control/cancellation for the remaining work; preserve actual
in-flight receipts. `store/worker.rs` now owns the mail-cache connection and local
leases behind 32 bounded commands; queued writes drain without their observers.
Source checkpoint `db8c82a` is pushed to main and exact remote equality was verified.
Its restart test exposed a SQLite 3.51.1 Unix WAL open/close deadlock, confirmed in
a child-process debugger trace. Updating rusqlite to 0.40.2 / bundled SQLite 3.53.2
fixes that reproduction. The five worker tests, 546 Rust/adapter executions,
49 Python tests, Windows cross-compilation and 15 targeted native storage/lifecycle
flows pass; see the completion log for shipping evidence. Native screenshots
were reviewed. No performance measurements or production installation were done.
Google/lifecycle and backup-journal ownership remain in R91. Do not confuse
application state coordination with required SQLite or independent-process file
locking. Full database export/import is still to implement; use an independent
online snapshot connection, bounded copying and cancellation, followed by safe
import/rebinding rather than replaying another device's pending provider work.

## Shipped baseline

- Repository: public `sam-ruff/shep.so`, branch `main`. Direct pushes are authorized.
- Last installed source: `3567de2` (native folder Move/Delete reviews); its audit is `77d60cc`. The installed Linux executable remains that baseline; this handover does not install a new release or replace an open personal window.
- Installed binary SHA-256: `06c0cccb3d3cc6703b143f8e7fa019c1be7032533ae6d4e776a81cc6f91ef34a`.
- The latest search request **is delivered** in `a81d767`: search spans all cached folders in the selected accounts, displays result folders, retains read/flag/attachment filters, and returns to the browsing scope when cleared. R84 is complete, not an open TODO.
- Folder checkpoint evidence: 511 Rust test executions plus two drawing-adapter tests, 48 Python tests, hooks/Clippy, Windows compilation, strict docs and release/installer checks. Its full native run was 180/181; the corrected mail-menu focus assumption and final folder/search fixes passed a subsequent 18/18 targeted run. Do not describe that as a clean full-suite run on the final binary.

## Historical R35 foundation — superseded by e16590c above

The following notes describe the earlier foundation checkpoint only. Its modal limitation was subsequently removed by `e16590c`, described above.

Changes included in this handover:

- `src/ui/composing.rs`: `Composer.current: Session` owns draft metadata, native editor, recipient visibility, dirty timestamp and pending save revision. Recipients/account/subject are independent of generic dialog fields, via `ComposeField` and `compose_field`.
- Opening another form no longer clears a pending draft autosave. Autosave works while that form is visible; only one save per current session is in flight, with newer edits coalesced behind it.
- Window close observes an in-flight save and persists the newest revision before exiting. Failure cancels that close attempt while retaining the text. A late old failure cannot cancel a newer save/close request.
- A delayed attachment picker uses the latest owned draft if its identity still matches, even with another dialog visible. Matching durable-send receipts retire that session without clearing an unrelated form; older receipts preserve newer edits.
- `src/ui/mod.rs`, `views.rs`, `outgoing.rs` and reader/conversation tests use the owned session. Native observation exposes `compose_fields`; `fields` still describes the currently visible form, preserving existing modal test contracts.
- Five new controller regressions cover form isolation/autosave, delayed attachments, hidden-session send retirement, editing guards and pending-save close/failure ordering. Existing composer tests remain.
- `scripts/e2e.py` adds `test_compose_session_preserves_fields_through_preferences_and_graceful_restart`, using actual native controls and an isolated persistent fixture.

No new `ReplyContext` model field or delivery-body transformation remains: those exploratory edits were removed rather than shipping unused schema. No session pool, automatic thread-to-draft association, inline layout or inline keyboard-focus handling has been implemented yet.

## Verification for this checkpoint

- Targeted composer Rust suite: 14/14 passed (`artifacts/logs/composer-state-tests.log`).
- Python harness/installer tests passed 48/48 (`artifacts/logs/handover-python.log`).
- Strict docs build passed. Native save/reopen, recipients/files and cross-folder search passed; the new Preferences/restart scenario passed after correcting its initial unsupported attempt to navigate while the modal was open. Dark attachment and restart images were reviewed. Full native/release verification is deferred.
- The normal commit hook enforces fmt, Clippy and full Rust/adapter tests. The checkpoint is the commit named `fix(drafts): isolate composer state and record handover`; see the completion entry and ignored `artifacts/logs/handover-commit.log`.
- No performance measurements, live provider operations, personal-mail automation, new optimized release, installation or platform runtime verification is claimed for this checkpoint.

The user also requested **R86 native close-to-tray as a TODO only**. It is recorded without implementation; do not include it in this handover’s claimed behavior.

## Resume R35 in this order

1. Preserve the current state-isolation and close/save regressions. Add a pool of draft sessions, moving native editor state between active/parked sessions and retaining unsaved/failed work. Track all pending sessions at shutdown; bound and coalesce queued work without dropping edits.
2. Move new messages/replies into the preview pane. Show the original conversation below replies, support collapse/resume and bin discard, and retain the matching reply on navigation. Keep draft content, recipients, Cc/Bcc, files and identity intact when opening Preferences or another draft.
3. Decide persistent reply association using account and stable Message-ID, with cache-ID hints. A folder move can invalidate an IMAP/cache identity. If separating the editor's new text from quoted history, add explicit, tested persistence and MIME assembly; retain forward HTML/CID behavior and the ability to omit a quote.
4. Extend `src/ui/native_input.rs::Focus` to actual inline To/Cc/Bcc/Subject/editor focus. Preserve text Ctrl+A, prevent Ctrl+D/letter shortcuts from mutating mail while editing, and define Tab/Escape behavior. Keep mouse usability and remapping.
5. Integrate `views.rs::reader`, `conversations.rs::conversation_reader`, selection/navigation, draft sidebar, attachment completions and `SubmissionQueued` handling. Newer draft/dialog intent must survive old background results. Preserve conversation scroll/background caching fixes.
6. Adapt existing native compose/reply/forward/attachment/discard/outbox scenarios to real inline controls; never fake `dialog == Compose` observations to keep tests green. Add two-reply switching, pending-save/send/failure, restart, compact dark/light, typing-edge and shortcut tests. Review actual WebP images (R77).
7. Run applicable verification, then build/verify/install an optimized release and push. Keep R35 open until the requested inline behavior and relevant tests are complete.

Useful files: `src/ui/composing.rs`, `composing_tests.rs`, `mod.rs`, `views.rs`, `conversations.rs`, `native_input.rs`, `sidebar.rs`, `outgoing.rs`; `src/store/drafts.rs`, `src/engine/outgoing.rs`, `src/compose.rs`, `tests/composing.rs`, and `scripts/e2e.py`.

## Operating notes

- Use `login:false` for shell tools in this environment. Do not spawn agents without authorization.
- Do not touch real mail with automation. The MCP skill is `.agents/skills/shep-e2e/SKILL.md`; every AI-driven scenario needs a saved automated equivalent. Use `--functional-only` for the full non-performance suite. For selected functional tests, use `python3 scripts/e2e.py -k NAME` without that flag: its current parser ignores `-k` when `--functional-only` is supplied.
- Logs/screenshots/databases belong under ignored `artifacts/`, never the repository root. Private `.claude/` local settings remain ignored and unmodified.
- Git hooks enforce fmt, all-target/all-feature Clippy and Rust/adapter tests; do not bypass them. Git identity: `Sam R <sam@technesci.co.uk>`.
- Quality/release workflows remain `.yml.disabled`; re-enable only when requested and self-hosted runners are ready. Docs publishing is enabled. Before pushing docs, run `.venv-docs/bin/zensical build --clean --strict`.
- Database encryption, large-mail ceilings, multiple backup destinations, continuous Drive account/settings sync, portable database export, palette editing, cross-platform installers and remaining platform/live-provider verification are still open. TODO contains the exact remaining scope.
- Credential-transfer protection has no recorded decision. Do not assume that Google login alone authorizes plaintext password sync or claim SQLite includes OS-keychain secrets.
