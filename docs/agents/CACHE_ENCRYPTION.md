# Local cache encryption work

R22 is in progress. Normal startup now runs through the root bootstrap worker
(`cache_cipher::bootstrap`) with the production policy `Existing`: a root that
already owns a device key takes the encrypted path, and every other root stays
plaintext exactly as before. No production root owns a key yet, because key
creation and plaintext conversion (`Policy::Migrate`) are reachable only from
tests until the blockers listed at the end of the bootstrap section are
resolved. No personal database has been changed by the development fixtures.

The desktop pins SQLCipher 4.19.0 (SQLite 3.53.4) in the vendored
`libsqlite3-sys` build, retaining the newer WAL/FTS fixes. Random device keys are
zeroizing, redacted in Debug output and supplied through the C key API before
schema access. The bounded credential actor admits and verifies newly stored
keys. Missing required keys fail without creating replacements. Device-root key
namespaces are separate from imported account/profile credentials.

SQLCipher protects database and journal pages. The build uses `TEMP_STORE=2` with
a documented native connection policy: ordinary plaintext opens default to FILE,
while a keyed main forces memory temporary storage and rejects FILE/DEFAULT
changes, including after replacing or removing a SQL authorizer. This avoids
introducing unbounded in-memory sorting into existing plaintext startup. Keying
an attached scratch database alone leaves a plaintext main's FILE policy intact.
Selection membership and
frozen reviews now use a private attached encrypted scratch database, with a
2 MiB page-cache target. Full selection ordering builds an index on disk and
walks it one row at a time instead of materializing an unbounded sort/window.
Conversation duplicate choice and newest-first ordering also use indexed scratch
metadata, loading only the requested 20-message page into Rust. Anchor/focus copy
precedence is preserved; both timestamps and stable ID ties sort descending.

Scratch uses DELETE rollback journals with synchronous OFF. It is disposable
session state, has no crash-durability guarantee, and is never reused after a
restart. Normal transaction rollback still protects current session operations;
main cache and durable receipt journals retain FULL durability. Qualify the main
WAL pragma: an unqualified journal_mode assignment also changes attached scratch.
The previous initializer therefore made scratch WAL despite requesting DELETE.
The owning cache worker keeps scratch alive until admitted jobs drain and drops
the connection before deleting its directory. Reopened sessions start empty.
Read-move TEMP projections are bounded at 128 entries. A separate shared-source
follow-up replaces ancestry TEMP/recursive sets with an indexed main-database
table and a 128-ID frontier; the published source is included in the desktop pin.
The remaining SQLite sort/temporary-index audit and portable-transfer scratch
still block production activation; guarded crash-orphan cleanup is described
below. Known remaining
queries include all-account/folder selection summaries, workspace DISTINCT and
Inbox GROUP BY counts, and recovered-view automatic indexes. File-based plaintext
sorting is preserved; those queries are not claimed safe for a large keyed cache.
See [SQLCipher's storage design](https://www.zetetic.net/sqlcipher/design/).

The static build disables SQLCipher automatic exit/finalizer cleanup and
initializes OpenSSL with the same process-lifetime policy as Rust TLS. A
subprocess regression reproduced a crash in `sqlite3Codec` when a still-owned
cache read followed automatic global cleanup; the patched fixture succeeds.
Explicit `sqlite3_shutdown()` still releases state and permits reinitialization
once all connections are closed. The vendored lifecycle patch and source hashes
record this change. Ordinary application shutdown must still drain admitted saves.
The vendored curl initializer applies the same Rust OpenSSL policy inside its
existing constructor before libcurl's first crypto call. A combined subprocess
regression verifies this ordering; application constructors cannot safely race
library constructors to set global crypto policy. CA probing and certificate
validation remain unchanged.

The keyed Store carries an immutable key handle for independent connection
owners. Backup-upload journals, profile transport/history and discovery use
that handle. The shared connection initializer is pinned to
`e3e69a4ea71baa6bcc6f98f2d567f6b542adc77d`; it retains normal workers/file locks
and propagates the initializer into nested discovery observations.

Plaintext migration first creates a private encrypted candidate using a
read-only source transaction. SQLCipher logical export preserves schema, FTS
and BLOBs; application/user versions are copied explicitly. The candidate must
pass SQLite integrity and SQLCipher authentication checks before it can be
considered for publication. Cancellation removes the candidate and keeps the
source. The candidate records a cheap fingerprint of the plaintext (length,
mtime, header change counter, WAL length) and publication refuses a candidate
whose source changed afterwards.

Publication (`cache_cipher::publication`) runs under the exclusive ownership
guard with the candidate in the cache folder and no open connection to the
plaintext. Its steps are, in order: checkpoint the legacy WAL with TRUNCATE and
switch the file to DELETE journalling so no WAL/SHM sidecar can outlive a
rename (a busy checkpoint or refused mode change means another connection
still holds the file and fails the step; an empty leftover log is removed, a
non-empty one fails); write `<cache>.encryption-journal` naming the candidate,
its user/application versions and the checkpointed plaintext fingerprint, and
stop the candidate from deleting itself; rename the plaintext to
`<cache>.plaintext-recovery`; rename the candidate to the cache name (same
folder, directory fsync after each rename); reopen the published file with the
key and confirm an authenticated schema read plus the journalled versions;
delete the plaintext recovery file; delete the journal. Never rename a main
database beside old WAL/SHM files.

The plaintext is disposed of by ordinary deletion only after the keyed reopen
passed, because the contract requires recovery material until a verified
replacement commits and nothing else; keeping a plaintext copy beside an
encrypted cache would defeat the feature. Earlier copies and storage-device
history are a documented limit, not an overwrite promise.

Startup recovery (`publication::recover`) derives the state from the journal
plus which of main/candidate/recovery exist and the main file header, then
resumes or rolls back deterministically: a checkpointed candidate is fully
re-verified (integrity, cipher authentication, versions) and continues; a
retired plaintext with a verified candidate continues; a published file whose
keyed reopen fails while the plaintext recovery file exists is moved aside and
the plaintext restored (no write reaches the encrypted file before
verification, so nothing is lost); a published file whose plaintext is already
disposed of and whose key is wrong is kept with its journal and reported, never
reset; a lost candidate restores or keeps the plaintext; any layout this
machine does not produce is refused with every file kept. A recovery file with
no journal is reported and kept, and blocks a new publication until reviewed.
Orphan cleanup then removes `.shep-encrypted-*.partial` candidates (and their
SQLite sidecars) not named by a journal and `.shep-cache-scratch-*` session
folders; import staging files are not touched. Cleanup is only safe because the
exclusive guard proves no cooperating owner; the bootstrap section below
describes how older, guard-less processes are excluded. Ten tests cover full publication with
uncheckpointed WAL rows, stale candidates with retry, a held plaintext
connection with retry, foreign guards, interruption before each of the seven
steps with resume, wrong-key rollback and refusal, lost candidates, ambiguous
layouts and orphan cleanup.

Bootstrap (`cache_cipher::bootstrap`) is the only production entry to a data
root. `Bootstrap::start` owns one thread, `shep-cache-root`, behind a bounded
channel of one request; the engine's `open_workspace` sends the data folder,
the legacy cache filename and a policy, and awaits the reply. Dropping that
future cancels staging through a watch token; the plaintext is kept. On the
thread, `open_root` takes the exclusive guard (or, under `Existing` only, joins
as a reader when a cooperating Shep already owns the root and has therefore
run recovery), reads the device-local `.cache-root` marker naming the key
namespace, loads the key through the bounded credential actor (`Existing`
never creates one; `Migrate` creates the key first and writes the marker with
no-clobber persist plus directory fsync second, so a crash between the two
leaves a plaintext root that admits a fresh key later), then walks a
deterministic inventory of every database in the root: the root folder,
`profile-sync/`, and each `profiles/<uuid>/` folder with its own
`profile-sync/`, taking `*.sqlite` files, the legacy name and journalled mains
whose file is absent, never dot-prefixed candidates or import staging. For each
main it runs `recover` before anything opens, authenticates the key with a
read-only keyed open of an already keyed file (a wrong key fails here with the
file untouched), and stages plus publishes a plaintext file under the same
exclusive guard. A plaintext straggler in a keyed root (installed by an older
Shep) is therefore converted at the next start under either policy; a reader
that meets one reports that the root is being encrypted and retries later. The
guard is then re-taken shared (`Guard::share`, unlock then try-shared so a
competing exclusive owner is reported rather than outranked) and handed out as
`Root { key, guard }`. Two Shep processes starting together do not fail each
other: joining as a reader and re-taking the shared lock both wait up to two
seconds for the other's short exclusive phase, and the marker is re-read after
sharing so a key created in between is refused rather than ignored. `Catalog::open_in`, `Store::open_in` and
`backup::journal::Journal::open_beside` route every connection through the
root's key decision and give the shared guard to their `store::worker::Worker`,
whose owner drops it after the connection: the reader guard is released only
when the last owner has drained its admitted writes. The profile transport and
history workers (`src/profile_sync/`) still open through the key handle alone
and do not yet hold the guard.

Import staging in a keyed workspace converts logically: a keyed private copy
attaches the selected file read-only and unkeyed, re-validates its schema inside
the copying transaction (that connection does not share the earlier read
snapshot) and the preview fixture marker, exports with `sqlcipher_export`,
copies the user version and application ID that logical export drops, then the
copy is reopened keyed for
the same full integrity, schema, foreign-key and review checks the plaintext
path performs; fences and installation use the same key, so no plaintext
staging copy or profile ever exists beside an encrypted cache. The plaintext
path keeps the page-copy backup API and its per-page progress. Page progress
for the keyed path is reported at start and end only.

Legacy process exclusion: every Shep from this change on holds the reader
guard for its whole session, on the plaintext path too, so the population of
guard-less processes is limited to older binaries. For those, publication's
checkpoint step is the detector: SQLite refuses to leave WAL mode while any
other connection, in this or another process, has the file open, because each
open WAL connection holds the shared DMS lock on the `-shm` index for its
lifetime, including while idle. A subprocess fixture proves that an idle older
Shep holding the legacy cache blocks publication with a "still open" error and
that the plaintext stays intact; after it exits, the next start completes the
conversion, and a plaintext open of the published file fails without modifying
it. The remaining gap is a legacy process that starts between our exclusive
acquisition and its first read, or one that opens a root database publication
never touches; both are documented limits, not detected states.

Activation is still blocked on: a user-facing or setting-driven way to select
`Policy::Migrate` (there is no staging flag today; migration runs only from
tests); the profile transport/history workers retaining the reader guard;
bounded selection-summary, catalog and recovered-view sorting, and the index
builds inside `sqlcipher_export` during migration staging and keyed import,
which sort under the keyed main's memory temporary storage; native key
recovery and platform startup checks, including Windows rename semantics under
antivirus or indexer handles, which are only compiled, not executed; and a
native scenario exercising a migrated root, since the demo fixture opener is
plaintext-only.

The root ownership guard allows current readers and excludes migration/key
creation while another cooperating process owns the cache. It explicitly
unlocks on drop so an unrelated transient fork cannot extend its flock.

The SQLite backup API supports copies between databases with compatible
encryption settings. It rejects plaintext/ciphertext conversion. Keyed raw export
uses logical conversion with a separately pinned, URI-readonly source; its atomic
candidate lives in the chosen export folder. This remains intentional unencrypted
user output, with the existing credentials-excluded contract. Import/migration
scratch is implicit application data and must stay keyed. The export-only
`DBFLAG_VacuumInto` patch preserves unindexed rowids as well as indexed mail/FTS;
without it SQLCipher renumbers rows in unindexed extension tables. Keyed import
keeps bounded cancellation through the progress handler and the exact
reviewed-copy publication by rename.
[SQLCipher backup API support](https://discuss.zetetic.net/t/using-the-sqlite-online-backup-api/2631/4)

Data inventory:

| Data | Current encryption integration |
| --- | --- |
| Mail/MIME, draft attachments, outgoing, bulk/move/folder receipts, settings/calendar | Keyed Store entry point |
| Backup upload archive, destination and session | Opened beside the cache with its key decision and shared root guard |
| Profile upload cache, history, discovery and nested observations | Keyed constructors/shared initializer; reader guard not yet retained |
| Local profile catalog, active profiles and imported-marker recovery | Routed through the bootstrap `Root`; production policy keys only roots that already own a key |
| Raw portable export | Keyed readonly source converted to intentional user-selected SQLite output |
| Import staging | Keyed logical conversion, fences and installation when the workspace is keyed |
| Selection/frozen reviews | Attached encrypted scratch, indexed incremental ordering, worker-owned cleanup |
| Profile ancestry | Indexed main-database scratch with bounded frontier, included in shared pin |
| Remote images and print previews | Already memory-only within Shep |
| Explicit saved attachments, printed PDFs and user-chosen exports/backups | User output; not an implicit cache file |

The existing plaintext file and recovery material must remain usable until a
verified replacement commits. A failure must offer retry/recovery without
resetting mail or acknowledging an incomplete migration. Already deleted copies
and storage-device history cannot be retroactively encrypted by migrating the
active database. Keep these implementation limits out of ordinary navigation.
