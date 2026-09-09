# Profile records on Google Drive

`profile_sync` implements transport, causal history and native first/new-device enrollment for the [shared profile format](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_FORMAT.md). Preferences supports creating a profile or reviewing/importing an existing one, with saved category choices and recovery. An ongoing cycle now publishes local edits and applies safe remote preferences, account names and new account definitions. Conflict/removal/endpoint reviews and other remaining work stay in [TODO](https://github.com/sam-ruff/shep.so/blob/main/TODO.md).

## Shared codec

Cargo pins `shep-profile-core` with its Drive/history features to published commit `43cdcf0f70f7dbff2f80b7828eb7e570d025b09a`. It validates the same major-1 operations used by Flutter. The fictional `tests/support/profile-operation.json` is copied unchanged from `bae0d86949a0138b90e71348cef0ab434d022dc6`'s `shared/profile-operation.json`; the HTTP round trip preserves its exact bytes, Unicode and unknown optional fields. Passwords, Google grants and device settings have no representation in this metadata format.

`Replica` uses the shared causal-history worker for merge, conflicts, tombstones and local edits. It retains the same operation/reservation identity in both history and transport journals. Account application still needs the native account lifecycle and revision checks; decoding or downloading an operation is not permission to apply it.

## File identity

Files use `application/json`, parent/space `appDataFolder`, and the name `shep-profile-<operation UUID>.json`. The transport category marker remains stable across future payload versions so older clients discover unsupported records and request an update.

| Custom property | Value |
| --- | --- |
| `shepType` | `profile`, the stable profile-record category |
| `shepFormat` | `operation-v1` |
| `shepNamespace` | Lowercase SHA-256 of the exact UTF-8 application namespace |
| `shepProfile` | Canonical non-nil profile UUID |
| `shepGeneration` | Canonical non-nil generation UUID |
| `shepOperation` | Canonical non-nil operation UUID |
| `shepSha256` | Lowercase SHA-256 of the exact uploaded record bytes |

These use Drive's private `appProperties` map, matching the committed client transport and `shared/profile-drive-file.json` fixture in `9289f53`. The namespace hash keeps key-plus-value sizes within Google's 124-byte property limit. Files remain in hidden app storage; this does not share them through ordinary Drive permissions. See [custom properties](https://developers.google.com/workspace/drive/api/guides/properties) and [app-data restrictions](https://developers.google.com/workspace/drive/api/guides/appdata).

Bind a session to the verified `drive:<permissionId>` and configured application namespace. Each pass obtains its own token through the existing staged Google connection and checks Drive permission/account identity before accessing profiles. Do not reuse a token across connection changes or assume different OAuth projects share app data. Real visibility between all registered platform clients remains a required live check from the [handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md).

The desktop consumes an unchanged copy of that metadata fixture in `tests/support/profile-drive-file.json` and verifies it against the original operation bytes. Git attributes disable newline conversion for both fixtures. This convention supersedes the earlier desktop-only `bb87ac2` transport prototype, which was never connected to user profile sync. No production cloud migration was performed. Discovery selects the stable category without hiding visible namespace/version mismatches behind its query.

## Discovery and upload

Read at most 100 metadata records per HTTP page, with bounded response parsing. The separate SQLite journal owns a 32-command FIFO and records page tokens, immutable record IDs and scan revisions. Empty intermediate pages are not completion. Repeated tokens, duplicate file/operation IDs, incomplete searches, another account/generation and stale results cannot advance a scan. Completed metadata is read in pages of 50; there is no total-history page ceiling. Restart resumes the saved position. Starting another scan retires only its previous local discovery, preserving pending uploads and profile history.

Reserve a Drive ID, then persist that ID and exact bytes before upload. Only the journal can create the transport's `DurableUpload`. Multipart requests fit the shared 1 MiB per-record bound. A lost reply or conflict verifies the same ID, ownership, name, identity, size and checksum; when Google omits its server checksum, fetch and compare the contents. Never normalize committed JSON or reserve a replacement ID for the same pending operation. Google's [pre-generated ID contract](https://developers.google.com/workspace/drive/api/guides/manage-uploads#use_a_pre-generated_id_to_upload_files) permits retry without creating a duplicate file.

Transport has no update/delete endpoint and rolling backup retention never selects these files. Local acknowledgment is separate from remote commitment; a failed acknowledgment retries the same saved record. The journal is provider state, not an account-password cache or an enrollment choice. Its callers must still enforce sync/category toggles, current connection lifecycle, complete ancestry and local application revisions.

## Causal pull and publish

`Replica::pull` resumes incomplete discovery or refreshes a finished scan, then
downloads/imports records in bounded pages. Missing parents remain pending in the
shared journal; draining finishes ready batches before a `Pulled` proof is issued.
Field/version reviews remain paged. Neither downloaded records nor such a proof
automatically applies account definitions or settings.

`publish_next` processes one queued edit per call. It rejects a proof from another
device, changed history or replaced scan, and compares all known remote/core/
transport IDs before reserving anything. Existing observed records are adopted
only with the exact operation/content identity. Commit ordering is shared-core
reservation, transport preparation, verified upload, transport acknowledgment,
then shared-core confirmation. Interrupted stages retain the original ID/bytes.

An empty result requires an explicitly reviewed new-profile intent and a history
containing only unconfirmed local edits. Previously observed/uploaded profiles
cannot use that path to recreate a missing generation. Multiple initial queued
edits may finish on the same pass; another pass discovers those confirmed files.
Deletion markers remain authoritative through stale/offline edits. Conflicts
retain both values until a revision-checked explicit resolution arrives.

This is a backend kernel. Enrollment must still finish local suppression and
account application. Its bounded coordinator fences Google lifecycle changes, retains an upload task through durable acknowledgment, and observes stop/category changes between writes. Current pulls re-read full
history: incremental polling/caching remains required before continuous operation
is finished. The per-workspace provider directory is protected by database-transfer guards.

## Enrollment and initial publication

Device-local enrollment stores the verified principal/namespace/profile/generation,
Create/Join origin, enabled/category choices, a revision and initial completion
state. Changes use the mail-cache owning worker. Options can be saved without
network/keychain access; enabling requires the appropriate saved Google grant.
Google disconnect pauses enrollment in the same transaction. Late results cannot
re-enable it. Database import archives enrollment, its seed and the join identity map, then requires new
device discovery rather than replaying the source's choices or history pointers.

`setup::discover` returns a private completed-scan/local-revision proof. Explicit
Create consumes that review and atomically persists all initial values, account
ID mappings and operation UUIDs. Legacy account IDs receive one saved shared UUID.
Before each history edit, save its original expected revision; retry the exact
request across restart. Seeds use the shared codec and split at its per-record
change bound. Missing/corrupt records are errors, never a fresh setup.

`setup::publish` replays that seed, pulls verified history, and finishes one
durable upload at a time. It checks local intent between writes. An in-flight
write still reaches both journal acknowledgments after a stop/disconnect; setup
stays pending and reports that an upload was saved. Conflicts, removals, stale
reviews and missing existing generations cannot become a completed first setup.
The engine coordinator retains ownership through these acknowledgments.

The explicit metadata adapter preserves IMAP/POP3 and independent SMTP security,
authentication and Sent options, and returns an account review candidate.
Joining can save that definition, with receiving/sending paused until explicit
credential reconnection. It never imports a password. Settings application commits
a bounded conflict-free page atomically against local preferences/connection/
Google/enrollment revisions. It currently implements appearance, quoted replies,
external-image policy, unified inbox, cross-account moves, conversation grouping
and unread badges. Device fields and backend metadata stay unchanged. Shared
preview-line values/extensions remain in history; other portable settings still
need shared-contract support and native implementation.

Account endpoint/removal reviews, conflict resolution
and enrolled-device incremental polling remain open. Initial enrollment is separate from a verified later publication. Password transfer still requires the outstanding protection choice.

## Verification boundary

Run `cargo test --all-features profile_`, `python3 scripts/test_profile_core.py` and `cargo test --all-features backup::drive`. The Python runner tests a disposable copy of the exact locked Git crate with its own committed test lock, leaving dependency/client checkouts untouched. `--update-lock` is only for a reviewed dependency update. Tests use production HTTP parsing against the scripted loopback server and real isolated SQLite files. They cover Unicode/extension preservation, restart/lost replies, metadata/content corruption, scope/identity rejection, pagination/revision failures, bounded reads and cancellation after queue admission. Two independent device stores exercise actual HTTP pull/publish, offline conflicts/resolution, account/profile removal, lost upload replies, both local acknowledgment gaps, stale/foreign discovery and more than 100 reverse-ordered ancestors. Existing Drive backup protocol tests protect the shared HTTP helper. This is protocol evidence; real Google enrollment and cross-client behavior remain unverified. Native fixture evidence is described below.

## Native controls and ownership

**Preferences → Accounts → Profiles and sync** discovers app-data records and
reviews an explicitly named first-device profile before uploading. Account and
settings choices save independently of provider work. Initial completion is
labeled as an initial copy: ongoing local edits and conflict reviews remain
unfinished. Setup failure retains the original seed and
offers Resume; it must never create a replacement operation to hide a failed save.

The owning profile coordinator receives at most 32 commands, separate from the
provider queue. Local category patches contain only touched fields, so a stale
screen cannot replace a newly created enrollment or re-enable a disconnected
device. The UI keeps later gestures separate from its single in-flight save.
Read-only HTTP and provider-slot waits can be cancelled; accepted history/cache
writes and admitted uploads retain ownership until their receipts settle.
Stop/close interrupts reads and checks between upload steps.

Production creation uses application namespace `so.shep`; an existing selection
retains its saved namespace. All participating clients must use the same exact
namespace and the verified Google project described in the handover. The chosen
journal directory is `profile-sync/` beside each workspace cache. It contains
`drive.sqlite` and one history file per shared binding hash, plus SQLite sidecars
and ownership files. Export rejects destinations inside this directory and
hard-link/symlink aliases to existing members. Another workspace has its own
journals even when it selects the same shared profile.

The MCP fixture uses an owned loopback Drive server and fake token only in the
nondefault test-support preview. Saved scenarios cover initial review/publication,
failure/retry/opt-out, local choices while a read is held, close/restart and compact
dark controls. These are native-control and protocol tests, not genuine Google
login or cross-client access evidence. Never point the harness at personal data.

A failed local enrollment read keeps category controls disabled. After a write
and subsequent status-read failure, later unsent choices remain available for
explicit retry; they do not launch another write against the stale snapshot or
trap a later window close. Actual admitted work still drains. The `invalid-local`
native fixture checks the error screen, disabled controls and graceful restart.


## After sign-in

A verified Google connection schedules one background discovery per session.
A saved **Check for shared profiles after Google sign-in** choice controls it.
**Not now** persists an opt-out; it can be enabled again in Profiles and sync.
Discovery errors keep a retry path and never appear as an empty Drive account.

No existing profiles produces an optional setup prompt. Multiple profiles or a
populated/customized workspace use the picker and import review. One complete
profile can enroll an untouched workspace automatically, with an atomic final
check for new local data or opt-out. Imported accounts still require Reconnect.
The result shows the profile and applied counts without switching the active tab.
The native fixture covers the post-login path, not actual browser OAuth consent.
Ongoing publication/application and genuine Google interoperability remain open.

## Existing-device discovery and import

The shared catalog owns its separate SQLite connection and observation journals.
It captures a Drive start token before listing, verifies each record, and replays
changes before returning a completed review. Later discovery uses the saved change
token. Restart retains unfinished progress; errors cannot masquerade as an empty
account. See Google's [change tracking](https://developers.google.com/workspace/drive/api/guides/manage-changes).
Native pages contain at most 50 profile summaries, with names, account/settings
counts and unavailable/conflicted states. The observation directory and its nested
files/aliases are protected by database transfer guards.

Choosing Review pulls the selected profile into its own causal history. The review
retains its local enrollment/preferences/connections/Google revisions and the
history's device/revision; it sends only counts and up to eight account summaries
to iced. Unsupported connection extensions block account application. Tombstoned
accounts stay absent. Unknown optional settings remain in shared history.

Import re-reads that frozen history and atomically saves enrollment, account
mappings and supported preferences through the mail-cache owner. Existing local
accounts/mail remain intact. Each imported account receives a fresh local UUID;
the shared-to-local mapping is persisted as `profile_join_v1`. Imported definitions
are listed in `profile_reconnect_v1`, excluded from background sync, and rejected
by provider account lookup until SaveAccount has acknowledged its device credential.
Preferences shows **Reconnect** for these accounts. Removing one clears its pending
reconnect marker; database import archives the source-device join mapping.

A saved review UUID makes retry after a lost acceptance acknowledgment idempotent.
Newer local preferences/categories or Google lifecycle reject unapplied reviews.
Read cancellation remains interruptible; admitted application commits drain before
close. Account linking
between already-populated devices, conflict/removal controls and protected password
transfer still need implementation. Real cross-client Google visibility is unverified.

## Common values and local edits

Enrollment saves a device-local `profile_replication_v1` checkpoint. Creation
retains the original seed values before pulling later records; joining commits
accepted raw fields and the local account mapping with the import. Retrying setup
or acceptance cannot resnapshot newer native preferences as already-shared values.
Database import archives this source-device state rather than replaying it.

The capture API reserves one exact local operation against the last common
revision of its field. Newer native edits remain separate, and optional shared
fields survive export. History admission returns a sealed receipt only after
verifying the operation is still the field's current version. An idempotent Edit
reply can describe newer history, so its revision alone cannot advance the local
basis. Category pauses retain the original pending request; admitted receipts
drain without restoring older options or local values.

The periodic loop uses these APIs. Conflict controls still need implementation;
conflicted edits retain their exact requests while unrelated fields progress.
Old profiles without a trustworthy common basis require recovery review, not an
assumed snapshot of today's cloud values. Existing unmapped local accounts require
explicit linking; local-only removal must not publish a shared tombstone.

## Complete setup before import

Desktop now uses the initialization protocol published by Flutter in
[`184b98a`](https://github.com/sam-ruff/shep.so/commit/184b98a), distributed with the
owned loopback harness in [`43cdcf0`](https://github.com/sam-ruff/shep.so/commit/43cdcf0f70f7dbff2f80b7828eb7e570d025b09a).
The shared source and three JSON fixtures were copied from that immutable client
commit; the active client worktree was not changed.

Setup persists separate start and completion operations around its metadata
chunks. Their IDs and expected revisions survive restart. Every record requires
`initialization-v1`. The shared worker verifies a completion descended from the
start and written by the same originating device, with all required ancestry
present. Native discovery and import require its initialized state. A finished
Drive listing, visible name or complete-looking settings page is insufficient.

An older saved seed can acquire these markers automatically only while its
history is empty and no operation has been admitted. Keep the existing metadata
and UUIDs. Previously admitted legacy operations cannot be rewritten or assigned
new ancestry; retain them for explicit recovery. Older completed profiles also
need a migration/recovery flow before enrolling new devices in this format.

Desktop applies eight shared settings, including Tooltips. Swipe actions, sender
pictures and preview-line count have no matching native setting yet; their
validated values stay in history and are neither overwritten nor counted as
applied preferences. This is format and fixture interoperability; actual
cross-client Google OAuth/app-data access remains unverified.


## Ongoing device updates

The existing bounded coordinator runs one cycle at a time. After enrollment it
checks after two seconds, then every 30 seconds; failure backs off for 60 seconds.
**Sync now** retries immediately. Disable the profile or either category in the
same card; Stop pauses the current session. Read-only work cancels on close, while
an admitted history/cache/upload write retains ownership through its receipt.

Before pulling, the cache owner captures exact local field intent and the history
owner admits it. Deferred conflicts/category pauses keep their original UUID and
basis. Remote application rechecks the selected profile, Google lifecycle,
category and each current native value in its transaction. A racing local edit
keeps its earlier basis. Typed native preference writes change only touched shared
fields, including deliberate reversions while earlier writes are pending.
The cache also persists per-field native edit generations, including account
names. Returning to an earlier value during a pull is still a new intent. History
receipts acknowledge their captured generation; remote application cannot erase
a later reversion. Database import archives these source-device generations.

New shared accounts get fresh local UUIDs and require Reconnect. Names can update;
changed existing endpoints and removals retain local accounts for review. No
existing credential is sent to a downloaded server. Unsupported settings remain in
history. Account linking, endpoint/removal/conflict review controls, incremental
history downloads, remaining portable settings and protected credentials remain
unfinished. Full-history checks and fixture success are not live Google evidence.
