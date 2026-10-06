# Mobile functionality audit

Reviewed on 6 October 2026 against desktop `main` at `4dec758`.
The GitHub [delivery tracker](https://github.com/sam-ruff/shep.so/issues/51)
links every finding and its implementation status.
Android and iOS share the Flutter client. This review compares user-visible
abilities, defaults, persistence and recovery, not desktop geometry, tray controls
or touch interaction choices. Source inspection and existing scenario coverage
are separate from new runtime evidence recorded in the completion log.

The review covered application navigation, accounts and providers, synchronisation,
mail lists/search/selection, individual and group actions, folder changes, reading,
composition/attachments/delivery, calendar, preferences, shared/local profiles,
backups, notifications and platform integration. The existing uncommitted change
to `flutter/rust/src/operations.rs` was excluded. An unrelated open desktop PR
was not treated as shipped behaviour.

## Confirmed gaps

Each ticket contains baseline source references and acceptance tests. Several
larger tickets need multiple implementation PRs; opening an issue does not close
the corresponding request in the main TODO.

| Area | Functional difference | Ticket |
| --- | --- | --- |
| Preferences search | No mobile catalogue or result-to-control navigation | [#26](https://github.com/sam-ruff/shep.so/issues/26) |
| Bulk review | Preparing a review releases the original selection, including when declined | [#27](https://github.com/sam-ruff/shep.so/issues/27) |
| Background error notices | Automatic checks use the immediate explicit-refresh error path | [#28](https://github.com/sam-ruff/shep.so/issues/28) |
| Calendar editor | New events are all-day; dates, start/end times and all-day status cannot be changed | [#29](https://github.com/sam-ruff/shep.so/issues/29) |
| Reply composition | No separate original-message toggle or configurable starting choice | [#30](https://github.com/sam-ruff/shep.so/issues/30) |
| Move destinations | No ranked chooser or safe account-qualified transfer | [#31](https://github.com/sam-ruff/shep.so/issues/31) |
| Mail search | Current-folder prefix matching/date order instead of cross-folder ranked search | [#32](https://github.com/sam-ruff/shep.so/issues/32) |
| Conversations | No linked-message conversation query, paging or expanded-message actions | [#33](https://github.com/sam-ruff/shep.so/issues/33) |
| Account editing | Existing accounts accept new passwords but configuration fields are locked | [#34](https://github.com/sam-ruff/shep.so/issues/34) |
| Mail scheduling | Serial account checks on a fixed 15-second foreground timer; no integrated push/delta owner | [#35](https://github.com/sam-ruff/shep.so/issues/35) |
| Remote images | Always blocked; no reviewed sender/domain/message exceptions or Contacts policy | [#36](https://github.com/sam-ruff/shep.so/issues/36) |
| Notifications | No OS new-mail delivery or unread launcher integration | [#37](https://github.com/sam-ruff/shep.so/issues/37) |
| Backups | No encrypted multi-destination backup, history, retention or restore implementation | [#38](https://github.com/sam-ruff/shep.so/issues/38) |
| Shared profiles | Limited settings reconciliation; ongoing account reviews/linking and protected passwords missing | [#39](https://github.com/sam-ruff/shep.so/issues/39) |
| Local profiles | One fixed database; no reviewed complete database transfer/workspace switching | [#40](https://github.com/sam-ruff/shep.so/issues/40) |
| Functional preferences | Missing reader size, editable colours, keymaps and help behaviour; labelled keys are not applied settings | [#41](https://github.com/sam-ruff/shep.so/issues/41) |
| Send preparation | MIME work precedes durable admission; no recoverable Preparing Outbox stage | [#42](https://github.com/sam-ruff/shep.so/issues/42) |
| Large mail | No equivalent staged incoming path or completed bounded body-cache audit | [#43](https://github.com/sam-ruff/shep.so/issues/43) |
| External entry/export | No Shep mailto activation or Save original message control | [#44](https://github.com/sam-ruff/shep.so/issues/44) |
| Activity | Recovery remains separated by domain without the bounded combined summary | [#45](https://github.com/sam-ruff/shep.so/issues/45) |
| Reader metadata | To displays an account label; subject and sender name cannot be selected | [#47](https://github.com/sam-ruff/shep.so/issues/47) |
| Logical mail folders | Archive/Trash/Spam use literal names without the desktop destination lifecycle | [#48](https://github.com/sam-ruff/shep.so/issues/48) |
| Pending bulk Undo | Lane implementation offers queued/paused Undo with exact lost-reply recovery; root integration/platform verification pending | [#49](https://github.com/sam-ruff/shep.so/issues/49) |
| Bulk owner wake | Undo/Resume/Retry during final History observation lost their pump wake; lane regression/fix implemented | [#58](https://github.com/sam-ruff/shep.so/issues/58) |
| History bounds and identity | Item pages accumulate in Dart, older completed groups are deleted, and delayed recovery can load details for a previously selected group | [#50](https://github.com/sam-ruff/shep.so/issues/50) |

## Implemented foundations

Source and saved scenarios establish that these are implemented paths, not
blanket missing features. Current device/provider verification is tracked
separately in [#46](https://github.com/sam-ruff/shep.so/issues/46).

- Multiple IMAP/POP3 accounts, TLS/STARTTLS, independent SMTP configuration,
  device credentials, staged reconnect, reviewed local removal and cleanup.
- Cached 50-row mail queries, unread/flag filters, date sort, unified and
  per-account folders, configurable swipe actions and visible action controls.
- Durable individual mail actions, exact receipts, optimistic field ownership,
  counted Undo, uncertain-result inspection and acknowledged cache repair.
- Native captured selection and durable bulk review/execution, Pause/Resume,
  History, inverse work and conservative restart.
- Folder creation and checked rename/move/delete with frozen metadata and
  bounded cache repair. Older claims that mobile has no folder mutation support
  are stale.
- Confined formatted WebView/plain reading, selectable body/sender address,
  quoted-history display, Find, attachment save, cached Forward and Print.
- From/To/Cc/Bcc composition, Reply all, durable files/drafts, autosave recovery,
  queued sending and separate SMTP/Sent-copy uncertainty.
- Google and CalDAV calendar connections, cached month/agenda, title/location
  editing, source permissions, ETag/resource identity and durable recovery.
  Older matrix rows marking all calendar providers Open are stale.
- Light/Dark/System, preview density, avatars, unified inbox and swipe choices.
- Google SDK consent/disconnection, shared profile discovery/publication,
  reviewed initial enrollment and eight-setting reconciliation.

## Scope and evidence limits

Calendar Undo, full recurrence editing, automatic replies and complete cache
encryption are also unfinished in desktop or in the common contract. They are
not counted as already-shipped desktop capabilities missing only on mobile.
Desktop has no event-description editor either; calendar parity work must
preserve descriptions but need not invent one.

Mobile-specific navigation, native selection menus, touch gestures, safe areas,
desktop window geometry, tray behaviour and installer packaging are excluded
from the functional defect list. User abilities such as copying headers,
notification delivery and opening a mailto draft remain in scope even when their
platform controls differ.

The repository's host, widget, Flutter-web, Android and protocol scenarios are
useful evidence of distinct layers. They do not prove Apple runtime behaviour,
live Google/provider compatibility, distribution signing or final latency.
Those gates remain explicit in #46 and the feature-specific tickets. All CI
must use the existing self-hosted runner pools.

## Delivery

The initial independent lanes are Preferences search (#26), retained bulk
selection (#27) and account-bound sync notices (#28), using GPT-6.1 Sol at high
reasoning. Further lanes follow as concurrent slots become available. The
primary thread reviews the complete diff and tests before merging each PR,
then checks the combined revision. Each PR must update the shared scenario
registry, parity status and completion evidence without claiming unexecuted
device/provider checks.

The table records the audit baseline. Account-bound delayed notices (#28) are
implemented in [PR #53](https://github.com/sam-ruff/shep.so/pull/53), retained
bulk selection (#27) in [PR #54](https://github.com/sam-ruff/shep.so/pull/54),
and Preferences search (#26) in [PR #55](https://github.com/sam-ruff/shep.so/pull/55).
Follow-up inspection expanded #50 to retain older completed receipts and fence
late recovery details to the exact selected group and page. The delivery tracker
and completion log distinguish these changes from the remaining open findings.
