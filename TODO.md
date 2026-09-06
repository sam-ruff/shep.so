# Shep TODO

Active requests from this chat. Add new requests here immediately, including corrections. Keep an item until implementation and relevant automated/native verification are complete; remove it only while recording its evidence in [the completion log](docs/COMPLETION.md). [The request audit](docs/REQUEST_AUDIT.md) maps the complete conversation, including work already delivered. The full product goal is still active.

## Current UI and interaction work

- [ ] **R50 — Immediate feedback:** flags and read/unread state update before network/SQLite completion. Coalesce rapid edits, reject stale acknowledgments, roll back failed changes and show an error. Audit other reversible controls for the same delay.
- [ ] **R47 — A. Keep → Inbox:** investigate the reported failed move, verify the actual provider path and destination refresh. Commit 590ab10 ships Inbox labeling, send-queue feedback and a spaced-folder protocol test preserving acknowledgment after logout failure; the personal-account root cause is not yet confirmed.

## Mail reading, search and bulk actions

- [ ] **R38 — Faithful HTML:** render layout, typography, tables, backgrounds, links and inline images in their intended positions. Keep selectable text, a plain-text option, current remote-image controls and no active email scripts. Use synthetic versions of the supplied examples in tests.
- [ ] **R44 — Find in message:** Ctrl+F within the open email, fast background searching, visible matches and next/previous navigation.
- [ ] **R18/R44 — Fuzzy matching:** use a maintained matching library for inbox and Move search, with relevance ordering and typo tolerance. Regression: an email with body exactly “test” ranks ahead of weaker matches. Keep stable paging and stale-query protection.
- [ ] **R41 — Forward:** add a forward control to the preview actions; preserve message content/attachments and correct draft semantics.
- [ ] **R41 — Print:** add a print control and a usable cross-platform print flow, with isolated automated equivalents.
- [ ] **R42 — Multi-selection:** Ctrl+A for all emails in the focused list, Ctrl-click toggles, Shift-click ranges, and a Select button beside conversation search for checkbox selection. Preserve scope across pages and distinguish list focus from text selection.
- [ ] **R42 — Bulk actions:** preview toolbar and keybinds act on selected messages. Review multi-message actions with Y/N/Enter/Escape support, accurate scope/count and clear partial-failure handling.
- [ ] **R45 — Drag mail to folders:** drag one mail or a selected group onto sidebar folders; highlight valid destinations, respect cross-account preference/provider support, and use the bulk-move confirmation.

## Folders and composition

- [ ] **R30 — Folder trees:** nested folders form collapsible groups, collapsed by default, honoring each IMAP hierarchy delimiter and nonselectable parents.
- [ ] **R30 — Folder context menus:** delete folders and move them inside another folder; review destructive scope, keep account/server/cache state consistent, and test failure/retry behavior.
- [ ] **R35 — Inline composer:** new messages/replies open in the preview pane; autosave while typing and support switching among multiple drafts and received mail without losing content, recipients or attachments.
- [ ] **R36 — Draft navigation:** a collapsible Drafts group, right-click deletion, and a discard bin in the draft editor. Delete cached attachments and prevent delayed autosaves from resurrecting discarded drafts.

## Account/settings sync and backups

- [ ] **R02/R49 — Continuous Drive sync:** use OAuth and private appDataFolder for accounts and portable preferences. Offer opt-in on the first Google setup; discover an existing setup automatically on another PC; provide an ongoing toggle. Sync appearance, shortcuts, contacts and other portable settings with conflict-safe updates and offline recovery. Keep machine-specific paths/window placement local.
- [ ] **R49 — Account credentials on new PCs:** complete the password-protection decision and secure import into the OS keychain. An asynchronous question asks whether to use an additional sync passphrase (once per new PC) or Google-only protection; no answer has arrived yet. Do not claim password sync is implemented.
- [ ] **R32 — Multiple backup destinations:** polished destination list/setup for Local, Google Drive, S3, FTP/FTPS and SFTP, with independently configurable connections and easy extension points.
- [ ] **R32 — Backup options:** compression and chosen passcode encryption at setup, password prompt on restore, multiple destinations enabled together, independent upload results and configurable per-destination rolling copies (e.g. ten unreadable encrypted archives).

## Storage, appearance and final quality

- [ ] **R22 — Encrypted local cache:** encrypt SQLite/mail/attachment data at rest and safely migrate existing personal data; keychain credentials and encrypted backups alone do not fulfill this request.
- [ ] **R23 — Large mail:** remove the incoming 25 MiB, snapshot 256 MiB and preview 32,000-character ceilings using bounded streaming/paging and background large downloads. Large mail must not hold up smaller messages. Avoid replacement arbitrary caps/unbounded RAM allocations.
- [ ] **R25 — Palette editor:** configure primary, secondary, background, surface, text, accent and related theme colors in Preferences, with persistent light/dark schemes and readable states.
- [ ] **R15/R17/R21 — Final usability review:** finish remaining redundant-copy, spacing, icon sizing, alignment and compact/large-font checks. Preserve the approved dog logo, WebP assets, mouse usability, draggable panes and clean shadcn-style controls.
- [ ] **R01/R06 — Provider/platform completeness:** close remaining POP3 protocol/lifecycle gaps, Google/CalDAV/SMTP integration evidence, independent-process coordination and Windows/macOS execution/distribution checks. Do not present fixture evidence as live-provider verification.
- [ ] **R03/R09 — Final performance gates:** keep measurements deferred while the host is busy; run backend/native gates at the end on an idle host. Do not weaken budgets. Keep quality/release CI disabled until requested; documentation publishing has a separately recorded exception in AGENTS.md.
- [ ] **R08/R10 — Ship verified work:** fmt, Clippy, Rust/Python and relevant native tests; optimized release/installer verification; install for the Linux user and push main at sam-ruff/shep.so. Keep logs/artifacts out of the root and preserve personal data.
