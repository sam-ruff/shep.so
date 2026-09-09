# Local profile history

The optional native `history` feature in `shared/profile-core` stores immutable
account/settings operations and merges their causal history. It is a foundation
for [continuous profiles](PROFILE_SYNC_HANDOVER.md). The optional [Drive transport](PROFILE_DRIVE.md) now verifies provider identity
and immutable files against this journal. A separate [discovery catalog](PROFILE_DISCOVERY.md)
now retains remote scan progress. Flutter discovery and [initialized publication](PROFILE_PUBLICATION.md)
and reviewed desktop/mobile enrollment are connected. Complete categories,
browser application and cross-client continuous sync remain unfinished. The browser currently shares the codec, not this SQLite journal.

## Ownership and bounds

`history::Worker` owns one SQLite connection on a dedicated thread, with 32 pending
commands. Accepted writes finish even if their observer is cancelled. Close drains
accepted work; other worker clones must be dropped before it can finish. Never
open this synchronous journal from an iced handler or add a profile snapshot
connection to the mail cache. Input validation/encoding also belongs off the UI
thread.

The journal uses bundled SQLite 3.53.2, WAL/FULL, private new files/directories on
Unix and an exclusive companion lock retained until its connection closes. Resolve
symlinks before choosing the lock path; never remove a live lock file. This
checkpoint ports the dependency update from desktop `db8c82a`, not that commit's
entire mail-cache owning-worker migration.

`Binding` separates application namespace, verified provider principal, profile
UUID and generation. Its hash addresses local files; its input strings are not
authentication. Transport must verify identity and file ownership before importing anything;
the optional Drive implementation enforces both independently of this binding. A fresh journal generates its device UUID and retains it on
reopen. This is device-local state, not a portable database: installation/import
rebinding must prevent cloning that UUID or replaying queued uploads elsewhere.

Field/version observations contain at most 50 entries. Values/uploads return one
bounded operation. Each command applies at most 32 ready operations; callers
continue `Drain` while `ready` is nonzero. Missing-parent records stay durable and
block new local edits. Zero waiting records proves only that received ancestry
has applied; it does not establish a complete cloud listing. History compaction,
large-history performance and full storage bounds remain to verify. The existing
codec's 256-parent bound can require reconciliation before another local edit.

## Edits and recovery

Generate one operation UUID and freeze the complete `LocalEdit` before submission.
After a lost reply, retry that same request. Reusing its UUID with different data
is rejected. Original remote bytes are retained exactly; a duplicate operation
with different bytes is rejected. Cycles and wrong bindings fail without partial
application. Unknown optional data remains in the original records; local field
replacements must preserve extensions from a reviewed version.

Capture `expected_revision` with or before the values shown for review. Do not
fetch a newer revision after displaying stale values and use it to overwrite
them. Per-target revisions allow unrelated changes, while changed reviewed
fields require a new review. Concurrent values remain explicit conflicts; a
resolution must identify every reviewed version. No wall-clock winner is chosen.

Account deletion hides its prior fields and prevents stale/concurrent edits from
resurrecting it; re-adding uses a new account UUID. Profile deletion ends that
generation. Concurrent duplicate tombstones retain their records without creating
an unresolvable value conflict. Log insertion, causal dependencies, derived field
versions and counters commit atomically.

Local operations keep exact upload bytes and SHA-256. Reserve a provider file ID
durably before sending; a retry cannot replace it. `Confirm` accepts only the
reserved identity/digest, but does not perform HTTP or prove Google committed it.
Transport must verify that exact owned immutable file before calling
it. Discovery checkpoints and acknowledged local application need separate
durable state; the journal's derived fields do not update mail accounts/settings.

`ExportAcknowledgedRecord { expected_revision, after }` returns one exact
original imported or confirmed-local operation, excluding unsent local records.
A partial covering index bounds this traversal. Import alone does not authenticate
an original: reconciliation additionally checks every returned record against the
completed provider inventory. Reopening an ongoing-sync owner restarts that
verification; normal cycles verify newly acknowledged records. Losing only the
catalog metadata cannot reuse proof from a surviving observation history.

## Flutter integration and evidence

`flutter/rust/src/profile_history.rs` admits one active request/journal per native
workspace independently of mail/provider capacity. Switching bindings drains the
old owner first; stale close requests cannot close another binding. The production
bridge's owned request task retains admission through cancelled observations.
`NativeProfileHistory` uses background JSON encoding and decoding. Its future
controller must serialize dependent commands and fence profile/session changes.

Core tests cover independent stores and arrival order, conflicts/reviews, unknown
data, missing parents/restart, tombstones, transactional failure, immutable IDs,
upload reservations, queue cancellation and independent-process/symlink ownership.
Native tests hold provider capacity while accessing history and check binding
switch/reopen isolation. A shared actual Dart FFI scenario also runs on the
isolated Android emulator. It copies synthetic records between two temporary
stores with credentials locked; it is not a Google or Settings-control E2E test.
See [client testing](../CLIENT_TESTING.md) and [completion evidence](../COMPLETION.md).

There are no passwords or OAuth grants in this format. Local metadata SQLite is
still unencrypted (R22). Credential protection, Google project configuration,
live cross-client appData access, Apple execution and actual desktop/mobile/browser
Settings flows remain open. Operational instructions live in
[AGENTS.md](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/AGENTS.md).
