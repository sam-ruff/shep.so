# Immediate actions and background synchronisation

Status: approved and being implemented, 20 September 2026. This describes the
target architecture; adoption across the app is incomplete. Work remains tracked in
[TODO](https://github.com/sam-ruff/shep.so/blob/main/TODO.md).

When someone deletes ten messages, the review should use local selection data
without waiting for earlier server requests. After confirmation, the messages
disappear, counts update and Undo appears immediately. Shep saves the request
locally and completes the server moves in the background. If two messages
cannot be moved, only those two need recovery; the eight successful moves stay
complete. Browsing and editing continue throughout.

Apply that interaction contract across mail, folders, calendar, drafts, accounts
and preferences. The successful local result is visible immediately; remote
confirmation remains a separate fact.

## Approved architecture

Introduce a common action lifecycle with domain-specific execution and recovery.
Every mutation enters through a local action coordinator. UI handlers submit
intent and render projected state; they do not orchestrate provider calls.

| Part | Responsibility | Existing code to build on |
| --- | --- | --- |
| Immediate projection | Paint the requested change and feedback using bounded cached metadata | `src/ui/mail_actions.rs`, `src/ui/bulk.rs` |
| Durable admission | Save identity, intent, dependencies and recovery data before any remote write | `src/store/bulk.rs`, `src/store/outgoing.rs`, mail and folder journals |
| Background execution | Claim eligible work, enforce account ownership, perform provider steps and save receipts | `src/engine/dispatch.rs`, `src/engine/account_work.rs`, `src/engine/bulk.rs` |
| Reconciliation | Combine confirmed data with current local intent; handle failures, Undo and incoming sync | `src/store/write_ledger.rs`, `src/mail_actions/runner.rs`, browser field intents |
| Activity and recovery | Present pending work, grouped errors and applicable recovery actions | Existing toasts, bulk History, Outbox and recovery controls |

Desktop individual controls now submit observed message identities through the
local selection writer into the existing group journal. Their temporary overlays
retire when a page observes the saved revision. Bulk reviews and confirmations
use bounded local observations, independently of provider waits. Calendar and
prepared Send requests also have durable admission. A bounded shared owner
schedules mail, folders, calendar and Outbox, with account reservations and
priority for acknowledged cache repair. Stop acknowledgements identify their
close request. The client inventories record remaining domain and recovery gaps.
`shared/action-core` defines common projection retirement, rejection, uncertainty
and cache-repair states; domain journals remain the durable owners.

## Action lifecycle

At input, perform cheap local validation, assign a stable action ID and apply a
temporary projection in the same UI update. No SQL, credentials, MIME parsing or
network work belongs in that update. A full local command queue rejects the
request visibly without leaving a false projection.

An independent background writer admits the action durably. Save the intent and
its local ownership together, then publish an admission revision. The UI retires
its temporary projection only when its read model includes that revision, so the
change is neither counted twice nor briefly undone. Before admission, feedback
means “saving”; after admission it means “waiting to sync”. A crash before the
local commit cannot be promised durable recovery. Normal close flushes accepted
local writes, and local save failures retain actionable recovery.

Only admitted work may reach a provider. Record dispatch before sending, and
persist the provider result before declaring completion. Keep provider success
and local cache application separate: an acknowledged change with a failed cache
update enters repair, never a fresh send or automatic rollback.

Use a common status envelope for saving, queued, running, waiting, complete,
failed, needs review and cancelled. Domain records retain their detailed stages,
including partial transfer and SMTP receipts; the common envelope must not erase
those distinctions. Status summaries are derived from the owning journal rather
than becoming another independently writable source of truth.

## Data and ownership

Keep one authoritative durable owner for each action. Reuse current journals
through adapters first. A shared activity index may reference their records, but
must not become a second runnable queue for the same provider operation.

The common contract needs:

- A stable action ID, schema version, profile binding and optional parent group.
- Explicit desired fields, local intent revisions, dependency IDs and the base
  versions needed to detect conflicts. Store “set read”, not “toggle read”.
- A logical entity reference plus verified physical identity/lineage. IMAP UIDs,
  account connections and calendar ETags remain domain-owned constraints.
- Dispatch ownership, attempt identity, retry eligibility and receipt reference.
  Local command deduplication does not make a remote operation idempotent.
- Correlated errors and recovery policy. Secrets stay in secure storage; action
  records refer to credential slots. Bodies, files and draft recovery data stay
  in their existing protected stores, not the shared activity index.

Use typed domain payloads and injected storage/provider traits. A proposed
`shared/action-core` crate should contain pure lifecycle, ownership and
reconciliation rules, with shared fixtures. Platform storage, scheduling,
credentials and provider execution stay outside it. Flutter uses its Rust
bridge; browser consumers use the same contract through WASM where suitable.

On native SQLite, admit related intent and local state atomically where they
share a database. The browser already separates its group journal from mail
IndexedDB. Preserve that explicit receipt/cache handshake; do not pretend there
is a transaction spanning both databases. Use stable admission IDs and recovery
for any unavoidable boundary, and keep incomplete bulk staging non-executable.

## One projected view

The visible state is the confirmed baseline plus outstanding durable intent,
plus any newer input awaiting local admission. Rows, folder totals, unread
counts, badges and readers must use that same revision and ownership rules.
Large memberships and aggregate calculation remain in indexed background
storage. The UI retains one metadata page and bounded temporary changes.

Incoming sync updates the confirmed baseline without overwriting outstanding
intent. A failed old action removes only its own projection, then recomputes the
view with newer edits still applied. Never restore an old whole-message, event
or preferences snapshot. For example, an old failed “mark read” must not undo a
later flag or move.

Dependent work is revalidated when a predecessor fails. A move into a folder
whose creation failed pauses with a dependency error. An independent flag edit
can survive a failed move once its current physical source is verified. Receipt
identity changes must rebind later work before dispatch.

Incoming observations carry their scope and causal checkpoint. A stale listing
must not undo a newly acknowledged write; later verified changes from another
device must still become visible. Extend the current write-ledger protection
with durable ownership and provider evidence, rather than relying solely on a
time-based grace period or inventing a global cross-device clock.

## Failure and Undo rules

| Outcome | UI and recovery |
| --- | --- |
| Local admission fails | Withdraw only this action's temporary effect; retain entered content and offer retry. |
| Offline or known transient failure before application | Keep the requested local state, show waiting status and retry with bounded backoff. |
| Authentication required | Keep the pending action and offer reconnect; do not trigger interactive login silently. |
| Definite permanent rejection | Recompute from the confirmed baseline plus newer intent; explain what failed and offer the appropriate retry/edit action. |
| Server result unknown | Show “needs checking”; retain original recovery data and inspect the server. Never guess that it failed or blindly repeat it. |
| Server acknowledged, local cache update failed | Keep the receipt and repair the cache without repeating the remote operation. |
| Partial group failure | Preserve successful items, recover failed items individually and show a grouped summary. |

Preserve the existing explicit device-only fallback for refused mail moves.
Those rows stay locally moved with an unsynchronised warning and retry/review
controls. That policy was previously requested; this proposal does not silently
replace it with automatic rollback. Other definite failures use conditional
rollback as described above.

Undo is a newer user decision. Before dispatch it cancels the queued effect;
after acknowledgment it schedules an inverse using the actual receipt identity.
During an unknown outcome it records the Undo request and waits for verification.
Handle Undo racing local admission through the same ordered admission path, so
it cannot address a job that does not yet exist. Do not offer Undo for effects
the provider cannot reverse.

## How this applies across the app

| Area | Immediate result | Domain boundary to preserve |
| --- | --- | --- |
| Mail flags, move, Archive and Trash | Indicators, location, list counts and Undo update together | Per-field ownership, actual source/destination identities and exact group membership |
| Folder create, rename and delete | Project the tree after required review | Child-operation dependencies, physical mailbox mapping and destructive scope |
| Calendar create, edit and delete | Update the grid and agenda; preserve edited content for recovery | Permissions, stable create identity, ETags and conflict review |
| Preferences | Apply locally, save and publish portable changes in the background | Field-level intent; remote sync failure does not revert a valid local preference |
| Draft edits and discard | Update the editor/list; keep recoverable content until local admission | Autosave revisions, attachments, tombstones and send/discard exclusion |
| Send | Close the composer into a visible queued/sending Outbox item | Never label queued mail “sent”; preserve exact submission identity and uncertain SMTP recovery |
| Account connect/reconnect | Show the saved request and testing/connecting status | Credentials and endpoint validation finish before activation; never claim authentication succeeded speculatively |
| Account removal | Hide the reviewed account while local removal commits | Tombstones, provider exclusion and credential cleanup; server mail remains untouched |
| Backup, import, restore and external operations | Immediate queued/progress feedback and continued navigation | Do not present an unfinished backup, profile switch, print or irreversible operation as completed |

Once account removal commits locally, credential cleanup failure never restores
the account or its mail. Cleanup remains a separately retryable job. Likewise,
keep the last verified connection active until a reconnect has passed checked
credential activation.

Required confirmation reviews remain. They prepare from local metadata and do
not wait for provider completion. Large Select All still freezes exact captured
membership off-thread; never substitute the currently loaded rows. Navigation,
search and other read operations continue to use their existing cached-read
paths rather than becoming durable mutation jobs.

## Scheduling and lifetime

Retain bounded queues, separate read/persistence/provider capacity and the
existing account coordinator. Order conflicting writes and their dependencies;
independent accounts proceed concurrently. Coalesce only undispatched replaceable
field edits. Never coalesce away an attempted send, receipt or uncertain move.
Batching remote requests is an optional later optimisation, not a requirement
for immediate feedback.

Queued durable work survives reopening. On exit, stop claiming work, flush local
admission and preserve the active provider step's receipt or uncertain state;
do not wait for the whole queue to finish. Desktop tray execution can continue
when enabled. Browser tab closure and mobile suspension may pause execution;
resume from the durable journal when the platform permits. Fence removed
accounts, changed credentials, stale tabs and switched profiles before dispatch.

Use existing toasts for immediate feedback and a common activity view for
waiting or failed work. Ordinary success stays quiet. Errors survive toast
expiry and offer the applicable Retry, Reconnect, Review or Undo action. A
timeout in one account must not block unrelated work or dismiss its errors.

## Migration and verification

Start with individual and bulk mail actions, using the ten-message delete as
the first acceptance case. Reuse existing move journals and provider adapters;
introduce shared admission and projection ownership, then remove the global
pending-action review barrier. Retire the individual and bulk double counting
paths together. This first slice must prove immediate review, confirmation,
Undo and failure recovery while an earlier read operation is deliberately held.

Next adapt folder and calendar actions, then preferences/account lifecycle and
Outbox activity. Keep specialised send, credential and transfer execution behind
their current recovery protocols. Track desktop, Flutter and browser behaviour
in the parity matrix and shared client scenarios at every migration step.

Migrate one domain at a time. Each record has exactly one dispatcher throughout
upgrade. Old pending and uncertain records remain recoverable; version fences
prevent older writers from executing an incompatible schema. Never promote a
historical unknown outcome to a fresh queued action.

Acceptance requires real input-to-visible feedback within the existing 100 ms
target, alongside the existing handler, page and memory budgets. Measure review
readiness and confirmation feedback separately from server completion, including
ten selected rows in a 100,000-message mailbox. Held-provider native and browser
tests must prove continued navigation, stable counts and visible errors.

Use pure transition tests and mocked provider boundaries for later edits versus
old failures, pre-admission Undo, partial groups, stale sync, disk failure,
dependency rejection and receipt/cache repair. Add restart tests at every
admission/dispatch/acknowledgment boundary, plus tab loss, mobile suspension,
profile switches and removal. Review native/browser screenshots; report live
provider and Apple coverage separately. Implementation evidence belongs in the
completion log; this architecture does not establish timing or platform results.

## Approved decisions

- “Immediate” means visible local feedback within 100 ms; remote completion has
  its own progress and failure status.
- Ordinary reversible actions keep the requested state while offline; permanent
  rejection removes only the failed effect, subject to existing domain policy.
- Existing confirmation, permission and credential activation boundaries remain.
- The action contract is shared across clients, with platform storage and
  specialised provider recovery behind adapters.
- Roll out mail first and preserve existing journals; an app-wide replacement in
  one release is unnecessary and would make recovery harder to verify.

## First migration and remaining work

Desktop mail review freezes and summarises selection in one local transaction.
Confirmation projects the group immediately while prior individual writes finish;
Undo before admission releases the frozen review without creating a provider job.
Once admitted, the existing bulk journal owns execution and recovery.

Calendar save/delete uses correlated requests and the same projection outcomes.
Definite rejection restores the confirmed view while retaining editable recovery
content. An unknown result needs a read-only server check before adopting its
state; an acknowledged cache error must never cause another mutation.

Calendar actions now have durable admission, startup recovery and receipt-first
cache repair through the existing serial worker. Normal close flushes admission
and the active provider step, preserving queued work for reopening. Database
imports retain acknowledged cache repairs and fence unattempted work for review.
Pre-dispatch offline/authentication failures remain Waiting with visible Retry
and Cancel; dispatched timeouts retain uncertain recovery.

Desktop Send saves an exact draft/account snapshot, attachment identities and a
reserved Message-ID as Preparing through the local persistence lane, then closes
the matching composer. The existing durable worker prepares MIME before a checked
transition to Queued; provider capacity is acquired afterward. Only Queued may
dispatch, and its atomic transition to Submitting prevents automatic replay after
a crash. Preparing and queued cancellation are local, conditional on dispatch not
having started. Imported preparations require review. Final latency measurements
and equivalent browser/mobile preparation phases remain open.

Native and browser folder creation also save logical requests before provider
capacity. Namespace planning freezes an exact target before CREATE; a durable
receipt precedes catalog repair. Unknown results require read-only inspection.
Pending names cannot be used as physical Move destinations. Browser checked
subtree changes now retain exact reviews, outgoing-target fences and receipts
through bounded repair. Mobile folder adoption remains a separate migration.

Native group flags now persist their receipt before updating the cache. Cache
repair and its Undo transition commit together; repair cannot repeat STORE or
wait for provider capacity. Reader controls use the same projected flag metadata
as rows, including bounded observations for an offscreen reader.

The [browser inventory](IMMEDIATE_ACTIONS_BROWSER.md) records durable individual
admission, Activity, receipt repair and safe orphan-queue recovery. Native
individual controls now admit into the existing group journal with observed
lineage and field ownership; History retains receipt-based Undo across restart.
The bounded owner shares capacity across mail, folders, calendar and Outbox,
prioritises cache repair and drains active steps on correlated close requests.
Independent accounts within one group now share a retained lease while exact
item claims and repairs preserve conflicting account order. Automatic recovery
preserves Pause and newer Undo decisions. A storage failure still applies the
bounded cooldown to its group. Candidate discovery uses indexed pages of 50 keys
and yields between pages. Deferred cursor resets prevent wakeups and completions
from starving later accounts. The 100,000-row key-seek tests do not bound every
predecessor-history query or establish input-to-visible latency.
Offline/auth policies and remaining domain
adapters still need work. Shared scenarios and client parity retain
those gaps explicitly.
