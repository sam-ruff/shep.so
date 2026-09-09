# Profile records on Google Drive

`profile_sync` implements transport, causal history and first-device enrollment primitives for the [shared profile format](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_FORMAT.md). Existing-profile enrollment, account application and continuous sync remain open in [TODO](https://github.com/sam-ruff/shep.so/blob/main/TODO.md). Preferences exposes initial profile creation, category choices and recovery; this does not yet provide continuous account sync.

## Shared codec

Cargo pins `shep-profile-core` with its history feature to published client commit `9289f5327b71bb6aaff463ee965eab48c13df85a`. It validates the same major-1 operations used by Flutter. The fictional `tests/support/profile-operation.json` is copied unchanged from `bae0d86949a0138b90e71348cef0ab434d022dc6`'s `shared/profile-operation.json`; the HTTP round trip preserves its exact bytes, Unicode and unknown optional fields. Passwords, Google grants and device settings have no representation in this metadata format.

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
re-enable it. Database import archives enrollment and its seed, then requires new
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
The future engine coordinator must retain ownership through these acknowledgments.

The explicit metadata adapter preserves IMAP/POP3 and independent SMTP security,
authentication and Sent options, but returns an account review candidate only.
It does not connect an account or import a password. Settings application commits
a bounded conflict-free page atomically against local preferences/connection/
Google/enrollment revisions. It currently implements appearance, quoted replies,
external-image policy, unified inbox, cross-account moves, conversation grouping
and unread badges. Device fields and backend metadata stay unchanged. Shared
preview-line values/extensions remain in history; other portable settings still
need shared-contract support and native implementation.

Existing-device enrollment, account reconnection/removal reviews,
ongoing local change capture, conflict resolution and incremental polling are
not connected yet. A successful initial seed is not proof that later local edits
have synced. Password transfer still requires the outstanding protection choice.

## Verification boundary

Run `cargo test --all-features profile_`, `python3 scripts/test_profile_core.py` and `cargo test --all-features backup::drive`. The Python runner tests a disposable copy of the exact locked Git crate with its own committed test lock, leaving dependency/client checkouts untouched. `--update-lock` is only for a reviewed dependency update. Tests use production HTTP parsing against the scripted loopback server and real isolated SQLite files. They cover Unicode/extension preservation, restart/lost replies, metadata/content corruption, scope/identity rejection, pagination/revision failures, bounded reads and cancellation after queue admission. Two independent device stores exercise actual HTTP pull/publish, offline conflicts/resolution, account/profile removal, lost upload replies, both local acknowledgment gaps, stale/foreign discovery and more than 100 reverse-ordered ancestors. Existing Drive backup protocol tests protect the shared HTTP helper. This is protocol evidence; real Google enrollment and cross-client behavior remain unverified. Native fixture evidence is described below.

## Native controls and ownership

**Preferences → Accounts → Profiles and sync** discovers app-data records and
reviews an explicitly named first-device profile before uploading. Account and
settings choices save independently of provider work. Initial completion is
labeled as an initial copy: joining existing profiles, ongoing local edits and
conflict reviews remain unfinished. Setup failure retains the original seed and
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
