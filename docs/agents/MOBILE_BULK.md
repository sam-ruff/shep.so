# Mobile group actions: captured selection and the durable journal

This page records how the Flutter client mirrors the browser's captured selection and durable group execution ([Browser group actions](BROWSER_BULK.md)). The operational rules stay in [AGENTS.md](https://github.com/sam-ruff/shep.so/blob/main/AGENTS.md); this page explains where the native code meets them and where the evidence lives.

## Selection controls

`flutter/lib/model/mail_selection.dart` remains the bounded controller: one request in flight, at most 32 queued gestures, 50 observed rows, explicit arrival recapture, aliases, stale and lost observations, and release. The membership itself lives in the native SQLite capture (`flutter/rust/src/selection.rs`); nothing in Dart holds a whole inbox.

`Workspace` now owns the controller (`selection`) and feeds it the current scope (folder, account, query, filter, order and the pending individual edits). Every rendered `MailTile` registers itself as an observation while it is mounted and withdraws when it leaves the list, so observations follow the visible page only. Changing folder, account, filter, order or query ends selection mode; a refresh re-observes the rendered rows and arrivals stay unselected until they are chosen explicitly. Choosing an arrival that lies outside a prior capture extends that capture without discarding the existing selection.

Controls, all visible and labelled: the Select button beside Search enters and leaves selection mode; the toolbar shows the count (`All 130 selected`, `3 selected`, `No messages selected`), Select all, Clear and Done plus the seven group actions. Rows show a desktop-style 16 px checkbox (4 px radius, subtle border, accent fill) labelled `Select <subject>`/`Deselect <subject>`, and their semantics label ends with `selected`/`not selected`. A tap in selection mode toggles the row without opening the reader; a long press enters selection mode or, once an anchor exists, extends the range from the anchor like Shift+click. The row's Actions menu carries `Select up to here` as the visible equivalent of that gesture, and swipes are disabled while selecting. Selection never reads a message body and there are no keyboard shortcuts on mobile to consume.

Select all is a captured `all` gesture on the SQLite capture; it never loops over loaded rows. On the web preview the count clamp had to become a literal bound because dart2js shifts are 32-bit.

## Durable journal

`flutter/rust/src/groups.rs` stores group work in the profile database (schema25) in dedicated tables: `group_jobs` (action, approved fields, state, scope, approval and Undo revisions, revision counter), `group_items` (frozen position, cache id, account, physical folder and UID, baseline flags, state, claim attempt, receipt, reason), `mail_intents` (per-message per-field revisions reserved by individual actions) and `group_clock`. The tables hold metadata and physical identities only, never MIME or secrets. Keeping them beside the mail cache lets each receipt and its cache write commit in one transaction under the writer FIFO; the ephemeral TEMP capture stays separate.

Commands travel through the existing `request` bridge as `{"op":"groups","command":{...}}`:

- `prepare` freezes the live capture at its expected revision (`selection::Freeze`), copies it in pages of 50 into `group_items` with each message's current physical identity, records rows that are no longer cached as skipped so the review count is exact, releases the temporary frozen copy and leaves the job in `review`. The original capture stays with the selection controller through failure or dismissal. A failed page marks the job `interrupted`; interrupted preparation cannot execute.
- `inspect` returns the exact saved job in one read transaction. It cannot approve, recover a live owner or execute work. Lost preparation or approval replies use this identity before another decision.
- `approve` verifies every account still exists, bumps the group clock and makes the job `running`. From that moment the paging plan projects the job's fields over its pending items (newest job first per field, individual edits above both), so the list and its counts paint before any step runs. `decline` retires the review in 50-row transactions.
- `step` executes at most one owned step under `Operations.groups`. Removed/missing sources, replaced lineage, superseded fields and proven already-applied values become distinct skips. Claiming atomically saves the group attempt UUID and a subordinate existing mail-action row, original approval, accepted fields and dispatch identity. Only the group owner can dispatch or repair this row; public individual routes and history collection exclude it. Provider acknowledgement is persisted before the cache transaction. The group receipt takes its actual destination/UID from that acknowledgement, never a cache reread. A `repair`/`undo_repair` pauses its own group, so that group makes no further provider claim until its repair completes; an active group repairs it before any other step, while a paused one waits for its own Retry, Resume or Undo. Other groups keep running. Unknown UIDs use exact read-only receipt inspection; unknown MOVE or flag replies remain `uncertain`. Only typed proven refusals allow explicit Retry. IMAP dispatch retains the frozen review connection and saved credential slot across capacity/account waits. A child succeeds only with a saved receipt: a credential request becomes a retryable failure and an already-placed message a skip, with fixed public text and no provider call. Never-sent children are cancelled rather than left waiting.
- A matching cached flag skips as already applied only when the cache provably reflects the server: no unknown legacy completion and no running, unsaved-acknowledgement or unconfirmed action on that field. Otherwise the step sends its explicit idempotent write, and only that write's cache completion records the field. A message whose earlier move is unsaved (`pending_moves`) defers the step as a retryable failure without provider work, because its cached folder and UID are stale. Pages project repair items with the fields they claimed under the same ownership rules, so acknowledged changes do not reappear.
- `undo` records a separate decision revision, cancels unstarted children locally and queues actual acknowledged identities for inverse work. It retains the original approval cutoff; a new Undo clock does not acquire newer fields. Late acknowledgements join the same inverse queue after their cache handshake. `mail_intents.applied_revision` records actual cache completion separately from reserved intent. An older flag ACK can repair its physical baseline while a pending newer choice remains projected; it cannot overwrite a newer completed field. Proven same-value choices reserve ownership during bounded item completion, including after their History record is removed.
- `pause` cancels only queued/waiting children and retains original pending membership for Resume; a running provider claim may finish its receipt. `retry` covers typed failures and acknowledged cache repair, retaining the exact repair attempt. Persistent cache faults return idle and do not repeatedly re-wake from History. `accept` retires only unconfirmed or unsaved local intent without classifying provider success; for a repair it keeps the saved acknowledgement and does no provider or cache work, so an unrepairable receipt never leaves its group stuck. `history` returns 20 groups and `items` 50 rows; explicit `remove` drains at most 50 actual child rows/receipts per transaction before deleting bounded parent membership.

Restart checks the saved child attempt first: acknowledged `sending`/`reversing` work becomes `repair`/`undo_repair`; a child still queued or waiting was never sent, so its item returns to the queue; any other claim without acknowledgement becomes `uncertain`/`undo_uncertain`. Staging remains interrupted and abandoned reviews retire in bounded transactions. Unknown provider mutations are never replayed automatically. Preparing a new review retains completed receipts and refuses a twenty-first active group. Schema27 fences older cache writers before relying on group child ownership and applied-field versions. Migration keeps old bytes and revisions, leaves applied completion at unknown zero and does not invent lineage or connection proof for old reviews.

Migration retains each observed old field revision as `legacy_revision`, an
unknown ownership fence rather than cache completion. Cancellation cannot erase
it, and an older ACK cannot overwrite it. A new explicit flag choice may need
one actual dispatch even when the cached value already matches; only its cache
completion establishes a known baseline. Proven alias folding keeps cache values
consistent with newer completed or legacy ownership while pending input remains
projected separately. Conflicting replacement bytes still refuse the merge.

Account removal takes `Operations.groups` before the account lock so no owned step can dispatch or write a receipt meanwhile; the removal review counts the account's queued, in-flight, failed, uncertain and inverse work as `groups`, requires the explicit discard confirmation for them, cancels those items, abandons reviews that froze the account and leaves completed receipts alone.

## Flutter controller and controls

Approved groups offer Undo for pending or sending work before any acknowledgement,
including the paused notice and History. The local transaction cancels pending
items, removes forward projection and keeps sending items until their actual
receipt can join the inverse queue. Newer individual fields remain authoritative.
Same-group native Undo retries return its current status without advancing the
original Undo revision. Dart retains at most 32 exact decision requests plus one
latest capacity rejection, and inspects unknown outcomes before any retry. A
dedicated Retry Undo notice survives History close and unrelated pump errors.
Explicit removal clears only that group's retained request; held replies cannot
recreate its state or feedback. Explicit Undo/Resume/Retry wakes received during
the pump's final History read start the same owner again after it becomes idle.

`model/group_history.dart` owns one 20-group page and one 50-item page, with
visible Newer/Older groups and Previous/Next messages controls. Indexed native
cursors report exact last pages and retain surviving boundary rows after
removals. Details keep their exact group, item and page generation through
delayed Retry/Accept, page changes and same-group re-entry. Group pages and detail
pages each keep one running read and its latest replacement with a shared idle
future. Command revisions reject old page results and errors after removal or
newer decisions. Detail failures keep their scoped Retry
above the scroller while group controls remain available.

The existing `MailGroups` owner observes active/attention records separately from
the displayed page, including exact older targets. Its iterative observation
loop has the same command fence and coalescing bound.
Accept, Retry and Remove refresh this independent attention snapshot through the
same coalesced owner; accepting checked state still does not claim server success.
Status indexes keep
completed history out of admission and next-step seeks; each active group offers
one indexed eligible item before the next-step choice. This does not introduce
another dispatcher or authorise replay of an uncertain provider step.

Attention counting still visits matching failed/uncertain entries, and each
group summary aggregates its members. Indexed completed-history seeks do not
establish a fixed cost for large attention sets or large active groups.

`flutter/lib/model/mail_groups.dart` drives the journal: it prepares the review from the controller's snapshot, approves or declines, pumps steps until the journal is idle or paused (coalescing repaints to one per 250 ms and refreshing History every ten steps), announces completion for six seconds, and exposes Undo, Pause, Resume, Retry, Accept, Remove, History and item pages. `NativeRepository.groupStep` resolves `requires_credentials` with the device credential store for one step. Saved groups recover at startup: runnable groups continue, paused groups wait in History.

`flutter/lib/ui/mail_groups.dart` holds the selection toolbar, the frozen review dialog (counts per account and folder, Cancel and the action verb), the progress notice with Pause, Undo and a History shortcut, the paused/attention notices, the completion toast with Undo, and the Group History screen (20 cards with status, Pause/Resume/Undo/Remove, expandable 50-item pages with Retry and Accept current state). Everything uses the theme tokens and `ShepIcon`s; there are no Material switches or elevation.

## Evidence

- `flutter/rust/src/groups_tests.rs`: exact 125-row staging and painting, newer-intent and already-applied skips, Undo with cancelled and reversed items, credential requests, unconfirmed pauses that never repeat, explicit retry of definite flag failures with inverse receipts, restart classification and abandoned-review retirement, the removal fence with a held provider step, the 20-job and 50-item bounds. `groups_tests/receipts.rs` and `groups_tests/repair.rs` cover acknowledged cache gaps, per-group repair gating and Accept, repair projection, unproven same-value skips, unsaved earlier moves, credential replies and never-sent attempts after restart.
- `flutter/test/bulk_receipt_native_controls_test.dart`: the real library behind History controls for a persistent cache fault, Undo, Retry and Accept current state, with light and dark repair goldens.
- `flutter/test/mail_groups_test.dart` (controller over the synthetic journal), `flutter/test/bulk_controls_test.dart` with `test/support/bulk_controls_scenario.dart` (real controls in light and dark: checkbox, long-press range, Select all, review counts, decline, immediate paint, Pause/Resume, Undo, injected failed and unconfirmed steps, History retry and acceptance) and the FFI journal case in `native_repository_test.dart`.
- `flutter/integration_test/bulk_android_test.dart` through `scripts/clients/android_e2e.py --bulk-only` (`android_bulk_fixture.py` hands over a 130-message POP3 profile; the native SQLite journal executes, undoes and survives reopening) and the Appium flow `flutter/e2e/bulk_native.mjs`.
- `flutter/e2e/bulk.mjs` through `scripts/clients/flutter_web_e2e.py --bulk` against `test/bulk_main.dart`.

`test/support/group_repository.dart` is the synthetic journal used by previews and host tests; it mirrors the Rust contract but is not the production path.

## Limitations

Queued successors retain strict frozen folder/UID checks even when captured
lineage proves an earlier acknowledged move. Exact same-account successor
adoption is tracked in #66 after the receipt/destination prerequisites. This
conservative skip does not authorise unknown MOVE replay or cross-account work.

The upgrade regressions simulate schema26 by removing columns from a current
database; a fixture built from the real released schema26 DDL is not committed.
An accepted unsaved acknowledgement keeps its child in repair, so later groups
on that field keep sending explicit writes rather than trusting the cache.

Review keys are a browser and desktop matter. Each IMAP step opens its own provider session, so large IMAP groups remain slow; destination planning #48, cross-account moves, large-group performance, Apple and live-provider execution remain open. The live capture remains available through preparation and declined or dismissed reviews. Approval releases only its matching capture after durable confirmation. Group intent paints after the saved decision; acknowledged cache gaps repair through the existing owner and do not authorise another provider mutation. Real FFI/control fixtures remain separate evidence from mocked wire acknowledgement and live providers.
