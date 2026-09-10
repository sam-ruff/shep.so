# Multiple backup destinations

Preferences → Backups can keep several local folders, S3/SFTP/FTP destinations and one
Google Drive destination active together. Add destination preserves the existing setup. Select
its named row to change its schedule, retention, account-password inclusion or
passphrase. Removing a destination keeps its saved copies and upload receipts.
Re-adding the same destination can resume a retained upload with its original
passphrase.

The existing encrypted format is unchanged: all copies use Argon2id and
AES-256-GCM with compression. Set up each destination by saving its first copy;
its own OS-keychain passphrase then supports its automatic schedule. Each
provider job has a destination-specific busy key, journal row and retention
policy, so one destination does not replace another's configuration or block its
upload. Existing single-destination settings migrate when another is added.

Identical destinations are rejected, including a second Drive entry under the
same login and aliases of a local folder. Lexical checks run without filesystem
access; real folder identity checks run on the storage worker. Imported database
profiles clear these device-local destinations and schedules.

The provider checkpoints below cover Local, Drive, S3, SFTP and FTP/FTPS.
Optional archive-format compression/encryption controls, richer persistent
failure history and live cloud verification remain unfinished. R32 stays active.
Ordinary preview cannot create backups or access credentials. The explicit
combined-backup native fixture uses only owned local directories and an isolated
credential worker, with no real account passwords or network connections.

Verification is recorded by the integrating agent in the completion log. New
regressions cover migration, switching/removal, independent encrypted uploads,
per-destination retention/passphrases, missing-keychain isolation, delayed
metadata after target edits, restart and duplicate/symlink alias rejection.

The lane verification passed 37 backup-filtered Rust cases, nine preferences
cases, all-target/all-feature Clippy, 55 Python tests and the saved native
`test_multiple_backup_destinations_setup_and_restart` flow. That flow adds a
second destination, rejects a duplicate folder, edits retention/name, switches
between settings, restarts, cancels and confirms removal, restarts again and
edits the retained setup at 900×640 in dark appearance. Light/dark WebPs were
reviewed under ignored `artifacts/e2e/3ee3c19c5507`; logs use the
`artifacts/logs/multiple-backups-*` prefix. This selected flow is not full-suite,
live Google or Windows/macOS evidence. Final hook and integration results belong
in the completion log.

## S3-compatible storage

Choose S3-compatible storage, then enter the HTTPS endpoint, bucket, signing
region and folder prefix. Path-style addressing is the default; switch it off
for virtual-hosted bucket addressing. Enter the access key and secret key and
choose **Test and save connection**. The read-only test checks listing and
existing-copy access; the first backup checks upload permissions. Only a
successful test saves credentials in the active profile's OS keychain. Empty
key fields reuse those saved credentials. Changing the endpoint, bucket or
prefix clears unsaved keys; removing the destination during a test prevents
that result from saving credentials.

Signing uses the maintained [rusty-s3 action API](https://docs.rs/rusty-s3/latest/rusty_s3/trait.S3Action.html)
with the existing asynchronous HTTP client. Uploads retain one reserved object
key and exact encrypted bytes in the durable journal. A conditional create
prevents replacement, and an uncertain reply is recovered by checking the
same object's metadata and bytes. See AWS's [conditional write contract](https://docs.aws.amazon.com/AmazonS3/latest/userguide/conditional-writes.html).
Retention follows a complete listing and a confirmed new copy; only matching
Shep filenames with ownership/checksum metadata qualify. Conditional deletion
uses the checked ETag so changed objects are kept, following the
[DeleteObject contract](https://docs.aws.amazon.com/AmazonS3/latest/API/API_DeleteObject.html).
Restore checks the downloaded checksum before existing password/decryption and
merge validation. Signed URLs and service error bodies never appear in errors.

Endpoint host case/default ports and trailing prefix slashes normalize to the
same destination. Duplicate effective targets are rejected even when region or
addressing-style settings differ. S3 secrets, passphrases and schedules are not
imported by archive restore or portable Google account/settings sync.

This provider currently uses access-key credentials, HTTPS and single-object
uploads within the existing snapshot limits. It does not manage temporary-role
credentials, multipart uploads, bucket creation or historical versions in
versioned buckets. Providers must support the conditional write/delete contract;
permission or compatibility errors retain the pending copy and existing backups.
No live AWS, third-party S3 or OS keychain session was exercised by automation.

S3 regressions use object-scoped loopback HTTP servers and a test credential
backend. They cover uncertain PUT/restart recovery without another upload,
foreign/missing objects, corrupt downloads, incomplete/repeated listings,
conditional retention, alias detection and verified-key replacement races.
The saved native `test_s3_backup_setup_native_validation_and_saved_target` flow
covers invalid inputs, masked keys, preview isolation, persisted configuration,
restart and compact dark layout. Native preview never makes cloud requests.


The S3 adaptation to main also drives its actual setup-save queue while an
unrelated shared preference changes and a backup receipt arrives. Its typed
save preserves both newer values and its edited retention. Keep this regression
alongside the existing multi-destination metadata/remote-settings race test.


## SFTP

SFTP uses [russh](https://docs.rs/russh/0.63.2/russh/client/index.html) and the
[raw russh-sftp requests](https://docs.rs/russh-sftp/3.0.0/russh_sftp/client/struct.RawSftpSession.html).
Rust 1.89 is the minimum for the maintained SSH library. Production traffic uses
the configured host/port, a pinned SHA256 host key and password authentication.
A fingerprint probe deliberately rejects the offered key and never sends a
password. The native review requires an explicit verification checkbox before
using a probed fingerprint; changed-host results cannot authorize another server.
Host-key or username changes identify a new authenticated destination and cannot
reuse the old keychain password, passphrase or automatic-backup readiness.
Duplicate effective host/port/folder locations remain rejected across usernames
and key changes, preventing overlapping retention policies.

Passwords are saved through the existing profile-owned credential worker only
after a successful read-only connection test. Rechecking current settings before
saving keeps a late result from reviving a removed connection. The folder must
already exist at its absolute canonical path; symbolic-link aliases are rejected.
User-visible errors omit server-controlled status bodies.

The existing upload journal reserves the final filename and encrypted bytes.
An exclusive staging file receives a persisted creation receipt before bytes are
written. Retry verifies its already written prefix and resumes at that offset;
unexpected contents are kept untouched. Fsync runs when the server advertises it.
SFTP v3 rename commits the copy without replacing an existing destination; the
POSIX overwrite extension is never used. An uncertain rename is recovered by
verifying the original final filename's size and streamed checksum. An existing
unconfirmed staging file without its creation receipt requires inspection and is
not overwritten automatically.

Directory listings have bounded pages/entries and validate Shep filename, regular
file type and archive header before retention. A failed or repeated listing keeps
all old copies. Only the header is downloaded while listing; restore and upload
verification stream bounded chunks. The raw SFTP dependency currently ignores its
Config packet-size limit, so a length-delimited stream guard rejects oversized
announcements before its parser can allocate their payloads. Requests are issued
one at a time, and the SSH channel has a bounded eight-message buffer.

SSH/SFTP regression peers listen only on loopback with generated fixture keys.
Their file state uses a bounded 32-job owner. Tests cover changed-key refusal
before authentication, incomplete/fractured packets, partial upload continuation,
uncertain commit/journal reopen, byte-preserving restore, foreign files and
credential-saving races. Native preview supplies fictional fingerprint reviews
and disables password authentication/cloud writes. These tests are separate from
live server, real OS-keychain and Windows/macOS verification. SFTP private-key and
agent authentication, automatic remote folder creation, FTP/FTPS and the other
remaining R32 options are follow-up work.

The SFTP source checkpoint passes 15 targeted Rust cases, including successful
rolling retention, refusal to overwrite foreign final or unconfirmed staged
files, IPv6 aliases, and redacted server errors. The saved native scenario passes
host failure/retry, copied fingerprint review, disabled unverified replacement,
changed-key confirmation, persisted settings and compact dark layout. Reviewed
WebPs are under ignored `artifacts/e2e/6a585034d0e2`. The final binary passes all
five selected SFTP, S3, multiple-destination, existing setup and compact-layout
flows in 19.489 seconds; its SHA-256 is
`817f02150a35386039904ae0a0ab14b630b229dfa199bc013d45c62ecf171d92`.
Hook and integrated shipping receipts belong in the completion log. Python checks passed with seven
platform-dependent skips; strict Zensical building passed. No performance
measurements or live server writes were performed.

## FTP and FTPS

The FTP/FTPS provider uses maintained [libcurl Rust bindings](https://docs.rs/curl/latest/curl/easy/struct.Easy2.html)
with bundled libcurl and its FTP protocol enabled. The native library runs only
inside background-owned requests; the bounded provider dispatcher limits
concurrency. Cancellation closes a one-shot observer and the progress callback
stops abandoned transfers. Connection setup has a 15-second deadline; a request
has a 300-second total deadline and a 30-second inactivity limit. Read-only
connection testing has an overall 30-second deadline before any keychain write.

Explicit FTPS (STARTTLS, port 21) is the default. Implicit FTPS (port 990) and
clearly labelled plain FTP are selectable. FTPS requires TLS for both control
and data, verifies the certificate chain/hostname, and never falls back to
plaintext. See the [libcurl TLS requirement](https://curl.se/libcurl/c/CURLOPT_USE_SSL.html)
and [certificate verification](https://curl.se/libcurl/c/CURLOPT_SSL_VERIFYPEER.html)
contracts. Passwords pass through the existing credential owner after successful
connection validation; secrets are absent from URLs, SQLite and MCP observations.
Verbose protocol logging stays disabled, server error bodies are discarded, and
only numeric transport/status codes enter error messages. Control replies and
collected data have bounded callbacks. Passive data stays on the control peer's
address, ignoring server-supplied PASV IPs.

Every request checks CWD/PWD before transferring data or mutating files. Use the
server's absolute canonical folder; aliased paths are rejected. MLSD is required
for reliable entry types and complete, bounded directory listings. Failed,
malformed or duplicate listings preserve existing copies.

FTP has no portable conditional STOR or rename. Each reserved copy therefore
gets its own exclusively created directory, a persisted creation receipt before
writing, an encrypted `mail.shepbackup`, and a small `shep-commit.json` record.
The public commit record contains only format, filename, ciphertext size and
checksum. Retry verifies any saved prefix before appending to an interrupted
archive or commit record. An uncertain committed transfer is recovered from the
same manifest and archive without another upload. Unconfirmed directories
without a creation receipt are kept for inspection; unexpected contents are
never overwritten. Restore verifies the committed checksum before the existing
passphrase/decryption/merge path. Retention verifies the archive and exact owned
folder contents before deletion, preserving folders with unrelated files.

Protocol tests use an object-scoped loopback peer with a bounded file-state owner
and generated fixture certificates. Native preview continues to reject FTP
connections and cloud writes. Live FTP/FTPS servers, real OS-keychain sessions and
Windows/macOS execution remain separate from these checks. Combined runs and
optional formats are described below; richer persistent history remains R32 work.

The release archive includes the upstream libcurl and curl-rust license notices
under `licenses/`. This addition does not enable the dormant release workflow.
The backend passed a full Windows GNU all-target/all-feature check; actual
Windows/macOS FTP execution is still unverified.

The FTP lane carries the exact SFTP setup-deadline follow-up from main
`ff03b02`: channel and subsystem setup each have a 15-second deadline, with the
real held-channel/retry regression. It does not replay main's other ancestry.

FTP delivery evidence: 28 matching FTP/SFTP Rust regressions pass, including
keychain/removal races, alias rejection before mutations and response/cancellation
bounds. Full Windows GNU all-target/all-feature checking and Clippy pass. The
final binary passes all six selected FTP, SFTP, S3, multiple-destination, existing
setup and compact-layout native scenarios in 25.612 seconds. Its SHA-256 is
`e9ab18cae9d3aa5b6aea47e96cc7b32373ba53fda316cc697cb81c3c5d500f14`.
Final reviewed FTP WebPs are under ignored `artifacts/e2e/be05e8c32b7a`; logs use
`artifacts/logs/ftp-*`. Python checks pass with seven platform-dependent skips,
and strict documentation building passes. This is selected correctness evidence;
no performance measurements, actual Windows/macOS execution or live server
verification is claimed. Mandatory hooks and main integration remain the final
shipping receipts.


## Back up all included destinations

Each named destination has an **Include** checkbox for **Back up all**. This
selection is independent of its automatic schedule and is saved on this device.
Existing destinations are included when migrating; excluding one keeps its
settings, individual controls and schedule intact. A destination needs its first
successful copy with the chosen format before it can join a combined run.
Encrypted copies also require the saved OS-keychain passphrase. **Setup** opens an unfinished destination's own form.

The action waits for settings persistence, then admits at most 32 independent
jobs to the existing bounded provider queue. The eight provider slots remain
shared with other provider work; UI sends never block. Rows show preparing,
uploading, receipt/retention completion, success or a specific failure. **Retry**
only requeues that failed destination. Old attempt results cannot overwrite a
new retry, and overlapping provider work returns an explicit busy result. Close
waits for admitted backup jobs, including before their first worker event.

Each destination reads its own saved passphrase and rechecks inclusion, identity
and first-copy readiness after credential access. Recovery uses the original
journal reservation and exact encrypted bytes. An unresolved upload cannot be
silently replaced by a new snapshot. Exclusion/removal prevents queued work from
starting; it does not interrupt an already accepted remote write. Individual
Back up now and Restore controls remain available.

Run results remain visible for this session. Last successful backup, inclusion,
schedules and unresolved upload journals survive restart; a persistent multi-run
failure history is a separate follow-up. Snapshots are prepared independently per
destination and may reflect mail arriving between their capture times. Optional compression and encryption are described below.


The combined-run correctness suite covers independent actual encrypted local
copies with distinct saved passphrases, first-copy gating, queue saturation,
settings acknowledgment and edit/exclusion races, held credential access,
duplicate admission, failed-only retry and stale completion rejection. Saved
native `test_backup_all_native_*` flows use the same upload and journal paths;
they verify exact ciphertext/filename preservation after a lost acknowledgment,
retry and restart, saved inclusion choices, Setup, navigation during upload and
close waiting until both copies and metadata are durable. Light progress/error
and compact dark controls are reviewed. These are fixture correctness checks,
not live provider or performance measurements.


## Optional archive formats

`backup::format::Options` is shared by every destination: Zstd or no compression,
and passphrase encryption or no encryption. Existing settings default to both
compression and encryption. Password export is rejected unless encrypted; restore
also rejects credential entries in unencrypted copies, even with a valid checksum.
Provider login credentials remain in the OS keychain independently of archive
protection. The native form hides its passphrase/password controls when protection
is disabled and explains the resulting file access exposure.

The shared writer emits `SHEPBK02`; the reader retains `SHEPBK01` compatibility.
V2 starts with a 40-byte header: magic (8), flags (1: compression bit 0, encryption
bit 1), reserved zeros (7), random Argon2id salt (16), random STREAM nonce (7), and
one reserved zero. Unsupported flags/reserved bytes fail. Encrypted records use
RustCrypto `aead` 0.5.2 `EncryptorBE32`/`DecryptorBE32` with AES-256-GCM, a fresh
Argon2id-derived key (version 0x13, 19 MiB memory, two iterations, one lane,
32-byte output), and the library-owned record counter/final marker. See the
[RustCrypto STREAM API](https://docs.rs/aead/0.5.2/aead/stream/index.html).

Each record has a four-byte big-endian length whose high bit marks the final
record. Nonfinal plaintext is exactly 64 KiB; the final record is at most 64 KiB and
may be empty. Encrypted lengths include the 16-byte tag. The complete header and
record prefix are authenticated as AAD. Restore requires the final record and
EOF, rejecting reordered/spliced records, truncation, malformed lengths and
trailing bytes. Unencrypted files instead end with a SHA-256 checksum over the
header, prefixes and payload; this detects corruption but provides no authenticity
or confidentiality. Zstd precedes framing/encryption. Bounded writer/reader frames
prepare for R23, but the surrounding snapshot/journal still retains its existing
Vec-based 256 MiB raw-mail / 768 MiB decoded-snapshot limits. This is not completion of
large-mail streaming.

A pending journal row owns its original options and bytes. Reopening setup and
changing the format never re-encrypts that upload or overwrites its saved key.
Encrypted pending copies require the original typed/saved passphrase, even when
the edited form now disables encryption. Plain pending copies never request or
save a passphrase, even if the new form enables it. A late receipt updates history
but only acknowledges readiness for the format actually uploaded. A subsequent
new-format first copy enables that format's automatic/combined use. Unencrypted
manual, combined and automatic copies do not access the backup-passphrase store.
The old saved passphrase is retained for recovery of existing encrypted copies.

Native format/restore scenarios use only `backup_run` owned fixture folders and
its isolated credential worker. Their calendar identifiers are valid fictional
Google identities so production import validation remains intact. Ordinary preview
still rejects restores and cloud writes. The saved options flow checks account
password exclusion, independent settings, actual plaintext/encrypted copies,
wrong-passphrase recovery, restore, restart and compact dark controls.

The fixed V2 compatibility fixture is generated independently by
`scripts/backup_format_vector.py` with libargon2 and Python AES-GCM. Rust tests
consume its committed hex without additional runtime or CI dependencies.


Format checkpoint verification: 98 backup-filter Rust checks pass, followed by
13 final format-specific checks including the independent compatibility vector.
All twelve selected native backup/provider/setup/compact flows pass in 63.782 seconds.
Final format WebPs under ignored `artifacts/e2e/d6bc3f77c0f8` were reviewed,
including light restore/password recovery and compact dark controls with visible
save actions. Binary SHA-256:
`337404cbbf769a3a417b284bc20c545b848cfa159a9761eba00473178c331f7f`.
Full Windows GNU all-target/all-feature checking passes, Python runs 81 tests
with 74 passing and seven platform skips, and strict documentation builds pass.
Logs use `artifacts/logs/backup-formats-*`. Mandatory hooks and root integration
are the final shipping receipts. Persistent destination failure history, complete
large-snapshot streaming and actual provider/platform execution remain open.


## Persistent destination activity

Backups offers Recent activity for the selected destination. Its latest twenty
attempts survive restart, with start time, the actual archive format and the
result. Unconfirmed uploads remain distinct from confirmed copies whose keychain,
receipt or retention work needs attention. Final notices name the destination,
including when a combined run uses different archive formats.

Activity uses the existing cache worker, an indexed table and bounded reads:
twenty rows per destination, at most 640 retained rows overall, and at most 2,048
characters of diagnostic detail per row. Passwords, tokens and archive bytes are
never added to this table. The upload journal remains the authority for reserved
object/session identity and immutable bytes; activity cannot replace a receipt.
A newer confirmed recovery marks only earlier uncertain attempts for that same
destination and reserved object as recovered. Confirmed copies cannot be rewritten
as failed uploads.

Only the latest unresolved attempt offers Retry. The controller saves current
settings first, checks the selected target again after acknowledgment, and the
backend rejects a stale activity review. A row with a reserved object can resume
only its matching local journal entry. Imported activity without that entry
cannot create another copy through Retry; it directs the user to inspect copies
or explicitly start a new backup. A locked keychain keeps its original passphrase
requirement. Navigating away or a saturated read queue never blocks the UI.

Database schema v4 adds the indexed history table. Full database imports still
accept exact v2 and v3 schemas, reject unknown objects, and migrate only their
private staged copies. Recovery of an already-prepared old imported profile
applies the migration without repeating operation fencing or overwriting its
prepared preferences. Portable backup snapshots and Drive profile values do not
include destination activity.

Saved native history scenarios use `backup_run="recover"` for lost upload
acknowledgment and `backup_run="warning"` for an acknowledged copy followed by one
failed fixture-keychain save. They operate only on owned temporary folders and an
isolated credential worker. Both click Retry, compare the exact filename/bytes,
restart twice, and capture light plus 900×640 dark activity controls. No real
provider or personal credentials are used.


History checkpoint verification: 109 backup-filter Rust checks and 28 transfer
checks pass. All 22 selected native backup/provider/compact/database-transfer
flows pass in 137.645 seconds. Final light and compact-dark history WebPs under
ignored `artifacts/e2e/25b2646dfdd1` and `artifacts/e2e/fa94ba50129d` were reviewed.
The tested binary SHA-256 is
`1f541abf0bea2dc9c97a18a17d9ff14db916522a39a00e50c00237cbd2a8cc51`.
Full Windows GNU checking, Python's 81 tests (seven platform skips), and strict
documentation builds pass. Logs use `artifacts/logs/backup-history-*`; normal hooks
and root integration remain the shipping receipts. This does not complete the
journal-channel migration, large snapshots or actual remote-provider/OS testing.


## Journal channel ownership

The upload journal now uses a dedicated `store::worker::Worker` named
`shep-backup-journal`. Its 32-command FIFO owns the connection, separate from mail
cache reads and settings writes. Per-operation `spawn_blocking` jobs and the
shared journal connection mutex are removed. Connection creation, keyed factory
policy, WAL and synchronous FULL durability stay at the constructor boundary.

Cancelling before admission prevents that request from becoming a later write.
Once admitted, a job drains even if its observer or the last journal handle is
dropped. Provider checkpoints still await their durable acknowledgments. A failed
transaction rolls back without replacing the reserved archive or stopping later
work. Target/object/session matching and explicit committed-copy removal are
unchanged; channel ownership does not create a second retry authority.

Deterministic tests hold the owner while filling all 32 slots, cancel the overflow
request and admitted observers, then verify the last accepted session and original
archive after restart. A separate close test drops every handle with preparation
and its commit receipt queued, waits for their owner-side barrier, and reopens the
actual database to verify exact bytes and the receipt. The ordinary provider,
retention, lost-acknowledgment, restore and native close/recovery flows remain the
regression contract. Large archive buffers remain R23 work.


Owner checkpoint verification on the integrated tree: all 114 backup-filter
checks pass, including the three deterministic owner tests and the keyed
encrypted-journal test, which now runs through the same worker. All eight
selected native combined backup, history retry/restart and format restore
scenarios pass in 44.089 seconds. Light and compact-dark WebPs in ignored
`artifacts/e2e/0e89116c39b8` and `artifacts/e2e/dfd12e41b5da` were reviewed.
Tested binary SHA-256:
`7cffc2f20e6e20c79d187be0f578e72343af0027d7246a488744650098afccf1`.
Logs use `artifacts/logs/e2e-journal-owner.log` and `artifacts/logs/test-*.log`.
Normal hooks and root integration are the final shipping receipts.
