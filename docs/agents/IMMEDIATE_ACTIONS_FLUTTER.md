# Flutter immediate actions

This records the Flutter mutation owners and the current adoption of the common
action lifecycle. The target contract is in [Immediate actions](IMMEDIATE_ACTIONS.md).

## Mail

Individual Archive, Trash, Spam, Move, read and star controls enter through
`Workspace.action` and `Workspace.change`. The workspace applies a bounded
per-message projection synchronously, versions each changed field and serialises
changes for the same canonical message. A failed older action rolls back only
fields it still owns. `MoveFeedback` owns visible Undo and retains acknowledged
move recovery.

`NativeRepository.mutate` reserves a stable action identity before crossing the
Rust bridge. The Rust profile database is the durable owner. It records the
exact requested fields and current account with `mail_intents` in the admission
transaction, before credentials, account locks or provider capacity. The same
Rust operation remains the only dispatcher. It records queued, waiting, running,
succeeded, repair and uncertain states in `individual_mail_actions`; an
interrupted running action becomes uncertain on reopen and is never replayed.
The paged `mail_actions` request returns at most 50 records per Activity page.
Startup resumes only queued or credential-waiting records
with the same action identity. Running records become uncertain before any UI
owner can inspect them. The mail and reader banners show waiting work, allow
pre-dispatch cancellation and retain rejected, repair and uncertain reviews.
Mail Activity exposes bounded history, provider inspection for uncertain moves
and Undo. Undo derives the inverse from the Rust-owned physical baseline and
runs through the same executor with a stable action identity.

IMAP mutations continue to validate the active credential slot under the account
lock. Moves retain `pending_moves` and `move_receipts`; an acknowledged provider
write followed by a cache failure becomes repair. A provider result that was not
saved becomes uncertain. Account removal fingerprints these records, requires an
explicit unresolved-work decision and deletes them only with the committed local
removal.

Bulk mail already has its own authoritative Rust journal in `groups.rs`. Selection
freezes exact membership in SQLite and the group executor owns dispatch, receipts,
Undo and uncertainty. It must not be copied into the individual action table.

Schema 17 preserves per-message lineage through acknowledged moves and proved
Sent aliases. Other physical/content replacements invalidate it. Provider claims
recheck durable status and field ownership under the account lock; duplicate
requests cannot dispatch twice. Mutable read/star values are not physical
identity. SQLite applies pending fields to mailbox queries and reader metadata,
including after reopening, without copying the action journal into Dart.

Individual provider receipts commit before cache work. Acknowledged repairs
survive restart and apply only to proven physical lineage and still-owned fields;
they cannot overwrite a replacement or move the cache behind a later confirmed
move. Cache-only repair requires no credentials and never repeats the provider
write. An acknowledged move without a destination UID retains Repair until a
read-only inspection proves the identity, including after cache failure/restart.
Undo uses that resolved dispatch identity. Definite typed refusals reject once;
pre-dispatch credential failures remain Waiting.

Claims save the still-owned subset of requested fields atomically with Running.
Newer choices can supersede one field while the others continue. Receipt repair,
inspection, Activity and Undo use this accepted subset; the original request
remains immutable for deduplication. Remote moves combined with flag changes are
rejected before admission because they require separate provider receipts.

## Other owners

| Domain | Current owner | Adoption gap |
| --- | --- | --- |
| Draft text, files and discard | Rust draft tables and revision checks | Session-owned text/error and Retry survive navigation; revision and attempt ownership reject late saves after Send/discard/removal. Suspension flushes pending text. Forced process loss before local admission remains uncommitted. |
| Send and Sent filing | `outgoing`, `outgoing_meta` and `outgoing_sent` | Queued admission returns before delivery, freezes account/credential binding and retains the draft on queued cancellation. Resume reuses its attempt; silent bounded Outbox refresh preserves review choices and errors. Common attention and scheduling fairness remain open. |
| Accounts and reconnect | Schema 18 credential-slot journal, account FIFO and removal tombstone | At most 32 nonsecret attempts admit before probes; matching forms close, visible activity offers Cancel/re-entry, and activation remains checked. Restart requires passwords. Execution generations and removal fences reject delayed results. |
| Preferences | Dart `SettingsStore`, then profile history | Owned save error/status and visible Retry retain local values through navigation and unrelated mail failures. Field intent and profile publication keep their existing owners. |
| Calendar | Dart repository calls | Add durable native admission, ETag conflict ownership and uncertain-result review. |
| Folder changes | No complete Flutter physical-folder owner | Implement provider-backed folder journals before projecting these actions. |
| Profile publication and sync | Dedicated Rust publication, history and sync journals | Adapt their existing phases; never create a second runnable queue. |

## Remaining mail work

- Admission precedes the Dart provider queue; local rejection settles without
  waiting for an older provider request. Queued Undo cancels its durable request.
  Activity returns at most 50 records per page and retains recovery errors.
  Production input carries the observed lineage token; replacement rejection
  and proven alias acceptance have native and Workspace regressions.
- Restart scheduling uses indexed 50-row cursor pages and at most 32 requests,
  with one action per account in a complete sweep. Waiting accounts do not block
  later accounts. Failed page reads stop automatic pumping and expose Activity
  Retry; disposal fences a delayed page. A 100,000-row database-step test verifies
  late-page seeks without sorting. Broader scheduling and suspension remain open.
- Live IMAP verification remains for the bounded flag inspection command. Unit
  coverage uses the provider trait and proves inspection never repeats a write.
- Add live IMAP, suspension and Apple evidence. Current coverage uses the actual
  Rust bridge/database boundary and mocked provider tests.
- Extend the shared `action-core` policies as more Flutter domains adopt the
  envelope. The Rust bridge already validates persisted mail states with its
  shared status type and remains an adapter around the existing executor.
