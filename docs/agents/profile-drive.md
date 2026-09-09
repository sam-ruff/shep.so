# Profile records on Google Drive

`profile_sync` implements transport and durable discovery/upload preparation for the [shared profile format](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_FORMAT.md). Enrollment, applying changes and native sync controls remain open in [TODO](https://github.com/sam-ruff/shep.so/blob/main/TODO.md). This backend alone does not provide continuous account sync.

## Shared codec

Cargo pins `shep-profile-core` to client commit `bae0d86949a0138b90e71348cef0ab434d022dc6`. It validates the same major-1 operations used by Flutter. The fictional `tests/support/profile-operation.json` is copied unchanged from that commit's `shared/profile-operation.json`; the HTTP round trip preserves its exact bytes, Unicode and unknown optional fields. Passwords, Google grants and device settings have no representation in this metadata format.

The client branch's newer causal-history worker is the integration point for merge, conflicts, tombstones and local edits. Keep its operation/reservation identity consistent with the transport journal. Account application still needs the native account lifecycle and revision checks; decoding or downloading an operation is not permission to apply it.

## File identity

Files use `application/json`, parent/space `appDataFolder`, and the name `shep-profile-<profile UUID>-<generation UUID>-<operation UUID>.json`. The transport category marker remains stable across future payload versions so older clients discover unsupported records and request an update.

| Custom property | Value |
| --- | --- |
| `shepProfile` | `1`, the profile-record category |
| `shepNamespace` | Lowercase SHA-256 of the exact UTF-8 application namespace |
| `shepProfileId` | Canonical non-nil profile UUID |
| `shepGeneration` | Canonical non-nil generation UUID |
| `shepOperation` | Canonical non-nil operation UUID |
| `shepSha256` | Lowercase SHA-256 of the exact uploaded record bytes |

These use Drive's `properties` map, available to callers with access to the file. The namespace hash keeps key-plus-value sizes within Google's 124-byte property limit. Files remain in hidden app storage; this does not share them through ordinary Drive permissions. See [custom properties](https://developers.google.com/workspace/drive/api/guides/properties) and [app-data restrictions](https://developers.google.com/workspace/drive/api/guides/appdata).

Bind a session to the verified `drive:<permissionId>` and configured application namespace. Each pass obtains its own token through the existing staged Google connection and checks Drive permission/account identity before accessing profiles. Do not reuse a token across connection changes or assume different OAuth projects share app data. Real visibility between all registered platform clients remains a required live check from the [handover](https://github.com/sam-ruff/shep.so/blob/feat/mobile-web-clients/docs/agents/PROFILE_SYNC_HANDOVER.md).

## Discovery and upload

Read at most 100 metadata records per HTTP page, with bounded response parsing. The separate SQLite journal owns a 32-command FIFO and records page tokens, immutable record IDs and scan revisions. Empty intermediate pages are not completion. Repeated tokens, duplicate file/operation IDs, incomplete searches, another account/generation and stale results cannot advance a scan. Completed metadata is read in pages of 50; there is no total-history page ceiling. Restart resumes the saved position. Starting another scan retires only its previous local discovery, preserving pending uploads and profile history.

Reserve a Drive ID, then persist that ID and exact bytes before upload. Only the journal can create the transport's `DurableUpload`. Multipart requests fit the shared 1 MiB per-record bound. A lost reply or conflict verifies the same ID, ownership, name, identity, size and checksum; when Google omits its server checksum, fetch and compare the contents. Never normalize committed JSON or reserve a replacement ID for the same pending operation. Google's [pre-generated ID contract](https://developers.google.com/workspace/drive/api/guides/manage-uploads#use_a_pre-generated_id_to_upload_files) permits retry without creating a duplicate file.

Transport has no update/delete endpoint and rolling backup retention never selects these files. Local acknowledgment is separate from remote commitment; a failed acknowledgment retries the same saved record. The journal is provider state, not an account-password cache or an enrollment choice. Its callers must still enforce sync/category toggles, current connection lifecycle, complete ancestry and local application revisions.

## Verification boundary

Run `cargo test --all-features profile_` and `cargo test --all-features backup::drive`. Tests use production HTTP parsing against the scripted loopback server and real isolated SQLite files. They cover Unicode/extension preservation, restart/lost replies, metadata/content corruption, scope/identity rejection, pagination/revision failures, bounded reads and cancellation after queue admission. Existing Drive backup protocol tests protect the shared HTTP helper. This is protocol evidence; real Google enrollment and cross-client/native UI behavior remain unverified here.
