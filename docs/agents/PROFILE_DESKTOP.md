# Desktop profiles

Preferences → Profiles and sync discovers existing setups through the shared
Drive catalog. Sign in with Drive access first, then enter the application
namespace configured for the same Google Cloud project as the other devices.
Discovery and reviewed publication use the same shared Drive format. Desktop
account/settings import remains active work. Flutter publication and reviewed enrollment have separate
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
It does not apply a discovered profile to the desktop. Future reviewed application
must preserve newer local edits, independent account mappings and credential
activation before enabling provider work. Shortcuts, contacts, other portable
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
or desktop enrollment. Native large-page controls, complete settings/categories,
credential protection, Apple/live Google and authenticated cross-client access
remain active work.
