# Flutter profile enrollment

Reviewed enrollment is implemented on the client review branch; the completion log
records exact execution and shipping evidence. Publication and
continuous reconciliation remain separate operations. The current enrollment
path reviews account metadata and eight mobile preferences; it does not transfer
passwords, mail, drafts or Google grants.

## Review and application

Preferences → Profiles and sync opens profiles found by the authenticated Google
catalog. An initialized profile can prepare a local review. Incomplete discovery,
missing setup records, a removed profile or a changed source prevents preparation.
Account and preference categories can be selected independently; individual rows
can be kept. Connection details include incoming and SMTP identity/security.
Conflicts and unsupported fields remain visible and unavailable for application.

Existing stable account mappings keep their device credentials and cached mail.
A different connection is offered as a separate account, initially unselected.
Removed local mappings stay suppressed unless the user explicitly selects a new
connection. Enrollment does not deduplicate accounts by email address. Imported
accounts show **Reconnect required** in Connections and refuse credential lookup
or provider commands until a new device credential pair is activated.

Apply processes the frozen selections with saved progress. Pause finishes its
accepted step. Reopen and Resume recover the same account IDs and preference
receipt; a lost reply cannot create another account or overwrite newer preferences.
Newer local account changes are kept. Account refresh updates metadata without
replacing unsaved drafts, the reader or cached mail.

## Device persistence

The shared catalog exports one original immutable operation at a time, fenced by
scope, profile generation and source revision. Enrollment imports those bytes into
an independently owned local journal, reusing the publication journal on the
originating device. It never copies the catalog database, device UUID, credential
slots or upload queue. Accepted imports are idempotent across a lost copy cursor.

Native schema 11 retains the source, baseline account fingerprint, review choices,
progress and per-account receipts. Field preparation reads one field per step;
review planning and observations contain at most 50 rows. Application commits one
account, its independent empty credential slot, Reconnect marker, shared mapping
and receipt together under the account operation lock. Unrelated provider jobs do
not own this cache-only work.

The eight portable preferences are appearance, left/right swipe, preview lines,
sender pictures, unified inbox, quoted history and tooltips. Explicit removal means
reset to the mobile default. Device preferences retain per-field revisions and a
review receipt in the same platform preference write. Normal local saves merge
changed fields, including dirty values retained after a failed save. Application
compares each field's reviewed value and revision, preserving changes made and
then changed back. UI generations also protect edits made while storage is pending. Preference control
callbacks read current state so two changes before repaint retain both edits.
Only one enrollment can await a preference receipt across Google scopes.

Each platform receipt now freezes the eight field revisions from the original
application. Retrying returns current preferences for display and those original
revisions for native acknowledgment. Rust validates and saves the exact receipt;
a different retry cannot replace it. Legacy receipts omit the revision map: a
later current snapshot must never be substituted as proof of the original write.
Explicit local save intent advances its field revision even when an earlier failed
save left the same value in storage. Receipt revisions cannot exceed the current
revision of their own field.

Immediate painting also requires the local UI generations captured with the
review. A reverted edit or an already applied receipt does not briefly repaint an
obsolete imported value. After restart, progress remains visible while storage
checks the durable receipt; reading and preference editing stay available.

Receipt metadata, local mappings and platform credential slots stay on the device.
Unknown optional operation fields remain in original history; unsupported account
extensions cannot silently become a reduced connection definition.

## Verification and remaining work

Saved host/native tests cover paged reviews, original-record retries, account
receipts, cached mail/draft preservation, changed connections, Reconnect activation,
newer preferences and account edits, pause and disconnect. Receipt checks cover
failed native commits, exact retry after restart, invalid or legacy revisions,
reverted intent after a failed save and the immediate UI before persistence. Actual Flutter control
scenarios cover review pages, connection details, category/row choices, application
retry, leaving a lost settings acknowledgment to change appearance, retaining that
newer choice on Resume, Reconnect status, independent mail browsing, rapid preference
changes and footer clearance above the device navigation inset.

The Android and Flutter Playwright/Appium runners include enrollment scenarios:

```sh
python3 scripts/clients/android_e2e.py --device emulator-5554 --enrollment-only
python3 scripts/clients/flutter_web_e2e.py --enrollment
```

Execution, visual review and shipping evidence belong in the completion log after
verification; adding these commands alone is not an execution claim. Google source
fixtures do not establish live authorization or same-project interoperability.
Apple execution remains required. Desktop/browser enrollment, ongoing reconciliation,
all portable categories, conflict resolution, shared removal reviews, credential
protection and legacy database transfer remain active in TODO.
