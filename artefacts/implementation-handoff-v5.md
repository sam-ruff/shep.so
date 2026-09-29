# Implementation handoff v5: profile-sync cycle during a review (supersedes v4)

Commit `ab61264` (`fix:`) on top of `81ac41f` applies the one change requested by review v3.

## Change

`src/ui/profile_sync.rs`: when an automatic profile-sync cycle fails while a shared
setting or account review list is open, the error is now recorded with
`background_notice` instead of `notice(error, true)`. A tray-hidden window therefore
stays hidden and a pending Quit is kept. Failures of other review work (for example a
rejected review choice) still use the ordinary notice and reopen.

Tests added (same file):

- `offline_cycle_with_an_open_review_keeps_the_tray_window_hidden`: tray-hidden window,
  account review open, cycle fails offline; window stays hidden, close intent unchanged,
  activity error and in-app notice show the error. Repeated after a real tray Quit.
- `offline_cycle_without_a_review_keeps_the_tray_window_hidden`: same without a review.

Docs: `AGENTS.md` tray section, `TODO.md` entry and `docs/COMPLETION.md` evidence updated.

## Checks (this VM, isolated target dirs, 4 jobs, logs in `artifacts/logs/v6-*`)

| Command | Result |
|---|---|
| `cargo fmt --all -- --check` | pass |
| `cargo clippy --all-targets --all-features -- -D warnings` | pass |
| `cargo test --all-features` | pass: 1,692 passed, 0 failed, 22 ignored (both new tests ok) |
| `cargo build --profile test-ui --features test-support` | pass |
| `python3 scripts/e2e.py --functional-only -k test_tray_native -k test_background_sync` | pass: 18 tests OK, first run |
| `zensical build --clean --strict` | pass, no issues |

The VM had lost the native build packages since the previous attempt; the orchestrator
reinstalled the same list before the tests ran. The first `cargo test` attempt failed at
link time only for that reason (missing `-ldbus-1`).

Repository hooks were not installed in this worktree (`core.hooksPath` unset); the
commit was made after running the equivalent checks above by hand.

## Not changed / remaining

No new native scenario: this path needs a failing automatic profile-sync cycle while
hidden in the tray, which no existing fixture drives with real input. Known limits from
v4 are unchanged and recorded in TODO: no native capture of the 30-second banner, an
unrelated visible error holds the banner back, automatic move recovery has App-level
evidence only, the blank formatted reader in the screenshot is unreproduced, and
Flutter/browser parity remains an open gap.
