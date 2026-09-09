# Desktop reconciliation engine

`src/profiles/sync/runner.rs` and `src/store/profile_sync.rs` implement bounded
ongoing preference reconciliation. **Automatic scheduling, reviewed subscription
creation and Preferences controls are not connected.** Existing enrollment and
publication do not silently enable this engine. Flutter/browser equivalents,
account changes, full categories and conflict-resolution controls remain open.

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

## Required next integration

1. Derive subscriptions only from completed backend reviews, preserving selected
   fields and their original changes. Offer explicit enablement and stable saved
   choices. Do not accept UI-supplied baselines or bind an arbitrary Google identity.
2. Drive bounded steps from the existing profile engine owner outside Preferences.
   Serialize its local history with publication/enrollment, suspend while a frozen
   review is active, and recheck the active grant/namespace before each step.
   Acquire provider capacity before the Google lifecycle read lock. Keep cached
   mail, drafts and preference saves on their independent queues; retry with backoff.
3. Deliver canonical small preference snapshots through existing UI generations.
   Preserve newer native edits, including reverted intent, while results are pending.
   Apply normal appearance/query/reader effects without stealing focus.
4. Connect master/field switches, Sync now, pending/error progress and checked,
   paged conflict resolution. Preserve unresolved requests until review commits.
   Add saved native flows and reviewed light/dark/compact captures.
5. Extend Flutter through its own SQLite/SDK lifecycle and add Android/Playwright
   equivalents. Continue accounts/categories, switching, restoration and browser
   integration under the full [profile contract](PROFILE_SYNC_HANDOVER.md).

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
profile controls are regression checks for the shared fixture; there is no native
ongoing-sync control yet. Final checks and shipping are in [completion](../COMPLETION.md).
These fixtures do not establish live Google access, authenticated Flutter
interchange, Apple execution or final performance.
