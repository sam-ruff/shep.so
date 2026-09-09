# Desktop profiles

Preferences → Profiles and sync discovers existing setups through the shared
Drive catalog. Sign in with Drive access first, then enter the application
namespace configured for the same Google Cloud project as the other devices.
Discovery, reviewed publication and account/preferences enrollment use the same
shared Drive format. Flutter publication and reviewed enrollment have separate
[contracts](PROFILE_ENROLLMENT.md).

The desktop uses the saved active grant, OAuth client, Google lifecycle revision
and verified Drive principal. Each accepted network step obtains a fresh checked
access token and verifies the provider identity. Permission choices for the next
sign-in do not replace that active grant. Namespace configuration is device-local
and excluded from portable preference exports and backups.

A separate 32-command queue owns one discovery session. Its network steps share
the existing provider slots; cached mail and ordered draft/preferences writes
retain their independent queues. Acquire provider capacity before the Google
connection lock so queued profile work cannot prevent disconnect. One accepted
step retains the read lock; disconnect can retire the grant between steps.

The catalog retains original records and scan/error progress under the private
application-data directory, scoped by namespace/principal. It has its own owning
worker and lock, separate from the mail database. The controller keeps one request
in flight and at most 50 profile summaries. Pause or leaving Preferences stops
scheduling additional steps. Close drains accepted catalog work. Changed grants
and request generations reject both old data and old errors.

Retry resumes the saved request; Restart scan retains observed history while
rechecking source files. Incomplete scans, missing history, conflicts and unfinished
setup stay explicit. An empty completed listing is only empty for that Google
application's private Drive space; it is not proof that another project's setup
is absent. Profile metadata is not a credential or account-activation receipt.

`src/profiles/preferences.rs` maps eight currently shared desktop fields explicitly.
Reviewed enrollment applies only those selected fields, preserving newer local
edits and independent account mappings. Imported accounts require explicit
credential activation before provider work. Shortcuts, contacts, other portable
settings, ongoing reconciliation and protected passwords remain open.

The nondefault `test-support` feature supplies a synthetic loopback Drive fixture
for the real desktop controls. Its endpoint constructor rejects non-loopback
addresses, hostname resolution, credentials, query/fragment overrides and redirects;
production has no endpoint override. Fixture tests establish protocol/controller
behavior, not live Google consent or same-project cross-client access.

## Reviewed publication

**Publish this device's setup** offers a named profile, account inclusion and eight
individual preference choices. **Review profile** saves the displayed preferences
first, then freezes the account list and selected values in the mail database.
Preparation and review return at most 50 account rows. Connection details expose
the actual endpoints and usernames; passwords, tokens, mail and drafts are excluded.
Approval rejects changed accounts or selected preference values.

The database stores exact planned operations and stable local/shared account IDs.
Each staging step saves its exact revision and operation UUID before writing the
separate history journal. Each upload uses the shared tracked uploader, retaining
its reserved Drive file ID and confirming exact media before advancing the receipt.
The causal completion marker follows every account/settings record. A failed
confirmation remains pending; retry checks the reserved file before another POST.
The history owner closes after each accepted step. Pause, leaving Preferences and
changed grants stop scheduling further work; reopening retains the saved review.

Changing the active OAuth client forces a full scan while retaining known records.
A missing record remains a failure, including when another Google project's app
space is empty. The client ID itself does not identify a Google Cloud project.
Completed discovery is required before preparing, approving, staging or uploading.

Store/protocol tests exercise changed reviews, 75-account paging, exact staging
recovery, mail database reopen after a lost Drive confirmation and an independent
catalog reading the completed profile. Native control results and shipping belong
in the completion log. This is an initial publication, not ongoing reconciliation
for enrolled devices. Complete settings/categories,
credential protection, Apple/live Google and authenticated cross-client access
remain active work.


## Reviewed enrollment

Click a discovered profile to prepare its review. Original immutable records enter
an independently owned history, using the same binding path as publication; the
catalog database, its device ID and its upload queue are never copied. Existing
offline operations and queued uploads remain intact. Source and history revisions
must still match before approval. Unsupported or conflicting fields stay visible
and unavailable, with their original records retained.

Reviews expose at most 50 account/preference rows, category choices and individual
selections. Changing a selection or encountering a recoverable error retains the
current page of the same review. Details show incoming/SMTP endpoints, TLS/authentication, usernames
and Sent policy. Matching shared identities preserve all local metadata, mail,
drafts and credentials. Differing connections or locally removed accounts start
unselected and require explicit approval to add a separate connection. Email
addresses are never used as a deduplication identity.

The mail database freezes account metadata and per-field preference revisions.
Each account step commits metadata, its mapping, reconnect guard and receipt in
one transaction. Newer edits/removal of the original connection win, including
when the approved choice was to create a separate account. Preference application
and its receipt also commit together; explicit newer intent wins even when changed
back to the original value. GUI saves carry the portable fields actually edited,
so an older whole-window snapshot cannot undo an unrelated imported preference.
The reader and inbox apply the accepted preference effects immediately.

Imported accounts show **Reconnect required** in Preferences → Accounts. Sync,
server mutations, sending and server Sent access check this durable guard before
reading credentials. Reconnect requires newly entered incoming and, when separate,
SMTP passwords. The guard remains after a partial keychain failure or changed
account; both writes must succeed before checked activation. Local cached mail
remains usable. Backups skip guarded password entries; restoring metadata without
a complete password pair keeps the account guarded. Restores cannot silently
activate an already guarded account.

Preparation and application run one bounded background step at a time. Leaving
Preferences or Pause stops further steps; reopening observes durable receipts.
Store/protocol tests cover lost receipts and file reopen, matching/differing
accounts, removal, newer preferences, 78-row paging, unsupported settings and
preserved offline history. Native controls cover review/details, application,
reconnect entry, browsing, reopen, compact cancellation and a newer theme edit.
See the completion log for final runs and shipping.

This is reviewed initial enrollment. Automatic first setup, continuous
reconciliation, shared removal/conflict resolution, remaining categories/settings,
protected credential transfer and live Google/Apple
verification remain active work.


## Large lists

The saved native page scenarios use 51 synthetic profiles, 75 local accounts and
75 offered accounts. Discovery exercises First/Next in light and compact dark.
Publication reviews 50/25 account rows and connection details before cancellation.
Enrollment reviews 50/26 account/preference rows, toggles a second-page choice,
returns between pages and applies the selected accounts with reconnect guards.
Skipped new connections say **Not imported**. Final execution, reviewed captures
and shipping are recorded in the completion log. This is isolated protocol/native
coverage; live Google, continuous synchronization and Apple remain separate work.


The [reconciliation engine](PROFILE_RECONCILIATION.md) retains later local edits,
copies incremental original records and applies supported remote preferences with
atomic receipts. Completed publication/enrollment reviews offer explicit paused
sync choices, followed by master/field controls and authenticated background work
outside Preferences. Storage/provider/native controls are verified in completion.
Initial enrollment never silently enables ongoing sync. Checked conflict resolution,
complete categories/settings/accounts and Flutter/browser equivalents remain open.
