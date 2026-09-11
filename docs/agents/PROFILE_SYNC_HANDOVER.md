# OAuth and cross-device profiles: implementation handover

This is the next implementation priority (client R75, R02/R49 and main's
R92). **Full cross-client profile sync is unfinished.** Desktop now has reviewed
ongoing sync for eight preferences, alongside Google OAuth, encrypted Drive backups
and initial enrollment; since the 2026-09-09 merge the desktop implementation is
main's `src/profile_sync` (see [profiles](profiles.md) and [profile-drive](profile-drive.md)). Complete account/settings sharing remains open. This document records the
contract to implement and verify in Rust desktop and Flutter, with browser parity
tracked separately. Keep the linked entry first in the root TODO until delivery.

See [desktop profiles](PROFILE_DESKTOP.md), [Flutter enrollment](PROFILE_ENROLLMENT.md) and the [desktop reconciliation engine](PROFILE_RECONCILIATION.md) for the current boundaries. Desktop scheduling and controls now have protocol/storage/native coverage. Checked desktop preference decisions now have paged native controls and durable recovery; complete categories/settings/accounts and Flutter/browser ongoing reconciliation remain open.

## Existing code to reuse

| Area | Existing implementation | Integration requirement |
| --- | --- | --- |
| Desktop Google authorization | `src/providers/google.rs`, `google/tokens.rs`, `google/scopes.rs` | System browser, PKCE/state, explicit Drive/Calendar consent, staged grants, refresh and scope checks exist. Desktop discovery now verifies profile identity and uses the durable shared catalog; reviewed publication and enrollment now apply selected account metadata/preferences. Continue automatic setup and reconciliation. Do not replace staged activation with a token overwrite. |
| Desktop Google lifecycle | `src/store/google_lifecycle.rs`, `src/engine/google_lifecycle.rs` | Local disconnect, cleanup retries and stale-result protection must also fence profile jobs. |
| Desktop Drive transport | `src/backup/drive.rs`, `src/backup/journal.rs` | Reuse bounded HTTP, pagination, identity checks and acknowledged-upload recovery. Profile files need their own namespace and retention rules. |
| Shared profile Drive transport | `shared/profile-core/src/drive/` | Optional native provider verifies the principal, owned bounded file pages and exact remote media against durable journal reservations. [Wire contract](PROFILE_DRIVE.md) and scripted HTTP tests exist; [Durable discovery](PROFILE_DISCOVERY.md) now resumes verified scans and change replay in separate observation journals; Flutter now binds saved grants and discovery controls through [native sessions](PROFILE_MOBILE.md); Flutter initialized publication and reviewed account/preferences enrollment are connected. Desktop discovery, reviewed initial publication and account/preferences enrollment are connected; continue automatic desktop setup/reconciliation and browser publication/enrollment. Live same-project access is unverified. |
| Desktop account definitions | `src/model.rs` (`Account`, `Preferences`), `src/store/connections.rs` | Map explicitly to portable fields. SQLite and local credential-slot identifiers are not the sync wire format. |
| Shared causal metadata history | `shared/profile-core/src/history/`, `flutter/rust/src/profile_history.rs` | One owning worker, immutable records, conflicts/tombstones and durable upload reservations have native/FFI/Android evidence. [History boundaries](PROFILE_HISTORY.md): the optional provider now connects HTTP, with a separate durable discovery catalog; Flutter publication and reviewed enrollment/application are connected; desktop reviewed account/preferences application is connected; browser application and continuous reconciliation remain open. |
| Flutter accounts | `flutter/lib/data/accounts.dart`, `native_repository.dart`, `flutter/rust/src/accounts.rs` | Retain stable account IDs across clients; use native lifecycle operations when applying changes. |
| Flutter credentials | `flutter/lib/data/credentials.dart`, `flutter/rust/src/connections.rs` | Stage a complete incoming/SMTP pair in secure storage, then activate its local slot atomically. Never copy another device's slot ID. |
| Flutter Google connection | `flutter/lib/model/google_connection.dart`, `flutter/lib/data/google_native.dart` | Native SDK consent and durable local metadata/cleanup pass host/Android/Appium/offline-preview checks; see [configuration and limits](GOOGLE_MOBILE.md). Flutter profile discovery now binds the saved grant to its verified Drive principal. Reviewed Flutter enrollment/application is connected. Automatic restore, safe switching, continuous reconciliation and live provider verification remain open. |
| Flutter settings | `flutter/lib/data/settings_store.dart`, `flutter/lib/model/preferences.dart` | Portable settings currently differ from desktop fields. Add explicit mappings and preserve unsupported fields. |
| Browser beta login | `backend/src/google.rs`, `web/src/auth.ts` | Existing identity verification gates backend access. It does not grant Drive, Calendar or Gmail access. |

Desktop `e16590c` and client `1ea12a6` were inspected for the profile mappings. Desktop
storage foundation `db8c82a` subsequently moved the mail cache/local leases to a
32-command owning worker and updated rusqlite to 0.40.2 / SQLite 3.53.2 after
reproducing the old 3.51.1 Unix WAL open/close deadlock. Its 546 Rust/adapter tests,
49 Python tests, 15 selected native scenarios and Windows cross-check pass;
see the desktop completion log. Port/review the dependency fix and channel
ownership deliberately before adding a second snapshot connection to the client
cache; neither that foundation nor this document implements database transfer.
Development
continues on separate worktrees: desktop main and client
`feat/mobile-web-clients`. Review newer changes before porting; do not merge the
entire desktop database schema into Flutter.

## Google application and identity

Create the desktop, Android, iOS and browser OAuth clients under the same Shep
Google Cloud project, with the correct platform registration for each. Google
describes cross-client consent in terms of one project; use separate production
and test projects. Never reuse a desktop loopback redirect as a mobile callback.
Use platform-supported Google authorization and the system browser, not an
embedded email WebView. See [cross-client identity](https://developers.google.com/identity/protocols/oauth2/cross-client-identity)
and [native authorization](https://developers.google.com/identity/protocols/oauth2/native-app).

Profile storage uses `drive.appdata` and `appDataFolder`. It is hidden app storage
for the signed-in Google account. It cannot be shared using ordinary Drive file
permissions, so “sharing” here means that person's Shep installations. Do not
request full Drive access or publish account files in My Drive. See
[Google's app-data rules](https://developers.google.com/workspace/drive/api/guides/appdata).

**Required live prerequisite:** prove that each registered platform client can
list and read the same synthetic app-data file created by the other client, in
both directions. Same-project configuration is the intended setup, not evidence
that the deployed clients already share a namespace. A different custom OAuth
project must not be silently treated as an empty existing profile. Explain the
application mismatch and offer explicit migration using a portable export.

Bind local sync state to the verified Google identity, application namespace and
profile UUID. Google identity comes from validated provider responses, never a
typed email address. An ID token is not a Drive access token. Validate issuer,
audience/authorized client, expiration and nonce when using OpenID Connect.
Request identity scopes for sign-in and Drive permission for profile sync;
Calendar permissions are separate opt-ins. Gmail provider authorization remains
a separate feature. Desktop login now has independent Drive and Calendar off/read/edit choices,
bound to staged retry and activation. Profile identity scopes and equivalent
mobile/browser provider consent still need implementation.

The current desktop `BackupTarget` also binds an upload to its OAuth client ID.
Do not reuse that exact key as a shared profile ID: Android, iOS and desktop have
different registered client IDs. Share the configured application namespace and
verified user/profile identity while retaining client-specific grant binding
locally. The live cross-client file visibility test above must include ownership
metadata; a same-name file alone is not proof of compatibility.

Each device obtains and stores its own grant. Refresh tokens, local grant IDs,
client secrets, credential slot IDs and cleanup journals must never travel in a
profile. Scope denial must leave other enabled services usable. A cancelled or
failed account switch retains the previous committed connection. A late result
from the previous identity cannot publish, apply or delete the new identity's data.

## First login and continuing use

| Situation | Required behavior |
| --- | --- |
| First setup, no cloud profiles | After successful discovery, offer **Sync accounts and settings** with an opt-out. Either desktop or Flutter may create the first named profile. Turning it down keeps a local setup and can be reversed in Preferences. |
| New device, one existing profile | Discover and enroll it automatically after sign-in and any required unlock. Show the profile and applied account/settings counts. Account credentials that cannot be imported show **Reconnect**, never a false signed-in state. |
| New device, multiple profiles | Show a profile picker with account counts and last sync status. Permit a separate new profile. Do not merge different profiles automatically. |
| Existing local accounts, first enrollment | Preserve local mail/drafts. Match stable shared IDs; review potentially duplicate legacy accounts and conflicting settings before replacing anything. Offer cloud profile, separate profile or reviewed merge. |
| Enrolled device | Apply remote changes in the background; persist local edits before uploading. Preserve newer local edits through delayed remote results. Show pending, synced, offline and actionable error states. |
| Offline or locked credentials | Continue cached work and queue local changes durably. Retry with backoff after connectivity/unlock; restart must retain the same changes. Never treat a failed list as an empty profile. |
| Sync disabled | Stop new reads/writes for that category/profile on this device. Keep local data and the remote profile. An already committed upload is still acknowledged; prevent later callbacks from re-enabling sync. |
| Sync re-enabled | Fetch and reconcile from the saved common history before publishing offline edits. Present conflicts; do not upload an old whole-settings snapshot over newer remote values. |
| Account removed | Distinguish **Remove from this device** from **Remove from synced profile**. Local-only suppression is durable so the next poll does not re-add it. Shared removal publishes a tombstone; other devices review locally unsent/pending work before removal. |
| Profile changed | Flush or retain that profile's pending edits, then switch to isolated state. Never send profile A's pending writes to profile B. |
| Google disconnected here | Stop this device's sync and remove its local grants with retryable cleanup. Retain cached mail and remote data. Do not revoke the project-wide grant on behalf of other devices. |
| Cloud profile deleted/reset | Require an explicit reviewed action separate from disconnect. Offline devices must not recreate a deleted profile from stale state; resuming requires re-enrollment into a new generation. |

Preferences needs a clear **Profiles and sync** section: Google account,
connect/switch/disconnect, profile picker/name, master sync switch, category
switches, credential-transfer status, last successful sync, pending changes,
conflict review, **Sync now**, and recover/reconnect controls. Sync completion
must not steal focus or reset another Preferences edit. Device settings may
override portable defaults without publishing those overrides back to the profile.

## Versioned interchange to implement

Do not serialize `Preferences` wholesale or use SQLite as the shared profile.
The initial metadata codec and native/WASM fixtures now live in `shared/profile-core`;
see [the implemented format subset](PROFILE_FORMAT.md). Native causal metadata history now has independent-store and Android bridge coverage;
see [the history API](PROFILE_HISTORY.md) and [Flutter publication](PROFILE_PUBLICATION.md).
Reviewed desktop/Flutter publication and initial account/preferences application
are connected. Browser publication/enrollment, continuous reconciliation and the
remaining settings/credential format are still open. The following is a design target, **not a shipped
wire format**; freeze exact field names/enums and crypto parameters together with
the implementation and golden fixtures before either client writes production
files.

- Every envelope identifies format name, major/minor version, required
  capabilities, profile UUID, profile generation, device UUID and immutable
  operation UUID. Generate a fresh device UUID on installation/import; do not
  clone one from a desktop transfer.
- A profile contains its display name, account records keyed by stable UUID,
  named portable setting fields, optional protected credential records and
  deletion tombstones. Never use an email address or device credential slot as
  the record key. Keep incoming and outgoing server configuration together when
  reviewing identity changes.
- Account fields map to the existing common names: `id`, `name`, `email`,
  `protocol`, `host`, `port`, `username`, `incoming_security`, `incoming_auth`,
  `smtp_host`, `smtp_port`, `smtp_username`, `smtp_security`, `smtp_auth`,
  `smtp_separate_password`, `sent_copy` and `sent_folder`. Resolve legacy SMTP
  security defaults explicitly before export. Reject unsupported authentication
  or weakened security rather than silently downgrading it.
- Portable categories cover account definitions, appearance and reader settings,
  shortcuts, contacts, image policy/sender/domain exceptions, notification
  preferences, mail organization and portable backup policy. Keep category
  toggles local to each enrollment, so one device cannot turn another's sync on.
- Keep absolute paths, window position/size and pane geometry, runtime status,
  last-backup/readiness metadata, local notification permissions, delivery and
  credential journals, selected message/cache UID, local device identifiers and
  Google grant state local. A shared notification preference cannot grant an OS
  permission. Per-message image exceptions need a portable message identity
  before they may sync.
- Preserve unknown optional fields through read/edit/write. A client that cannot
  safely preserve them must become read-only for that record. Reject an unknown
  major version or required capability with an upgrade instruction; never replace
  it with an empty default profile. Bound parsing and process large collections
  in pages without imposing the existing mail-backup ceiling on profile history.

## Concurrency, failures and removal

Use a bounded command channel and a worker that owns each enrollment's state.
Persist local operations and the last applied remote checkpoint transactionally.
The UI receives small progress/state events; it must not await HTTP, keychain,
crypto or database work. Speculative discovery cannot fill the interactive queue.

An existing encrypted Drive backup is not an existing continuous profile.
If discovery finds only legacy backups, offer an explicit password-prompted
restore/migration using the existing `backup::Snapshot` decoder, then create a
new profile after review. Preserve its accounts and supported settings through
the portable mapping and local credential activation. Do not overwrite, rename
or remove rolling copies during enrollment, or classify a decryption failure as
an empty Google account. A Flutter legacy-backup decoder/adapter needs shared
fixtures before that migration can be offered there.

Use immutable change records with unique operation IDs and causal parent/version
information, plus rebuildable profile discovery metadata. This avoids a single
last-writer-wins JSON file dropping another device's offline edits. Reserve and
persist an upload identity before sending; a lost response must verify that exact
record before retrying. File names are not unique in Drive. Complete pagination,
reject incomplete/looping lists, and identify owned profile records by validated
metadata and content, separate from rolling backup files.

Merge independent fields. Concurrent changes to the same field or account's
connection identity require a retained conflict with **Use this device / Use
synced value** choices. Never choose by wall-clock time. A conflict resolution is
another durable operation referencing both versions. Concurrent creation on two
first devices produces two discoverable profiles, not an overwritten singleton.
Never deduplicate accounts using email alone: separate incoming hosts/protocols
can legitimately have the same address.

Account deletion wins over stale edits to that account generation. Preserve
tombstones and conflict evidence until an explicit, tested compaction protocol
establishes what offline devices must do. Re-adding an account uses a new identity.
Do not garbage-collect profile records with backup retention. Schema/crypto
upgrades need compatibility negotiation and rollback-safe publication; an older
client cannot erase a newer profile it cannot interpret.

Remote application reuses account lifecycle staging and revision checks. A
failure after cloud acknowledgment must retry local application, not publish a
duplicate remote change. Removal must fence delayed autosaves, reconnect results,
provider receipts and profile callbacks. Keep SMTP, MOVE, APPEND, folder actions
and their execution receipts out of continuously synchronized profile records;
sync must never cause a second device to execute an old outgoing action.

## Credential protection: Google-only (decided 11 September 2026)

Sam chose Google-only protection: account passwords on a new device unlock with
Google sign-in alone, with no separate sync passphrase. A passphrase was
declined; do not add one without a new decision. Desktop implements the
contract below behind an explicit toggle that is off by default; Flutter and the
browser have not implemented it. Do not claim password sync on a client until
that client ships and verifies this contract.

Google-only means anyone who can read the user's Drive app data (the user's
Google account, or Google itself) can recover the synced passwords. Say so
plainly in each client's UI and docs, and never label the data as encrypted
against Drive while Drive also holds everything needed to decrypt it.

Keep passwords out of the append-only causal history, so a changed or removed
password does not persist in immutable records; the replaceable app-data vault
below holds them instead. No device writes a synced password to SQLite or logs;
only the OS keychain or platform secure storage holds it. The receiving device
still performs its own Google OAuth flow; Google refresh tokens are never
portable profile credentials.

### What the encryption does and does not protect

The vault key is a separate file in the same app-data space, so the encryption
is not protection from Drive: anyone who can read that app data (the signed-in
Google account, Google, or any holder of an app-data access token) can read the
passwords. It does keep passwords out of the causal history, local caches and
SQLite, logs, database exports and Drive backups; binds each password to one
profile generation, shared account, field and server login; and makes removed
passwords in deleted vault files unreadable once their key file is deleted.
Password length is not hidden. UI copy: **Sync account passwords through your
Google account**, followed by a sentence saying anyone with access to that
Google account's Drive app data could read them.

### Files

Two kinds of app-data file, both `application/json` in `appDataFolder`, owned
by the verified principal and never matched by profile discovery (`shepType`
`profile`) or backup retention (`shepBackup`). Change streams report them as
unrelated app data. Each file is created once and never updated in place, so no
Drive revision history accumulates; replacing one means creating its successor
and then deleting the old file.

| Property | Key file | Vault file |
| --- | --- | --- |
| name | `shep-credential-key-<key UUID>.json` | `shep-credential-vault-<file UUID>.json` |
| `shepType` | `credential-key` | `credential-vault` |
| `shepFormat` | `credential-key-v1` | `credential-vault-v1` |
| `shepNamespace` | lowercase SHA-256 of the namespace | same |
| `shepProfile`, `shepGeneration` | the enrollment binding | same |
| `shepKey` | the key UUID | the key sealing every entry in the file |
| `shepSequence` / `shepRevision` | key sequence | vault file revision |
| `shepSha256` | SHA-256 of the exact bytes | same |

A key file is compact JSON with fields in this order: `format`
(`so.shep.credential-key`), `major` 1, `minor` 0, `algorithm` (`A256GCM`),
`profile`, `generation`, `key`, `sequence` (1 or more) and `material` (standard
padded base64 of 32 bytes from the OS random generator). A vault file is compact
JSON: `format` (`so.shep.credential-vault`), `major` 1, `minor` 0, `profile`,
`generation`, `key`, `revision` and `entries`, sorted by account UUID text then
`incoming` before `smtp`. A sealed entry has `account`, `field` (`incoming` or
`smtp`), `revision`, `device`, `endpoint` and `sealed`; a removal marker has
`account`, `field`, `revision`, `device` and `removed: true` and no secret. UUIDs
are canonical lowercase; revisions are 1 to 2^53 - 1 so every client represents
them exactly; at most 512 entries and 1 MiB per file.

### Envelope

`sealed` is standard base64 of the version byte `0x01`, a fresh random 12-byte
nonce and the AES-256-GCM ciphertext with its 16-byte tag. The plaintext is the
UTF-8 password (1 to 4096 bytes). The authenticated data is the UTF-8 text of
these lines joined by `\n`: `so.shep.credential-vault`, `1`, profile UUID,
generation UUID, key UUID, account UUID, field, decimal revision and endpoint.
Tampering, another key or any changed context fails authentication and the
password is not used.

`endpoint` is the lowercase hex SHA-256 of
`so.shep.credential-endpoint/1\n<kind>\n<host>\n<port>\n<username>`, computed
from the portable connection: kind `imap` or `pop3` with `host`, `port` and
`username` for `incoming`; kind `smtp` with the `smtp_*` fields for `smtp`. The
host is ASCII-lowercased. A client offers a password only to an account whose
own portable connection gives the same endpoint, so a changed or downloaded
server definition never receives an existing password.

An unknown format, major version, algorithm, envelope version or field is an
update error, never an empty vault. A vault with a newer minor version is
readable but the client must not rewrite it. `shared/profile-core`'s `vault`
module (feature `vault`, pure Rust, WASM-compatible, no I/O) implements this.
`shared/credential-vault-fixtures.json` holds exact key and vault file bytes,
authenticated data, envelopes, merge and key-selection cases and rejections;
`tests/test_credential_vault_fixtures.py` checks them with an independent
AES-GCM and `shared/profile-core/tests/vault.rs` checks the Rust codec.

### Key custody

- **Where it lives:** only in the key file. A device downloads it for one pass
  and keeps it in memory; it never enters the keychain, SQLite, logs or exports.
- **Created once:** a device that must seal an entry and lists no key file for
  the binding creates one with sequence 1, then lists key files again. Drive v3
  has no compare-and-swap, so two first devices can race. The canonical key is
  the highest sequence, then the smallest key UUID text, so every device picks
  the same one. A device whose new key lost deletes it before writing. A vault
  file already sealed under another key stays readable while that key exists,
  and the next writer re-seals its entries under the canonical key.
- **Rotated:** any write that turns a sealed entry into a removal marker
  (account removal or a device turning the toggle off) creates a new key with
  sequence one higher than any listed, re-seals every remaining entry under it,
  writes the vault, deletes the vault files it merged and then deletes every key
  file that no remaining vault file references.
- **Removed:** a key file is deleted only after no listed vault file references
  it. When no sealed entry remains, every vault and key file for the binding is
  deleted. An entry whose key has gone is unreadable: it keeps its place in the
  merge but is dropped when rewritten, and the device that published it
  publishes it again from its keychain.

### Concurrent writers

Each writer lists every key and vault file for the binding, opens every
readable vault file and merges them per account and field: the higher revision
wins; on a tie a removal marker, then the greater device UUID, then the greater
sealed text. It applies its own changes at the winning revision plus one,
creates one new vault file (revision one more than any listed) and then deletes
only the vault files it merged. A file written concurrently is not deleted and
is merged by the next pass, so no writer drops another device's entry. Removal
markers persist, so a stale concurrent file cannot restore a removed password.

### Device rules

- **Toggle:** device-local, off by default, available only while Accounts sync
  is on. While off, a device neither publishes nor imports.
- **Local state:** SQLite holds only the toggle, a vault device UUID and, per
  shared account and field, the last revision this device published or
  imported plus the last revision whose import failed. Never a password, a
  digest of one or key material. Database import archives this state.
- **Publish:** for each shared account mapped on this device, not suppressed,
  not removed in the history and not awaiting reconnection: the incoming
  password, and the SMTP password when the account uses a separate one with SMTP
  authentication. Publish when the slot is absent, unreadable, a newer removal,
  or this device's keychain value changed since the revision it last
  synchronised. A newer remote revision for the same endpoint is imported
  instead of overwritten; a different endpoint waits for the reviewed
  reconnection rules.
- **Import:** when every required field of an account has a readable entry that
  is newer than this device's revision, matches its endpoints and differs from
  the keychain, stage the pair in new keychain slots, test the incoming
  connection (and the SMTP connection when the account uses a separate SMTP
  password), then write the active keychain entries, clear the Reconnect marker
  and record the revisions in one cache transaction, and delete the staged slots. On failure delete the staged slots,
  keep the active pair and record the failed revision so that the same revision
  is not retried automatically. This lets an account imported through
  `profile_reconnect_v1` become usable without retyping its password.
- **Removal:** an account removed in the shared history makes any device write
  removal markers for its entries and rotate the key. Removing an account only
  on this device, or keeping it local, marks the entries this device published.
  Turning the toggle off marks every entry this device published. A removal
  marker never deletes a password from another device's keychain.
- **Google disconnect or a changed profile:** stop publishing and importing;
  nothing is deleted from Drive.

## Complete database transfer (main's R83, shipped in `93d4289`)

Settings must offer full export/import with file selection, progress, cancellation
and errors. Export a consistent online SQLite snapshot including original mail,
cached content, attachments, drafts, account definitions, configuration and
database-backed history. Use bounded background copying; never read the whole
database into a UI message or copy only the main file while WAL writes continue.
This is separate from continuous profiles and existing limited mail backups.

Validate format/schema, integrity and referenced state before replacing anything.
Keep the current installation recoverable until imported data commits. A transfer
must retain pending-operation evidence while preventing a second installation
from replaying sends, moves, cleanup or unfinished uploads. Device grants, slots,
file paths and missing external journal/staging files need explicit rebind or
recovery handling. No keychain secret appears in a plain SQLite export; explain
reconnection or separately protected credential transfer before the user relies
on it for migration.

Flutter has a different native cache schema and separately stored preferences.
Its profile importer consumes the shared interchange records; it cannot open a
desktop SQLite file as its own database. A future complete Flutter mail import
needs a tested desktop-to-native adapter and must preserve original bytes/draft
ownership. Do not present a profile import as a full-mailbox transfer.

## Acceptance and implementation order

1. Configure and verify the platform OAuth project/namespace; implement scoped
   sign-in and lifecycle integration. Save denied/cancelled/partial-scope,
   expired/revoked, account-switch and locked-keychain automated cases.
2. Freeze the shared profile codec and fixtures. Test Rust↔Flutter mapping,
   unknown fields/versions, legacy SMTP defaults and the selected credential
   protection mode before production uploads.
3. Implement durable profile creation/discovery/enrollment, then incremental
   merge and category/profile toggles. Use two independent synthetic device
   stores with the production transport against a scripted Drive service.
4. Exercise first setup in both directions; existing populated devices;
   simultaneous creation; duplicate legacy accounts; independent and conflicting
   offline edits; tombstones; disabled/re-enabled sync; profile/account switching;
   restart at each upload/apply checkpoint; lost replies; incomplete listings;
   schema upgrades and credential rollback. Never equate a local lock with
   coordination between devices.
5. Add equivalent actual Settings flows to desktop MCP, Flutter host/Android
   integration and browser Playwright tests. Keep synthetic credentials isolated.
   Review light/dark/compact screenshots, progress/recovery and navigation while
   a sync is held. Add Apple and genuine multi-client Google verification as
   separate evidence; fixture success does not establish either.
6. Verify database transfer independently: concurrent WAL writes, complete MIME
   and draft files, large streaming copy, cancellation, corrupt/future schema,
   import rollback, local credential rebinding and pending-action non-replay.

Update the top TODO item, client parity matrix, shared scenario contract and
completion log as behavior lands. Keep remaining work explicit. Quality/release
CI remains disabled; use the repository's normal checks and authorized review
branch shipping, without changing the personal installation.
