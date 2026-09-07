# Shep handover — 2026-09-07

The user stopped feature work to conserve credits and requested this handover, a cleaned TODO and a push. **The full application goal is not complete.** Resume from [TODO.md](TODO.md), preserving the original requirements in [docs/REQUEST_AUDIT.md](docs/REQUEST_AUDIT.md). Read [AGENTS.md](AGENTS.md) for operational rules and [docs/COMPLETION.md](docs/COMPLETION.md) for evidence; do not repeat historical work based only on the latest chat message.

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
