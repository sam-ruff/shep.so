# Shep handover

Development resumed after the credit-limited checkpoint. **The full product goal
is unfinished.** `TODO.md` remains the authoritative active request list; older
implementation and shipping evidence is in [the completion log](docs/COMPLETION.md)
and [request audit](docs/REQUEST_AUDIT.md).

## Working constraints

Work in `shep-clients` on `feat/mobile-web-clients`. Push verified checkpoints
promptly with the configured owner identity and Conventional Commits; never skip
hooks. The root `shep.so` main worktree has independent active edits: do not commit
it, merge clients into main or replace the installed desktop. Delegated promo
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

Desktop ongoing synchronization and paged conflict reviews are already pushed in
[`28c2884`](https://github.com/sam-ruff/shep.so/commit/28c288448842b7d09543fc28ef1f2f14bf36f142).
Preserve their shared-owner, active-grant, full-rescan, original-record, durable
receipt and newer-intent checks. Their 23 surrounding native flows, three final
reruns, 54 profile checks and 498 mandatory hook tests are recorded in completion.
Do not replace them with Flutter-only evidence.

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
   app-data space must not imply empty setup. The password-protection choice is
   unanswered; legacy backups and full database migration remain separate work.
2. Preserve shipped browser group Undo/recovery, alias/receipt and worker ownership
   regressions. Current browser recovery checkpoint `df4f29c` passes 137 Chromium,
   127 unit and 56 production HTTPS fixture stages. Continue abandoned review and
   staging cleanup; exact evidence/failed baselines remain in the completion log.
3. Connect Flutter's SQLite selection capture to durable group execution, then
   Select/Done/Clear/all, review, Undo, History and recovery controls. Loaded-row
   actions are not full-mailbox parity.
4. Continue account/calendar/backup, composition, cache/large-message, remote-image,
   keymap, lifecycle, performance and platform gaps in TODO. Port later committed
   desktop changes deliberately; preserve `desktop-main:` request-number collisions.
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
