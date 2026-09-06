# Development

```sh
bash scripts/install-hooks.sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
python3 -m unittest discover -s tests -p 'test_*.py'
cargo build --profile test-ui --features test-support
python3 scripts/e2e.py --functional-only     # use while the machine is busy
# On an otherwise idle machine, at the end:
cargo bench --bench responsiveness
python3 scripts/e2e.py
python3 scripts/performance_gate.py
```

Linux native E2E additionally needs `xvfb`, `xdotool`, `zenity`, `xclip`, and ImageMagick with WebP support. The optimized `test-ui` profile avoids measuring debug rendering. `scripts/check.sh` runs the full suite; `SHEP_SKIP_E2E=1` explicitly omits GUI tests when X11 is unavailable.

The native MCP server is configured in `.mcp.json`. Read [the repository E2E skill](https://github.com/sam-ruff/shep.so/blob/main/.agents/skills/shep-e2e/SKILL.md). Its batch tool performs real clicks, double-clicks, drags, typing and shortcuts, plus bounded waits, observed-state assertions and WebP screenshots. Every AI-driven scenario must have an equivalent automated test. Fixture mail exists only behind the nondefault `test-support` feature. The production application does not launch fixture workspaces.

All generated logs and evidence belong under ignored `artifacts/`; never leave logs in the repository root. Performance budgets, methodology and the validated Linux baseline are in [docs/PERFORMANCE.md](../PERFORMANCE.md). The [completion audit](../COMPLETION.md) distinguishes implemented behavior, remaining work and verification that still needs live services/platforms.

**Documentation deploys automatically to GitHub Pages.** Quality and release workflows remain disabled as `.yml.disabled` files until the self-hosted Linux, Windows and macOS runners are ready. [AGENTS.md](https://github.com/sam-ruff/shep.so/blob/main/AGENTS.md) records how to re-enable them when requested.

Conventional Commits drive semantic-release on `main`. Pre-commit runs fmt, Clippy and Rust tests; commit-msg validates the commit format. Release tooling uses Node 24 and `npm ci`. `npm run release:dry` checks the proposed release without publishing. The dormant release workflow runs only after a successful quality build, verifies the tested commit, prepares the Cargo version/archive/checksums, and publishes the GitHub release. Current release packaging produces Linux archives.

## Extending Shep

`src/ui/` contains iced presentation and bounded prefetch caches. `engine.rs` dispatches bounded channels to background jobs; `store.rs` keeps SQLite work off the UI thread. Mail sync, flags and moves serialize per account to avoid racing stale server snapshots.

Cached search and message loads have reserved workers independent of provider operations. Speculative prefetch has its own smaller queue, and settings/drafts save in order on a separate worker. The dispatcher is tested with every provider worker blocked and its command queue full while local reads and saves continue.

Preferences use versioned acknowledgements so an older background update cannot undo newer choices or pane resizing. Message-detail results are invalidated after mail changes, including late prefetch errors and old flag states. Backup completion updates only its destination's timestamp and schedule readiness without overwriting settings changed during the upload. Copy lists and restore actions are bound to the selected destination. Retention protects the acknowledged copy even after a clock correction; local uploads never overwrite an existing file.

Implement `providers::MailProvider`, `providers::CalendarProvider` or `backup::BackupProvider` for a new provider, register its factory/configuration, and add deterministic contract tests. Wire-protocol code belongs in the provider; the UI only sends commands and handles events. Keep channel capacity, concurrency limits, cancellation, TLS verification and retention invariants intact.

The approved White Swiss Shepherd logo and dark variant are in `assets/`; [assets/README.md](https://github.com/sam-ruff/shep.so/blob/main/assets/README.md) records image-generation prompts and derivations. Font licensing is included alongside the embedded fonts. MIT license.
