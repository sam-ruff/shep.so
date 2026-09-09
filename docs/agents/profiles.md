# Database transfer and profiles

The [user guide](../backups.md) explains moving a database between computers. Development rules remain in [AGENTS.md](https://github.com/sam-ruff/shep.so/blob/main/AGENTS.md); verification and shipping belong in the [completion audit](../COMPLETION.md).

## Local profile identity

`profiles.sqlite` lists local profiles and the next-launch choice. The original workspace remains `shep.sqlite`, named **My mail**, with its existing credential keys. Imports get a fresh device-owned UUID and `profiles/<uuid>/shep.sqlite`. Neither imported values nor a shared Google profile ID can select that path or credential namespace.

The catalog uses its own bounded 32-command SQLite worker. Pages contain at most 50 profiles. Rename/selection use revision checks so an older window cannot overwrite a newer choice. The running engine keeps its current Store and credential scope until exit; changing the next-launch choice cannot retarget in-flight work. There is no hot profile switch.

## Complete transfer

Export pins a consistent read transaction and copies SQLite pages through a separate connection. Its bounded progress/cancel/completion channels do not occupy provider capacity or the mail-cache worker for the whole copy. Export protects the profile catalog, all profile caches, journals and operation paths, including aliases.

Import copies into a private file, checks schema versions 2/3 against an independently created schema, runs integrity/foreign-key checks and validates connection metadata before review. Newer/foreign schemas and extra triggers/views are rejected. SQL work is cancellable; raw mail is not collected into a single application buffer. The complete database format has no encrypted-snapshot 256 MiB ceiling, but still needs disk space for a staged copy and any pinned WAL history.

Confirmation consumes the reviewed file rather than reopening the user's source path. Device preparation commits in one transaction, then closes SQLite before publishing the file without overwrite. Publication is the commit boundary: later cancellation or registration failure must preserve the saved copy. A ready marker lets catalog refresh adopt that same file after interruption. A damaged unselected profile produces a warning without hiding other profiles; a missing selected file is never silently recreated.

## Pending work and device settings

Import archives changed metadata in `imported_operations`, retaining original MIME in its existing rows. Pending outgoing deliveries/Sent uploads, bulk steps and unacknowledged folder changes become unconfirmed review work. Existing move acknowledgments remain available for their recovery path. An import must never cause an unacknowledged provider action to repeat automatically. The review requires an explicit acknowledgment when pending actions exist.

Imported credential-cleanup requests are archived and cleared. Notification baselines reset quietly. Local window dimensions and backup location stay device-specific; automatic backup/readiness and Google connection/grant state are reset. The imported account/calendar definitions, cached events, mail, drafts and portable preferences remain available. Passwords, Google tokens and external encrypted-backup upload journals are outside this database format.

## Credentials

`credentials.rs` owns OS calls on one bounded 32-command FIFO thread shared by the engine's adapters. Accepted writes drain even if their observer disappears. Missing-password restore checks and writes in that same queue. Imported profiles use `profile:<local UUID>:<key>` and never fall back to another profile's secrets.

Portable account/CalDAV IDs are validated before they can address credentials. Reserved Google/backup keys, SMTP-key aliases and overlapping connection keys are rejected. Comparison includes case-insensitive aliases because Windows credential target names are [case-insensitive](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentiala). CalDAV discovery's `caldav:<64 hex digits>` IDs remain valid, including in encrypted backup/password restore.

## Shared profiles

OAuth/profile sync uses the [Flutter interoperability handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md) and shared profile-core operations. Desktop SQLite remains a local transfer format. First-device creation, new-device discovery/import, continuous field reconciliation and portable-setting reviews are implemented; endpoint/removal decisions and broader live interoperability are tracked separately.

Connection reviews read at most eight mapped native accounts per page. Each freezes the profile/Google/consent and native connection generations, the exact history revision and all reviewed versions. A stale action cannot be retargeted after another review row disappears. Pausing account sync clears its controls and invalidates pending decisions.

**Keep this device's connection** publishes the reviewed local endpoints, preserving other field intent. **Add shared connection** assigns a fresh native account UUID and requires reconnection. The old account keeps its endpoints, credentials, mail and server identities, is named “(previous setup)” and remains local-only. It can later be removed through the normal account-removal review. No existing message UID or password is redirected to a different server. The shared operation retains its original optional fields and exact conflict resolutions.

Native account changes and a durable pending operation are committed before shared-history admission. Restart retries that saved operation; it does not add another native account. Remote removals require a separate decision and cannot be revived by accepting an old connection review. Account password transfer, post-enrollment linking/suppression, removal choices and actual cross-client Google verification remain open. Local imports use reconnection while credential protection awaits the recorded user choice.


Shared account removal is reviewed separately from endpoint choices. Keep on
this device records durable suppression while preserving that account, cached
mail and keychain identity. Later local edits remain local. Review removal opens
the normal local-data confirmation, including draft and unfinished-work checks;
Cancel leaves everything intact. A confirmed local removal clears stale review
controls and does not delete server mail or publish a shared removal.

Keep decisions validate the selected profile, sync consent, Google lifecycle,
native connection revision and exact reviewed remote history before committing.
No cloud tombstone is inferred from removing an account only on this device.
Global account removal, post-enrollment linking/suppression controls and password
transfer remain separate follow-ups. The saved native `profile_account_removal`
scenarios cover Keep, Cancel, confirmed removal and restart; Rust fixtures retain
actual cached mail through remote removal and reject stale choices.
