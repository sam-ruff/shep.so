# Fast headless UI tests

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
forwarded into queues and handled on the test thread; tasks that are ready at
once, such as focus operations, run before the next input. The app's one-second
`Tick` subscription is emulated while a check waits. Nothing sleeps for a fixed
time: `expect("dialog", "Move")` reads the same observation the native harness
writes to its state file and waits, with a ten-second deadline, until it holds.

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

Put it in the matching module (`mail.rs`, `preferences.rs` or `shortcuts.rs`),
start from `Harness::start()` or `Harness::with_size(900., 640.)`, and follow the
native batch step by step. Fixture options that the native harness passes as
command-line flags (`mail_actions`, `long_folders` and similar) are not
available here yet; scenarios needing them stay native only.
