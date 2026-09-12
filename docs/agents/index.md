# Agent documentation

This section holds the detailed reference for coding agents and maintainers. Human-facing instructions belong in the short [user guides](../index.md).

## Instructions and status

Read the repository's [AGENTS.md](https://github.com/sam-ruff/shep.so/blob/main/AGENTS.md) before changing code. It is the authoritative source for development rules, data-safety invariants, authorization boundaries and quality gates; this site does not replace it.

- [Completion audit](../COMPLETION.md): implemented evidence, active work and remaining product gaps.
- [Performance evidence](../PERFORMANCE.md): budgets, methodology and limits of the measured results.
- [Search comparison](search-demos.md): three demo branches, matching approaches and common examples.
- [Native E2E skill](https://github.com/sam-ruff/shep.so/blob/main/.agents/skills/shep-e2e/SKILL.md): isolated MCP scenarios and required automated equivalents.

## Detailed reference

- [Development and architecture](development.md): commands, release setup, worker boundaries and extension points.
- [Installation](installation.md): installer options, platform services and asset conventions.
- [Mail](mail.md): navigation, drafts, delivery recovery, Sent copies and reading behavior.
- [Calendar](calendar.md): discovery, access, write acknowledgments, conflicts and connection removal.
- [Google and backups](backups.md): authorization, destination identity, scheduling, resumable uploads and additive restore.
- [Google sign-in client](google-sign-in.md): Shep's compiled-in Desktop OAuth client, build variables, self-configured grants and the Google Cloud setup.
- [Multiple backup destinations](MULTIPLE_BACKUPS.md): several local, S3/SFTP/FTP and Google Drive destinations with independent schedules, retention and passphrases.
- [Local cache encryption](CACHE_ENCRYPTION.md): SQLCipher device keys, staged migration and recovery boundaries (in progress).
- [Database transfer and profiles](profiles.md): staged import, local profile selection, credential isolation and the desktop profile sync implementation.
- [Profile Drive records](profile-drive.md): shared metadata transport, durable pagination and immutable upload recovery.
- [Profile metadata format and current limits](PROFILE_FORMAT.md).
- [Local profile history](PROFILE_HISTORY.md): causal merge, durable upload identities, native worker/bridge and remaining enrollment work.
- [Google profile files](PROFILE_DRIVE.md): verified identity, bounded file pages and immutable upload recovery from the shared crates.
- [Durable profile discovery](PROFILE_DISCOVERY.md): saved scans, change replay and isolated remote summaries.
- [Desktop profile discovery](PROFILE_DESKTOP.md): active Google grant, durable scans and read-only iced controls; the desktop implementation now arrives from `main`.
- [Desktop reconciliation engine](PROFILE_RECONCILIATION.md): bounded ongoing preference reconciliation and its Flutter/browser equivalents.
- [Mobile Google connection](GOOGLE_MOBILE.md): platform Google Sign-In packages, stored bindings and cleanup state.
- [Flutter profile discovery](PROFILE_MOBILE.md): saved Google binding, native session ownership and real discovery controls; reviewed Flutter enrollment is connected.
- [Flutter profile publication](PROFILE_PUBLICATION.md): frozen reviews, initialized setup, immutable upload receipts and pause/retry controls.
- [Flutter profile enrollment](PROFILE_ENROLLMENT.md): independent device history, reviewed account/preferences application and credential reconnect boundaries.
- [Browser profiles and sync](PROFILE_BROWSER.md): server-mediated Google consent, worker-owned WASM history, per-identity storage, discovery, reviewed publication/enrollment and onboarding.
- [Profile sync handover](PROFILE_SYNC_HANDOVER.md): next-priority OAuth, Rust/Flutter enrollment, portable records, conflicts and verification requirements; implementation remains open.
- [Browser group actions](BROWSER_BULK.md): abandoned-review cleanup, the wider Undo lifecycle and the desktop-matching review keys.
- [Mobile group actions](MOBILE_BULK.md): native selection controls over the SQLite capture and the durable Flutter group journal with receipts, Undo, History and recovery.
- [Limits](limits.md): protocol requirements, storage ceilings and unverified behavior.

## Documentation maintenance

User pages should answer the next practical question in plain language. Preserve longer implementation details here, and keep the completion audit explicit about fixture coverage versus live-provider verification.

`zensical.toml` defines navigation. `requirements-docs.txt` pins the builder. `docs/images/` contains approved WebP assets for both the README and site. Screenshots must use isolated fictional fixtures, never personal inboxes.

Recreate the screenshot scenarios with:

```sh
cargo build --profile test-ui --features test-support
python3 scripts/e2e.py NativeFlows.test_documentation_screenshots
```

Review `docs-mail-light.webp` and `docs-calendar-dark.webp` under the printed `artifacts/e2e/<run>/` directory before copying them into `docs/images/` as `mail-light.webp` and `calendar-dark.webp`. The saved flow moves the pointer away from controls so tooltips do not cover the app. These captures prove native layout only; they do not verify live services.

The Documentation workflow is the only enabled workflow. It builds on GitHub-hosted Linux and deploys `main` through GitHub Pages. Keep the quality and release `.yml.disabled` files dormant until explicitly authorized to enable them.

GitHub Pages must be created with `build_type=workflow`. Private repositories require a plan that supports Pages; GitHub rejects setup with HTTP 422 otherwise. Do not change repository visibility to bypass this restriction without the user's explicit authorization. Once the account supports Pages, an administrator can configure it and dispatch the workflow:

```sh
gh api --method POST repos/sam-ruff/shep.so/pages -f build_type=workflow
gh workflow run docs.yml --ref main
```
