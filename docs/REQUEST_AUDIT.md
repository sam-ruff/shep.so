# Conversation request audit

Audited against the user messages, source, AGENTS.md and completion evidence on 2026-09-06. “Delivered” refers to existing committed behavior, not completion of the entire product. “In progress” includes uncommitted code and does not imply shipping or a full passing test suite. Open items are maintained in [TODO.md](https://github.com/sam-ruff/shep.so/blob/main/TODO.md); completed evidence stays in [COMPLETION.md](COMPLETION.md).

| ID | Request and subsequent corrections | Status / evidence |
| --- | --- | --- |
| R01 | Native cross-platform Rust + iced client; multiple saved accounts; IMAP, POP3 and SMTP; extensible account backends | Core delivered; remaining provider/platform checks in TODO. `model.rs`, `providers/`, `engine.rs`, `store.rs` |
| R02 | Optional Google login to save account details to Drive | OAuth and encrypted account backups delivered; continuous account sync remains open (R49) |
| R03 | Extremely responsive UI; bounded channels/background work; strict speed/usability requirements; regular tests and CI performance gates; defer measurements until the end because PC is busy | Architecture, requirements, tests and dormant gates delivered; final measurements explicitly deferred |
| R04 | Good mouse use, not keyboard-first; native remappable shortcuts, especially M for Move | Delivered with mouse parity; 590ab10 adds secondary bindings and sidebar-only Inbox key |
| R05 | Drive email backup with configurable rolling copies; easily add account/backup providers | Local/Drive provider interfaces and rolling encrypted copies delivered; expanded destinations in R32 |
| R06 | Calendar tab; Google Calendar and home-server CalDAV sync | Delivered with local/native/wire evidence; live and cross-platform verification remains open |
| R07 | Clean shadcn-like iced components; flat Swiss Shepherd side-profile logo generated with image skill; approved logo preserved; WebP assets and dark logo; easy appearance preference | Delivered; palette editor remains open (R25) |
| R08 | Create GitHub repo under corrected owner sam-ruff, direct main pushes authorized; push completed code | Repo/main pushes delivered; ongoing shipping obligation |
| R09 | Semantic release CI, unit/E2E/MCP tests, fmt/Clippy pre-commit and CI, batchable MCP with short waits, repo test skill, all AI scenarios automated, document testing/releases in AGENTS, disable GitHub CI but retain files | Delivered baseline; quality/release remain dormant. A separate concurrent documentation task recorded authorization for docs CI/Pages in AGENTS; no quality/release enablement is authorized here |
| R10 | Linux release installer and desktop/dash launcher | Delivered and release/installer tested; install each verified update |
| R11 | Flagging, filtering and sorting | Delivered, including optimistic flag/read in 742b21e; remaining app-wide interaction work is tracked under R50 |
| R12 | Load app as user with Fastmail and debug sync; independent IMAP/POP3 and SMTP wizards with SSL/TLS, authentication and connection tests; supplied Fastmail settings | Delivered baseline; authorized read-only Fastmail diagnosis found FETCH syntax issue. New A. Keep move report in R47 |
| R13 | Block remote images by default; Block all / Contacts / Allow all; allow this email/sender/domain bar when blocked | Delivered with preferences, security and native tests |
| R14 | Explain/fix empty calendar dropdown; better event editor; double-click calendar to add an all-day event; use vertical space well | Delivered with no-calendar connection state, all-day/range editor and native tests |
| R15 | Remove useless local-device/privacy slogans, keeping useful explanations | Navigation copy cleaned; final copy review remains open |
| R16 | Inbox Up/Down; Tab between list/sidebar; sender address/copy dialog; wrapping attachments in reply bar; bigger initial/avatar icons; configurable font size; fix preview spacing and vertically misaligned buttons | Delivered baseline and native tests; continued compact/large-font review in TODO |
| R17 | Unified Inbox preference; optional cross-account moves; unified Inbox collapses with account children; custom folders at bottom; remove backup promo and duplicate syncing badges; clean list preview spacing | Delivered baseline; no claim that every visual issue is finished |
| R18 | Better fuzzy mail and Move search; Enter moves to the top choice | Delivered d3a530a: RapidFuzz, Best match ordering, exact-body regression, Unicode and fast Enter-to-move tests |
| R19 | Double-click mail opens full-window reader; Esc/close button with remapping | Delivered |
| R20 | Configurable collapsed replies/history instead of one continuous body; configurable separate-message conversation cards | Delivered and tested |
| R21 | Compact inbox header rather than an excessively thick top bar | Delivered; refresh-icon replacement is R53 |
| R22 | Encrypt local SQLite mail cache at rest | Open; keychain/backup protection is separate |
| R23 | Arbitrary email length/size; remove 25 MiB incoming, 256 MiB snapshot and 32k preview limits; background large downloads so other mail proceeds | Open; no higher-cap workaround is considered completion |
| R24 | Sidebar fits horizontally; sidebar/inbox/reader widths draggable; window dimensions persist across sessions | Delivered; long labels ellipsize and drag/layout state persists |
| R25 | Flagged outline red and entire UI palette configurable in Preferences | Red outline delivered; palette editor open |
| R26 | Ctrl-click folder multi-selection | Delivered, including native-event modifier snapshots and saved Ctrl-click flows in 590ab10 |
| R27 | Add account only in Preferences; clicking account heading collapses its folders with arrow | Delivered |
| R28 | Calendar sync refresh icon; remove Google/CalDAV sync caption and Workspace/Calendar breadcrumb; free calendar space | Delivered |
| R29 | Persist window and pane sizes | Delivered and SQLite reopen/close-order tests |
| R30 | Folder right-click delete/move into other folders; nested collapsible groups default collapsed | Open; current account collapse does not fulfill nested folder trees |
| R31 | Inbox item right-click menu | Delivered baseline; reported immediate-dismiss regression now R54 |
| R32 | Multiple backup options simultaneously; good setup UX; Google Drive/S3/FTP/SFTP/local; compression and passcode encryption; restore password; rolling unreadable copies | Open beyond existing single Local/Drive configuration |
| R33 | Separate Contacts Preferences section | Delivered |
| R34 | Save-success toast and slightly depressed button state | Delivered, with acknowledgment-based toast and failure preservation |
| R35 | Compose in preview pane, autosave while typing, read other mail and work on several replies/drafts | Autosave delivered; inline/multiple-draft UX open |
| R36 | Collapsible Drafts group, right-click delete, discard bin in draft editor; red discard confirmation button | Delivered 5aab267; collapsed preference, context/bin review, red confirmation, retirement/rollback/outgoing tests and native flows |
| R37 | Shortcut hints for buttons | Superseded by R51/R52: icon-only tooltip, primary key only and toggles |
| R38 | Render HTML like supplied examples, including the later raw-XHTML tags; preserve layout/images and selectable text | Native plain/quoted text selection delivered in 590ab10; faithful HTML remains open |
| R39 | Delete shortcut and optional second binding; final defaults Ctrl+D → Trash, Backspace + Delete → Archive; all remappable | Delivered in 590ab10: versioned slots/migration, conflict and native tests; older Delete-to-Trash default superseded |
| R40 | Clicking Mail while already open from another folder returns to unified/first Inbox | Delivered in 590ab10, native unified/first-account Inbox tests |
| R41 | Add Forward and Print controls in preview | Open |
| R42 | Ctrl+A list selection, Ctrl-click, Shift-click, checkbox Select mode beside search, bulk toolbar/keybind actions and Y/N/Enter/Esc confirmations | Open |
| R43 | Inbox (unread count) in sidebar | Delivered in 590ab10; cache counts ignore query/filter scope, with storage/native tests |
| R44 | Ctrl+F within email; fast search; fuzzy matching library and exact body “test” ranked first | Library/relevance search delivered d3a530a; Ctrl+F within a message remains open |
| R45 | Drag messages/selection from list into sidebar folders | Open |
| R46 | Highlight Move target used by Enter | Delivered in 590ab10; highlighted Inbox/Enter target and native visual evidence |
| R47 | Cannot move out of A. Keep into Inbox; display Inbox rather than INBOX | In progress; local metadata confirms folders exist, native return-move and wire/logout tests exist; actual reported personal-account cause not confirmed |
| R48 | I goes to Inbox only with sidebar focus; remappable and disableable | Delivered in 590ab10, including sidebar/list focus and native disable/remap tests |
| R49 | Drive appDataFolder continuously syncs accounts and as many settings as possible; first-time offer/toggle; existing cloud setup automatically loads on another PC | Open; credential-protection preference question pending |
| R50 | Flagging immediately reflects UI intent before database/network save; apply same treatment elsewhere appropriate | Flags/read/same-account moves delivered 742b21e; cross-account source feedback and typed results delivered 9c907d2; filtered destination projection, other controls and durable recovery remain open |
| R51 | Remove newly added tooltips from text-labeled controls; tooltips only on icons | Delivered in 590ab10, labeled controls unwrapped and native visual checks |
| R52 | Tooltip shows primary shortcut only; disable all tooltips or keyboard hints independently; searchable Preferences | Delivered in 590ab10; both tooltip toggles, primary-only hints, settings index/direct section navigation and native light/dark/compact tests |
| R53 | Sync mail becomes refresh icon at top right | Delivered in 590ab10; mouse sync/busy/navigation native tests |
| R54 | Right-click menu immediately disappears; fix and add tests | Delivered in 590ab10: background Changed no longer dismisses the menu; target refresh, mouse release, sync completion, action and dismissal tests |
| R55 | Audit whole conversation; maintain TODO.md immediately for every request; update AGENTS and remove items only when complete | Delivered: TODO.md and full audit plus immediate-tracking/removal rules in AGENTS.md; ongoing maintenance required |
| R56 | No root log files; delete accidental ones | Ongoing requirement; all current agent logs use ignored artifacts/logs |
| R57 | Preload messages, adjacent emails and next pages; WebP for image loading | Delivered baseline; maintain while large-mail/HTML work proceeds |
| R58 | Additional useful features required | Existing sender actions, outgoing recovery, conversations, connection removal and calendar discovery delivered; Forward/Print/selection/settings sync remain explicit open requests |
| R60 | Immediate optimistic archive/move and app-wide reversible-action feedback; persist principle in AGENTS.md | Principle recorded; immediate flags/read/moves delivered in 742b21e and 9c907d2, with remaining reconciliation/recovery under R50/R60 |
| R61 | Raw GitHub installers for Linux/macOS/Windows, user-local default, optional system install and app menus; first install commands in README/docs | Recorded, open |
| R62 | Fix nonworking read/unread and add integration coverage for basic mail behavior | Delivered 742b21e; IMAP NO detection, selective flags, dispatcher/cache/reopen/native coverage |
| R64 | Transparent GNOME desktop icon matching system theme | Recorded, open |
| R63 | Fix shortcut × and extend native E2E coverage across functionality paths | Clear controls delivered 742b21e; native search-focus isolation for default/remapped mail actions and wrong-row clear regression delivered 9c907d2; final functionality coverage audit remains open |
| R65 | Frequent background sync (user prioritizes rapid arrival), immediate startup check and independent manual Refresh | Delivered d4ecb21; 15-second default, saved seconds interval, independent coalescing refresh, virtual-time and native arrival/retry tests |
| R66 | Record Dungeonwalk vectoriser/remove.bg credential discovery in AGENTS.md | Delivered 742b21e; discovery pointer only, no keys copied |
| R67 | Select an inbox message, then click away to count it as read | Delivered 9c907d2: deliberate selection, immediate read-on-leave, explicit-unread protection, rollback and native navigation tests |
| R68 | Refreshing counted toast with Undo for archive, delete and move | Immediate counted feedback, refreshed lifetime, dismissal and failure reconciliation delivered 9c907d2; Undo remains open |
| R59 | Investigate and fix the newly failed CI build | Delivered eea1dfb; strict local build and GitHub run 34026001754 passed |

The main omissions were already present in the completion log but were not an adequate live checklist: faithful HTML, local encryption/large-mail streaming, folder trees/mutations, inline multiple drafts/discard, palette editing, multiple backup targets and continuous settings/account sync. The newer bulk-selection, drag/drop, find, optimistic feedback, tooltip/settings-search and context-menu requests are now explicit TODO entries. Passing the existing suite does not close them.
