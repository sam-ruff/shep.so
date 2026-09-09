# Desktop reconciliation engine

`src/profiles/sync/runner.rs` and `src/store/profile_sync.rs` implement bounded
ongoing preference reconciliation. Completed publication/enrollment reviews offer
**Sync these preferences**, followed by explicit master and per-preference choices.
The background owner continues outside Preferences. Flutter/browser equivalents,
account changes and full categories remain open. Checked preference conflict reviews are connected.

## Device state and ownership

The mail Store owns subscription choices, local field revisions, immutable
pending edit requests and remote application receipts. It keeps only supported
portable fields; credentials, account slots, backup metadata and device paths
remain outside the wire records. One enabled subscription is allowed, and the
workspace retains its profile binding through Pause. A different profile needs a
reviewed workspace switch, which is not implemented by the enable method.

A subscription must be seeded from a completed, checked publication/enrollment
review. It starts disabled. Current local values differing from that known
baseline remain pending. Reusing a subscription preserves its queued work;
seeding it again cannot reset its device identity or receipt progress.

Each runner command opens the independently owned enrolled history for one step.
Missing, replaced, rolled-back or removed histories fail without claiming a fresh device is
enrolled. Its observation catalog is separate from the discovery UI catalog,
under `profiles/discovery/ongoing/`; immutable device history remains under
`profiles/discovery/histories/`. Never transfer these local databases as portable
profile data.

## Reconciliation and recovery

A cycle records local field intent, refreshes discovery, copies original records,
drains causal work, applies eligible preferences and uploads queued operations.
It advances one bounded unit per call; field/version pages contain at most 50
entries and history application retains its existing 32-record bound.

The local request and UUID are saved before calling history Edit. Retry reuses the
exact request; a newer local edit is captured only after that receipt. An
idempotent Edit reply contains current history state, potentially including later
remote changes. The runner acknowledges the current field revision only if the
saved operation is its sole version. Otherwise it retains the known pre-edit
baseline so later remote values/conflicts must still be inspected.

Remote record positions are scoped to the observation history's device identity.
Rebuilding that disposable history resets the cursor before idempotent replay.
The enrolled device identity and queued uploads are preserved. A copy receipt
lost after import replays the same original bytes, without another operation.

Remote preference application rechecks local field intent inside the transaction
that updates preferences and its receipt. Newer local or reverted edits remain;
a conflicting remote value is retained for review. Concurrent history versions
stay explicit. A receipt failure rolls back both preference value and revisions.
A remote application does not echo back as a new local edit or replace
unsupported/device-only settings.

Master Pause refuses new steps while retaining pending work. Accepted history
and upload receipts can still be acknowledged. Field switches stop new capture
and application for that field; already recorded immutable operations remain in
the shared upload queue. Their causal dependencies cannot be selectively erased.
Stale toggles cannot resume a paused subscription.

Uploads use the existing reserved file identity and owned-content verification.
A committed upload with a lost response can finish after restart without another
upload. `last_synced` records a completed transport cycle; pending fields and
conflicts must still be shown. It is not proof of full convergence.

## Scheduling and controls

The dedicated profile engine owns UI reviews and background work. It checks the
saved grant and namespace before each step, takes provider capacity before the
Google lifecycle lock and defers when either is occupied. Frozen reviews suspend
background history changes. Idle/error checks use a 15-second interval; accepted
bounded steps yield between units. A new/reconnected owner starts a full scan,
so a different Google project's empty app-data space cannot inherit a previous
project's inventory or change token.

Setup derives selected fields and original values from completed backend reviews.
Approval/application records local preference revisions; setup compares them in
its transaction, preserving later and reverted intent. A subscription starts
paused. Master and field switches project immediately and coalesce newer choices
behind one in-flight revision-checked command. Failures restore the affected
choice, retain newer intent and reload the durable revision. Pausing retains
queued operations; an accepted provider step can finish before the pause commits.

Canonical preference snapshots use the existing UI/store generations. Pending
native edits remain visible until their own acknowledgment, without echoing
remote values or losing other fields. Names, pending counts, upload status,
errors and retry are visible. An original missing after partial catalog loss
stops upload; restoring it and restarting the scan recovers without another
operation. Every acknowledged original is proved after reopen, then only newly
acknowledged originals during ordinary cycles.

## Conflict decisions

The mail Store owns durable reviews and 50-row version pages. A review captures
local field intent, subscription/device identity and the history revision while
holding the history owner. Choosing the device value or one shared version
requires opening every version page. The backend rechecks local intent and sync
configuration before staging the decision, and History checks exact concurrent
version IDs before recording it. The existing causal protocol permits at most
256 independent versions in one decision; larger conflicts retain an explicit
update requirement.

An old pending edit is replayed exactly before reviewing it. Only History's
definitive Changed/Conflict reply permits retiring that rejected request; its
original bytes remain in a local audit row. An accepted edit with a lost reply
receives its own receipt instead. Storage/identity failures retain the request.

A decision is staged before History.Edit. An interrupted result exposes **Retry
saved decision** and cannot be cancelled or replaced with another UUID. Applying
the chosen value, advancing its field receipt and completing the review share
one mail transaction. Later or reverted local edits remain pending. Retry after
a lost result reuses the same operation; a later shared version is still inspected
by normal reconciliation. Newer history invalidates an unsaved review.

Cached pages and cancellation remain available through Google disconnection.
Opening/saving a decision checks the current grant, namespace and workspace
binding. The background owner pauses for a collecting/open/staged review; it
reopens with a full source scan afterward. Saving a decision does not acknowledge
its cloud upload. The UI preserves page/choice during background observations,
saves displayed Preferences before review/save, and retains errors through the
recovery-page read. Existing Preferences scroll position can persist across
reopened cards; use the visible First/Next controls or scroll to the top.

## Remaining integration

Deliver complete settings/categories and
account lifecycle synchronization. Extend Flutter through its SQLite/SDK lifecycle
with Android/Playwright equivalents, then browser integration, automatic setup,
restoration and reviewed workspace switching. Preserve unresolved requests until
a reviewed resolution commits. See the full [profile contract](PROFILE_SYNC_HANDOVER.md).

## Evidence

Seven Store regressions cover exact request replay/newer edits, per-field atomic
application, reverted intent, other-field progress, pause/re-enable, principal
isolation, receipt rollback and workspace-switch refusal. Five runner regressions
use real reviewed initial enrollment, independent histories and the loopback
Drive transport: device-to-device changes/conflicts; lost copy receipts and cache
rebuild; a lost local receipt followed by another device's value; missing/replaced/rolled-back
history; and a committed upload with a lost response across Pause/restart.

Run `cargo test --all-features profile` for this engine and related profile
contracts. The first complete run passes 41 checks. Initial test-fixture failures
(identity setup and missing Drive change type) remain under ignored logs. Native
profile controls at that earlier checkpoint exercised the shared discovery fixture; ongoing controls were connected afterward. Final checks and shipping are in [completion](../COMPLETION.md).
These fixtures do not establish live Google access, authenticated Flutter
interchange, Apple execution or final performance.

The connected-control continuation adds partial-metadata-loss/missing-original
recovery, acknowledged-history export, atomic reviewed baselines, rapid native
choice ordering and the background owner's saturation/reconnect/project checks.
Native control evidence and shipping are recorded in [completion](../COMPLETION.md).
Fixture success does not prove live Google, Apple or full cross-client parity.
