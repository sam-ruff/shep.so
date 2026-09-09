# Multiple backup destinations

Preferences → Backups can keep several local folders and one Google Drive
destination active together. Add destination preserves the existing setup. Select
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

This checkpoint does not add S3, FTP, FTPS or SFTP, optional archive-format
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
