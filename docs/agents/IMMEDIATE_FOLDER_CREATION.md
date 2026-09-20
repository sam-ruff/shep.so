# Saved folder creation

Desktop creation uses the existing `folder_creations` journal. The local
selection FIFO saves a UUID, account binding, raw parent and leaf name before
provider capacity or namespace discovery. The matching form closes after this
save. A logical sidebar entry shows progress under the chosen account and
parent; it is never a mailbox identity or a move destination.

The shared action owner claims creation alongside mail, calendar, folder
changes and Outbox work. It owns provider capacity and the account operation
lock. Read-only planning saves an exact encoded target before CREATE. The
dispatch marker precedes the command, and a tagged acknowledgement is saved
before listing or caching folders. Confirmed existence and acknowledged CREATE
remain distinct in the record. Cache repair cannot send CREATE again.

Unconfirmed or interrupted CREATE requires an explicit check of the saved
target. An absent target permits a separately chosen retry. Stopping tracking
retains the unconfirmed record and never deletes server folders. Queued work
can be cancelled; accepted work drains its receipt on close. Read-only planning
and inspection can stop, and saved queued work resumes on restart. POP3
creation commits its local catalogue and outcome together.

Schema 9 adds journal data and status/identity indexes to the existing table.
Legacy and imported unconfirmed work requires inspection. Imported known
receipts retain cache-only recovery. Completed and cancelled request identities
remain idempotent even after another request uses the same name. Account
removal reviews active requests and removes their local records atomically.
Pending creation prevents rename/delete from changing its target before the
receipt is cached.

The sidebar exposes at most 32 outstanding requests. New admission enforces
that bound; older imported requests appear as earlier entries are resolved.
Connection changes require cancelling safe unsent work and submitting a fresh
request, rather than silently rebinding an old target.

Browser gateway and mobile adapters have independent migration work. Native
fixture controls establish local admission and recovery behaviour; mock
provider and wire protocol tests establish the CREATE boundary. They do not
establish live provider parity.
