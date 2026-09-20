# Browser immediate actions

This inventory follows the [approved contract](IMMEDIATE_ACTIONS.md). It describes
the existing owner for each mutation and the remaining adoption work. A visible
local result is separate from provider confirmation.

| Domain | Entry point and durable owner | Current boundary and remaining work |
| --- | --- | --- |
| Individual Archive, Trash, Move, read and flag | `Workspace.change` in `web/src/model.ts`; `BrowserIntents` in `mail_intents.ts`; `GatewayRepository.mutateWithReceipt` in `provider.ts` | Immediate field projections and input-order admission exist. The individual intent owner admits a metadata action record atomically, persists dispatch and acknowledgment separately, resumes safe queued work and exposes checked recovery in Activity. Offline waiting, durable projection, safe queued cancellation and receipt-based Undo use the same owner. |
| Group mail actions and Undo | `GroupUI`, `BrowserGroups`, `BulkJournal` and `BulkExecutor` | Exact captured membership, one dispatch owner, receipts, cache repair, partial outcomes and History remain authoritative. Group intent claims do not create individual activity records. Activity links to the existing History controls. |
| Draft edits, attachments and discard | `ui.ts` composer; `GatewayRepository.saveDraft`, attachment methods and draft IndexedDB transactions | Autosave and file identities remain under draft locks. Discard/send tombstones and current-editor ownership must survive any later coordinator adoption. Common admission/progress presentation remains open. |
| Send and Sent copies | `GatewayRepository.queueSend/send`, `outgoing` store, `sent.ts`, Outbox in `ui.ts` | Composer closes after atomic local draft/queue admission. Only unsent queued records automatically resume, with separate account capacity. Reserved submission identity and exact draft/files precede SMTP. Delivery uncertainty and Sent-copy failure retain their existing checked recovery. Activity links to Outbox; queued does not mean delivered. |
| Account connect/reconnect | `accounts.ts`, `GatewayRepository` account methods and `accountConnections` metadata | Form closes after saving a nonsecret attempt. Probes and checked activation run in the background; saved failure/interruption has Retry and Dismiss controls. Passwords remain in tab memory. Newer decisions, dismissal, profile close and account removal fence late activation. |
| Account removal | `account_removal.ts`, browser mail transaction and group removal journal | Reviewed local deletion is authoritative; no provider deletion. Reviews and the transaction now include individual action records, including records whose field intent already finished. Late result writes cannot recreate removed records. |
| Preferences and portable settings | `Workspace.savePreferences`, `profile_settings.ts`, `profile_publication.ts`, `profile_enrollment.ts` | Local values and field revisions commit in one authoritative settings envelope; the old settings key is a compatibility mirror. Publication retains its existing durable stopped/error/Resume controls. Failed local application cannot produce an enrollment receipt. Activity links to Accounts and profile sync. |
| Calendar edits | `ui.ts` event form and `Repository.saveEvent` | Preview has fixture saving; production gateway rejects unsupported calendar writes. Desktop optimistic calendar behaviour must not be claimed as browser provider parity. |
| Folder changes | Browser navigation and Move destination choices | Full provider folder create/rename/delete jobs are not implemented in this client. Future adoption must preserve physical mailbox identity and queued child dependencies. |
| Profile publication/import | Profile controllers, worker-owned shared history and local profile store | Exact staged requests and independent device history remain authoritative. Account application and credential activation retain their checked boundaries. |
| Print and downloaded attachments | Printing/attachment controllers and workers | Preparation and external browser/download completion retain their existing lifetimes. These operations must show progress without claiming provider or external success. |
| Backup/restore | No complete production browser equivalent | Remains a parity gap; no optimistic success is presented. |

## Individual action ownership

Mail database schema 13 adds `mailActions`, owned by `BrowserIntents`; schema 14
adds connection-attempt removal ownership. Ordinary
registration saves its explicit fields, lease revision and original physical
identity in the same transaction as field ownership. It stores no MIME, body,
password or OAuth data. Group claims continue to use their own journal and do
not add individual records. There is no second dispatcher.

Only a queued or waiting individual record may begin dispatch. The existing provider path
records Running first; replay of the same started lease is rejected. Its
acknowledgment callback saves the actual receipt before fallible cache work.
Receipt-bearing records retain Repair through subsequent errors. Successful
cache/intent completion retains at most 20 recent successful records for Undo;
unrelated success cannot
erase a different failure. Unknown results retain Uncertain, while interrupted
Running records remain explicit unfinished work, never automatic retries.

Each tab holds a profile-scoped Web Lock for its action owner. A replacement
tab resumes only Queued or Waiting records after proving the old owner has
released that lock. Adoption checks earlier unresolved work for the account;
dispatch checks the saved account configuration and physical source identity.
At most four accounts resume concurrently through the existing mutation method.
Disconnected IMAP work remains Waiting until reconnect or the periodic check,
including newly requested foreground changes.
A live tab's queued work cannot be stolen by another tab.

Activity returns at most 20 records plus one continuation key. Admission is
bounded at 128 unresolved records and rejects additional work visibly. The
existing safe cache repair operation is available for acknowledged receipts;
reviewed definite failures can be dismissed. Interrupted or uncertain work has
an explicit checked acceptance of current folder state. This retires only its
still-owned local intent, without inferring a provider outcome. Account and tab
locks refuse that decision while the original owner is active. Group History and Outbox retain
their own specialised recovery controls. Account removal includes these records
in its review and atomic deletion, and schema fencing closes older writers.

Mailbox queries and captured selections derive pending individual fields inside
their existing readonly source snapshot. They inspect at most 148 metadata
records and retain only fields still owned by that lease and physical lineage.
Waiting, running and uncertain desired state survives reload; this projection
does not claim server confirmation. Rejection, cancellation and checked
acceptance retire it. Cancellation compares the exact saved record atomically,
so a started operation cannot be cancelled as though it never ran.
An earlier unresolved operation on the same message field must finish or be
reviewed first; cancellation cannot revive an older superseded field decision.
Different fields can be cancelled while their predecessor remains active.

Recent changes exposes saved Undo. Inverse admission reserves a new revision in
the same transaction that verifies the original receipt and marks its Undo
decision. Only fields still owned by the original action are eligible; newer
choices survive. The inverse uses the actual acknowledged identity and the
existing dispatch path, so it can wait offline, recover after restart or retain
its own uncertainty. Unknown move destinations do not enable Undo.

## Send, connections and preferences

Send first saves the frozen draft and a local `queued` Outbox record in one
transaction. Existing draft locks protect its files and prevent concurrent
editing. Its composer closes with an explicit queued notice before reservation,
MIME preparation or SMTP. Queued work continues after reconnect/restart through
the existing Send method. At most four accounts run concurrently. Preparing or
submitting work is never automatically repeated; the original reservation and
wire receipt govern recovery. Returning a queued message to Drafts is local and
creates a fresh editable identity. Admission failure retains the open editor.

Connection attempts save account configuration, attempt identity and status,
never passwords. Activation rechecks that identity under the connection lock
after both probes, then commits account settings and retires progress together.
Dismissal invalidates an unfinished activation; failure preserves a previously
usable connection. Reload shows interrupted/failed attempts and requires password
entry before retry. Removal reviews include attempts and atomically clear them.
Activity reads at most 21 attempts to display 20 plus an overflow indication,
using these same records. Reconnect opens the existing credential controls;
dismissal compares the exact captured attempt and cannot retire its replacement.
Connection failures and individual mail failures remain independently visible.

Preference values and ownership revisions share one localStorage write. A failed
legacy mirror does not undo an admitted choice; failed authoritative admission
cannot be acknowledged as a profile application. The existing publication
journal preserves exact retries and durable errors independently of this device
save. No credential or mail action is imported through preference state.

## Verification and remaining rollout

Drafts retain a session-owned latest revision, save status and file request through
closing and navigation. Failed storage retains editable text and Retry; sign-out
waits for accepted saves and page leave warns about uncommitted work. Stable Drafts
rows preserve held pointer input through late failure. Attachment retry reuses
exact identities/bytes after a lost reply, while Use saved attachments reads the
current files before retiring a refused request. Successful Send and account
removal retire the session. Unadmitted text remains tab-local; checked cross-tab
text conflict review is still being implemented.

Focused tests cover atomic admission failure, restart retention, replay refusal,
receipt dominance, independent group ownership, bounded pages/admission and
account removal. Browser scenarios exercise actual Activity controls and retained
errors through unrelated work and reload, immediate admission rejection while
earlier work is held, acknowledged cache repair without provider replay, queued
continuation after tab loss, offline projection/cancellation, saved Undo after
reload and refusal to repeat interrupted dispatch. Desktop
and narrow dark Activity screenshots were reviewed. Exact execution evidence
belongs in the completion log after integration.

Remaining work includes common summary adapters for the remaining domains, shared WASM
lifecycle fixtures and cross-browser execution. Activity reconnect directs
users to the existing Preferences controls. This slice does not establish the 100 ms performance target, live
provider parity, whole-client memory bounds or mobile/Apple behaviour.
