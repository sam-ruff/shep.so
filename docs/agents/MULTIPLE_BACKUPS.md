# Multiple backup destinations

Preferences → Backups can keep several local folders, S3/SFTP destinations and one
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

This checkpoint does not add FTP or FTPS, optional archive-format
compression/encryption, a combined manual Back up all action, or live cloud
verification. R32 remains active for those features and their tests. Local/Drive
protocol and encrypted-file tests are separate from native fixture tests, which
cannot create backups or access credentials.

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
