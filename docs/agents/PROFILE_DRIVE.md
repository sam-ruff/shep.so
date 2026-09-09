# Google profile files

The optional native `drive` feature in `shared/profile-core` connects the
[local history worker](PROFILE_HISTORY.md) to Google Drive. It verifies identity,
reads bounded metadata pages, imports verified operations and publishes one
durably queued operation at a time. Scripted HTTP tests exercise the production
implementation with isolated journals. No client Settings screen calls this
transport yet; live Google access and working profile sync are not established.

## Identity and scope

`Drive::connect` retains a short-lived secret access token and verifies
`about.user.permissionId`. Renewed grants must match the saved `drive:` principal.
The owning client obtains and refreshes the token through its platform Google
authorization, fences late results after identity/category changes, and keeps
refresh tokens in the existing secure platform store. Tokens are never serialized
by this transport; provider error bodies and request errors are not echoed.

Production requests use the fixed Google HTTPS endpoint, certificate validation,
no redirects, a 10-second connection timeout and a 30-second request timeout.
Loopback tests share the production redirect/timeout builder, with a shorter test
deadline and no system proxy. Endpoint injection exists only inside private tests. Run calls from an
owned provider task, never iced update/view or Flutter's UI isolate. Serialize
profile work in the client and retain ownership through accepted persistence.

The namespace is configured for the shared Shep application project, independently
of platform OAuth client IDs. Its hash is an identity label, not proof that clients
use the same Google project. A different namespace found in the visible app data
fails explicitly. A different OAuth project can expose entirely different empty
app data; this API alone cannot detect that configuration mistake. Complete the
registered-client visibility checks in the [interoperability handover](PROFILE_SYNC_HANDOVER.md)
before allowing automatic enrollment or interpreting absence as first setup.

## Immutable wire format

Files use `application/json` in `appDataFolder`, with the exact UTF-8 operation
bytes from the journal. Their name is `shep-profile-OPERATION_UUID.json`.
[App data requires the Drive app-data scope](https://developers.google.com/workspace/drive/api/guides/appdata).
Private `appProperties` contain:

| Key | Value |
| --- | --- |
| `shepType` | `profile` |
| `shepFormat` | `operation-v1` |
| `shepNamespace` | Lowercase SHA-256 of the configured namespace's UTF-8 bytes |
| `shepProfile` | Canonical nonnil profile UUID |
| `shepGeneration` | Canonical nonnil generation UUID |
| `shepOperation` | Canonical nonnil operation UUID |
| `shepSha256` | Lowercase SHA-256 of the exact stored operation bytes |

These seven properties fit Google's
[124-byte key-plus-value limit](https://developers.google.com/workspace/drive/api/guides/properties).
The shared `profile-drive-file.json` fixture describes the unchanged bytes of
`profile-operation.json`; the native wire test consumes both. Keep the fixture,
codec and future browser transport in agreement.

Before reading media, require an untrashed owned file, the private app-data space,
correct MIME type/name/metadata, a valid file ID and a declared size within the
existing 1 MiB operation bound. Recheck metadata after listing, bound streamed
media by that exact size, hash it and compare its decoded namespace/profile/
generation/operation. A provider checksum, if present, must also agree. Invalid,
changed, future-format or unowned records never become empty profiles.

## Discovery and upload recovery

`list_page` returns at most 50 metadata entries. JSON responses have a separate
256 KiB bound and reject duplicate keys. Require explicit `incompleteSearch=false`
and a files array. Empty pages with a next token remain partial; repeated current
tokens and duplicate IDs within a page fail. Google documents that
[pagination can change as files arrive or disappear](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/list).
There is no durable scan/catalog yet: its owner must record all visited tokens and
file identities in bounded background storage, reject longer cycles/duplicates,
restart expired scans and reconcile arrivals before publishing a discovery result.
Neither one final page nor zero missing received parents establishes enrollment.

`upload_next` checks the session against the immutable worker binding, obtains one
queued operation, reserves a Drive ID and waits for that reservation to persist
before posting multipart metadata and exact media. Google supports
[retrying pre-generated IDs without creating duplicates](https://developers.google.com/workspace/drive/api/guides/manage-uploads).
Only a metadata GET returning 404 permits the create attempt. Existing records are
never overwritten, patched or deleted. Successful, conflicting or uncertain POST
responses require a fresh owned metadata read and matching actual media before
the queue is acknowledged. A 409 or checksum property alone is insufficient.

At most one POST occurs per call. Failure or cancellation retains the operation
bytes and reserved ID for a later retry. A restarted worker can confirm the
committed file without posting again. A mismatching reserved file remains an
explicit conflict. These files are separate from backup retention; no backup
cleanup may remove profile operations or tombstones.

## Remaining integration

Implement durable discovery/catalog and explicit creation/enrollment, category
controls, reviewed conflict/account matching and actual account/preferences
application. Wire the same provider contract to desktop/Flutter authorization and
the separate browser client. Complete live same-project tests, Apple execution,
sync lifecycle/performance and the outstanding credential-protection decision.
No passwords, OAuth grants or mail actions are portable operations. Device-local
history remains unencrypted under R22. The existing native navigation performance
failure remains open; this transport does not change that path.
