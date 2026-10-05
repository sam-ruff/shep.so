# Fast headless UI tests

## Real mail server scenarios

Run `python3 scripts/test_real_mail.py` on Linux with Docker and OpenSSL. A
scenario name substring selects a smaller run, for example
`python3 scripts/test_real_mail.py held_server`. `--profile dev` uses the debug
build for development; the default `test-ui` profile is optimised. No mailbox
credentials or external server are required. These tests are explicitly ignored
in ordinary Cargo runs, so routine unit tests remain offline.

Each scenario owns a digest-pinned GreenMail container with random loopback TLS
ports, fictional accounts, an in-memory credential backend and a separate SQLite cache.
The runner trusts the repository's localhost test certificate only in its child
process. Production TLS and hostname checks stay enabled. GreenMail validates
explicit SMTP AUTH but permits unauthenticated delivery by design; this suite
does not establish production SMTP relay policy.

The real App receives widget input through the existing `iced_test` harness.
Its engine uses production IMAP/POP3/SMTP, cache, journals, background checks and
IDLE. Only workspace and credential construction are replaced. Independent IMAP
reads check physical message counts, exact Message-IDs, UIDs, flags and attachment
bytes. Tests do not dispatch mail commands or manufacture provider receipts.

Coverage includes receiving/search, read-on-leave, flags, Archive/Trash/Move and
Undo, folder creation/move/delete with cancelled reviews, selection beyond the
50-row page, scoped bulk work, drafts across navigation/restart, SMTP/Sent,
reply headers, forwarding attachments and compact dark controls. Stress
scenarios repeat flag, move, Send and Refresh inputs, switch screens during
pending work and preserve edited drafts when new mail arrives.

A bounded TLS relay can delay or hold actual server replies while the independent
oracle keeps working. Tests first observe a held response, then exercise local
navigation and saving. A slow multi-megabyte download runs beside cached reading.
Input p95 has a fixed 100 ms headless budget. New mail must appear within five
seconds after SMTP acknowledgement, with ordinary polling set to an hour to
exercise push. These are input-to-state/layout and arrival observations, not
native window presentation or monitor latency measurements. Run timing scenarios
without competing builds; thresholds are never relaxed for host load.

Logs, fictional caches, state observations, snapshots and timing reports remain
under ignored `artifacts/`. Failures retain their evidence; owned containers are
removed on ordinary completion and panic. A force-killed test process cannot run
Rust destructors, so its labelled container may require explicit cleanup.

`first_download_preserves_active_composer_and_focus` holds the first mail page
while a new draft is being typed. It checks that arrival retains the composer,
continues typing into the same Subject field without refocusing, and preserves
the full draft through restart. Automatic first-row selection yields to a current
composer; explicit mail selection still opens the chosen reader.

This suite does not cover formatted-HTML renderer subscriptions, OS pickers,
printing, tray/badge delivery, Google/CalDAV, remote-provider behaviour or
Windows/macOS execution. Existing native scenarios remain necessary for those
surfaces. Restart tests reopen the app/cache in the same test process after the
engine drains; they are not process-crash tests.

## Fixture scenarios

The native suite in `scripts/e2e.py` drives the real window on Xvfb and takes
most of an hour. A second, much faster layer runs the same app inside
`cargo test`, with no display, using the official
[`iced_test`](https://docs.rs/iced_test/0.14.0) crate. It lives in
`src/ui/simulator_tests/` and needs the `test-support` feature, so
`cargo test --all-features` and the pre-commit hook run it.

## How it works

`Harness::start()` builds the real `App`, starts the demo engine on an
in-memory fixture store (the same workspace `desktop.start` opens) and waits
for the first mail page. Input goes through the actual widget tree:
`click_at(x, y)` sends a pointer move, press and release; `key("ctrl+comma")`
takes xdotool-style chords; `type_text` presses one key per character. The
coordinates match the native scenarios at the same window size, because the
layout code and bundled Noto fonts are the same.

The harness does what the iced runtime would do. It keeps one widget-state
cache across rebuilds, so text focus and scroll positions survive between
messages, redraws after each update, applies widget operations such as focus
requests, and answers window queries. `iced_test::Simulator` itself rebuilds
widget state for every instance, which would lose focus mid-scenario, so it is
used for snapshots only. Selectors and input events come from `iced_test`.

Background work is real. Engine events and every `Task` the app returns are
forwarded into queues and handled on the test thread. Tests wait for observed
widget focus before typing, including controls whose focus task is delayed.
The app's one-second
`Tick` subscription is emulated during input and while checks wait. Assertions
do not sleep for a fixed time: `expect("dialog", "Move")` reads the same
observation the native harness writes to its state file and waits with the
shared hang deadline. Performance checks have their own fixed budgets.

## What belongs here

Scenarios whose value is in UI logic: preferences and their saves, settings
search and reveal, shortcut capture, conflicts and disabling, focus guards on
mail shortcuts, Move dialog input, drafts and context menus, and dropdown
dismissal. Each one names the native scenario it mirrors.

Keep the native scenario as well. AGENTS.md still requires real-control
evidence, and only the native suite proves X11 input, window-manager focus,
presented pixels, file pickers, the tray, badges, printing and timing. HTML
rendering is also native only: the harness does not run the HTML renderer
subscriptions.

## Snapshots

`Harness::snapshot()` renders through `Simulator::snapshot`. Pixel hashes depend
on the host's fallback fonts, because iced's font system also loads system
fonts, so no reference images or hashes are committed. Scenarios compare
snapshots taken in the same run instead, for example that switching appearance
back restores identical pixels.

## Adding a scenario

Put it in the matching module (`mail.rs`, `preferences.rs` or `shortcuts.rs`)
as a plain `async fn`, add its name to that module's `scenarios!` list, start
from `Harness::start()` or `Harness::with_size(900., 640.)`, and follow the
native batch step by step. `scenarios!` runs each one through `harness::run`,
whose runtime and thread have 8 MiB stacks: under `#[tokio::test]` the demo
engine overflowed Tokio's default 2 MiB worker stack in unoptimised Windows
builds. Fixture options that the native harness passes as
command-line flags (`mail_actions`, `long_folders` and similar) are not
available here yet; scenarios needing them stay native only.
