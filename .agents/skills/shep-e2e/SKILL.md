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


For outgoing attachments, create fixture files under the `artifacts` directory returned by `desktop.start`. Click the actual Attach files button, then batch `{"type":"choose_file","path":"/absolute/current-run/fixture.txt"}`. The action drives the real Zenity dialog; it never calls app update methods. It requires `zenity` and `xclip`, uses an isolated clipboard/GTK profile and restores app focus after closing the picker. It verifies the entered path through native copy and clipboard ownership before confirming. GTK filename validation can require another Return; bounded retries target only that picker window. Root-display captures include the isolated picker and its popups. Omit `path` to test cancellation. Wait for `draft_io == false` and the expected `draft_attachments` entry, then allow a short visual settling interval before reviewing a screenshot. Cover multiple wrapped files, removal, save/reopen after deleting source fixtures, Cc/Bcc, disabled preview sending, Reply all via mouse/shortcut and compact composer controls. Keep these steps in `scripts/e2e.py`; never select personal files.

For separate-message conversation reading, start with `conversation_mail: true`. The fixtures include a three-message exchange across Archive/Sent/Inbox and a 25-message thread. Keep the selected inbox anchor separate from the expanded body: observe `selected_id`, `reader_message_id` and `loaded_message_id`, and wait for the loaded ID before Reply. Cover mouse flag/move on an older message, its attachment and reply recipient/reference, collapse/expand, General's grouping preference, dark/full-window/compact layouts, and Earlier/Later paging during sync. Use the saved conversation tests in `scripts/e2e.py`; metadata observations are not an action API. A missing list entry can be transient during refresh, so `wait_for` retries it to its existing deadline.

For calendar discovery, start an isolated `empty_calendars: true` preview. Open Preferences → Calendars → Add CalDAV calendar. The fixture accepts URL `https://calendar.example.test/`, username `alex` and password `fixture-password`, and supplies writable Personal plans plus read-only Team holidays. Other values produce a recoverable fixture error, without real network/keychain access. Batch selection/deselection, disabled empty selection, saving both, failure/retry and reconnect without duplicates. Observe `calendar_choices`, `calendar_selected`, `calendar_sources`, `calendar_error`, `calendar_discovering` and `calendar_saving`. Use `readonly_calendars: true` to test the existing Home event's read-only view and the writable new-event default. Preserve the saved native equivalents and review light/dark/compact WebP evidence; provider loopback tests establish actual PROPFIND/Google-list behavior separately.


For connection removal, use only the isolated fixture workspace. Preferences → Accounts/Calendars trash controls open a count review; Escape/Cancel leaves data intact. Observe `removal`, `removal_error`, `removing`, `removal_cancel_transfers`, `account_count`, `calendar_count`, `credential_cleanup`, `removed_google_calendars` and `events`. Confirming removal must preserve the other account/calendar and allow normal inbox navigation. Removing a Google calendar offers an explicit restore action; cached events return on sync, not when the connection row is restored.

Use `pending_transfer: true` on `desktop.start` for one unfinished cross-account move. The Remove button must stay disabled until the extra cancellation checkbox is selected. All fixture journals live in `tests/support/fixtures.rs`; do not write them through the observation oracle. The saved removal flows include draft counts, mouse/keyboard cancellation, remaining-mail navigation and light/dark/compact WebP captures. Credential failures and cleanup/reconnect ordering use injected Rust fixtures rather than manipulating the user's keychain.

For outgoing recovery, use `desktop.start(outgoing_mail=true)`. It seeds an uncertain delivery and an accepted message whose server copy was not acknowledged. Observe `outgoing_pending`, `outgoing_rows`, `outgoing_selected`, `outgoing_confirmed`, `outgoing_error`, `draft_count` and `busy`. Open Outbox from the sidebar. Without its review checkbox, returning an uncertain delivery to drafts, recording it as sent and repeating an ambiguous copy upload must stay disabled. Review before choosing those actions. Keeping an accepted copy locally needs no network and removes its pending row. Verify the retained draft text after returning to drafts, and Sent contents after keeping copies locally.

The saved outgoing flows use real mouse controls, preview-only connection errors and WebP captures in light/dark/900×640 layouts. Error notices and completed stages can change a centered dialog's position; wait for the result and presentation before clicking the next control. The SMTP wizard flow edits the Sent folder and all copy policies while preserving connection-test results, and confirms that preview account saving remains disabled. All server operations in these native fixtures are disabled. Actual SMTP delivery and IMAP Sent failure/restart contracts use injected Rust transports and local protocol streams, never personal mail.

For Google device disconnection, the default isolated preview has two Google calendars and five cached events. Preferences → Calendars → Google connection has Disconnect; scroll to the controls in a 900×640 window. Test both mouse Cancel and Escape before confirming, then observe `google_lifecycle.disconnected`, `google_lifecycle.cleanup_pending`, `google_connected` and `google_archived`. The fixture skips all real credential operations. Both calendars, all five events and the mail accounts must remain; opening a cached event must show read-only access and mail navigation must still work. Keep light/dark/compact captures and scenarios in `scripts/e2e.py`. Keychain failure/retry, reconnect, restart and stale snapshot behavior belong in the Rust fixture tests; never run this flow on a personal Google connection.


For partial Google permissions, launch `desktop.start(google_permissions="calendar")`, `"drive"`, or `"read-only"`. These use fixture grants only. Observe `google_grant.access.known`, `drive`, `calendar_read`, `calendar_write`, `google_archived` and `event_access`; never replace OS credentials or authenticate to a personal Google account. The saved native scenarios review the permission labels, opening cached read-only events, Backups and continued mail navigation in light/dark/900×640 layouts. Scroll to the Google card's bottom before inspecting its permission controls; additional rows can move the Disconnect button. Grant activation/restart/failure contracts live in Rust tests with fake keychains and loopback services.


For sidebar overflow use `desktop.start(long_folders=true)`. `click`/`double_click` accept `button: 3` for right-click and `modifiers: ["ctrl"]` (also shift/alt/super). Modifiers are released after failures. Use `hover` to position the pointer for tooltips or scrolling, and `resize` with width/height (900–2560 × 640–1600) for native window resizing. Both are batchable. Test Ctrl+click selection/deselection, plain-click reset, empty selection and overlapping account scopes. Observe `selected_folders`, `collapsed_accounts` and `sidebar_labels`; a selected empty set must show zero messages.

Drag the sidebar edge at x=222 in a standard new fixture, or x=200 at compact width. Observe `sidebar_width`, `saved_sidebar_width`, `saved_window_size` and `preferences_saved`; resize must retain a usable reader. Contacts lives in its own Preferences tab. Save contacts and wait for `saved_toast == true` after the actual storage acknowledgment, then dismiss it. Screenshots verify both the compact tab layout and the toast.

Right-click an inbox row, including a row different from the selected message. Observe `context_subject`/`context_menu`; exercise mouse flag/move, Escape/outside dismissal, Shift+F10 and arrow/Enter opening. The compact flow replies to the right-clicked sender. Keep the automated equivalents in `scripts/e2e.py`; do not use the oracle to invoke actions. Review menu positioning and red flag contrast in light/dark/compact WebP captures. Only icon controls have tooltips; optional key hints show the primary remapped binding.


For current shortcut/reading work, observe `shortcuts` (primary map), `shortcut_secondary`, `reader_text_ready`, `reader_selected_text`, `inbox_unread`, `settings_search`, `settings_matches`, `settings_group`, `tooltips` and `shortcut_tooltips`. Test Ctrl+D, both archive keys, conflict/secondary clear, and I with sidebar/list focus plus disable/remap. For text copying, use real selection drags, Ctrl+C and a paste into search; full-reader Ctrl+A selects text when that editor has focus. Keep this distinct from the pending multi-message selection request.

Preferences search is the header field. Search for a setting (e.g. tooltip/font/TLS), click the result and operate the actual control. Save captures with all tooltips off, label-only icon hints and primary-only hints, plus compact dark search. Labeled navigation/reply controls should have no tooltip. The mail refresh icon is at x=1400,y=36 in the standard fixture.

`test_context_menu_survives_mouse_release_and_sync_refresh` starts a fixture sync, opens a real right-click menu and waits for sync completion before selecting an action. Fixture sync must emit Changed, matching the production refresh path. Keep the regression plus outside/Escape dismissal. Xvfb readiness/`-noreset` prevents display resets between its initial probe and the app connection; logs stay inside each run.


For optimistic mail behavior use `desktop.start(mail_actions="slow")` or `"fail"`. Observe `mail_pending`, `unread`, `starred`, `mail_rows`, `total` and `notice`. Click the actual reader toolbar (archive x=652, read x=740, flag x=784, y=100 at 1440×920), assert the immediate change while `mail_pending > 0`, then await completion or restoration. Two serialized delayed fixture edits may need a 5,000 ms wait_for deadline. Do not increase performance thresholds or substitute direct oracle writes. The saved flows also navigate while pending and move Archive back to Inbox.

Shortcut primary and secondary fields each have their own clear × (approximately x=982 and x=1150 in a standard Preferences window). Test the primary and secondary separately, cancel an active capture, revisit saved settings and verify the disabled action does not execute. The compact dark case checks layout; do not infer hit targets solely from a state assertion.


For background mail checks, launch `desktop.start(background_sync=true)`; add `sync_failure_once=true` for a failed initial check followed by recovery. The fixture delivers **New mail from the background** through the ordinary cache/update path. Observe `background_sync`, `refreshing`, `sync_round`, `mail_check_seconds`, `total` and `notice`. Automatic activity must leave the refresh icon idle; click it while background work is pending and await the queued manual round. Repeated clicks coalesce. Keep the saved three sync scenarios, including searching Preferences for background checks, rejecting zero seconds and saving a valid interval without retaining the old error. The production scheduler's long waits/overlap/timeout cases use virtual-time Rust tests, not real-time performance measurements.


For ranked search use `desktop.start(search_mail=true)`. Search for `test`; the older **Quick note**, with body `test`, must be first and the sort control must read Best match (`sort == "Relevance"`). Select Newest first from the actual menu to put **Testing checklist** first. Clearing search restores the browsing sort; starting another search selects Best match again. Repeat during a fixture refresh. The Move scenario searches `cafe`, checks Café is highlighted, moves with Enter and opens that sidebar folder to verify its contents; it also types `archvie` and Return without a settling wait and checks Archive afterward. Preserve both saved native equivalents and WebP evidence.

For draft navigation, use the counted Drafts group in the sidebar; collapsing it persists without hiding account folders. Right-click a draft for Open/Discard, and verify that an in-progress mail refresh does not dismiss the menu. Discard reviews accept Enter/Y, with Escape/N or Keep draft cancelling. The composer bin opens the same review and includes attachment scope. Observe `draft_rows`, `drafts_collapsed`, `saved_drafts_collapsed`, `draft_context`, `discard_pending`, `draft_attachments` and `editor`. `desktop.start(discard_failure_once=true)` fails the first discard and permits retry, only in the isolated preview. Preserve the saved `test_drafts_collapse_context_cancel_and_discard` and `test_draft_bin_cancel_failure_retry_and_compact_dark_review` equivalents. Review their WebP captures; Rust tests establish actual cached-file deletion, restart tombstones and delivery exclusion.

Read-on-leave scenarios should start with `mail_actions="slow"` or `"fail"` and deliberately click an unread inbox row before leaving it. Startup selection and hover alone must not mark it read. Observe `read_candidate`, `mail_rows.<index>.unread`, `unread`, `mail_pending`, `selected` and `folder`; after a tab change allow native layout to settle, and wait for the intended `read_candidate` after clicking so the preselected first row cannot satisfy the test by itself; verify the old indicator changes while saving is still pending. Explicitly mark it unread, leave again and refresh to establish that choice survives. Cover failure after switching folders and arrow navigation through the Unread filter, where a completed read removes the old row. Preserve the three `test_read_on_leave_*` native equivalents in `scripts/e2e.py`.

For immediate counted action toasts, observe `action_toast.label`, `action_toast.count` and `mail_pending`. Use slow/failing action fixtures and actual Archive, Ctrl+D, both Archive keys and the Move dialog. Assert feedback while the backend is still pending; exercise repeated actions across unified accounts, dismissal before acknowledgment, failure and destination changes. The six-second refreshed lifetime has an injected-clock Rust test, so do not add wall-clock performance measurements. Saved native flows also cover slow failed cross-account moves and dark/900×640 toasts. Dismiss is near x=1390,y=874 in the standard window; inspect actual layout for compact coordinates.

Keep `test_search_mouse_focus_blocks_default_and_remapped_delete_chords` and the default delete/archive flow: iced can report Ctrl+D as uncaptured while search is focused. Clearing search must retain the original inbox count after default/remapped Delete; the same key must work once the field is unfocused. With the old bottom completion notice removed, the bottom-scrolled Inbox shortcut row is near y=660 and Delete is near y=600 at 1440×920. The Inbox-clear test also asserts Delete is unchanged to catch wrong-row clicks.


Undo scenarios use the existing slow mail fixture and optional `desktop.start(undo_failure_once=true)` to reject only the first inverse. Click the actual toast Undo near x=1340,y=874 (x=800,y=594 at 900×640). The persistent Retry Undo is near x=1308,y=874. Observe `action_toast.undo`, immediate `total`, `mail_pending`, `undo_failures`, restored account/folder and final destination contents. Save grouped archive, delete failure/retry, move-from-destination, cross-account preference change and compact dark automated equivalents. Review screenshots for Restoring rows and persistent errors; assertions alone do not establish layout. Protocol tests separately cover changed UIDVALIDITY, missing receipts, exact-byte recovery, ambiguous copies and server rejection. These fixtures never mutate personal mail. When waiting for a serialized group, observe pending count progress in separate at-most-five-second `wait_for` steps.


For HTML reading use `desktop.start(html_mail=true)`. This adds only fictional styled sign-in mail, mislabeled/escaped XHTML and a long formatted letter. Observe `html_ready`, `html_formatted`, `html_error`, `html_height`, `html_scroll`, `html_selected_text`, `html_quotes_hidden`, `html_loaded_images` and `html_link`. Drive Formatted/Plain text with real buttons, selection/copy with native drags/Ctrl+A/Ctrl+C, and paste into search to verify clipboard text. Keep the saved `test_html_*` flows; no private authorization codes belong in fixtures.

Review HTML screenshots in light/dark, full-reader and 900×640 layouts. The short Prototype fixture tests quote toggles, sender details and all image exceptions. Its reply bar must remain visible and clickable even when the HTML frame is taller than the actual message. The long-letter flow scrolls and archives while a delayed flag save remains pending, asserting the immediate toast. These are functional delayed-provider checks; performance measurements remain deferred.

Click the HTML body before testing arrows, Page Up/Down (`Prior`/`Next`), Home and End. Assert reader scrolling keeps `selected` unchanged; then click the inbox and verify Up/Down select mail again. Preserve `test_html_keyboard_scroll_is_scoped_to_the_focused_reader`.


Find tests use `html_mail=true` for long formatted/plain alternatives and the wide Projects report. Open with Ctrl+F or the reader's search icon; wait for `find_open` and native `focused_input == "find-message"` before typing. Observe `find_query`, `find_pending`, `find_count`, zero-based `find_active`, `find_match_case` and `find_error`. Enter advances, Shift+Enter goes backward, and Escape closes Find before a full reader. Verify Ctrl+D does not act on mail while typing. Navigate to another message to check stale results disappear. For no-match reopen tests, wait for the bar to open before checking the already-zero count.

Preserve the five saved `test_find_*` flows, including HTML/plain quote expansion and both remapping slots. Closing the find bar changes control positions; use a short presentation wait before clicking mode/quote controls, and scroll back to those controls after a revealed match. At 1440×920, the bar is near y=158, Aa x=1261, previous x=1300, next x=1344 and close x=1388. At the bottom of Shortcuts, Forward is y=780, Find y=720, Inbox y=660 and Delete y=600. Review highlighted phrases and scroll destinations in light/dark/compact/full-window captures; state alone cannot prove alignment.


Forward tests use the reader footer arrow or F. Search the prototype fixture for four original attachments; observe `forward_pending`, `draft_forward`, `draft_forward_html`, `fields`, `editor`, `draft_attachments` and `draft_in_reply_to`. To/Cc/Bcc start empty and a forward must not inherit reply-thread headers. Wait for native `focused_input == "to"` before typing. Save/close with Escape, then reopen from Drafts and verify the original files/text and edited recipients. No fixture sending reaches a real server. With `mail_actions="slow"`, navigate and edit another draft while preparation is pending; the result must stay in Drafts. With `"fail"`, preparation fails once and can retry, without a partial draft. Keep mouse/keyboard/remap/disabled/input-isolation and light/dark/compact saved equivalents. Adding Forward at the bottom of Shortcuts moves the bottom-scrolled Find/Inbox/Delete rows up by one row; verify actual positions before updating tests.

The prototype fixture's Forward arrow is near x=835,y=784 after its four attachments wrap; Reply all is near x=766,y=784. An ordinary message without attachments has its footer lower. A visible bottom notice moves bottom-scrolled settings rows: dismiss Draft saved through its actual × and await `notice == null` before clicking the Forward clear controls near y=780. In search-isolation tests, Alt+F may insert an f into the native field; observe that query before clearing it, then await the restored selected row. A previously matching count or focus label alone can be stale.


Print flows use `desktop.start(print_browser="pdf" | "dialog" | "fail")`, with Chrome/Chromium and Poppler installed. The harness owns a fresh profile, explicitly uses X11, and never opens the personal browser. Drive the actual footer icon or Mod+P. `print_output` checks an actual browser-created PDF using count/text/minimum pages and saves a first-page WebP; `browser_screenshot` captures the isolated display, `cancel_print` presses native Escape, and `focus_app` restores native Shep input. Observe `print_pending`, `print_revision` and `print_source` only. Test full formatted/plain/long output, source identity while navigating, retry, shortcut isolation and compact dark attachment wrapping. Review subject/sender headers and CID images in the PDF, plus the real printer dialog before cancellation. The current shortcut rows at the bottom are Print y=780, Forward y=720, Find y=660, Inbox y=600, Delete y=540, without a bottom notice. Keep the saved native equivalents and do not interpret launch completion as a printing receipt.


For HTML frame preparation, use html_mail=true and wait for html_view_current,
which confirms the displayed frame matches the current native viewport/scroll.
The html_cache_ids, html_cache_hits and html_cache_bytes observations establish
bounded neighbor preparation without timing claims. Save rapid navigation,
End/Home, pane drag and compact resize as automated equivalents; verify selected
text belongs to the final message. Preserve image-policy, quote and Find flows.
The refresh-icon scenario captures Mail and Calendar in light/dark and compact
layouts, including a pending mail refresh. Inspect the icon geometry in the
actual WebP captures, rather than relying on state assertions alone.

During rapid multi-row navigation, wait for each intended `selected` subject
before the next click, without waiting for its HTML body. This verifies every
native input was consumed and avoids interpreting queued clicks on an unchanged
row as a double-click. When returning from a calendar dialog to Preferences,
wait for that tab and its layout before clicking a settings category.

Use `html_delay_ms` (integer 0–2000) with `html_mail=true` to inspect loading
without blocking iced. The HTML fixture adds a CSS-background report in Archive.
Save before/after captures and assert the body origin stays fixed. The horizontal
track now sits at the bottom of `html_body_visible`; at the parent scroll origin,
derive real drag coordinates from that observed rectangle. `html_pan_target` is immediate thumb intent and
`html_pan` is the painted position. Find editing keys must leave pan unchanged.
For quote controls, close Find, scroll the parent to the top, await geometry and derive the
button below `html_body_bounds`, rather than reusing a pre-render coordinate.
The compact Prototype flow asserts usable visible body space with all four
attachments and opens Forward to establish the files remain available.

For image-arrival scroll stability, start with `html_mail=true` and
`image_delay_ms=2000` (0–5000). Open Trash → Delayed illustrated report, allow its
images, scroll with real Page Down input and capture the reading position before
arrival. Both images add 400 logical pixels above the viewport; the scroll must
advance by that amount while the same paragraph stays visually fixed. Repeat in
compact dark mode with an active Find match. Observe pending downloads rather
than sleeping through navigation: `remote_image_pending == 0` plus cached bytes
proves the late results arrived after switching to another message. Verify that
the new message remains at the start with its own image policy.

`html_failure_once=true` rejects the first current Load in the fixture renderer.
Click the actual Retry formatted message button, verify the same subject returns,
and exercise Plain text/Formatted afterward. The normal fixture Retry is near
x=733,y=468. Preserve these four `test_html_*` automated equivalents and review
their before/after WebP captures. No personal mail or desktop processes are used.

The 120% scaling scenario uses Preferences → General → Interface size. At the
standard fixture size, open near x=1145,y=623; its menu opens upward and 120 is
near x=1140,y=509. Wait for interface_scale == 120 and html_view_current after
returning to Mail. Review both the scaled HTML and Mail/Calendar refresh icons.

For unread launcher badges, use `desktop.start(desktop_badges=true)`. This starts a
private session bus without service activation and observes Shep's actual LauncherEntry signals. The default
harness still has no session bus. Never point this fixture at the user's bus.
Observe `desktop_badge.count`, `visible`, `history` and `uri`; count history catches
brief regressions that a final-state assertion misses. `count_observed_ids` proves
that the new page snapshot includes the pending message even outside its folder.
These are observations only. Install `dbus-daemon` and `busctl` for this coverage.

Preserve the four saved `test_desktop_badge_*` scenarios: read and leave the folder,
background arrival and failure, archive/delete/move with Undo, and the preference.
Search Preferences for badge, open Mail & performance, then click the actual
checkbox near x=340,y=431 in the standard fixture. Wait for the private-bus zero
and hidden signal when disabling. The badge count always spans all mail accounts.
Review the preference/mail WebP captures; the private-bus observer does not render
a desktop dock, and its results must not be described as a GNOME visual test.

Preserve `test_filtered_preferences_do_not_leave_pixels_outside_scroll_view`:
open compact dark Preferences, search for badge, select Mail & performance, then
resize by one pixel and back. Capture before filtering, after filtering and after
the full repaint. Compare the empty bottom-margin pixels in the saved WebP
captures to detect stale dropdown text; the state oracle cannot prove clipping.
The pre-filter capture has a lossy-compression boundary next to visible content,
so its exact clipping is covered by `tests/software_rendering.rs` and visual
review. Do not substitute full-window redraws for fixing the renderer.


For list multi-selection, preserve the saved `test_mail_selection_*` scenarios.
Observe `mail_selection.mode`, `count`, `pending`, `visible`, `available` and
`list_focus`; these are state observations, never actions. Use real Ctrl/Shift
clicks, Ctrl+A, Select/Done and the row checkboxes. At the standard fixture size,
Select is near x=574,y=155 and the first two checkboxes are x=274,y=218/322;
their padded click targets also include x=260. Clear unchecks while keeping the
mode; Done/Escape exits. Wait for pending=false after captures or page changes.
Repeated Select All explicitly includes arrivals, while passive refresh preserves
exact membership. Scope/search changes clear immediately. Test reader/search
text selection, sidebar focus, cross-page ranges, checkbox gestures without
read-on-leave, and light/dark/compact layouts. After scrolling Shortcuts to the
bottom, Select all messages is y=480; Delete/Inbox/Find/Forward/Print retain their
prior bottom-relative positions. Preserve the saved bulk scenarios below alongside these controls.

For bulk actions, use actual Select/checkbox/Ctrl+A inputs before the preview
toolbar or keys. Observe `bulk.review_count`, `action`, `staging`, and the bounded
`jobs`/`items` result pages. Confirmations accept Y/Enter and cancel with N/Escape.
After Select, wait for `mail_selection.mode == true` and
`mail_selection.drawn == true` before clicking checkboxes. The latter observes
the current row widget draw epoch through test-support code; a controller reply
can arrive before the new controls draw. This is an observation, not an input
API or a frame-latency measurement. Avoid fixed sleeps for this transition.

Use `mail_actions="slow"` and wait for `bulk.jobs.0.running == 1` before Undo;
otherwise a test might only cancel unsent work. Forward plus inverse each carry
the fixture's 1.8-second delay, so the saved two-step completion wait explicitly
allows five seconds. This is functional fault injection, not a performance
budget. Immediate totals/toasts must be asserted before that completion wait.

The `unread` observation is the open message's boolean. For group counts use
`inbox_unread.<account>` and `mail_rows.<index>.unread`. First/second fixture rows
belong to the work account; the third is personal. A mixed-account Projects move
uses checkboxes near x=274,y=218/426. The Move default uses each original account.

Preserve the saved `test_bulk_*` flows: review/cancel/archive/Undo, flag/read/unread
and mixed-account Move, failure/History detail pages, and compact dark reviews and
pending Undo. At 1440x920, History is near x=1330,y=36; its first failed group is
near x=700,y=490 in the saved scenario. Inspect both per-message errors and the
red Trash confirmation. Compact toast Undo is near x=800,y=594. Engine/storage
tests separately establish persisted receipts, process-lock exclusion, restart,
graceful stopping and reviewed account cleanup; do not claim live-provider or
native restart coverage from an in-memory MCP workspace.

Keep consecutive checkbox clicks in the bulk scenarios. Their separate motion
events must target separate rows even when processed together; do not add sleeps
between clicks to mask cursor-batching defects. The root input wrapper has a
widget-level regression for that sequence, with redraws between motion and
press. Captured popup motion shares the root tracker; preserve dropdown and
interface-scale native scenarios when changing input dispatch.

Preserve the two arrival selection flows. `background_sync=true` supplies a
fictional incoming message through the normal cache refresh. Select an existing
row before it arrives, assert unchanged selection on arrival, then use checkbox
or Ctrl-click and a Shift range. Verify the ensuing bulk review count. No direct
state mutation may substitute for those inputs.

`mail_rows.<index>.group_pending` observes ownership used by row controls. The
slow group/individual conflict flow flags the second and third fixture rows,
checks the disabled second-row flag button and rejects a conflicting context-menu
action, then verifies ordinary flagging works after the group commits. Keep the
context menu open/keyboard path and screenshot evidence alongside engine/store
ownership tests.


For restart coverage, start with `persistent=true`. The harness owns a marked
`fixture.sqlite` beside that run's state file; the app seeds it once and never
opens an unmarked database in demo mode. `desktop.close` sends the native
WM_DELETE_WINDOW request and waits for that owned process to exit. If it does
not close, inspect the pending confirmation/work; never kill it implicitly or
launch a duplicate. `desktop.restart` reuses the owned display and fixture cache,
archives the previous state observation, and waits for a fresh loaded mail page,
including an empty Inbox. `crash=true` explicitly kills only the owned app.
Restart also works as a batch action: `{"type":"restart","crash":true}`.
The returned process IDs are observations. Never supply personal paths or PIDs.

The graceful group-close scenario inspects the fixture SQLite journal read-only
while the app is closed, then restarts it; this establishes the saved receipt
and queued remainder before startup recovery runs. Do not substitute state-file
writes or fabricated acknowledgments for close/crash input. `bulk_history=true`
seeds fictional completed groups and a paused group for pagination/Continue;
all subsequent actions use actual native controls. Saved History scenarios
exercise Undo retry, unconfirmed-result review and both kinds of pagination.

`bulk.history_jobs` observes IDs on the visible History page; `bulk.jobs` tracks
recent worker state separately. Use `history_loading`, `jobs_offset`,
`selected_job`, `items_after` and each item's `position` to verify page changes.
Scroll the actual History dialog to its footer before Older/Next/First page;
then inspect a screenshot proving the next page returns to the top. The saved
127-scenario suite includes compact dark mouse acceptance and formatted-reader
graceful restart. The empty-Inbox restart scenario archives all 120 fixture
messages and checks Archive after reopening, covering startup without a selected
message. Performance remains deferred.

For mail dragging, use `hover` to position the cursor, `mouse_down` to hold the
left button, then `hover`/`scroll`, assertions and screenshots before `mouse_up`.
These actions are batchable and restricted to the owned fixture display. The
harness releases held input on cleanup. Avoid nesting a complete `drag` action
inside a held gesture. Escape/right-click cancels in the app; follow with
`mouse_up` to release the physical input. Assert the selection remains unchanged
and no row/context/folder click leaked through cancellation.

Observe `mail_drag.active`, `count`, `target`, `account`, `valid` and `reason`.
These describe the actual gesture; they cannot start or complete one. Source
coordinates for the first/second standard rows are x=402,y=245/347. Archive is
x=85,y=399; personal Projects is x=95,y=636 before expanding unified Inbox.
Wait for confirmed selection membership (`mail_selection.pending=false`) before
dragging a selected group. Select All then Next page must still review all 120
fixture messages. Common folders preserve each source account; explicit account
folders require the enabled cross-account preference for a transfer.

Hold over a collapsed account or Inbox until the expanded observation arrives;
the actual widget uses a 600 ms dwell. With `long_folders=true` at 900×640, wheel
four ticks while holding over the sidebar, then hover the Japanese folder near
x=85,y=438. Capture the floating destination label and check its old shadow has
been erased near the Preferences footer. The saved scenario performs this pixel
check without a forced full repaint. Review all light/dark/compact/120% captures.
`pop3_account=true` changes only the personal fixture account: cross-account
transfers reject it, while moving its own mail between local folders works.

Preserve the ten saved `test_drag_*` automated equivalents: single source identity
and pending Undo; group review/cancel and mixed accounts; Escape/right-click/
outside/no-op cancellation; preference rejection/enabled transfer; hover reveal;
failure rollback and continued navigation; POP3 restrictions; compact dark and
scaled controls; sidebar scrolling/Unicode/shadow cleanup; and full cross-page
selection. These are functional fixtures, not live-provider or latency evidence.

For nested folder trees use `desktop.start(nested_folders=true)`, optionally with
`persistent=true` or delayed `mail_actions`. Work uses slash-delimited Projects
and Teams; Personal uses dot-delimited Home plus literal `Notes/flat.name` with
NIL delimiter. Projects holds mail and has children; Teams/ and Teams/Remote are
containers. Japanese labels retain an encoded server identity. Observe
`expanded_folders`, `saved_expanded_folders`, `sidebar_index` and `sidebar_rows`;
these are read-only, never an action interface. The sidebar group row's chevron
expands without opening mail. A Ctrl-click on a container must not add it to a
combined query or turn it into a drop target.

Keep all five `test_nested_folder_*` scenarios in scripts/e2e.py. They exercise
mouse and Left/Right/Enter, ancestor collapse, native close/restart, nonselectable
and flat folders, nested hover/drop/Undo, decoded Move search/review/toast labels,
Ctrl-selected parent/child folders, compact dark/120% layout and keyboard scroll
reveal. Wait for `focused_input == folder-search` before typing into Move, as in
other saved flows. Record WebP evidence: state cannot prove a keyboard target is
visible. The compact flow clicks the actual revealed row, and verifies saved
window dimensions across normal process restart. Do not infer live IMAP server
behavior or performance results from this fixture.

Use the batchable `paste` action with `text` for Unicode clipboard input. It owns
an xclip selection only on the fixture display, verifies the exact UTF-8 bytes
(including spaces/newlines), then sends native Ctrl+V. Cleanup terminates that
owned clipboard process. Synthetic xdotool typing of Japanese produced an empty
field intermittently; keep ordinary `type` coverage for ASCII and the saved
Unicode folder scenario's native paste/search/Enter checks.

Remember an action's source with the saved `selected_mail_subject()` helper:
it resolves `selected_id` against `mail_rows` in one observation. Startup page
readiness does not mean the reader body and its `selected` subject have loaded.
Keep action tests independent of that body load.
