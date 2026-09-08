# Shep TODO

Active unfinished requests. Read [handover.md](handover.md) before resuming. Completed behavior and test evidence live in [docs/COMPLETION.md](docs/COMPLETION.md); [docs/REQUEST_AUDIT.md](docs/REQUEST_AUDIT.md) preserves every original request. Add requests immediately, and remove them only after relevant verification and shipping. The full product goal remains unfinished.

## Resume first: composition

- [ ] **R35 — Inline composer:** replace the reply/new-message popup with an editor in the preview pane, with the conversation below replies. Preserve multiple drafts, recipients and attachments when switching mail; restore the matching reply when returning to a conversation. Autosave while typing, keep navigation immediate during send, and preserve recoverable failures. Current source separates draft/editor state from other forms and coalesces autosaves; the inline view, session switching and native editing-focus guards are still missing. See the handover for exact files and verification.
- [ ] **R77 — Composer typing artifacts:** renderer clipping fix ebddf54 is shipped. Preserve its regression and repeat compact light/dark visual checks after inline integration.

## Mail, folders and desktop integration

- [ ] **R88 — Compact email list and unread styling:** remove sender avatars/initial icons from the inbox/conversation preview list and reduce each row’s height, following the supplied compact horizontal example (sender, subject, snippet and time). Retain Shep’s action buttons, selection controls and usable mouse targets. Unread mail should have a subtle background highlight, a dot indicator and a bold subject; keep unread styling distinguishable from selected/hovered rows in light and dark themes. Verify compact/resized layouts, row actions and read/unread transitions when implemented. Backlog only for now.

- [ ] **R87 — Refresh animation speed/direction:** make the refresh icon spin more slowly and clockwise. Preserve manual-refresh-only animation and verify its direction and visual pacing when implemented. Backlog only for now.

- [ ] **R86 — Native close-to-tray (backlog only):** closing the main window should hide Shep in the native system tray/menu bar, with Restore/Open and explicit Quit actions. Keep background mail sync and notifications working, provide a preference, and handle desktops without tray support gracefully. Cover Linux/Windows/macOS behavior and preserve pending drafts/operations on actual Quit. User explicitly requested tracking only; do not implement during this handover.

- [ ] **R30 — Folder context menus:** reviewed Move/Delete, nested groups and recovery controls shipped in 3567de2. Finish combined-folder optimistic scopes, aggregate common-folder account choice, history paging/account removal and cache convergence after accepted uncertainty. Keep original mail when an outcome is unconfirmed; acceptance is not server success.
- [ ] **R73 — Moved mail missing from destination:** recovery/projection checkpoint a81d767 is shipped. Finish actual adapter wire/journal integration, broader Undo/bulk/folder-history lifecycle and repeated-move aliases, plus independent-process coordination with R01.
- [ ] **R47 — A. Keep → Inbox:** confirm the personal-account failure's root cause and actual destination refresh. Inbox labels and spaced-folder protocol checks are shipped; live diagnosis remains open.
- [ ] **R50/R60 — Optimistic interaction everywhere:** immediate flags/read/move/Undo feedback is shipped. Finish membership/count reconciliation when switching filtered or combined folders, ambiguous cross-account outcomes, other reversible controls and durable restart recovery. Preserve newer intent and visible rollback errors.
- [ ] **R82 — Native new-mail notifications:** default-on popup/sound/details, independent preferences and isolated Linux/native fixtures shipped in ded5aca. Finish actual Windows/macOS delivery, macOS bundle integration and real desktop popup/sound review. Initial imports remain quiet; fixture success is not live desktop verification.
- [ ] **R70 — Dock/taskbar badges:** Linux Unity/Dash-to-Dock adapter and preference shipped in 90776fa. Finish Windows/macOS adapters/execution, actual desktop rendering and ambiguous-provider/restart count reconciliation.

## Accounts, portability and backups

- [ ] **R02/R49 — Continuous Drive sync:** OAuth/appDataFolder account definitions and portable preferences, first-device opt-in, existing-setup discovery on another PC, ongoing toggle, conflict-safe changes and offline recovery. Keep local paths/window placement device-specific.
- [ ] **R49 — Portable account credentials:** decide passphrase-protected transfer versus Google-only protection, then implement secure keychain import. No answer to the earlier protection question was recorded; do not claim password sync works.
- [ ] **R32 — Multiple backup destinations and options:** polished Local/Drive/S3/FTP/FTPS/SFTP setup with extensible providers, multiple destinations enabled together, per-destination results and rolling retention, optional compression and chosen-passcode encryption, and password-prompted restore. Existing single Local/Drive support does not complete this.
- [ ] **R83 — Full-database export/import in Preferences:** export all cached mail/original MIME/attachments, settings and account definitions through a consistent online SQLite snapshot; provide file selection, progress/cancellation/errors and safe schema/integrity-checked import to another PC. Preserve database-backed state and pending-operation safety. Keychain secrets need separately protected transfer or reconnection (R49); coordinate with R22/R23.

## Storage, appearance and installation

- [ ] **R22 — Encrypted local cache:** encrypt SQLite/mail/attachment data at rest and migrate existing personal data safely. Keychain credentials and encrypted backups alone do not fulfill this.
- [ ] **R23 — Large mail:** remove incoming 25 MiB, snapshot 256 MiB and preview 32,000-character ceilings through bounded streaming/paging and background large downloads that do not hold up small mail. Avoid replacement arbitrary caps or unbounded RAM; audit outgoing/attachment limits too.
- [ ] **R25 — Palette editor:** persistent light/dark primary, secondary, background, surface, text, accent and related colors, with readable controls in Preferences.
- [ ] **R64 — Transparent theme-matching launcher icon:** preserve the approved Shepherd silhouette, locate a clean alpha asset or remove only its background, then verify GNOME/dash grouping and theme changes. Inspected `../shep-website` and `../shep-clients/web/public` copies were opaque; ignored imagegen candidates have not replaced the approved assets. Dark candidates were rejected for ragged edges.
- [ ] **R61 — Direct download installers:** GitHub raw scripts for Linux/macOS/Windows, home-directory installation by default, optional all-user/elevated installation, native application-menu integration and isolated download/extraction/update/cancellation tests. Make these the first install option near the top of README and in install docs.
- [ ] **R15/R17/R21 — Final usability review:** finish redundant-copy, spacing, icon sizing/alignment, compact/large-font and clipped Shortcuts-label checks. Preserve mouse usability, saved draggable panes, WebP assets and the approved design. Filtered-settings renderer correction 0d2feb1 is shipped.

## Verification and shipping

- [ ] **R63 — Final E2E functionality-path audit:** map every visible feature to realistic native happy/error/recovery scenarios as remaining features land. Remapping/clear controls and rapid native keyboard ordering are shipped; preserve deliberate rapid-input tests without extra waits.
- [ ] **R01/R06 — Provider/platform completeness:** finish POP3 malformed/duplicate UIDL and lifecycle gaps, rejected-command handling in remaining async-imap helpers, Google/CalDAV/SMTP integration evidence, independent-process write coordination and actual Windows/macOS execution/distribution. Distinguish fixture from live evidence.
- [ ] **R03/R09 — Final performance gates:** measure only at the end on an idle host; do not weaken budgets. Keep quality/release CI disabled until requested. Documentation CI has a separately authorized exception.
- [ ] **R08/R10 — Ship remaining verified work:** working checkpoints may push directly to `sam-ruff/shep.so` main. Run applicable hooks/Rust/Python/native checks and release/installer verification before installing production updates. Keep logs under ignored artifacts and preserve personal data. The handover push is a source checkpoint, not a new installed release.
