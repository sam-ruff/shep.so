# Flutter profile publication

**Profiles and sync → Create profile** saves a named set of account definitions
and the eight current Flutter preferences to private Google app data. The user
chooses accounts and settings, reviews the frozen values, then publishes. Account
reviews page through 50 rows; each row opens connection details. Passwords, OAuth
grants, mail, drafts, device paths and window geometry are excluded.

This is publication only. Applying a profile on another device, ongoing sync,
complete desktop preference categories and credential portability remain open.
The [sync handover](PROFILE_SYNC_HANDOVER.md) remains the full product contract.
Existing profiles and legacy backups are retained separately.

## Durable review and publication

Native mail schema 10 adds publication reviews, immutable planned operations and
explicit local/shared account mappings. Preparing the review streams account
metadata into SQLite; Flutter retains one page. Approval checks the frozen account
fingerprint and preference values against the current local values. A changed
review must be cancelled and prepared again. UUID account identities are retained;
legacy local IDs receive an explicit shared UUID. Email addresses are never used
for identity matching. Publication does not activate accounts or credential slots.

The session uses the existing verified Google principal and namespace binding.
Preparation, approval and each publication step require completed discovery;
missing or failed discovery cannot be treated as an empty setup. One owned step
stages one operation or uploads one file. The exact history edit, including its
revision and operation UUID, commits to the mail database before the separate
history journal receives it. A lost cross-database receipt retries that edit.

The independent journal lives beside the mail cache under `.published-profiles`,
keyed by the binding digest. Its worker drains and closes before an accepted step
finishes. Pause finishes that step and starts no next step. Reopening uses the same
review, operation identities, reserved Drive IDs and exact bytes. Google-token
refresh rebinds the same catalog; disconnected or superseded sessions cannot show
late results or start more uploads. Already accepted work retains its receipt.

Production upload uses `Drive::upload_next_tracked`: it validates the owned remote
file and exact media, records the file identity in discovery, then confirms the
local upload queue. If catalog persistence fails after Google accepted a file,
the queue remains pending and retry verifies the same file before continuing.
Later deletion of an own-uploaded file remains an explicit discovery failure.

## Initialization barrier

The shared codec adds required capability `initialization-v1` and the sole-change
operation `profile_setup` with `complete: false` or `true`. The preparing operation
is the root. Completion must descend from it and use the same originating device
identity; another device cannot finish setup or create local edits in a preparing profile. Later edits
retain the capability. Reinitializing an existing history is rejected.

A profile is initialized only when its single completed marker has fully applied,
there are no waiting or ready operations, and the profile is not removed. Cloud
listing completion alone is insufficient. Legacy metadata histories without this
marker remain uninitialized. Enrollment must use this barrier and copy original
records into an independently owned local journal, never clone the catalog's
observation database or device UUID.

## Verification

Shared protocol/storage tests cover causal setup, independent device identity,
late ancestry, legacy histories, immutable upload receipts and retry after a
catalog write failure. Native tests cover changed reviews, migration, explicit
account IDs, 75 accounts/78 records, 50-row paging, lost staging receipts, unchanged
mail/credential slots, close/reopen and a held upload while mail capacity is full.

Dart tests cover lost approval replies, pause, disconnect, stale settings and
account pages. Saved host and Android controls exercise review, details, upload
failure/retry, light/dark appearance and mail reading while publication is paused.
`profile_creation.mjs` drives the same four review/retry/appearance/navigation
flows through Flutter web Playwright and native UiAutomator2/Appium. These use
isolated providers; they do not establish live Google or Apple execution.

Run the commands in [client testing](../CLIENT_TESTING.md). Executed counts,
reviewed captures, retained failures and shipping are in [completion](../COMPLETION.md).
Operational instructions remain in [AGENTS.md](https://github.com/sam-ruff/shep.so/blob/main/AGENTS.md).
