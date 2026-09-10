# Browser group actions: review cleanup, Undo lifecycle and review keys

This page records the browser behaviour behind `web/src/bulk_journal.ts`, `bulk_executor.ts`, `bulk_client.ts` and `bulk_ui.ts` for abandoned reviews, the wider Undo lifecycle and the desktop-matching review keys. The operational rules stay in [AGENTS.md](https://github.com/sam-ruff/shep.so/blob/main/AGENTS.md); this page explains how the code meets them and where the evidence lives.

## Review ownership and cleanup

Every frozen review carries the `owner` token of the tab session that prepared it. `BrowserGroups` holds a Web Lock named `shep.bulk.tab.<profile>.<owner>` from `start()` until `stop()`, and the selection worker passes the token into `BulkJournal.prepare`. A review is abandoned when its job is `cancelled`, `interrupted`, or in `review` state without a currently held owner lock. Reviews prepared without a token (older data or direct journal use) count as abandoned.

Declining a review, closing its dialog, pressing Escape, or closing the dialog while preparation is still running calls `BulkJournal.cancel`, which fences the job as `cancelled` in one strict transaction; the owner's next wake removes its rows. A tab that closes with a review open releases its lock, so the next owner's startup or another tab's periodic sweep retires it. Lost staging becomes `interrupted` through the existing recovery step and is retired the same way.

`BulkJournal.sweep(limit)` runs only under execution ownership. Each strict transaction picks one abandoned job (cancelled and interrupted first, then reviews whose owner lock is gone, scanning at most 50 review jobs), fences it as `cancelled`, deletes at most 50 item rows and deletes the job record once its last page is gone. Runnable jobs, receipts, applied intent and pending cache repair are never touched, and no provider step runs. `BulkExecutor.run` sweeps up to four transactions before each wake's first step and before the recovery status is read for the new owner; `BrowserGroups` also runs a bounded sweep every 30 seconds between runs, skipping it while a review is being prepared or a run is active and ignoring a refusal when another tab owns the journal. A wake waits for an in-flight sweep so the two never contend for the lock in one tab. Worker projections treat `cancelled` like staging, so a retiring review is invisible to list queries.

## Undo lifecycle

The durable rules are unchanged: an Undo reserves a newer intent revision, a forward claim owns fields with an older revision, and an Undo claim owns only fields still owned by the group it reverses. The saved regressions now cover the wider lifecycle:

- Scope or page changes after the group ran: the prepared preview is used only when it matches the current view; otherwise the durable decision drives the worker projection and the restored rows and counts arrive with the next query. The unread count in the header always follows the Inbox.
- Approval queued behind an earlier group: a group approved while an earlier one is still sending stays `ready` behind it; Undo before its first step leaves every row `pending`, shown as cancelled before sending, and never calls the provider.
- Partial failure: Undo restores only acknowledged rows, a failed forward row stays failed, and Retry is refused for forward rows once the group is undoing.
- Overlapping groups: the newer per-field choice wins. Undoing the older group skips rows the newer group owns with a saved explanation; undoing the newer group restores its own receipt baseline.
- Unconfirmed results still pause the group; Undo and Resume never repeat the ambiguous step, and only the checked folder review retires it.

## Review keys

`web/src/shortcut_keys.ts` owns the remappable `approve` (default `y`) and `decline` (default `n`) shortcuts alongside Enter and Escape. In the group review, Enter or the approve key activates the primary control, which receives focus once the review is prepared; the decline key or Escape closes and retires it. In History, the approve key accepts the single checked folder review, the decline key or Escape first clears checked reviews and otherwise closes the dialog. Keys are ignored when the event was already handled, when a text field, editor or select consumes them, and for native Enter/Space on a focused control. Both shortcuts are excluded from mail browsing and from the formatted reader frame's forwarded shortcut list. Enter in the Move folder field opens the review rather than approving it.

## Evidence

- `web/src/bulk_journal.test.ts`: abandoned-review classification, bounded sweep transactions, untouched approved work and receipts, cancel refusals, aborted-transaction rollback and executor startup retirement, using the real journal on an in-memory IndexedDB.
- `web/src/bulk_undo.test.ts`: overlapping groups, queued approval, partial failure and unconfirmed results through the real journal and executor.
- `web/src/bulk_client.test.ts` and `shortcut_keys.test.ts`: tab lock lifetime, periodic sweep scheduling, refused sweeps and key mapping.
- `web/e2e/bulk-controls.spec.ts`, `bulk-recovery.spec.ts` and `preferences-controls.spec.ts`: real Chromium controls for declined, closed and live-tab reviews, review and History keys, remapping, and the Undo scenarios above.

Cross-account transport, large-group performance and native equivalents remain open.
