# Shep handover

This handover records the credit-limited stopping point. The user explicitly resumed development on 2026-09-08; continue from the restart order below. **The full product goal is unfinished.**

## Start here

- Work in the `shep-clients` worktree on `feat/mobile-web-clients`. All combined Flutter, browser, Rust backend and delegated promo work is on that review branch.
- Read `AGENTS.md`, `TODO.md`, `docs/CLIENT_PARITY.md`, `shared/client-scenarios.json` and the latest entries in `docs/COMPLETION.md`. `docs/REQUEST_AUDIT.md` preserves request traceability.
- The root `shep.so` main worktree has independent, actively edited desktop work. Do not commit it, overwrite it, merge clients into main or replace the personal installation. The `shep-website` worktree still has the agent's original uncommitted files; its website source/assets were already copied and committed on the combined branch. Its older README is superseded; do not recopy it.
- Latest checkpoint: [`1ea12a6`](https://github.com/sam-ruff/shep.so/commit/1ea12a677829dcd71c4246b87cae46327f9c749f), pushed and verified on the remote review branch. The earlier Undo/handover checkpoint is `d4da04c`; final shipping documentation follows each code checkpoint in branch history. Pushes to the review branch are authorized. Use the configured owner identity and Conventional Commits; never skip hooks.

## Product decisions that must survive

One monorepo: Rust + iced desktop at the root, Android/iOS in `flutter/`, a **separate desktop-style browser client** in `web/`, and promo in `website/`. The initial separate `shep.flutter` repository and hosted Flutter layout were superseded. Keep the restrained shadcn-inspired theme and approved Shepherd design, configurable K-9-inspired mail rows/previews/swipes and clickable equivalents.

The browser beta requires server-verified Google login and an explicit owner allowlist, expandable by an administrator. `backend/` provides Rust verification and authenticated IMAP/POP3/SMTP transport; ordinary browsers cannot speak those TCP protocols directly. No persistent server-side mail/password storage is authorized. Browser credentials currently remain in tab memory; secure remembered credentials are unfinished.

VPS installation is authorized, but the actual SSH target, exact verified owner identity and Google OAuth configuration are still missing. Never infer login identity from Git contact details. Site assets are staged, **not deployed**. Android/App Store destinations must honestly reflect publication status.

Quality/release workflows remain `.yml.disabled`; documentation CI is enabled. Re-enable quality/release only when requested and trusted runners are ready. Apple execution, signing, store publication and coordinated distribution remain open.

Keep R75: replace manual Google-token setup with provider OAuth “Sign in with Google”; beta login alone does not connect Gmail/Calendar/Drive. Keep R76: Preferences automatic replies with one message assigned to searchable account groups/all accounts, multiple entries, start/end times, visible timezone, overlap handling and honest per-provider results, preferably scheduled server-side. Linux app-store submissions remain R72.

## Current work and phone installation

The user resumed development after the credit-limited handover. R79 then authorized installing the normal app on the owner’s Android phone. Shep 0.1.0 (1), production-flavor ARM64 release, was built with its native Rust library, development-signed, installed without clearing data and launched successfully. Package/process verification passed; the final screen observation found the phone locked. The APK and build/signature evidence remain in ignored `artifacts/flutter/phone-install/` and `artifacts/logs/phone-*`. Do not preserve pairing codes, addresses or device identifiers in repository records, and do not run fixture automation on that phone. Full parity is still unfinished.

R63 now has two deterministic failed-before regressions for shortcut capture/held presses interrupted by background mail completion. Mounted capture state and retained controls fix the failures, with an additional saved cancellation/current-setting scenario. Final Chromium regression passes 133/133 scenarios, all 123 unit tests and 15 targeted History/Find/Preferences controls pass, and light/dark/History WebP captures are reviewed. All 56 production HTTPS fixture stages, strict documentation and mandatory hooks (381 Rust tests) pass. Shipping is verified in [`1ea12a6`](https://github.com/sam-ruff/shep.so/commit/1ea12a677829dcd71c4246b87cae46327f9c749f); see the completion log. The full run also exposed History waiting for Undo preview preparation after the durable group had completed. Independent preparation tokens and persistent Refresh recovery now pass deterministic held-result and failed-preview controls. Keep the earlier failing evidence below as historical evidence.

## Previous shipped checkpoint

Browser group Undo prepares a worker counterfactual and paints restored rows/counts before saving the decision or finishing another query. Rejection rolls back while preserving newer reader flags/body and leaves a workspace recovery error after History closes. Restored metadata retains its physical baseline for immediate follow-up actions.

`web/src/bulk_projection.ts` computes previews in derived SQLite only. The journal/source cannot be mutated by a preview. `mailbox_store.ts` releases the mail snapshot before derived query work and observes receipts after source copying; per-field applied revisions reconcile journal/cache ordering. An in-flight successive move uses its cached dispatch folder until its receipt supplies the inverse identity. `model.ts` coordinates the bounded preview, current page, rollback and scope/cache/account invalidation. `bulk_ui.ts` keeps History progress independent of preview preparation.

Preserve existing mail schema 12/journal schema 6 lineage, canonical-alias proofs, acknowledged physical receipts, cache-only repair and conservative uncertain-write recovery. Never dispatch with a projected or obsolete UID. Query/selection workers retain bounded metadata pages and separate body reads; do not replace exact frozen membership with loaded-row loops.

## Previous checkpoint verification and retained failures

- 123 browser unit tests pass; all 28 targeted bulk control/executor scenarios pass.
- Full Chromium run: **128/129 pass**. An existing Find-remapping scenario retained Control+f after Control+g/reload. Its unchanged test file then passed **9/9 across three repetitions**. This is not a clean full-suite pass; shortcut capture across asynchronous redraws remains R63.
- 39 Python tests, 30 parity contracts, TypeScript, changed-file formatting and strict pinned documentation checks pass. Production build passes. Final production Rust HTTPS passes all 56 fixture stages. Mandatory hooks pass formatting, Clippy and 381 root/shared Rust tests, with two personal-account diagnostics intentionally ignored.
- Reviewed synthetic light/dark/compact WebP captures are under ignored `artifacts/browser-undo-visuals/`. Logs: `artifacts/logs/browser-undo-push-*`. Failed evidence: `artifacts/browser-undo-failures/`, especially `full-first/` and `push-targeted/`. Do not discard or relabel those failures.
- Earlier checkpoint evidence includes actual Android/Appium/Flutter-browser controls and 56 production Rust HTTPS fixture stages. This browser-only increment does not rerun unchanged Android/desktop UI suites or establish Apple/live-provider/performance parity.

## Restart order

1. Finish browser Undo lifecycle coverage: queued approval, changed scope/page, partial failures, overlapping groups, startup notifications and abandoned review/staging cleanup. Preserve the fixed shortcut/History controls and their failed-before traces in `artifacts/browser-capture-failures/`.
2. Connect Flutter’s existing native SQLite capture and Dart controller to exact durable group execution, then Select/Done/Clear/all, review, Undo, History and recovery controls. Its current loaded-row actions are **not** full-mailbox parity.
3. Continue the account/calendar/Google/backup, composition, cache/large-message, remote-image, keymap, lifecycle and platform gaps in TODO. Port later committed desktop work deliberately, preserving both request histories and `desktop-main:` identifier collisions.
4. Complete Apple/live-provider tests, idle-host performance, deployment and distribution when the required environments/configuration exist. Push verified review-branch checkpoints promptly.

## Verification commands

Run from the client worktree; save output in ignored `artifacts/logs/`.

```sh
cd web
./node_modules/.bin/tsc --noEmit
./node_modules/.bin/vitest run
./node_modules/.bin/playwright test --workers=1
npm run build
cd ..
CARGO_TARGET_DIR=artifacts/backend-target cargo test --manifest-path backend/Cargo.toml real_browser_beta_gate -- --ignored
python3 -m unittest discover -s tests -p 'test_*.py'
python3 scripts/clients/check_parity.py --base HEAD
artifacts/docs-venv/bin/zensical build --clean --strict
```

Wait for Playwright/Vite to actually terminate before editing app source or regenerating WASM. `npm test`/build hooks regenerate shared WASM; direct Vitest avoids regeneration. Git hooks run root formatting, Clippy and Rust tests; use `CARGO_TARGET_DIR=artifacts/root-target` for the existing build cache. See `docs/CLIENT_TESTING.md` for Android/Appium/Apple setup. Keep personal data and secrets out of fixtures, screenshots, logs and commits.
