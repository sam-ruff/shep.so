# Durable profile discovery

The optional native `drive` feature now includes a saved remote catalog in
`shared/profile-core/src/drive/catalog/`. It verifies operation files, resumes
interrupted scans and keeps small profile summaries. The [Flutter client](PROFILE_MOBILE.md)
now calls it from Profiles and sync through its saved Google connection. Desktop
and separate browser integration remain open. Creation, enrollment and applying shared
accounts/preferences remain in the [sync handover](PROFILE_SYNC_HANDOVER.md).

## Scan and recovery

Capture and persist a Drive change-stream token before listing profile files.
Stage each complete metadata page before downloading its records, then replay
changes from that token. Google returns current file state through its
[change stream](https://developers.google.com/workspace/drive/api/guides/manage-changes).
This closes the arrival gap during listing without claiming an atomic snapshot.

Request `spaces=appDataFolder`, `includeRemoved=true` and
`restrictToMyDrive=false`; restricting to My Drive would exclude app data.
Require either a continuation token or a final `newStartPageToken`. Drive change
tokens do not expire; invalid responses still fail explicitly. See the
[changes API](https://developers.google.com/workspace/drive/api/reference/rest/v3/changes/list).
Keep this distinct from a file-list continuation that needs a full restart.

Persist visited tokens, full-scan file IDs and pending metadata in SQLite. Longer
pagination loops and duplicate file-list IDs fail. Repeated change events are
allowed, but every occurrence must still match the immutable file and actual
media. A known file disappearing, changing identity/content or losing its profile
marker leaves a saved error. A full rescan retains known identities and fails if
any remain missing. Unrelated unknown app-data changes are ignored.

File acceptance spans two databases, so order matters:

1. Verify metadata, exact size/digest and decoded operation identity.
2. Save a prepared file identity in the catalog.
3. Import original bytes into its separate remote observation journal.
4. Save the summary and verified receipt, then remove that pending item.

If step 4 fails after the history commit, the prepared identity survives. Retry
imports idempotently; a full rescan cannot forget a now-missing file. Advance the
page only after its imports and bounded ready-history drains finish. Complete
means the observed stream is caught up and known files are accounted for. Missing
ancestry, conflicts and removed generations remain explicit in the summaries.

## Ownership and observations

`Discovery::open(path, Scope)` binds the catalog to a verified `drive:` principal
and configured application namespace. Every `advance(&Drive)` checks that same
binding. One background thread owns the catalog connection and at most one remote
journal, behind a 32-command queue. An exclusive canonical-path file lock also
excludes another process. New Unix files/directories use private permissions;
SQLite metadata remains unencrypted under R22.

Each advance admits at most one provider step. Read-only observations remain
available while HTTP is held. Cancelling a GET leaves saved progress intact;
accepted SQL commands finish even if their observers disappear. Explicit full
refresh increments the revision, rejecting old HTTP data and errors. Saved
failures need `retry(expected_revision)` or a full refresh. `close` drains after
the other handles are dropped.

| Observation | Bound and meaning |
| --- | --- |
| State | Revision, phase, counts, pending/incomplete work, saved error and last completed revision |
| Profiles | Keyset pages of at most 50 summaries; summaries can be provisional during refresh |
| Name | One current value or explicit conflict, with no timestamp winner |
| Accounts/settings | Visible connection definitions and setting intents, including explicit resets; no credentials or activation claim |
| Work | At most 50 pending metadata entries, one bounded record, history drains of 32 |

Remote observations have their own journals. They never open the enrolled local
journal or replace its offline edits/device identity. Token/file history grows on
disk; no total-history cap or catalog performance result is claimed.

## Verification and next integration

Scripted HTTP plus real temporary SQLite tests cover interrupted pages, arrivals,
repeated events, missing files, long loops, duplicate operation identities,
conflicting names, missing parents, tombstones and 52-profile paging. A real
SQLite trigger fails the catalog receipt after history commits. Held success and
failure responses test revision fencing. Cancelled observers, queue saturation,
canonical aliases and an independent child process test ownership. These are
host protocol/storage tests; Android control fixtures exercise the Flutter view; actual Google wire tests remain host-only.

Flutter now binds its saved grant, verified principal and lifecycle generation
through the native session bridge. Discovery may open its isolated local catalog
before the secure identity save, but no profile content is displayed or discovery advanced
until that binding is acknowledged. Continue desktop/browser integration; a
caller-supplied history binding alone is not authentication.
Enrollment must import original records into an independently owned local journal
through a bounded reviewed transfer; copying observation SQLite would clone the
device UUID. Preserve the [causal initialization barrier](PROFILE_PUBLICATION.md)
and tracked own-upload receipts used by Flutter publication. Later deletion of
an uploaded file must remain explicit even before a subsequent cloud listing.

Apply accounts through reviewed lifecycle operations, preserving local mail and
drafts. Never reuse credentials against a remotely changed endpoint or import a
device-specific credential slot. Keep explicit reconnect state, local suppression,
category toggles, conflict review, protected credential choice, live same-project
visibility and browser/Apple execution open. An empty catalog alone does not
complete the separate legacy-backup migration probe.
