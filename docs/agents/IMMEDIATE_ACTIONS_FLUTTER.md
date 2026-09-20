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
The bounded `mail_actions` request returns at most 50 recent records for activity
and recovery UI work. Startup resumes only queued or credential-waiting records
with the same action identity. Running records become uncertain before any UI
owner can inspect them. The mail and reader banners show waiting work, allow
pre-dispatch cancellation and retain rejected, repair and uncertain reviews.

IMAP mutations continue to validate the active credential slot under the account
lock. Moves retain `pending_moves` and `move_receipts`; an acknowledged provider
write followed by a cache failure becomes repair. A provider result that was not
saved becomes uncertain. Account removal fingerprints these records, requires an
explicit unresolved-work decision and deletes them only with the committed local
removal.

Bulk mail already has its own authoritative Rust journal in `groups.rs`. Selection
freezes exact membership in SQLite and the group executor owns dispatch, receipts,
Undo and uncertainty. It must not be copied into the individual action table.

## Other owners

| Domain | Current owner | Adoption gap |
| --- | --- | --- |
| Draft text, files and discard | Rust draft tables and revision checks | Expose common saving and failure activity without duplicating draft ownership. |
| Send and Sent filing | `outgoing`, `outgoing_meta` and `outgoing_sent` | Adapt existing delivery and append phases to the common status envelope. |
| Accounts and reconnect | Credential-slot journal, account FIFO and removal tombstone | Surface preparation, activation and cleanup activity while preserving checked activation. |
| Preferences | Dart `SettingsStore`, then profile history | Add field-level common status while retaining unsaved edits and publication ownership. |
| Calendar | Dart repository calls | Add durable native admission, ETag conflict ownership and uncertain-result review. |
| Folder changes | No complete Flutter physical-folder owner | Implement provider-backed folder journals before projecting these actions. |
| Profile publication and sync | Dedicated Rust publication, history and sync journals | Adapt their existing phases; never create a second runnable queue. |

## Remaining mail work

- Add a dedicated paged Activity screen. The current mail-route banners expose
  the first bounded waiting and review records.
- Post-receipt Undo must use the saved move identity. Uncertain actions still
  require a provider inspection flow beyond the current Refresh/Review control.
- Add live IMAP, suspension and Apple evidence. Current coverage uses the actual
  Rust bridge/database boundary and mocked provider tests.
- Extend the shared `action-core` policies as more Flutter domains adopt the
  envelope. The Rust bridge already validates persisted mail states with its
  shared status type and remains an adapter around the existing executor.
