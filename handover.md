# Shep handover — 2026-09-08

The user stopped feature work to conserve credits and requested this handover, a cleaned TODO and a push. **The full application goal is not complete.** Resume from [TODO.md](TODO.md), preserving the original requirements in [docs/REQUEST_AUDIT.md](docs/REQUEST_AUDIT.md). Read [AGENTS.md](AGENTS.md) for operational rules and [docs/COMPLETION.md](docs/COMPLETION.md) for evidence; do not repeat historical work based only on the latest chat message.

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

The sibling repository is a dirty `feat/mobile-web-clients` worktree with active
client work. Preserve those edits. Its instructions require client work on that
review branch, with parity/scenario tracking; only read-only discovery was done
here. Re-read the relevant instructions before editing. `flutter/rust` has its
own cache and credential-slot lifecycle, so do not assume desktop SQLite is its
profile interchange format. Google appDataFolder and native OAuth primary docs
have been opened; a shared cross-platform OAuth project/namespace needs explicit
verification and documentation. No Flutter sync document was written yet.

R86/R90/R91 remain active after that priority. Read-only inspection confirms bulk
claims a durable item before waiting for provider capacity (`engine/bulk.rs`),
and several window-close branches still require another click after other saves.
Use channel-owned control/cancellation for the remaining work; preserve actual
in-flight receipts. Storage still owns its connection and in-memory lease set
through `Arc<Mutex<_>>` (`store.rs`/`store/bulk.rs`); Google/lifecycle coordination
also remains in the R91 audit. Do not confuse application state coordination
with required SQLite or independent-process file locking.

## Shipped baseline

- Repository: public `sam-ruff/shep.so`, branch `main`. Direct pushes are authorized.
- Last installed source: `3567de2` (native folder Move/Delete reviews); its audit is `77d60cc`. The installed Linux executable remains that baseline; this handover does not install a new release or replace an open personal window.
- Installed binary SHA-256: `06c0cccb3d3cc6703b143f8e7fa019c1be7032533ae6d4e776a81cc6f91ef34a`.
- The latest search request **is delivered** in `a81d767`: search spans all cached folders in the selected accounts, displays result folders, retains read/flag/attachment filters, and returns to the browsing scope when cleared. R84 is complete, not an open TODO.
- Folder checkpoint evidence: 511 Rust test executions plus two drawing-adapter tests, 48 Python tests, hooks/Clippy, Windows compilation, strict docs and release/installer checks. Its full native run was 180/181; the corrected mail-menu focus assumption and final folder/search fixes passed a subsequent 18/18 targeted run. Do not describe that as a clean full-suite run on the final binary.

## Current source checkpoint: R35 foundation, not inline composition

The composer **still opens a modal**. Do not claim the inline view or several simultaneously open reply sessions are implemented.

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
