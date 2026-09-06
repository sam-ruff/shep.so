---
name: shep-e2e
description: Run and extend realistic Shep native desktop tests through its batchable MCP harness, with automated equivalents and visual evidence. Use for Shep mail, calendar, preferences, shortcut, or responsiveness UI changes.
---

Use the native iced window. This repository has no browser UI; Playwright does not exercise the app.

Read `AGENTS.md` for performance budgets, disabled CI, and platform setup. Build the isolated test executable with `cargo build --profile test-ui --features test-support`. Run `python3 scripts/e2e.py` for the repeatable suite, or `python3 scripts/e2e.py --capture-only` for the initial layout.

The stdio MCP server is `python3 scripts/mcp_harness.py` from the repository root. Use a 2025-11-25 MCP initialization handshake, `tools/list`, then `tools/call`. An example client is `McpClient` in `scripts/e2e.py`; use it directly from automation when an interactive host cannot register another MCP server.

Call `desktop.start` once per independent scenario. It owns an isolated Xvfb display, a 1440×920 native window and an in-memory fixture workspace. It returns the artifact directory. Set `empty_calendars: true` on `desktop.start` to test the no-calendar state. The nondefault test feature supplies fixture messages and observation hooks; normal builds must never expose personal mail to the harness.

Use `desktop.batch` for related actions. Example arguments:

```json
{"actions":[
  {"type":"key","key":"ctrl+k"},
  {"type":"type","text":"prototype"},
  {"type":"wait_for","path":"total","op":"eq","value":1},
  {"type":"key","key":"Escape"},
  {"type":"key","key":"m"},
  {"type":"wait_for","path":"dialog","op":"eq","value":"Move"},
  {"type":"wait","ms":100},
  {"type":"screenshot","name":"move-search-result"}
]}
```

Supported actions: `click` and `double_click` with window-relative `x,y`; `drag` with start `x,y`, `end_x,end_y` and `duration_ms` (0–2000); `type` with text; `key` with an xdotool chord; `choose_file` with an optional fixture `path` (omit to cancel the open native picker); `scroll` with signed amount; `wait` with milliseconds; `assert`/`wait_for` with a dot-separated state path, comparison and value; `screenshot`; `state`. A batch has at most 100 actions, each explicit wait at most 2,000 ms, total explicit waits at most 10,000 ms. Prefer `wait_for` to arbitrary sleeps. Inspect the full action result: a batch stops at its first failure and captures a screenshot.

State observations include current tab/dialog, selected subject, query results, cache occupancy, prefetched-page availability, appearance, saved pane ratio, filter/sort, page offset, shortcut map and handler p95. Never use state-file writes or direct app methods to simulate user actions. Verify the same visible flow with mouse and keyboard where both are supported. Tests should include typing `M` in a text field without moving mail, remapping `M`, switching appearance, cached navigation, search, drafts, calendar and interaction during sync.

**Save every AI-driven test scenario as an equivalent deterministic test in `scripts/e2e.py`.** Reuse the same `McpClient` and `desktop.batch` path. Add Python harness unit tests for protocol/validation changes and Rust tests for backend invariants. A manual MCP pass is not a substitute for the automated equivalent.

Use `desktop.screenshot` to inspect visual output, and preserve WebP screenshots under ignored `artifacts/e2e/`. Review spacing, focus, clipping, contrast and empty/error states in light/dark and compact layouts. State assertions alone cannot prove visual quality. Do not commit personal email, credentials or OAuth responses.

Call `desktop.stop` when finished; it terminates only processes it owns. The current harness supports Linux/X11; do not infer macOS/Windows E2E coverage from a Linux pass. If a flow needs a live service, use a local fake-server integration test where possible, and state clearly which live-provider behavior remains unverified.


Defer performance measurement while the host is busy: `python3 scripts/e2e.py --functional-only`. Run the full native suite, backend benchmark and `python3 scripts/performance_gate.py` at the end on an otherwise idle host. Keep all logs in `artifacts/logs/`, never the root. Mouse resize, flag/filter/sort, draft reopening and compact layouts need saved automated scenarios.

Opening a panel changes the widget tree asynchronously. Follow navigation with a state assertion and a short 80 ms settling wait before typing into a newly focused field. For Move, wait for `focused_input == "folder-search"`, which observes iced native focus rather than merely the dialog state. Do not replace waits for backend completion with fixed sleeps. Include double-click reader/Escape, all-day event save, arrow/Tab navigation and selection scrolling, fuzzy Enter-to-move, unified expansion, cross-account transfer in fixtures, image exceptions, sender details and wrapped attachments. New fixture images must never make external requests. Connection-test fixtures explicitly report that live probes are unavailable; the ignored live diagnostics cover actual authentication only with explicit user authorization.


For outgoing attachments, create fixture files under the `artifacts` directory returned by `desktop.start`. Click the actual Attach files button, then batch `{"type":"choose_file","path":"/absolute/current-run/fixture.txt"}`. The action drives the real Zenity dialog; it never calls app update methods. It requires `zenity` and `xclip`, uses an isolated clipboard/GTK profile and restores app focus after closing the picker. Omit `path` to test cancellation. Wait for `draft_io == false` and the expected `draft_attachments` entry, then allow a short visual settling interval before reviewing a screenshot. Cover multiple wrapped files, removal, save/reopen after deleting source fixtures, Cc/Bcc, disabled preview sending, Reply all via mouse/shortcut and compact composer controls. Keep these steps in `scripts/e2e.py`; never select personal files.

For separate-message conversation reading, start with `conversation_mail: true`. The fixtures include a three-message exchange across Archive/Sent/Inbox and a 25-message thread. Keep the selected inbox anchor separate from the expanded body: observe `selected_id`, `reader_message_id` and `loaded_message_id`, and wait for the loaded ID before Reply. Cover mouse flag/move on an older message, its attachment and reply recipient/reference, collapse/expand, General's grouping preference, dark/full-window/compact layouts, and Earlier/Later paging during sync. Use the saved conversation tests in `scripts/e2e.py`; metadata observations are not an action API. A missing list entry can be transient during refresh, so `wait_for` retries it to its existing deadline.

For calendar discovery, start an isolated `empty_calendars: true` preview. Open Preferences → Calendars → Add CalDAV calendar. The fixture accepts URL `https://calendar.example.test/`, username `alex` and password `fixture-password`, and supplies writable Personal plans plus read-only Team holidays. Other values produce a recoverable fixture error, without real network/keychain access. Batch selection/deselection, disabled empty selection, saving both, failure/retry and reconnect without duplicates. Observe `calendar_choices`, `calendar_selected`, `calendar_sources`, `calendar_error`, `calendar_discovering` and `calendar_saving`. Use `readonly_calendars: true` to test the existing Home event's read-only view and the writable new-event default. Preserve the saved native equivalents and review light/dark/compact WebP evidence; provider loopback tests establish actual PROPFIND/Google-list behavior separately.


For connection removal, use only the isolated fixture workspace. Preferences → Accounts/Calendars trash controls open a count review; Escape/Cancel leaves data intact. Observe `removal`, `removal_error`, `removing`, `removal_cancel_transfers`, `account_count`, `calendar_count`, `credential_cleanup`, `removed_google_calendars` and `events`. Confirming removal must preserve the other account/calendar and allow normal inbox navigation. Removing a Google calendar offers an explicit restore action; cached events return on sync, not when the connection row is restored.

Use `pending_transfer: true` on `desktop.start` for one unfinished cross-account move. The Remove button must stay disabled until the extra cancellation checkbox is selected. All fixture journals live in `tests/support/fixtures.rs`; do not write them through the observation oracle. The saved removal flows include draft counts, mouse/keyboard cancellation, remaining-mail navigation and light/dark/compact WebP captures. Credential failures and cleanup/reconnect ordering use injected Rust fixtures rather than manipulating the user's keychain.

For outgoing recovery, use `desktop.start(outgoing_mail=true)`. It seeds an uncertain delivery and an accepted message whose server copy was not acknowledged. Observe `outgoing_pending`, `outgoing_rows`, `outgoing_selected`, `outgoing_confirmed`, `outgoing_error`, `draft_count` and `busy`. Open Outbox from the sidebar. Without its review checkbox, returning an uncertain delivery to drafts, recording it as sent and repeating an ambiguous copy upload must stay disabled. Review before choosing those actions. Keeping an accepted copy locally needs no network and removes its pending row. Verify the retained draft text after returning to drafts, and Sent contents after keeping copies locally.

The saved outgoing flows use real mouse controls, preview-only connection errors and WebP captures in light/dark/900×640 layouts. Error notices and completed stages can change a centered dialog's position; wait for the result and presentation before clicking the next control. The SMTP wizard flow edits the Sent folder and all copy policies while preserving connection-test results, and confirms that preview account saving remains disabled. All server operations in these native fixtures are disabled. Actual SMTP delivery and IMAP Sent failure/restart contracts use injected Rust transports and local protocol streams, never personal mail.

For Google device disconnection, the default isolated preview has two Google calendars and five cached events. Preferences → Calendars → Google connection has Disconnect; scroll to the controls in a 900×640 window. Test both mouse Cancel and Escape before confirming, then observe `google_lifecycle.disconnected`, `google_lifecycle.cleanup_pending`, `google_connected` and `google_archived`. The fixture skips all real credential operations. Both calendars, all five events and the mail accounts must remain; opening a cached event must show read-only access and mail navigation must still work. Keep light/dark/compact captures and scenarios in `scripts/e2e.py`. Keychain failure/retry, reconnect, restart and stale snapshot behavior belong in the Rust fixture tests; never run this flow on a personal Google connection.
