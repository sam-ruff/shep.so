# Local cache encryption work

R22 is in progress. Normal startup still opens the existing cache; the staged
encryption APIs are not activated until migration and recovery are complete.
No personal database has been changed by the development fixtures.

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
The owning cache worker keeps scratch alive until admitted jobs drain and drops
the connection before deleting its directory. Reopened sessions start empty.
Read-move TEMP projections are bounded at 128 entries. A separate shared-source
follow-up replaces ancestry TEMP/recursive sets with an indexed main-database
table and a 128-ID frontier; the published source is included in the desktop pin.
The remaining SQLite sort/temporary-index audit, crash-orphan cleanup and
portable-transfer scratch still block production activation. Known remaining
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
source. Guarded legacy WAL checkpoint/close, atomic replacement and crash
recovery are unfinished; never rename a main database beside old WAL/SHM files.

The root ownership guard allows current readers and excludes migration/key
creation while another cooperating process owns the cache. It explicitly
unlocks on drop so an unrelated transient fork cannot extend its flock. Legacy
process exclusion and retained ownership through every admitted worker write
remain required before activation.

The SQLite backup API supports copies between databases with compatible
encryption settings. It cannot perform plaintext/ciphertext conversion. Portable
export/import must retain independent snapshot ownership, bounded cancellation
and exact reviewed-copy publication while adding logical conversion. Explicit
user exports are distinct from application-owned cache/temp data.
[SQLCipher backup API support](https://discuss.zetetic.net/t/using-the-sqlite-online-backup-api/2631/4)

Data inventory:

| Data | Current encryption integration |
| --- | --- |
| Mail/MIME, draft attachments, outgoing, bulk/move/folder receipts, settings/calendar | Keyed Store entry point |
| Backup upload archive, destination and session | Keyed journal constructor and Engine routing |
| Profile upload cache, history, discovery and nested observations | Keyed constructors/shared initializer |
| Local profile catalog, active profiles and imported-marker recovery | Explicit keyed constructor and propagation; bootstrap activation still pending |
| Import staging and portable export/import | Encrypted conversion/routing still required |
| Selection/frozen reviews | Attached encrypted scratch, indexed incremental ordering, worker-owned cleanup |
| Profile ancestry | Indexed main-database scratch with bounded frontier, included in shared pin |
| Remote images and print previews | Already memory-only within Shep |
| Explicit saved attachments, printed PDFs and user-chosen exports/backups | User output; not an implicit cache file |

The existing plaintext file and recovery material must remain usable until a
verified replacement commits. A failure must offer retry/recovery without
resetting mail or acknowledging an incomplete migration. Already deleted copies
and storage-device history cannot be retroactively encrypted by migrating the
active database. Keep these implementation limits out of ordinary navigation.
