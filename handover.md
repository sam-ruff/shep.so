# Shep handover

This handover records the credit-limited stopping point. The user explicitly resumed development on 2026-09-08; continue from the restart order below. **The full product goal is unfinished.**

## Start here

- **New highest priority:** implement Google OAuth and shared account/settings profiles using [the profile sync handover](docs/agents/PROFILE_SYNC_HANDOVER.md). This supersedes the earlier restart order below. Cover first setup on either desktop or Flutter, new/existing devices and configurable sync. The document is a proposed implementation contract; continuous sync and credential portability remain unfinished. The user explicitly requested this OAuth follow-up at the top of TODO after writing the handover.

- Work in the `shep-clients` worktree on `feat/mobile-web-clients`. All combined Flutter, browser, Rust backend and delegated promo work is on that review branch.
- Read `AGENTS.md`, `TODO.md`, `docs/CLIENT_PARITY.md`, `shared/client-scenarios.json` and the latest entries in `docs/COMPLETION.md`. `docs/REQUEST_AUDIT.md` preserves request traceability.
- The root `shep.so` main worktree has independent, actively edited desktop work. Do not commit it, overwrite it, merge clients into main or replace the personal installation. The `shep-website` worktree still has the agent's original uncommitted files; its website source/assets were already copied and committed on the combined branch. Its older README is superseded; do not recopy it.
- Latest code checkpoint: [`da6f2e8`](https://github.com/sam-ruff/shep.so/commit/da6f2e803fbddf1e485418b925923eeeb08afd1e), covering unread-account counts, following native profile history `e9115f9` and search materialization `e568c84`. The code head is pushed and verified on the remote review branch; final evidence and limitations are recorded in the completion log. The earlier Undo/handover checkpoint is `d4da04c`; final shipping documentation follows each code checkpoint in branch history. Pushes to the review branch are authorized. Use the configured owner identity and Conventional Commits; never skip hooks.

## Product decisions that must survive

One monorepo: Rust + iced desktop at the root, Android/iOS in `flutter/`, a **separate desktop-style browser client** in `web/`, and promo in `website/`. The initial separate `shep.flutter` repository and hosted Flutter layout were superseded. Keep the restrained shadcn-inspired theme and approved Shepherd design, configurable K-9-inspired mail rows/previews/swipes and clickable equivalents.

The browser beta requires server-verified Google login and an explicit owner allowlist, expandable by an administrator. `backend/` provides Rust verification and authenticated IMAP/POP3/SMTP transport; ordinary browsers cannot speak those TCP protocols directly. No persistent server-side mail/password storage is authorized. Browser credentials currently remain in tab memory; secure remembered credentials are unfinished.

VPS installation is authorized, but the actual SSH target, exact verified owner identity and Google OAuth configuration are still missing. Never infer login identity from Git contact details. Site assets are staged, **not deployed**. Android/App Store destinations must honestly reflect publication status.

Quality/release workflows remain `.yml.disabled`; documentation CI is enabled. Re-enable quality/release only when requested and trusted runners are ready. Apple execution, signing, store publication and coordinated distribution remain open.

Keep R75: replace manual Google-token setup with provider OAuth “Sign in with Google”; beta login alone does not connect Gmail/Calendar/Drive. Keep R76: Preferences automatic replies with one message assigned to searchable account groups/all accounts, multiple entries, start/end times, visible timezone, overlap handling and honest per-provider results, preferably scheduled server-side. Linux app-store submissions remain R72.

## Current work and phone installation

The user resumed development after the credit-limited handover. R79 then authorized installing the normal app on the owner’s Android phone. Shep 0.1.0 (1), production-flavor ARM64 release, was built with its native Rust library, development-signed, installed without clearing data and launched successfully. Package/process verification passed; the final screen observation found the phone locked. The APK and build/signature evidence remain in ignored `artifacts/flutter/phone-install/` and `artifacts/logs/phone-*`. Do not preserve pairing codes, addresses or device identifiers in repository records, and do not run fixture automation on that phone. Full parity is still unfinished.

R63 now has two deterministic failed-before regressions for shortcut capture/held presses interrupted by background mail completion. Mounted capture state and retained controls fix the failures, with an additional saved cancellation/current-setting scenario. Final Chromium regression passes 133/133 scenarios, all 123 unit tests and 15 targeted History/Find/Preferences controls pass, and light/dark/History WebP captures are reviewed. All 56 production HTTPS fixture stages, strict documentation and mandatory hooks (381 Rust tests) pass. Shipping is verified in [`1ea12a6`](https://github.com/sam-ruff/shep.so/commit/1ea12a677829dcd71c4246b87cae46327f9c749f); see the completion log. The full run also exposed History waiting for Undo preview preparation after the durable group had completed. Independent preparation tokens and persistent Refresh recovery now pass deterministic held-result and failed-preview controls. Keep the earlier failing evidence below as historical evidence.

## Current profile Drive checkpoint

The optional native `drive` feature in `shared/profile-core` adds verified Google
principal/namespace binding, 50-file metadata pages, checked operation downloads
and one journal-backed upload at a time. It persists a reserved Drive ID before
POST and confirms actual remote bytes after response loss, conflict or restart.
See [the wire contract and integration limits](docs/agents/PROFILE_DRIVE.md).
The shared wire fixture and 34 core tests pass; compatibility checks and shipping
are recorded in the completion log. The initial durability fixture queried the
wrong test table; that failure remains in ignored logs, corrected without changing
the production schema. No client Settings screen uses the transport yet.

Continue with a durable per-principal/application discovery catalog, visited
file/token tracking, profile creation/enrollment and actual Preferences/account
application. Bind calls to the platform's saved Google connection and own pending
work through lifecycle changes. A final page or complete received ancestry is not
an atomic cloud snapshot. Same-project registered-client visibility is still
unverified; a namespace hash cannot detect a different OAuth project's empty app
space. Keep the credential-protection choice open. Main, installed desktop and
personal phone remain untouched; the existing native timing gate still fails.

## Current profile history checkpoint

The native [profile history](docs/agents/PROFILE_HISTORY.md) now implements durable
immutable operations, causal field merge, explicit conflict reviews, account/profile
tombstones and reserved upload identities. One background connection owns each
journal; the Flutter production bridge and shared host/Android FFI scenario exercise
two isolated stores without credentials. Root/native SQLite is updated to 3.53.2;
main's separate mail-cache worker migration is not included. Final checks pass 411 root/shared and 68 mobile Rust tests, 90 Flutter host tests,
Android history integration, 41 Python checks, 32 parity contracts and strict docs.
Full native functional verification passes 118 flows before the final index and
14 relevant flows afterward without compilation. The earlier under-load 12/14
run retains two input failures in R63. The final backend budgets pass (search
35.537 ms); **native navigation and the combined gate still fail** at 154.81–162.33
ms against 150 ms. A previous cached test build also fails; do not claim the
responsiveness issue fixed or weaken the gate. The new ARM64 APK is unsigned;
no personal app was replaced. Detailed logs/shipping remain in the completion log.

The optional Drive provider is now implemented above. Restart with durable
creation/discovery/enrollment, then category controls and actual account/settings
application. Received ancestry is not a complete cloud listing. Keep upload bytes
and reserved IDs, stale-review checks and device-local journal identity through
retries; full database import needs explicit device/ownership rebinding. No profile
UI, Google upload or password sync is delivered by this metadata increment. Live
Google/project setup, Apple and the credential-protection decision remain open.

## Flutter native Google consent checkpoint

Shipped and verified as [`6c4bb65`](https://github.com/sam-ruff/shep.so/commit/6c4bb65fbc02c96dd258ed9942e64e6282bca181). Mandatory hooks pass formatting, Clippy and 398 root/shared Rust tests (two personal diagnostics intentionally ignored).

Flutter Preferences now offers native Google sign-in with explicit Drive and Calendar permissions, separately displayed saved access, cancellation/retry and reviewed local disconnection. SDK tokens remain outside portable preferences; secure metadata records identity, enabled services and pending cleanup. Unknown device writes pause Google work until a successful read reconciles the result. Mail navigation remains available during held consent. See [mobile Google setup](docs/agents/GOOGLE_MOBILE.md).

Verification passes 89 Flutter host tests, the separately configured SDK-boundary fixture, two named Android control scenarios, seven Appium flows, seven offline Flutter Playwright flows, clean analysis, 40 Python tests and 31 parity contracts. The production ARM64 release build contains the Rust bridge and bundled font/license assets, with checked preview markers absent. It is development-signed and has no registered Google client configuration; no live sign-in or new personal-phone installation is claimed. Final test evidence and shipping are recorded in the completion log. Reviewed synthetic captures are under ignored `artifacts/flutter/google-reviewed/`; initial native/browser/test-navigation failures remain recorded.

Continue Calendar/Drive provider use and verified profile identity/discovery/enrollment/merge. Safe seamless switching, automatic SDK session restoration, live registered Google access and Apple execution remain open. Reconnect retains the saved subject; selecting another currently requires explicit local disconnection. Do not silently choose the outstanding password-sync protection policy. Quality/release workflows remain disabled.

## Desktop scoped consent checkpoint

Shipped and verified on the review branch as [`3e1181b`](https://github.com/sam-ruff/shep.so/commit/3e1181ba68692b63104cec4f926f3391ecfab470). Preferences now saves separate Drive and Calendar off/read/edit choices for the next sign-in. The active grant remains usable until new consent commits; denied or changed setup preserves it. Requests, omitted-scope responses, staged retry and activation use the exact selected set, and broader returned grants cannot activate an unselected service. All 118 native functional scenarios, 398 root/shared Rust tests, 46 selected Google tests, 39 Python tests and 31 parity contracts pass. Light/dark/compact screenshots are reviewed; strict documentation and Clippy pass. Shipping is recorded in the completion log.

Continue verified cross-client profile identity, provider integration and durable discovery/enrollment/merge. This desktop increment does not establish live Google access, mobile/Apple execution or continuous sync. Preserve the common profile format and pending credential-protection decision below.

## Shared profile codec checkpoint

[`80979d2`](https://github.com/sam-ruff/shep.so/commit/80979d26db8d44e2f9caaed4f02e535807f9b826) adds the initial [profile metadata codec](docs/agents/PROFILE_FORMAT.md), explicit shared account mappings, a bounded native validation request and common Rust/Dart FFI/WASM fixtures. Root/shared hooks pass 390 tests; mobile Rust 67, actual Dart FFI 14, standalone WASM 23 cases plus malformed-record checks, backend 35, Python 39 and parity 31 pass. Flutter analysis and strict documentation pass. Failed duplicate-extension encoding evidence stays in ignored `artifacts/logs/profile-codec-*`.

No production profile is uploaded or enrolled. Continue scoped authorization/verified identity and durable discovery/enrollment/causal merge from the full handover. Complete remaining settings/categories and credential protection separately; keep the unresolved protection decision. Validate committed upload bytes/identity and preserve optional data when adding persistence. The standalone WASM check runs in Node; actual browser Settings, Android/Apple controls and live Google remain separate work.

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

1. **Highest priority:** implement Google OAuth and shared account/settings profiles using [the interoperability handover](docs/agents/PROFILE_SYNC_HANDOVER.md), as requested in desktop-main:R92/R75/R02/R49. Cover first setup on desktop or Flutter, new/existing devices, configurable sync and scoped authorization. Preserve the pending credential-protection choice and verify complete database transfer independently. The handover is the full contract. The initial shared metadata codec/account mappings and native/Dart/WASM fixtures are documented in [the format subset](docs/agents/PROFILE_FORMAT.md); they do not implement enrollment or causal sync. Scoped desktop/Flutter consent and native causal history are now implemented prerequisites. The optional Drive transport now verifies identity and immutable files. Next connect it to platform grants, durable discovery/enrollment and actual account/preferences application, preserving the original operation bytes and all remaining settings/credential requirements.
2. Finish remaining browser Undo lifecycle and abandoned review/staging cleanup. Saved-group startup notices now have compact light/dark controls, older-group targeting, inspection retry, acknowledged-cache recovery and live-owner/tab-loss evidence. All 137 Chromium scenarios, 127 units and 56 production HTTPS stages pass; shipping is verified in [`df4f29c`](https://github.com/sam-ruff/shep.so/commit/df4f29c2e12d21fc71353920696dd4958cde363a). Preserve `artifacts/browser-recovery-visuals/` and the failed stale-observation unit baseline.
3. Connect Flutter’s native SQLite capture and Dart controller to exact durable group execution, then Select/Done/Clear/all, review, Undo, History and recovery controls. Current loaded-row actions are **not** full-mailbox parity.
4. Continue account/calendar/backup, composition, cache/large-message, remote-image, keymap, lifecycle and platform gaps in TODO. Port later committed desktop work deliberately, preserving both request histories and `desktop-main:` identifier collisions.
5. Complete Apple/live-provider tests, idle-host performance, deployment and distribution when the required environments/configuration exist. Push verified review-branch checkpoints promptly.

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
