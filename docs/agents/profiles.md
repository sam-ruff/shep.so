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

## Cross-client sync remains next

Implement OAuth/profile sync using the [Flutter interoperability handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md) and its shared profile-core operation fixtures. Desktop SQLite is a local transfer format, not the Flutter interchange protocol. The local UUID introduced here is not the shared profile identity.

First-device setup, new-device discovery, continuous merge, offline recovery and configurable enrollment remain open. Portable credential protection also needs the outstanding user choice; local import uses reconnection. Existing Drive backup/Calendar OAuth does not establish cross-client profile synchronization or live Google interoperability.
