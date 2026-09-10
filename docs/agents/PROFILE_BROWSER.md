# Browser profiles and sync

The browser client's **Preferences → Profiles and sync** section mirrors the
[Flutter implementation](PROFILE_MOBILE.md) on top of the shared
`shared/profile-core` contract: explicit Google provider consent, a durable
per-identity catalog, reviewed [publication](PROFILE_PUBLICATION.md) and
[enrollment](PROFILE_ENROLLMENT.md), and first-setup onboarding. The beta
identity gate still only identifies the user; Drive access is a separate,
reviewed grant. Ongoing reconciliation, the remaining portable categories,
credential protection and live Google access remain open in the
[sync handover](PROFILE_SYNC_HANDOVER.md).

## Google consent through the beta server

`backend/src/profiles.rs` runs a second OAuth consent bound to the signed-in
beta session. `POST /api/profiles/connect` records the next-sign-in choices
(Drive app data, Calendar off/read/edit) and returns the Google URL with
PKCE/state/nonce and the exact scope list from desktop `consent.rs`;
`GET /auth/google/callback` exchanges the code through the `ProfileProvider`
trait, requires the consenting subject to equal the session identity, verifies
the Drive principal (`about.user.permissionId`) and interprets the returned
scopes intersected with the request. Tokens live only in the in-memory session
store, are refreshed server-side before Drive calls, disappear with logout,
session expiry or a replacing login, and are never logged or sent to the
browser. Denied, failed or mismatching consent redirects to
`/app/#profiles=denied|failed|mismatch` with the previous grant and choices
untouched; a changed choice changes saved access only after a successful
sign-in. `POST /api/profiles/disconnect` removes this session's grant only,
never the project-wide grant of other devices.

`POST /api/profiles/drive` proxies a fixed, validated set of app-data
operations (`about`, `list`, `start_page_token`, `changes`, `metadata`,
`media`, `generate_ids`, `create`) with bounded page tokens, file identities,
1 MiB media and 256 KiB JSON. `SHEP_PROFILE_NAMESPACE` configures the shared
application namespace; without it the connection endpoint reports profile sync
as unavailable. The production provider reports `live: true`; the test fixture
reports `live: false`, and the Preferences card says when live provider access
is not connected.

## Browser storage and history

`web/src/profile_store.ts` opens `shep.profiles.v1.<identity>` per beta
identity, separate from the mail cache: journal records, staged edits, the
discovery catalog and scan state, profile summaries, account mappings,
publication/enrollment reviews and preference receipts. Every write is one
strict transaction; disconnect marks cleanup durably before clearing, and an
interrupted clear is offered again as **Retry local cleanup**.

`web/src/profile_history_worker.ts` owns WASM `ProfileHistory` journals, the
new browser entry of `shared/profile-core` over `history::memory`, which
runs the same command contract as the native SQLite journal (imports, edits,
conflicts, tombstones, the initialization barrier, uploads) and is parity
tested against it in `shared/profile-core/tests/memory.rs`. The worker holds
no storage: the page persists each accepted record before treating the write
as acknowledged and restores journals from those records in sequence order. A
local journal gets a fresh device UUID kept in the store; observation journals
are never copied into local ones, and another device's local records are
refused on restore.

## Discovery, publication and enrollment

`web/src/profile_discovery.ts` follows [durable discovery](PROFILE_DISCOVERY.md):
capture the change token, list pages of at most 50, stage each page, verify
metadata, exact size, SHA-256 and decoded identity (`web/src/profile_drive.ts`)
before importing the original into the profile's observation journal, then
replay changes. Incomplete listings, repeated tokens, duplicate files, changed
identities, altered media and missing known files are saved failures; retry
continues from the saved step, pause finishes the accepted step, and an
incomplete or failed scan never presents an empty Google account.

`web/src/profile_publication.ts` freezes a review of the browser's account
definitions (shared UUID mappings, legacy IDs mapped durably) and the four
portable browser preferences by fingerprint, then stages the preparing root,
content operations and completion marker as exact durable edits, and uploads
one owned file per step: generate an ID, reserve it, create only after a
confirmed 404, verify the read-back metadata and exact media, record the
own-upload receipt in the catalog, then confirm the queue. Lost replies retry
the same operation and reserved ID without duplicating files.

`web/src/profile_enrollment.ts` copies originals one record at a time from
the observation journal into an independently owned local journal, builds
paged rows (account definitions with connection details, names, settings;
conflicts and unsupported fields unavailable), and applies one account per
step. Imported accounts are saved without passwords and show **Reconnect
required** until a reviewed reconnect verifies a password pair; mapped
accounts keep their identity, cached mail, drafts and Sent preferences; a
remotely changed connection is offered as a separate, unselected account.
Preferences apply through `web/src/profile_settings.ts`: per-field revisions
advance on every explicit save (including change-and-revert), newer local
edits are kept, and the receipt freezes the original revisions so a retry
returns the same proof.

Onboarding runs on the Mail tab after completed discovery on a browser
without accounts: no profiles offers **Sync accounts and settings** with a
durable **Not now**; one profile offers automatic enrollment; several show a
picker and never merge. Passwords, mail, drafts, tokens and device identity
are never part of any profile record.

## Verification

Run the commands in [client testing](../CLIENT_TESTING.md). Rust tests cover
exchange, scope binding, denial, mismatch, refresh, cleanup and proxy
validation; `web/src/profile_flows.test.ts` covers the store, history port,
discovery, publication, enrollment and preference device with fake IndexedDB
and the real WASM; `web/e2e/profiles.spec.ts` drives the actual controls in
Chromium with light/dark captures and axe checks; the ignored
`real_browser_beta_gate` exchanges consent and publishes through the real Rust
router. These use isolated fixtures and do not establish live Google, same
project cross-client visibility or continuous sync.
