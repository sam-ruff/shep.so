# Multiple backup destinations

Preferences → Backups can keep several local folders, S3 destinations and one
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

This checkpoint does not add FTP, FTPS or SFTP, optional archive-format
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
