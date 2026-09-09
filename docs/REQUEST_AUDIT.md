# Conversation request audit

**2026-09-08 priority update — desktop-main:R92 / R75 / R02 / R49:** the user prioritized full database transfer in Settings and Google account/settings profile sync, including first login from either desktop or Flutter and new/existing-device enrollment. They requested a Flutter implementation handover in this worktree, followed by OAuth implementation as the first TODO item referencing that document. [The handover](agents/PROFILE_SYNC_HANDOVER.md) now records the inspected integration points, proposed format, lifecycle/conflict rules, toggles, credential-protection decision and required tests. Writing it does not deliver OAuth/profile sync; implementation remains open at the top of TODO. Desktop-main:R83 complete database transfer remains separate from this shared profile format.

Audited against the user messages, source, AGENTS.md and completion evidence on 2026-09-07. “Delivered” refers to existing committed behavior, not completion of the entire product. “In progress” includes uncommitted code and does not imply shipping or a full passing test suite. Open items are maintained in [TODO.md](https://github.com/sam-ruff/shep.so/blob/main/TODO.md); completed evidence stays in [COMPLETION.md](COMPLETION.md).

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
| R15 | Remove useless local-device/privacy slogans, keeping useful explanations | Navigation copy cleaned; final copy review remains open. Preferences scale-control clipping fixed in 0d2feb1 with renderer/native pixel regressions; all 109 native scenarios pass |
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
| R38 | Render HTML like supplied examples, including the later raw-XHTML tags; preserve layout/images and selectable text | Delivered 1968e37 / 32120b4: MIME/XHTML selection, worker-rendered static HTML, native text copy, plain mode, quotes, inline/remote images, wide tables and focused-reader keyboard scrolling; 270 Rust, 20 Python and all 81 native functional scenarios pass; installed optimized build |
| R39 | Delete shortcut and optional second binding; final defaults Ctrl+D → Trash, Backspace + Delete → Archive; all remappable | Delivered in 590ab10: versioned slots/migration, conflict and native tests; older Delete-to-Trash default superseded |
| R40 | Clicking Mail while already open from another folder returns to unified/first Inbox | Delivered in 590ab10, native unified/first-account Inbox tests |
| R41 | Add Forward and Print controls in preview | Forward delivered in 9062dcf; Print delivered in b120721: complete cached MIME in Formatted/Plain mode, headers/CID images, background preparation, remappable Mod+P and browser printer/PDF selection; 289 Rust, 24 Python and 95 native functional scenarios verified (one startup-layout test rerun) |
| R42 | Ctrl+A list selection, Ctrl-click, Shift-click, checkbox Select mode beside search, bulk toolbar/keybind actions and Y/N/Enter/Esc confirmations | Native selection, frozen bulk reviews, persistent bounded jobs, per-message outcomes, immediate counts/toasts and grouped Undo are implemented. Shipped in af056a9: 347 Rust tests and all 116 native scenarios pass; optimized Linux build installed. Explicit new-arrival selection, group/individual coordination and remaining native recovery paths keep R42 open |
| R43 | Inbox (unread count) in sidebar | Delivered in 590ab10; cache counts ignore query/filter scope, with storage/native tests |
| R44 | Ctrl+F within email; fast search; fuzzy matching library and exact body “test” ranked first | Delivered d3a530a / c266035: library-based relevance search plus remappable Ctrl+F in formatted/plain message bodies, literal Unicode/whitespace matching, case toggle, highlighted next/previous, visible quote scope, wide-table reveal and native keyboard isolation; 277 Rust, 20 Python and all 86 native functional scenarios pass |
| R45 | Drag messages/selection from list into sidebar folders | Open |
| R46 | Highlight Move target used by Enter | Delivered in 590ab10; highlighted Inbox/Enter target and native visual evidence |
| R47 | Cannot move out of A. Keep into Inbox; display Inbox rather than INBOX | In progress; local metadata confirms folders exist, native return-move and wire/logout tests exist; actual reported personal-account cause not confirmed |
| R48 | I goes to Inbox only with sidebar focus; remappable and disableable | Delivered in 590ab10, including sidebar/list focus and native disable/remap tests |
| R49 | Drive appDataFolder continuously syncs accounts and as many settings as possible; first-time offer/toggle; existing cloud setup automatically loads on another PC | Open; credential-protection preference question pending |
| R50 | Flagging immediately reflects UI intent before database/network save; apply same treatment elsewhere appropriate | Flags/read/same-account moves delivered 742b21e; cross-account source feedback and typed results delivered 9c907d2; 90776fa adds observed pending identities and global unread count reconciliation across page scopes; filtered destination rows, ambiguous outcomes, other controls and durable recovery remain open |
| R51 | Remove newly added tooltips from text-labeled controls; tooltips only on icons | Delivered in 590ab10, labeled controls unwrapped and native visual checks |
| R52 | Tooltip shows primary shortcut only; disable all tooltips or keyboard hints independently; searchable Preferences | Delivered in 590ab10; both tooltip toggles, primary-only hints, settings index/direct section navigation and native light/dark/compact tests |
| R53 | Sync mail becomes refresh icon at top right | Delivered in 590ab10; mouse sync/busy/navigation native tests |
| R54 | Right-click menu immediately disappears; fix and add tests | Delivered in 590ab10: background Changed no longer dismisses the menu; target refresh, mouse release, sync completion, action and dismissal tests |
| R55 | Audit whole conversation; maintain TODO.md immediately for every request; update AGENTS and remove items only when complete | Delivered: TODO.md and full audit plus immediate-tracking/removal rules in AGENTS.md; ongoing maintenance required |
| R56 | No root log files; delete accidental ones | Ongoing requirement; all current agent logs use ignored artifacts/logs |
| R57 | Preload messages, adjacent emails and next pages; WebP for image loading | Delivered baseline; maintain while large-mail/HTML work proceeds |
| R58 | Additional useful features required | Existing sender actions, outgoing recovery, conversations, connection removal and calendar discovery delivered; Forward/Print/selection/settings sync remain explicit open requests |
| R59 | Investigate and fix the newly failed CI build | Delivered eea1dfb; strict local build and GitHub run 34026001754 passed |
| R60 | Immediate optimistic archive/move and app-wide reversible-action feedback; persist principle in AGENTS.md | Baseline delivered 742b21e; principle recorded, remaining app-wide audit under R50/R60 |
| R61 | Raw GitHub installers for Linux/macOS/Windows, user-local default, optional system install and app menus; first install commands in README/docs | Recorded, open |
| R62 | Fix nonworking read/unread and add integration coverage for basic mail behavior | Delivered 742b21e; IMAP NO detection, selective flags, dispatcher/cache/reopen/native coverage |
| R63 | Fix shortcut × and extend native E2E coverage across functionality paths | Clear controls delivered 742b21e with per-slot persistence and native tests; final functionality coverage audit remains open |
| R64 | Transparent GNOME desktop icon matching system theme | Recorded, open |
| R65 | More frequent/reliable background sync independent of manual Refresh | Desktop delivered upstream d4ecb21 / 2965ae2; this older client worktree must preserve it during integration; mobile/browser lifecycle work remains open |
| R66 | Record Dungeonwalk vectoriser/remove.bg credential discovery in AGENTS.md | Delivered 742b21e; discovery pointer only, no keys copied |
| R67 | Flutter Android/Apple, separate desktop-style browser client feature parity, same shadcn-inspired theme/configurability; corrected to root desktop + flutter/ + web/ + website/ in one monorepo | Worktree Flutter/browser surfaces and parity matrix implemented; full parity open; separate shep.flutter repository superseded |
| R68 | K-9-style mail list, swipe icons/previews, configurable swipes | Preview gestures/icons/preferences tested on Android and Chromium; native provider integration and full accessibility review open |
| R69 | Walkie Textie Flutter Playwright/native E2E, emulators/simulators, disabled CI and coordinated release tooling | Android integration/Appium and Playwright executed; Apple execution/signing and full release packaging open; CI/release stay disabled |
| R70 | Delegated worktree promo website, screenshots, user-agent-aware install and other-platform/store links | Delegated worktree source reviewed and copied to combined review worktree; 61 browser tests pass, two clipboard skips; publishing/store targets open |
| R71 | Separate web client at shep.so and shared network hosting; client-side credentials/Google login, no server storage | Separate browser and client-side cache connected to Rust gateway; production deployment and complete feature parity open |
| R72 | Linux app stores/distribution | Recorded as requested TODO |
| R73 | Keep feature parity with every desktop change; work only in worktrees for now | AGENTS reminder, parity matrix/checker and shared scenarios implemented; no main merge/push. Latest R74 authorizes VPS installation once configuration arrives |
| R74 | Hosted beta login restricted initially to owner; Rust backend on existing email VPS for login verification and SMTP/other mail protocols; later allow more users | Rust allowlist/session/mail gateway and production-browser fixtures verified; supersedes static-only/local-bridge transport. VPS/OAuth/owner configuration and live verification pending |
| R75 | Replace manual Google tokens with OAuth “Sign in with Google”; maintain parity across clients | Recorded, open; beta access login is separate from Google provider authorization |
| R76 | Preferences out-of-office replies for individual/all accounts and separate groups, reusable messages, start/end times and easy assignment UX | Recorded, open; named Automatic replies entries, reusable messages/schedules, searchable account selection, saved groups/Select all and per-account provider results |
| R79 | Install the current app on the owner’s Android phone using authorized wireless ADB | Production-flavor 0.1.0 ARM64 release built, development-signed and installed without clearing data; Android launch/package/process verified. Final screen check found the phone locked; evidence shipped in [`1ea12a6`](https://github.com/sam-ruff/shep.so/commit/1ea12a677829dcd71c4246b87cae46327f9c749f). |
| desktop-main:R67 | Select an inbox message, then click away to count it as read | Delivered 9c907d2: deliberate selection, immediate read-on-leave, explicit-unread protection, rollback and native navigation tests |
| desktop-main:R68 | Refreshing counted toast with Undo for archive, delete and move | Delivered 9c907d2 / 551f86c: immediate counted feedback and grouped Undo before/after acknowledgment, original-account/folder restoration, verified server identities and persistent failed-reversal retry; 81 native functional flows pass, including immediate toasts and Undo while mail saves remain pending |
| desktop-main:R69 | HTML layout moves during rendering, visual artifacts and slow readiness; investigate more pre-caching/rendering | Delivered across cd8f732, 6d83b72 and cc38af9: actual viewport loading, stale-frame isolation, retained fonts, bounded adjacent-frame preparation, stable controls, compact attachments/Find, late-image text anchoring and native Retry. All 104 native functional scenarios and 305 Rust/27 Python tests pass; release installed and pushed. Final idle-host timing remains R03/R09 |
| desktop-main:R70 | New-email count badges on the dock/taskbar launcher, as in the supplied GNOME screenshot | Linux publication and preference delivered in 90776fa, with private-bus Update/Query/reconnect tests, global optimistic count reconciliation, 313 Rust/29 Python tests and all 108 native scenarios. Windows/macOS, actual dock rendering and remaining ambiguous/restart cases stay open |
| desktop-main:R71 | Refresh icon looks malformed in the latest screenshot | Delivered for native Shep in cd8f732: shared Mail/Calendar SVG arrowheads corrected; normal/120% scale, light/dark and compact visual/native evidence; the browser crop was not separately reproduced |

The main omissions were already present in the completion log but were not an adequate live checklist: faithful HTML, local encryption/large-mail streaming, folder trees/mutations, inline multiple drafts/discard, palette editing, multiple backup targets and continuous settings/account sync. The newer bulk-selection, drag/drop, find, optimistic feedback, tooltip/settings-search and context-menu requests are now explicit TODO entries. Passing the existing suite does not close them.

Native Flutter continuation for R67/R69: the worktree now contains a real Rust bridge/cache/provider adapter, secure credential pairs, account setup, durable draft/autosave/discard controls and saved device scenarios. Host Rust/FFI tests and four additional native Android scenarios pass, including real SQLite/restart/discard controls and isolated device credentials. Full provider-success/recovery and Apple verification remain open. No parity or shipping completion is claimed. R75 and grouped/scheduled Automatic replies R76 remain active TODOs. Desktop main has independently advanced to f659926, including read-on-leave/grouped action toasts; their mobile/browser parity gaps remain recorded.

Latest independent desktop advance: 551f86c / 1c1372e ships responsive move Undo and provider move receipts. The later worktree continuation ports their receipt/exact-content recovery contract into the shared core and clients. Stable identities, durable unresolved intents, queued Undo and Undo after refresh are verified; the complete current-main integration and ambiguous-move review remain open.

Move continuation for R68/R73/R74: shared IMAP COPYUID/APPENDUID acknowledgments and exact raw-content recovery are exercised with loopback transcripts. SQLite/IndexedDB keep stable UI identifiers while server UIDs change; sync and restart preserve mapping, and unresolved transmissions cannot automatically repeat. Browser production controls run through Rust HTTPS fixtures; Android exercises the paged swipe controls. R76 remains the requested grouped, scheduled Automatic replies TODO. No request is closed by this partial worktree checkpoint.

Reply/attachment continuation for R67/R69/R73/R74: shared Rust and browser reply fixtures agree on Reply-To, recipient exclusions, references and quoting. Separate SQLite/IndexedDB file associations survive text saves and reopen; submitted drafts reject file changes. Android drives the real cancel/multi-file picker and removal/reopen controls, while the authenticated browser fixture verifies exact outgoing MIME/binary bytes and compact composer actions. Full composition/provider/Apple and Outbox/Sent parity remains open. R75 OAuth and R76 grouped, scheduled Automatic replies remain active TODOs; no main merge/push or deployment is implied by this evidence.

Outbox continuation for R67/R69/R73/R74: the browser persists exact prepared MIME/envelope before SMTP and caches the original local Sent copy. Real Rust HTTPS controls cover uncertain/rejected review, cancelled preparation, immutable attachment recovery and a separately requested new Send; backend tests protect active submissions and bind prepared content. Known delivery/rejection survives transient receipt expiry. Native Outbox, provider Sent lookup/append, bounded cache/Outbox reads and complete parity remain open. R75 OAuth and R76 grouped Automatic replies remain active TODOs. Upstream selectable HTML shipped in 1968e37 and still needs deliberate integration/client parity. This work remains uncommitted in the review worktree; no main merge/push or VPS deployment occurred.

Native Outbox continuation for R67/R69/R73: Flutter now has 20-row Outbox review, rejected/uncertain return with new draft/file ownership, manual mark and exact local Sent repair. Known SMTP acknowledgments survive subsequent cache failures; shared profile handles and exclusive OS ownership protect active operations. All 23 native Rust tests, 22 Flutter host tests, ten Android integration scenarios, five Appium stages and five Flutter Playwright stages pass. The Android scenario includes an actual second-process lock check and reviewed light/dark captures. Provider Sent lookup/append, complete composition/platform parity and current-main integration remain open. Desktop main advanced independently to 6fed03b with reader focus fixes; uncommitted HTML/find work is not imported. R75 Sign in with Google and R76 grouped, scheduled Automatic replies remain active TODOs. No request is closed; no client commit, main merge/push or VPS deployment occurred.


Native provider Sent continuation for R67/R69/R73: shared Sent discovery and exact Message-ID lookup now verify final tagged responses, with implicit-TLS/STARTTLS and hostname-refusal transcripts. Flutter journals destination before APPEND, requires reviewed retry after missing acknowledgment, keeps accepted results through cache failures, and allows the composer to close while its acknowledged send's copy finishes under owned account/capacity guards. Synced provider copies remove only untouched local copies; local edits and newer Sent preferences survive. All 32 native Rust tests, 23 Flutter host tests, ten Android integration scenarios, five Appium stages and five Flutter browser stages pass; the production APK and screenshots are reviewed. Native Sent handover identity/offline-local actions remain active gaps; browser Sent API/journal/UI equivalents, complete parity and shipping stay open. Desktop main independently advanced to c266035 / f5f13c9 with find-in-message; its committed work needs parity integration, while current uncommitted composition edits remain untouched. R75 OAuth and R76 grouped/scheduled Automatic replies remain active TODOs.


Offline local Sent continuation for R67/R69/R50: native Flutter actions use Rust's current stored identity to request credentials only for server-backed mail; local IMAP Sent flag/read/move and reopen work with a locked credential store. Host and Android controls verify missing/locked credential refusal and rollback. The open reader now exposes the same persistent error, Retry and Dismiss controls as the inbox. All 33 native Rust tests and 25 Flutter host tests pass, along with the expanded Android Outbox scenario and five Flutter browser stages. This is a partial uncommitted worktree checkpoint, not full parity or shipping completion. R76 is clarified as named reusable reply entries with searchable account assignment, saved groups/Select all and schedules; implementation remains a TODO. Desktop Forward subsequently shipped in 9062dcf / ac515a3; retain the committed-integration and mobile/browser parity gap alongside HTML/find and the earlier defaults.

### R77 — Prompt checkpoint push

The user explicitly requested pushing all current work as soon as possible. Commit and push the combined `feat/mobile-web-clients` review branch after its required checks; full parity remains an active goal. This changes the previous unpublished-worktree checkpoint assumption without authorizing a merge into main. Completed by commit [`d80f539`](https://github.com/sam-ruff/shep.so/commit/d80f539ba56c0dd9631d4305d453c841bcfe5901), pushed to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients) with the remote SHA verified. Required commit hooks, strict docs and final targeted Android/APK checks pass; the completion log records their evidence and limits. Main remains unchanged and full parity remains active.

### R67/R68 continuation — stable native Sent handover

Atomic native Sent adoption retains local and previous-provider IDs, protects local edits, preserves physical Undo destinations and groups known provider Sent folders. The completion log records 39 native Rust tests, 27 Flutter host tests, eleven Android scenarios, five Appium and five Flutter browser stages, plus reviewed screenshots. These are synthetic protocol/cache/control results, not live-provider or full parity evidence. Copy labels/history cleanup, independent reader retention and equivalent browser provider Sent recovery remain active.

### R67/R71/R74 — Browser provider Sent recovery

Browser Outbox now checks provider Sent, saves exact copies using client-owned reservations, retains acknowledgments through storage/HTTP failure and requires explicit review before an uncertain retry. Account Sent preferences persist independently of reconnect. The Rust service uses authenticated, pinned shared transport and transient receipts without a persistent mail/password database. The completion log records route/model/real HTTPS control evidence; local/provider identity handover/grouping, bounded history and the wider parity goal remain open. Desktop Print independently shipped in `b120721` / `c800256`; its client equivalents and deliberate integration are retained in TODO.

This browser Sent increment is committed and pushed as [`d24d87a`](https://github.com/sam-ruff/shep.so/commit/d24d87a5c52adb205b8bfdc18743a49354c9c3b1), with remote-SHA verification. The completion log records 33 backend tests, 49 browser tests, 14 Playwright scenarios, 36 real Rust HTTPS browser stages, 212 root hook tests and reviewed visual evidence. The wider R67/R71/R74 requirements remain active.


### R67/R68/R71/R74 — Browser Sent identity continuation

Atomic provider adoption now preserves original client IDs and persistent aliases, logical Sent membership, reader selection, queued actions and physical Undo destinations. Local edits persist during held sync and protect their separate copies. Actual IndexedDB migration/rollback, 57 browser tests, 15 Playwright scenarios, 34 backend tests and 40 real Rust HTTPS browser stages provide partial-feature evidence, recorded with screenshots and limits in the completion log. Copy labels/history, bounded reads, complete lifecycle/provider/platform parity and deployment remain active. R75 Sign in with Google and R76 scheduled grouped Automatic replies remain TODOs; no request is closed by this increment. The user's R77 prompt-push authorization continues to apply to the combined review branch.

The browser identity continuation is committed and pushed as [`b132fa2`](https://github.com/sam-ruff/shep.so/commit/b132fa2fa4f622315440b4a578409623d07ef1c0), with remote verification and all 212 root hook tests passing. R77's prompt-push instruction is fulfilled for this increment; the wider client goal remains active.


### R67/R69/R71/R73/R74 — Cached incoming files and reader retention

Both clients now decode cached attachments through shared Rust content code (WASM in a browser worker), with stable per-part content identities and exact binary/Unicode bytes. Native controls distinguish saved/cancelled/failed operations; a moved reader remains available after its Inbox page refresh. Corrupt attachment metadata keeps the cached body readable. Android DocumentsUI and offline Rust HTTPS browser scenarios exercise real save controls, with shared byte fixtures and reviewed screenshots. The completion log records the final tests and shipping commit. Apple export is implemented but awaits macOS compilation/simulator execution; large/deep MIME, lifecycle, broader parity and VPS deployment remain active. R75 OAuth and R76 grouped scheduled Automatic replies stay in TODO. Quality/release workflows remain disabled; coordinated version stamping includes the new shared content crate. R77 continues to authorize the prompt review-branch push.


Incoming attachment saving and reader retention shipped for review as [`abc44dc`](https://github.com/sam-ruff/shep.so/commit/abc44dcd0d85d38e607d8ff8c30315303800a03c) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required hooks passed formatting, Clippy and **215 root/shared tests**; the two opt-in personal-account diagnostics remain intentionally ignored. Pinned strict Zensical validation passed. R77's prompt-push instruction is fulfilled for this increment; full parity and the remaining requests above stay active. No main merge, deployment or workflow enablement occurred.


### R67/R69/R71/R74 — Reviewed client account removal

Both clients now implement the desktop's local removal review, including accurate counts, stale-review refusal, explicit unfinished-record acknowledgment, transaction rollback and protection against stale saves/reconnects. Native credential cleanup survives removal and restart, with a visible retry after keychain failure. Browser removal and normal writes check persistent removed identities atomically; the tombstone does not retain deleted content. Saved Android and Rust HTTPS browser controls cover cancellation, light/dark review, completion, retry and stale-tab behavior. Final verification and shipping evidence are recorded in the completion log. Connection editing, Apple execution and the broader parity/lifecycle requirements remain active. Upstream independently advanced to d3b34e7 with HTML preparation/compact layout/interface scaling; integration and matching client behavior remain tracked. R75/R76 remain TODOs, and R77's prompt review-branch push authorization persists.


Reviewed account removal shipped as [`18bf033`](https://github.com/sam-ruff/shep.so/commit/18bf0332ec4075004d5ce3ae42eb1a3271722f77) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote commit verified. Required formatting, Clippy and all **215 root/shared tests** passed; the two opt-in personal-account diagnostics remain intentionally ignored. Final pinned strict documentation validation passed. R77's prompt-push request is fulfilled for this increment. Full parity, Apple execution and VPS deployment remain active; quality/release workflows remain disabled and main was not changed by this work.


### R67/R69/R73 — Native credential handover prerequisite

Reconnect now stages a separate device credential pair and atomically activates the matching account configuration. Failed writes/activation preserve the previous pair; lost activation responses retain the committed new pair. Durable cleanup excludes active keys, and provider requests reject stale credential bindings under the account lock. Legacy SMTP TLS defaults are preserved. The native/host and saved Android control evidence is recorded in the completion log. Full connection editing, reviewed mailbox-identity migration, broader lifecycle/browser parity and Apple execution remain open. This is concrete progress toward the complete clients, not a claim that account management or the product is complete. R75/R76 remain TODOs and R77 continues to authorize prompt review-branch pushes.


Atomic native credential handover is committed and pushed as [`b9a0102`](https://github.com/sam-ruff/shep.so/commit/b9a0102397778444745a4afee8578284fea78748) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required formatting/Clippy hooks and **215 root/shared tests** pass; two opt-in personal-account diagnostics remain intentionally ignored. Pinned strict documentation validation passes. The owned Android emulator was stopped after verification. R77's prompt review-branch push is fulfilled for this increment; full parity remains active. Main, deployment and quality/release workflow enablement are unchanged.


### R44/R67/R69/R71/R73 — Find in displayed client mail

Flutter/native Rust and the separate browser/WASM now share the desktop's literal Unicode and whitespace matching contract, with original UTF-16 ranges, visible highlights, case matching, next/previous, quote visibility, stale-result protection and selection/Copy. Browser shortcuts can be remapped/disabled without replacing an older custom binding. Saved host, native, Appium and browser scenarios are tracked in the shared registry; final execution and shipping evidence follows in the completion log. Full HTML Find, large-text bounds, complete mobile keymaps, current-main integration and Apple execution remain open. R75 Sign in with Google and R76 scheduled grouped Automatic replies remain active. R77 continues to authorize promptly committing and pushing the review branch.

The Find increment now has 50 native Rust, 41 Flutter host, 65 browser, 34 backend, 19 Playwright and 48 authenticated HTTPS stage results, plus the 13 Android integration scenarios, six Appium stages and six Flutter browser stages. Reviewed light/dark captures and production APK/browser fixture-exclusion reports are recorded in the completion log. This fulfills the tested text-reader increment while the explicitly listed parity and deployment gaps remain active; shipping is recorded with the commit below.


Client Find is committed and pushed as [`50eab04`](https://github.com/sam-ruff/shep.so/commit/50eab047b3b9986b29c94b1810076e5c819b3f8a) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required formatting/Clippy hooks and **217 root/shared tests** pass; two opt-in personal-account diagnostics remain intentionally ignored. Pinned strict documentation validation passes. R77's prompt review-branch push is fulfilled for this increment. Full parity, Apple execution and VPS deployment remain active, and R75/R76 remain in TODO. Main and the disabled quality/release workflows were not changed by this work.


### Reader MIME prerequisite — R23/R38/R44/R67/R69/R71/R73/R74/R77

Shared selection now preserves alternatives, related roots and independent/ambiguous CID scopes, while cache text, replies and attachment classification agree across native Rust and browser WASM. Iterative MIME preflight and HTML text traversal protect the worker stack. Fourteen shared fixtures, native bridge/provider tests and actual Android/HTTPS browser controls provide the evidence recorded in the completion log. Formatted HTML, resource confinement, remote-image policy, full Find/selection and Apple parity remain active; raw HTML output is explicitly untrusted. Two iced functional runs each passed 57/58 with different intermittent input misses; isolated reruns pass, the first drag now settles its frame and rapid-input diagnosis remains in TODO. R75/R76 remain TODOs. R77 continues to authorize prompt pushes to the combined review branch, with no main merge or workflow enablement.


Shared MIME selection and nesting protection shipped for review as [`4226f57`](https://github.com/sam-ruff/shep.so/commit/4226f577fd38f5d2e3bd353d5b15a4ab8ab36eec) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the remote SHA was verified. Required formatting/Clippy hooks and all **225 root/shared tests** pass, with two opt-in personal-account diagnostics intentionally ignored. Pinned strict documentation validation passes. The dedicated Android emulator is stopped and logs/screenshots remain under ignored artifacts. The full-run iced input limitations above remain active; R77's prompt review-branch push is fulfilled for this increment. Full parity, R75/R76, current-main integration and VPS deployment remain open. Main and disabled quality/release definitions were not changed by this work.


### Confined browser HTML — R23/R38/R44/R67/R69/R71/R73/R74/R77

The separate browser now uses shared Rust/WASM preparation and an opaque sandboxed frame for authored CSS/tables/inline images, selection/Copy, Find across spans, quote visibility and plain-text choice. Saved control/security/visual scenarios and actual Rust HTTPS CSP/worker-retry evidence are recorded in the completion log. Preparation/display failure stays recoverable and stale navigation results cannot replace the current reader. The shared scenario registry explicitly keeps native Flutter WebView/Android/Apple execution, remote-image policy and large-document/performance parity open. R75/R76 and exact VPS/OAuth configuration remain active. R77 authorizes promptly committing and pushing this verified increment to the combined review branch; shipping evidence follows after required hooks. No main merge or workflow enablement is included.


### Confined HTML shipping checkpoint

Browser HTML rendering shipped for review as [`a9b07e2`](https://github.com/sam-ruff/shep.so/commit/a9b07e25769e4ba1424bc7016f1f66487635f9e7) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the remote SHA was verified. Required formatting/Clippy hooks and **229 root/shared tests** pass, with two opt-in personal-account diagnostics intentionally ignored. Evidence includes **69 browser unit tests, 26 Playwright scenarios, 49 Rust HTTPS stages, 34 gateway tests, 51 native Rust host tests, 26 Python tests**, strict pinned documentation validation, 24 parity contracts and inspected production artifacts. Synthetic initial-reader and Find WebPs were reviewed in light 1440×920 and dark 900×640 layouts. Logs/artifacts remain ignored. R77's prompt review-branch push is fulfilled for this increment. Flutter HTML/device execution, remote-image policy, full parity, R75/R76 and VPS deployment remain open; main and disabled quality/release definitions remain unchanged. Re-enable quality/release only when the trusted runners and release prerequisites are ready.


### Flutter confined reader — R23/R38/R44/R67/R69/R71/R73/R77

Flutter now prepares the shared document in bounded native Rust work and keeps a system WebView alive through reader updates. Saved Android integration, native Appium and Flutter browser scenarios cover layout/resources, Find, quote/plain choices, selection, links and recovery; actual execution and shipping evidence are recorded in the completion log. iOS targets 14.0 for WebP, with Apple execution still open. Remote-image policy, large-document/performance parity, current-main integration, R75/R76 and VPS configuration remain active. R77 continues to authorize a prompt review-branch push; no main merge or deployment is included.


### Flutter reader shipping checkpoint

The Flutter formatted-reader increment shipped for review as [`6e8475f`](https://github.com/sam-ruff/shep.so/commit/6e8475f4470534fd4d16836c8a9efefa8915f1f6) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the remote SHA was verified. Required formatting/Clippy hooks and all **229 root/shared tests** pass, with two personal-account diagnostics intentionally ignored. The platform/browser/protocol evidence, production artifact inspection and remaining limits are recorded above. Pinned strict documentation validation passes. R77's prompt push is fulfilled for this increment; full parity, Apple execution, R75/R76 and VPS deployment remain active. The owned emulator is stopped, artifacts remain ignored, and main was not merged or modified by this work. Quality/release definitions remain disabled; re-enable them when trusted runners and release prerequisites are ready.


### Committed desktop integration — R65/R67/R73/R77

The client review worktree is integrating desktop main through `10d8569`, preserving the existing shared MIME/provider contracts and both request histories. Colliding desktop-only R67–R71 requests are explicitly namespaced above. No independent uncommitted main-worktree edits are imported. Root/native/browser regression evidence for the merged source must be recorded separately from the historical main tests. Forward/Print, bulk/selection, read-on-leave/toasts, image policies/anchors and OS integration still need client parity. R75/R76 and VPS configuration remain open; R77 authorizes the prompt verified review-branch push.


The merged desktop now passes all 116 saved native functional flows in one run and 372 Rust/shared tests. The completion log records shared/native/browser checks, the optimized archive/temporary installer verification, reviewed screenshots and Android automation corrections. This remains a review-branch checkpoint with active client feature gaps; historical main results are not substituted for the merged-branch run.

All 14 Android integration scenarios and both native Appium paths now have passing evidence across the full and targeted runs; the completion log retains each intervening automation failure/correction. Flutter browser formatted/standard controls also pass. R67/R69/R73 remain open for full parity and Apple/distribution evidence; R77's prompt review-branch shipping step follows the final artifact check.


### Desktop integration shipping record

Committed and pushed as [`72ca625`](https://github.com/sam-ruff/shep.so/commit/72ca625961bee619747e98110adcd2dfe22eb8c0) on [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the remote SHA verified. Required hooks passed formatting, Clippy and all **372 root/shared tests**; two personal-account diagnostics remain intentionally ignored. The tests, production artifacts, native visual review and corrected Android automation are recorded above. R77’s prompt-push request is fulfilled for this increment. The dedicated emulator is stopped, artifacts remain ignored, and the separate main worktree and personal installation were not changed. Full parity, Apple execution, R75/R76 and VPS deployment remain active. Quality/release definitions remain disabled; re-enable them when trusted runners and release prerequisites are ready.


### Shared Forward preparation — R41/R67/R71/R73/R77

Complete cached MIME preparation now lives in the shared native/WASM content crate; root desktop uses the shared draft wrapper. It preserves an independent thread and exact file bytes while keeping outgoing HTML separate from safe display documents. Shared fixtures cover duplicate attachment metadata, independent CID scopes, corrupted resources and current size/count/nesting limits. Client atomic storage and Forward controls remain active gaps; verification and prompt review-branch shipping evidence follow in the completion log. Apple execution, R75/R76, broader parity and VPS configuration remain open.


### Shared Forward preparation shipping record

Committed and pushed as [`64d4936`](https://github.com/sam-ruff/shep.so/commit/64d493605a14f6a7c44e627be1d5423cc3f01772) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **377 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The complete 116-flow native run, shared native/WASM fixtures, browser/HTTPS regressions, native bridge/gateway tests, reviewed screenshots and production package checks are recorded in the completion log. Artifacts stay ignored and the temporary target symlink is removed. R77's prompt push is fulfilled for this increment. Full parity, Flutter/browser Forward controls and durable metadata, Apple/live verification, R75/R76 and VPS deployment remain open. Quality/release workflows remain disabled; re-enable them when trusted runners and release prerequisites are ready.


### Client Forward controls — R41/R67/R69/R71/R73/R77

Native Flutter and the separate browser now connect complete-source preparation to atomic draft/file storage, retained quotation/CID metadata, independent editor behavior and stable retries. Native schema migration/rollback, Outbox ownership and gateway MIME contracts have automated evidence; real Android controls, browser workers/IndexedDB and the Rust HTTPS harness cover restart and failure/recovery. The completion log records counts, visual review, automation corrections, remaining limits and the prompt review-branch push. R41 remains open for Apple/full-composition/live evidence; broader parity, R75/R76 and deployment prerequisites are preserved.


### Client Forward shipping record

Committed and pushed as [`21ca299`](https://github.com/sam-ruff/shep.so/commit/21ca299907ef943aa4046166a3c934e15b4779b3) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients), with the exact remote SHA verified. Required hooks passed formatting, Clippy and **377 root/shared Rust tests**; two personal-account diagnostics remain intentionally ignored. Client native/protocol/browser/HTTPS tests, reviewed captures, corrected automation and production artifact inspection are recorded in the completion log. The dedicated emulator is stopped and artifacts remain ignored. R77's prompt push is fulfilled for this increment. Main and the personal desktop installation were not changed. Full parity, Apple/live/performance evidence, R75/R76 and VPS deployment remain active. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Client Print controls — R38/R41/R67/R69/R71/R73/R77

Shared complete-source printing now connects to Flutter Android's native printer/PDF controls and an independent beta-protected browser preview. Native/WASM/host contracts and actual Android/Chromium output cover source identity, complete text, confined inline resources, header privacy, cancellation/retry, independent navigation/editor state and browser shortcut remapping/input isolation. UIKit is implemented but uncompiled/unexecuted here. The completion log records counts, reviewed PDFs/captures, the corrected test selectors/printer selection/artifact-export helper, remaining platform/lifecycle limits and prompt review-branch shipping. Legacy reader background/text defaults are also corrected; the wider R38 audit remains active. Full parity, R75/R76 and the missing deployment configuration remain traceable in TODO.


The Print increment shipped for review in [`67f206c`](https://github.com/sam-ruff/shep.so/commit/67f206caf2a8cdab9ccf46ab66c2012f332360c8), with the exact remote SHA verified and all required hooks passing. R77’s prompt-push request is fulfilled for this increment; [the completion log](COMPLETION.md) records the client tests, actual PDFs, production artifacts and remaining parity/platform/deployment work.


### Client read-on-leave — R65/R67/R68/R69/R71/R73/R77

The desktop read-on-leave default now has Flutter/browser control equivalents, explicit unread precedence and ordered quiet mutation recovery. Native paged cache queries remain available during provider work and project folder/filter/count intent in one read snapshot. Host, native Rust, Android controls and Playwright evidence, corrected failed runs and reviewed captures are recorded in [the completion log](COMPLETION.md). The full parity request remains active: close/OS/Apple lifecycle, large-cache performance, counted notifications and remaining client features are still TODOs. R77 continues to authorize prompt verified pushes to the client review branch; no main merge or personal installation is included.

The read-on-leave/paging and Drafts fixes shipped in [`5fc9560`](https://github.com/sam-ruff/shep.so/commit/5fc9560fda68109592e1be2aa6fcf16b0579d1c9), with the exact review-branch remote SHA verified and all required hooks passing. [The completion log](COMPLETION.md) records verification and remaining limits. R77’s prompt push is fulfilled for this increment; the full product goal remains active.


### Counted client move feedback — R50/R60/R65/R67/R68/R69/R73/R77

Flutter and the browser now mirror the desktop’s six-second counted Archive/Delete/Move/Restored notifications, destination/account grouping, Dismiss and grouped Undo. Unsent moves cancel behind pending reads; dispatched reversals wait for acknowledged identities. Partial failures retain dedicated Retry Undo, while acknowledged reversals only refresh metadata. Native controls also verify immediate projected row restoration and Undo from another reader. General Refresh status remains independent. The completion log records tests, corrected failures, visual evidence and shipping. Durable move-history/recovery, Apple/OS/close lifecycle, the full parity backlog, R75/R76 and VPS configuration remain active. R77 continues to authorize the prompt review-branch push.

Counted client feedback is committed and pushed as [`96275a1`](https://github.com/sam-ruff/shep.so/commit/96275a18e5c623bc6837578bf4227dc16c339a0e), with the exact review-branch remote SHA verified. Required hooks pass formatting, Clippy and **381 root/shared tests**; two personal-account diagnostics remain ignored. Strict docs and the client/native/browser evidence in the completion log pass. R77’s prompt push is fulfilled for this increment; the full goal and remaining TODOs stay active. Main, personal installation and disabled quality/release workflows remain unchanged.


### Captured selection prerequisites — R42/R67/R69/R73/R77

The native cache now captures complete query membership separately from loaded rows, and the Dart controller bounds pending gestures and row observations. Rust, controller and actual FFI tests cover scope/revision/alias/failure/cleanup contracts; see the completion log for execution and shipping. This checkpoint responds to the prompt-push request while keeping native/browser selection controls, exact bulk reviews and durable execution open. R75 Google provider sign-in, R76 grouped scheduled Automatic replies and the full parity/VPS backlog remain active.


### Captured selection prerequisite shipping record

Committed and pushed as [`646638d`](https://github.com/sam-ruff/shep.so/commit/646638d1e782eb938192ac83fa4e71243551a8bb) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 66 native Rust tests, 69 Flutter host tests, nine Android scenarios, 39 Python tests, 30 parity contracts, reviewed regression captures and production APK inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this increment. Full native/browser selection controls, durable bulk execution and the wider parity backlog remain active. Main and the personal desktop installation were untouched; artifacts remain ignored and the dedicated emulator is stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser captured-selection storage — R42/R67/R69/R71/R73/R77

The separate browser now has temporary SQLite worker captures, bounded queue/observations, exact frozen reviews, current cache metadata and a lazy production repository adapter. Real Chromium tests exposed and replaced a draft-blocking IndexedDB design; saved scenarios preserve the concurrency, rollback, migration, identity and tab-ownership contracts. The built worker also runs behind the Rust HTTPS beta gate. The completion log records verification and prompt review-branch shipping. The browser controller, native/browser controls, durable bulk execution and full client parity remain open; R75/R76 and deployment configuration remain tracked.


Committed and pushed as [`8295e3c`](https://github.com/sam-ruff/shep.so/commit/8295e3c40ff2a19cb211398f47e30fb9603c80d1) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 85 browser unit tests, 49 Playwright scenarios, 53 real Rust HTTPS stages, 39 Python tests, 30 parity contracts, reviewed layout regressions and production artifact inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this checkpoint. Native/browser selection controls, durable bulk execution and the full parity backlog remain active, including R75/R76 and missing VPS/owner/OAuth configuration. Main and the personal desktop installation were untouched; artifacts remain ignored and owned test servers are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser selection controls — R42/R67/R69/R71/R73/R77

Captured browser membership now reaches real Select/all/Clear/Done, checkbox, modifier/range and remappable focused-list controls. Controller and Chromium tests cover complete versus visible scope, unread/text isolation, pending navigation, arrivals, failures and retained intent. Dart shares the offscreen/range feedback corrections; its actual control replacement and both clients' durable bulk execution remain open. The completion log records tests, reviewed captures, corrected failures, production inspection and prompt review-branch shipping. Full parity, R75/R76 and missing deployment configuration remain traceable.


Committed and pushed as [`7f6d525`](https://github.com/sam-ruff/shep.so/commit/7f6d52573f04456a01a1a5d18618d1ae3bea483e) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 96 browser unit tests, 55 Playwright scenarios, 54 real Rust HTTPS stages, 71 Flutter host tests, 39 Python tests, 30 parity contracts, reviewed compact/large layouts and production artifact inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this checkpoint. Native control replacement and reviewed durable bulk execution/receipts/history/Undo remain active alongside full parity, R75/R76 and missing VPS/owner/OAuth configuration. Main and the personal desktop installation were untouched; artifacts remain ignored and owned test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser durable group prerequisite — R42/R67/R69/R71/R73/R77

The frozen browser selection now transfers exact metadata to a separate durable journal. Real Chromium tests cover 100,000-row staging with independent draft saving, partial/restart recovery, physical receipts/Undo, exclusive tabs, abrupt owner loss, atomic rollback and bounded history. The completion log records verification and prompt review-branch shipping. Provider execution, account lifecycle integration, individual coordination, native equivalent and actual group controls remain open; storage tests alone do not complete bulk parity. The full product goal, R75/R76 and deployment configuration remain tracked.


### Browser durable group prerequisite shipping record

Committed and pushed as [`bae6776`](https://github.com/sam-ruff/shep.so/commit/bae677639921759d866f1147b2d08e51c5ef7263) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 96 browser unit tests, 63 Playwright scenarios, 55 real Rust HTTPS stages, 39 Python tests, 30 parity contracts, reviewed layout regressions, corrected tab-close observation and production inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this increment. The journal is a storage prerequisite; provider execution, account cleanup/current-state coordination, native equivalent and visible review/History/Undo controls remain open alongside the full parity and deployment backlog. Main and the personal installation were untouched; artifacts remain ignored and owned browser test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Acknowledged browser mutations and cache recovery — R42/R50/R60/R65/R67/R69/R71/R73/R77

The browser separates provider receipts, cache completion and display refresh. Current individual controls retain committed state/counts and safe Undo after display-only failure, while the durable journal now tracks cache/identity reconciliation, migration and read-only inspection. The completion log records tests and prompt shipping. Full group execution, persistent individual coordination, account cleanup, native parity and visible group recovery controls remain open alongside the original product goal and deployment configuration.


### Acknowledged mutation and cache recovery shipping record

Committed and pushed as [`d2074db`](https://github.com/sam-ruff/shep.so/commit/d2074dba95a462cdc326135364477af9c4b6c2bf) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. The completion log records 101 browser unit tests, all 67 Playwright scenarios in a complete unchanged-source run, 55 real Rust HTTPS stages, 39 Python tests, 30 parity contracts, reviewed committed-action warning captures and production inspection. Strict documentation validation passes. R77's prompt push is fulfilled for this increment. Full group execution, persistent individual-intent coordination, account lifecycle integration, optimistic queries and native/visible group controls remain open alongside the original parity/deployment backlog. Main and the personal installation were untouched; artifacts remain ignored and owned browser test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Persistent browser intent — R42/R50/R60/R65/R67/R69/R71/R73/R77

Individual browser controls reserve field ownership before queued work, check it at dispatch, reconcile superseded input and retain acknowledged outcomes through action-record failures. Schema-six migration, alias transactions and account-removal review/cleanup preserve that ownership across tabs and missing mail. Saved unit and Chromium control/storage scenarios cover the increment; the completion log records final evidence and prompt review-branch shipping. Group claim/Undo primitives are prerequisites: the provider runner, visible group review/History/Undo, optimistic queries and native equivalents remain active TODOs. No request is closed by this checkpoint.


### Persistent browser intent shipping record

Committed and pushed as [`353d259`](https://github.com/sam-ruff/shep.so/commit/353d2597ecb57fb649adea54dd312d4a01b1b6ab) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Verification includes **107 browser unit tests**, **73 Playwright scenarios**, **55 real Rust HTTPS beta/mail stages**, **39 Python tests**, **30 parity contracts**, TypeScript, production browser inspection and pinned strict documentation validation. The reviewed compact removal capture and retained migration-assertion failure evidence are recorded above.

R77's prompt push is fulfilled for this increment. Group execution/recovery, bounded optimistic queries, native client parity and the wider product/deployment backlog remain active, including R75/R76 and missing VPS/owner/OAuth configuration. Main and the personal installation were untouched; artifacts remain ignored and owned test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser durable executor — R42/R50/R60/R65/R67/R69/R71/R73/R77

Exact frozen selections now connect to a single owned provider executor with approved/Undo field revisions, physical source checks, durable receipts, skipped outcomes, cache-only recovery, definite retry and conservative stop/restart. Atomic cache-applied revisions preserve newer actions across receipt recovery. The completion log records production-adapter Chromium evidence, corrected fixture setup and prompt checkpoint shipping. Application activation, optimistic query effects, group account cleanup, actual review/History/Undo controls and native parity remain active; this does not establish complete bulk or product parity.


Compatibility correction: mail schema 7 closes older database writers before the executor relies on atomic cache-applied revisions. The real upgrade scenario preserves prior clocks/statuses/roles, rejects older writes/reopening, and does not invent cache success from acknowledgment-only status. The final 87-scenario browser and 55-stage Rust HTTPS runs pass; combined shipping is recorded in the completion log.


### Browser durable executor shipping record

The executor [`3467937`](https://github.com/sam-ruff/shep.so/commit/346793738fda85502a8395c5578f86cc691c875a) and older-writer compatibility fix [`302bac2`](https://github.com/sam-ruff/shep.so/commit/302bac26e742c55883af5cf528c7f4fa8ec72d42) are pushed to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests** for each commit, with two personal-account diagnostics intentionally ignored. Final evidence includes **107 browser unit tests**, **87 Playwright scenarios**, **55 real Rust HTTPS beta/mail stages**, **39 Python tests**, **30 parity contracts**, TypeScript, production bundle inspection, reviewed layout regressions and pinned strict documentation validation.

R77's prompt push is fulfilled for this increment. The executor API is verified but not activated by application controls. Visible group review/History/Undo, optimistic query effects, group account cleanup, explicit ambiguous-result resolution and native equivalents remain active, alongside full client parity, R75/R76 and missing VPS/owner/OAuth configuration. Main and the personal installation were untouched; artifacts remain ignored and owned browser test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser group-aware account removal — R42/R50/R60/R67/R69/R71/R73/R77

Related group history and unfinished changes now join the actual Preferences removal review. The mail removal token, retained group ownership and bounded source/destination cleanup protect two-database failures and tab loss without repeating provider work or deleting unrelated receipts. Schema 8 fences older mail-removal writers; schema 4 retains account ownership and waits for an older receipt owner before upgrading. Ten saved real Chromium scenarios and reviewed light/dark controls verify this increment; complete regression and prompt shipping are recorded in the completion log. The global group revision conservatively invalidates a review when other group work changes. Actual group execution/review/History/Undo, optimistic queries, explicit ambiguity resolution, native parity and the original product/deployment requests remain active.


### Browser group account-removal shipping record

Committed and pushed as [`92452c3`](https://github.com/sam-ruff/shep.so/commit/92452c35523deb357ec4ee921fe887ee476487f2) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final verification includes **107 browser unit tests**, **97 Playwright scenarios**, **55 real Rust HTTPS stages**, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, production fixture/license/WASM inspection and pinned strict documentation validation. Reviewed light/dark removal and cleanup-warning captures are retained as ignored WebP artifacts.

R77's prompt push is fulfilled for this increment. Group-aware account review/cleanup and restart protection are verified; visible group execution/review/History/Undo, optimistic query effects, explicit ambiguity resolution and native equivalents remain active. R63 retains the intermittent Print click/reflow observation despite its passing focused and final full reruns. Full parity, R75/R76 and missing VPS/owner/OAuth configuration remain open. Main and the personal installation were untouched; owned test processes are stopped and the worktree contains no root log files. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Stable browser reader actions — R38/R41/R44/R50/R60/R63/R67/R69/R71/R73/R77

The intermittent Print observation is reproduced as asynchronous body/attachment layout movement. A fixed footer and continuously retained same-message action ancestry now preserve pointer position, held activation and keyboard focus. Native control Enter/Space no longer runs reader accelerators, and focused-row opening honors remapping/disable. Five new real-input scenarios join existing Print/reader regressions; the completion log records failed baselines, corrected fixtures, visual review, final checks and shipping. The wider native/list-input audit, bulk UI/optimistic queries and original parity/deployment goal remain active.


### Browser reader-action shipping record

Committed and pushed as [`cebcbe2`](https://github.com/sam-ruff/shep.so/commit/cebcbe24d990dbe7d889b71a4234f6395aa0e58a) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final verification includes **107 browser unit tests**, **102 Playwright scenarios**, **55 real Rust HTTPS stages**, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, reviewed light/dark footer and layout captures, production fixture/license/WASM inspection and pinned strict documentation validation. The reproducible failed baselines remain ignored and are described above.

R77's prompt push is fulfilled for this increment. The identified browser reader-action movement, cancelled press and native activation defects are fixed. Native footer/touch parity, the wider R63 input audit, browser/native group controls and optimistic queries, full client parity, R75/R76 and missing VPS/owner/OAuth configuration remain active. Main and the personal installation were untouched; owned test processes are stopped and root logs are absent. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### R67/R69/R63 — Flutter reader footer continuation

The browser loading/input fix is now being ported through Flutter controls: a responsive safe-area action footer, stable Forward/Print labels, shared host/Android held-touch and delayed-loading scenarios, and formatted Playwright/Appium plus actual Android PDF regressions. Compact text testing also exposed and fixes wrapping of the existing Inbox filter chips. Saved checks additionally preserve read-on-leave in Outbox, observe a held Forward without waiting for its busy animation, reject stale generated HTML fixtures on both preview platforms and enforce 44-pixel footer bounds under Flutter desktop density. The completion log retains failure evidence and records verification/shipping. Native keymaps, Apple, full group/query parity and the original feature/deployment goal remain active; R75/R76 remain requested TODOs.


### Flutter reader-action shipping record

Committed and pushed as [`48c6c18`](https://github.com/sam-ruff/shep.so/commit/48c6c18cf5e841c3e168720bbf2a01e69eb45ea0) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Verification includes **74 Flutter host tests**, the targeted **16 Android integration scenarios**, **12 Appium stages**, both Flutter browser previews (**16 scenarios**), **39 Python tests**, **30 parity contracts**, production APK isolation, final visual review and pinned strict documentation validation. The final density setting reruns eleven native scenarios and both browser paths; the preceding Android printer/attachment/Outbox/formatted evidence and test corrections are detailed above.

R77's prompt push is fulfilled for this increment. Flutter reader actions remain reachable through loading and scrolling, retain native held touches and keep 44-pixel targets. The full original goal remains active: native keymaps/lifecycle, group controls and bounded queries, remaining mail/calendar/Google/backup parity, Apple execution, distribution and VPS deployment are unfinished. R75/R76 remain tracked TODOs, and exact VPS/owner/OAuth configuration is still pending. Main and the personal installation were untouched; artifacts stay ignored and owned test processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### R42/R67/R73/R74 — Browser mailbox query prerequisite

The browser currently materializes all cached bodies on the main thread. A persistent worker query API is being verified before replacing that application path: per-profile OPFS/SQLite, source-incarnation and revision checks, readonly rebuild/replay, bounded metadata pages, separate bodies, exact current predicates and projected pending fields. The completion log records the initial large-cache deadline failure and subsequent verification. Application paging/reader integration, stale-result and Undo/count behavior, removed-account cleanup, independent body capacity, bulk projections and actual UI/HTTPS evidence remain active. This prerequisite does not establish browser performance or full feature parity. R75/R76 remain tracked TODOs and deployment configuration remains pending. R77 requires a prompt verified review-branch push.


### Browser mailbox query shipping record

Committed and pushed as [`3dd0837`](https://github.com/sam-ruff/shep.so/commit/3dd08372ad0ebcd9383a1611142906209948cea0) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Verification includes **107 browser unit tests**, the **108 Playwright scenarios covered by the 106/108 full run plus two corrected migration reruns**, the focused damaged-body regression, **55 real Rust HTTPS stages**, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, production fixture/license/WASM inspection and pinned strict documentation validation. The full-run failures and initial large-cache timeout remain recorded in the completion log and ignored artifacts; they are not described as a clean single full run.

R77's prompt push is fulfilled for this increment. Persistent query storage is a tested prerequisite; application list/reader activation, optimistic counts/rollback, independent body capacity during rebuilding, derived account-removal cleanup, bulk controls and full feature parity remain open. R75/R76 stay tracked TODOs; exact VPS/owner/OAuth configuration and Apple/distribution verification remain pending. Main and the personal installation were untouched. Root logs are absent, test artifacts stay ignored and owned browser/HTTPS processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser application paging — R42/R50/R60/R63/R67/R69/R71/R73/R74/R77

The pending integration connects the persistent query worker to the production browser Gateway/Workspace. Startup and list reloads no longer materialize every cached body. Coalesced queries return at most 50 metadata rows; two independent foreground readers, separate scan/speculative capacity and an eight-body/32 MiB cache preserve navigation and the active reader. Schema 10 adds account/folder/server-identity indexes without resetting schema 9's source incarnation or protected clocks. Sync scans return metadata pages; each flag-cache write retains one body.

Initial verification exposed a real input regression: body arrival detached a row Flag button during a held press. The saved scenario fails before the control-retention fix; its trace remains under ignored `artifacts/browser-paging-failures/initial/`. The same run's search test used a nonexistent searchbox role; its corrected target is the actual Search conversations textbox. Controlled unit checks also exposed a stale pending page overwriting immediate rollback after a synchronous reservation failure. Failed projections now retire before that result can publish. Shutdown cannot recreate read workers, removed-account projections are discarded, and evicted-metadata Undo reports missing source mail for review. Final regression and shipping evidence follow after validation.

The full product goal remains active. Bulk controls/projections, whole-client Outbox/draft/removal-snapshot bounds, provider known-ID/reconciliation metadata, physical derived-index cleanup retries, fuzzy relevance, large-message preparation, performance, Apple/live-provider execution and VPS deployment remain unfinished. R75/R76 remain tracked TODOs; exact VPS/owner/OAuth configuration is still pending. This browser-only change does not rerun native root/Flutter UI suites. Main and the personal installation remain untouched; quality/release definitions stay disabled.


The complete Chromium run passes **113/113 scenarios**, including both schema-eight/nine migration paths, 100,000-message cardinality/recovery, actual paging/selection/Undo, retained row/footer presses, account removal and reader layouts. Final review adds explicit Gateway shutdown and cache-incarnation guards: late callers cannot recreate workers, and old mutation results cannot paint reused IDs in a replacement cache. All **120 browser unit tests** pass; **31 affected production-control scenarios** pass again after those guards. The full 113-scenario run preceded those final guards; the unchanged storage/formatting/printing coverage is not described as a second full run.

TypeScript, pinned formatting, **39 Python tests**, **30 parity contracts** and pinned strict documentation validation pass. Reviewed light/dark/compact captures retain readable rows, reachable reader actions and explicit Retry. Production build inspection confirms the active query/read workers, required licenses, excluded preview fixture markers and unchanged recorded SQLite/shared-MIME WASM hashes. Real Rust HTTPS verification and required-hook shipping evidence follow.


The built application passes **55 real Rust HTTPS beta/mail stages** with the paged query path active. These use the production service and isolated scripted providers, not personal/live mail. Final shipping runs the mandatory formatting, Clippy and root/shared Rust hooks; the exact commit and verified remote branch are recorded next. Quality/release definitions remain disabled.


### Browser application paging shipping record

Committed and pushed as [`2cb6a46`](https://github.com/sam-ruff/shep.so/commit/2cb6a46404aaabf03995984e0a5abc602bac1703) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Required hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Verification includes **120 browser unit tests**, a **113/113 Playwright run** followed by **31 affected control reruns** after the final shutdown/incarnation guards, **55 real Rust HTTPS beta/mail stages**, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, production fixture/license/WASM inspection, reviewed light/dark/compact WebP evidence and pinned strict documentation validation. Earlier failed held-input and test-selector evidence remains recorded above and under ignored artifacts.

The production inbox now uses coalesced metadata pages with independent body reads, retained-reader/body budgets, current pending fields, explicit Retry and preserved counted Undo. R77's prompt push is fulfilled for this increment; the combined review branch also contains the prior Flutter, Rust beta backend and delegated promo website work. Full client parity remains active: bulk projection/review/History/Undo, whole-client Outbox/draft/removal bounds, physical derived-index cleanup retries, full search ranking, large-message preparation, Apple/live-provider execution, distribution and VPS deployment are unfinished. R75/R76 remain tracked TODOs; exact VPS/owner/OAuth configuration is still pending. Main and the personal installation were untouched. Test artifacts remain ignored, root logs are absent and owned browser/HTTPS processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


### Browser group controls — R42/R50/R60/R63/R67/R69/R71/R73/R74/R77

The browser now connects captured selection to frozen action reviews, approved full-membership execution, worker query/selection projections, bounded History, Pause/Resume, Undo and explicit failed/uncertain recovery. Mail schema 11 invalidates derived intent state; journal schema 5 tracks changed items and its own incarnation. Source-cache replacement guards protect dispatch and cache-only repair. Accepting an uncertain result retires only its unresolved local intent and never infers a provider outcome.

Targeted real controls verify mixed-account cancellation, all 125 messages, 50-item History pages, pending/acknowledged Undo, retained native presses through receipts, explicit failure/review and conservative startup recovery. The first control run exposed observational lock contention that stranded Undo; current-schema readers now observe without taking execution ownership. Its failure trace is retained under ignored `artifacts/browser-bulk-failures/controls-initial/`. A later recovery test used an incorrect appearance selector; the saved test now uses the actual Theme combobox. Both corrected scenarios pass. Shared projection checks cover off-page counts, newer individual choices and incremental journal transfer. Final regression, visual review and shipping follow.

Full parity remains active. Synchronous Undo rows/counts while local decision persistence is held, startup recovery notifications, abandoned-review cleanup, staging/alias/overlapping-group lifecycle, cross-account transport, large-group performance and native equivalents remain open. Broader mail/calendar/Google/backup parity, Apple/live-provider execution, distribution and VPS deployment remain open too. R75/R76 remain tracked TODOs; exact VPS/owner/OAuth configuration is pending. This browser-only increment does not rerun unchanged root/Flutter native UI suites. Main and the personal installation remain untouched; quality/release workflows stay disabled.


The initial full regression run passes 117/120 scenarios. Its failures expose a real concurrency regression and two outdated assertions: adding `removedAccounts` to the long projection snapshot blocked draft commits; the schema assertion expected 10 instead of 11; selection-summary assertions expected raw flags/folders instead of the newly shared optimistic values. Projections now derive valid owners from accounts in the same mail snapshot, leaving draft-fence storage outside the long read. Physical export remains separate from displayed values. Failed evidence stays under ignored `artifacts/browser-bulk-failures/full-initial/`; final reruns follow. History now identifies messages by subject/sender and original folder using a bounded metadata page.


The focused concurrency rerun passes all storage contracts, including draft saving during the 100,000-message capture. Its History test exposed repeated subject lookups slowing provider progress; History now retains one displayed metadata page until navigation or explicit Refresh. The same unchanged completion assertion passes with this fix; the timeout was not relaxed. That intermediate evidence remains under ignored `artifacts/browser-bulk-failures/history-metadata/`.


Final Chromium regression passes **120/120 scenarios**, and all **121 browser unit tests** pass. TypeScript, pinned formatting, **35 Rust backend tests** (the separate browser test is explicitly ignored here), backend Clippy, **39 Python tests**, **30 parity contracts** and pinned strict documentation validation pass. Reviewed light/dark/compact WebP captures show readable reviews, subject-labelled History and reachable recovery controls. The production build contains the query/read/selection workers and licenses, excludes fixture markers, and retains the recorded SQLite/shared-MIME WASM SHA-256 values. The public promo and protected app assets are staged for review, not deployed. Production Rust HTTPS and mandatory-hook shipping evidence follow.


The built application passes **56 real Rust HTTPS beta/mail stages**, including actual captured Flag/Undo controls and exact per-message transport calls. These use the production service with isolated identity/mail fixtures, not live personal providers. The promo build and separated site staging pass. Required formatting/Clippy/root Rust hooks run during the prompt review-branch commit; the verified commit and remote follow.


### Browser group controls shipping record

Committed and pushed as [`74082c7`](https://github.com/sam-ruff/shep.so/commit/74082c71458970f32dae1d7b252d23478bbffa5c) to [`feat/mobile-web-clients`](https://github.com/sam-ruff/shep.so/tree/feat/mobile-web-clients); the exact remote SHA was verified. Mandatory hooks pass formatting, Clippy and **381 root/shared Rust tests**, with two personal-account diagnostics intentionally ignored. Final verification passes **120/120 Chromium scenarios**, **121 browser unit tests**, **35 backend tests**, **56 real Rust HTTPS beta/mail stages**, backend Clippy, **39 Python tests**, **30 parity contracts**, TypeScript, pinned formatting, strict documentation, production fixture/license/WASM checks and promo build/site staging. Reviewed synthetic WebP evidence is retained in ignored `artifacts/browser-bulk-visuals/`. The initial failures and their fixes remain recorded above; no timeout or performance threshold was relaxed.

R77's prompt push is fulfilled for this increment. The combined review branch includes Flutter, the browser client, Rust beta backend and the delegated promo website. Full parity stays active: synchronous Undo before local persistence, startup/review cleanup, staging/alias/overlapping-group lifecycle, cross-account transport, native group controls, broader account/calendar/Google/backups, performance, Apple/live-provider execution, distribution and VPS deployment remain open. R75/R76 remain tracked TODOs. Exact VPS/owner/OAuth configuration is pending; assets are staged, not deployed. Main and the personal installation were untouched, the client worktree was clean after the code push, root logs are absent and owned browser/HTTPS processes are stopped. Quality/release workflows remain disabled; re-enable them when trusted runners and distribution prerequisites are ready.


R42/R50/R60/R67/R73/R77 continuation: browser reviewed groups retain proven identity continuity through acknowledged moves and validated canonical aliases; exact dispatch/Undo receipts remain separate from frozen metadata. Real successive-group and History-switch controls plus protocol/storage recovery and stale-source tests are recorded under “Browser group identity continuity” in [the completion log](COMPLETION.md). Prompt shipping targets the combined review branch. Synchronous Undo, native parity and the remaining product/deployment requirements stay active in TODO; this increment does not close the full goal.

The identity-continuity increment passes 125/125 Chromium scenarios, 121 unit tests and 56 real Rust HTTPS stages, plus Python/parity/documentation/build checks. R42's active entry is condensed to current delivered behavior and remaining requirements; its earlier prerequisite/checkpoint evidence remains in the completion log and this audit. No product request is removed. Commit hooks and the prompt R77 review-branch push follow.


R77 shipping is verified for the identity-continuity increment in [`396b806`](https://github.com/sam-ruff/shep.so/commit/396b806156e2fe91b74334b9d40a0b42c860ff88) on the combined review branch. Required hooks pass 381 root/shared Rust tests plus formatting/Clippy; final client evidence and unchanged limitations are recorded in the completion log. R42 and the full mobile/browser/deployment goal remain active. Main and the personal installation were not changed.


R42/R50/R60/R63/R67/R69/R71/R73/R74/R77 continuation: prepared browser group Undo paints rows/counts before local persistence, preserves immediate follow-up actions/newer reader flags, and reports rejection after History closes. Counterfactual queries retain actual dispatch folders without changing the durable source. History/query contention and restored-row source guards are covered by the saved regressions; failures, validation and prompt review-branch shipping are recorded under “Browser immediate group Undo” in [the completion log](COMPLETION.md). The complete mobile/browser/website/deployment goal remains active; native group controls and the broader lifecycle/parity gaps stay in TODO.


**R78 — Handover, TODO cleanup and push:** requested because credits are nearly exhausted. Added root `handover.md` and consolidated repetitive TODO history into remaining work, preserving all 40 active request entries. Full parity and the observed Find-remapping issue remain open; stop feature development after checkpoint shipping. See the completion log for exact verification and commit evidence.


R78 completed and R77 checkpoint shipping verified in [`d4da04c`](https://github.com/sam-ruff/shep.so/commit/d4da04c4ebcf39358f9227fb6190c822f8094a8c). Handover/TODO cleanup, 381 mandatory-hook Rust tests, 56 production HTTPS stages and qualified Chromium results are recorded in the completion log. Only R78 leaves the active TODO; all unfinished product requirements remain. Feature development is stopped at the user's credit-limit request.


2026-09-08: the user explicitly resumed the full client goal after R78 handover shipping. The stop instruction is superseded. Continue in the combined review worktree, beginning with the retained R63 shortcut-capture failure; all other parity and deployment requirements remain active.


R79 phone installation is fulfilled: the normal development-signed Android application was installed and launched without clearing data. The final phone screen observation was limited by the lock screen. Build identity and verification are recorded in [the completion log](COMPLETION.md), without pairing/device details. Documentation shipping follows. R63 resumes with deterministic failed-before/fixed-after browser shortcut capture and held-press controls; full keymap/lifecycle and product parity remain active.


R42/R63 continuation: the resumed full run exposed stale History progress while Undo preparation remained pending. Durable journal observations distinguish completed mail writes from the stale count. The implementation now refreshes History independently and preserves preview failure/retry controls; deterministic held observations/provider/preview checks retain the failed baseline. Full regression and shipping are recorded in the completion log.


R77 checkpoint shipping and R79 phone installation are verified in [`1ea12a6`](https://github.com/sam-ruff/shep.so/commit/1ea12a677829dcd71c4246b87cae46327f9c749f). The completion log records 133/133 Chromium scenarios, 123 browser units, 381 required-hook Rust tests and 56 production HTTPS fixture stages, alongside Python/parity/docs/build checks and reviewed visual evidence. R63’s reproduced shortcut-capture and stale-History defects are fixed; the wider coverage/parity goal remains active. R79 alone is removed from TODO, and the handover resumes with remaining group/native/lifecycle work. Main and the installed desktop are unchanged; site deployment still needs the recorded VPS/OAuth/owner configuration.


R42/R63/R67/R73 continuation: add browser startup recovery notices for saved failed/unconfirmed/interrupted/cache work, including groups older than the first History page. Indexed bounded observations, current execution boundaries and failed-inspection retention have unit/control evidence in the completion log. Observing a live owner cannot classify its running step as unconfirmed or repeat it. Native parity and all remaining product/deployment requests stay active.


R42/R63/R67/R69/R71/R73/R74/R77: browser startup saved-group recovery shipped in [`df4f29c`](https://github.com/sam-ruff/shep.so/commit/df4f29c2e12d21fc71353920696dd4958cde363a), with exact remote verification, 137 Chromium / 127 browser unit / 56 production HTTPS stages and mandatory hooks (381 Rust tests). Compact/expanded synthetic captures are reviewed. Broader group, native, provider and deployment gaps remain in TODO; the handover now resumes with highest-priority OAuth/shared profiles.


desktop-main:R92/R75/R02/R49/R67/R73/R77 continuation: shared profile metadata codec/account mappings and native/WASM fixtures are in verification. The implemented subset is documented in `docs/agents/PROFILE_FORMAT.md`; metadata validation does not establish OAuth, enrollment, conflict resolution, password transfer or database migration. Keep the highest-priority request and all its remaining lifecycle/platform work active.


desktop-main:R92/R75/R02/R49/R67/R73/R77: shared profile metadata prerequisite shipped in [`80979d2`](https://github.com/sam-ruff/shep.so/commit/80979d26db8d44e2f9caaed4f02e535807f9b826); exact remote SHA verified. Root/shared hooks pass 390 tests, mobile Rust 67, actual Dart FFI 14 and standalone WASM 23 common fixtures plus malformed-record rejection. Analysis/docs/release-stamping checks pass. OAuth/enrollment/merge, remaining settings/credentials, live platforms and database migration remain active; no request is removed on this foundation alone.


desktop-main:R92/R75/R02/R49/R63/R67/R73/R77 continuation: desktop explicit next-sign-in service choices and scope-bound candidate/activation semantics are under native verification. Existing tokens stay usable while choices change; mobile/browser consent and the broader OAuth/profile lifecycle remain active parity gaps.


desktop-main:R92/R75/R02/R49/R63/R67/R73/R77: desktop feature-scoped consent shipped as [`3e1181b`](https://github.com/sam-ruff/shep.so/commit/3e1181ba68692b63104cec4f926f3391ecfab470); exact remote SHA verified. Mandatory hooks pass 398 Rust tests and all 118 native functional scenarios pass, with reviewed light/dark/compact controls. The full OAuth/profile request remains active for mobile/browser consent, verified cross-client identity, enrollment/merge, remaining settings and protected credentials.


R75/R02/R49/desktop-main:R92/R63/R67/R69/R73 continuation: native Flutter Google SDK sign-in and Preferences permissions are being verified. Existing committed access survives changed choices, denial and failed metadata saves. Local disconnection persists before retryable SDK cleanup; SDK signOut does not revoke other devices. Seamless switching, automatic session restore, actual Calendar/Drive/profile integration and live/Apple verification remain open.

R75/R63 continuation: the native-only SDK package selection prevents unsolicited Google web SDK loading in previews; Playwright now rejects external requests. Unconfirmed device writes/readbacks pause Google operations until explicit reconciliation instead of claiming rollback. These changes are included in final verification; no scope item is removed.


R75/R02/R49/desktop-main:R92/R63/R67/R69/R73/R77: Flutter native consent now passes 89 host tests, the configured SDK fixture, two Android Google scenarios and seven Appium/offline Playwright flows. Production ARM64 packaging, analysis, Python/parity and strict documentation pass; reviewed captures and retained intermediate failures are recorded in the completion log. Full OAuth/provider/profile parity and live/Apple verification remain active; checkpoint shipping follows.


Flutter scoped consent is shipped in [`6c4bb65`](https://github.com/sam-ruff/shep.so/commit/6c4bb65fbc02c96dd258ed9942e64e6282bca181), with exact remote verification and mandatory hooks passing 398 root/shared Rust tests. Client/platform test counts, reviewed evidence and limitations are in [the completion log](COMPLETION.md). R77 prompt shipping is fulfilled for this increment; full R75/R02/R49 and client/platform parity stay in TODO.

R75/R02/R49/desktop-main:R92/R67/R69/R73/R77 continuation: the shared native causal
history and Flutter bridge now retain immutable metadata, offline conflicts,
removal tombstones and reserved upload identities. Core/native tests and the same
actual FFI scenario on host/Android pass without credentials; evidence, failures
and final checkpoint shipping are in [the completion log](COMPLETION.md). No scope
item is removed: authenticated discovery/enrollment/category controls, real
account/preferences application, credentials, browser history and Apple/live
verification remain active. The R75 Google replacement and R76 grouped scheduled
automatic-replies TODOs are retained, as are deployment and Linux store work.

R03/R09/R18/R44/R42/R67/R73 continuation: storage verification exposed repeated
literal FTS work in the desktop relevance query. The query now computes that
relation once while retaining exact-body priority and the common selection order.
The new planner guard and existing search/selection/bulk contracts pass; native
controls, measured budgets and shipping follow in the completion log. Mobile and
browser fuzzy-ranking parity stay open. No benchmark threshold is changed.

R03/R09/desktop-main:R70 follow-up: the first complete materialized-search benchmark
failed at 68.94 ms. A covering unread-account index removes per-page mail-row
lookups/sorting; reopen/projection and native badge verification accompany the
final performance gate. Full timing evidence and shipping remain explicit in the
completion log; no threshold or scope entry was removed.


R75/R02/R49/desktop-main:R92/R03/R09/R18/R44/R42/R67/R69/R73/R77 checkpoint:
`e9115f9` adds native causal history; `e568c84` and `da6f2e8` fix repeated FTS work
and per-page unread-count lookups. Final root/native/Flutter/Android checks and
native functional controls pass as scoped in the completion log. Storage budgets
pass; the combined performance gate still fails navigation at 154.81–162.33 ms,
with failures also on the earlier cached test build. R03/R09 and R63 remain active,
as do enrollment, real provider/application work, credentials and client/platform
parity. Shipping does not complete those requests.

R77 prompt checkpoint shipping: all three code commits are pushed, with the exact
remote `da6f2e8` head verified. The handover preserves next steps and the failing
native timing gate; this is review-branch progress, not full product completion.


R75/R02/R49/desktop-main:R92/R67/R69/R73/R77 continuation: the optional shared native Drive transport verifies the provider principal, owned bounded operation files and exact uploaded bytes against the durable journal. Scripted HTTP tests cover pagination failure, immutable reservations, uncertainty/restart and cancellation; a common wire fixture freezes the file contract. Client authorization/Settings integration, durable discovery/catalog, enrollment/category controls and real account/preferences application remain active. This is provider infrastructure, not live Google or complete profile sync. Final checks and prompt review-branch shipping belong in the completion log.

R77/R75/R02/R49/desktop-main:R92 shipping: provider code `557f8d5d1dd01ed9d2eee2decbd2a5235f036cac` and fixture portability `9289f5327b71bb6aaff463ee965eab48c13df85a` are pushed and the exact remote head is verified. Mandatory hooks pass 428 root/shared Rust tests; compatibility/strict docs and real Git newline conversion pass. See the completion log for scope. Keep production client integration, durable discovery/enrollment, credential policy and full parity active.


R75/R02/R49/desktop-main:R92/R67/R69/R73/R77 continuation: the optional shared
catalog now preserves staged discovery and file receipts, replays arrivals and
retains explicit missing/conflicting history across retry/rescan. Remote journals
cannot replace enrolled offline edits. Host protocol/storage and compatibility
checks are recorded in the completion log. Platform grants, initialized creation,
reviewed enrollment, own-upload identity integration, actual account/preferences
application, protected credentials and all client/platform parity remain active.
Prompt checkpoint shipping is recorded with the final code commit below.


R77/R75/R02/R49/desktop-main:R92 shipping: discovery code [`3c9b98d`](https://github.com/sam-ruff/shep.so/commit/3c9b98d514bf667064f5cd92a22d4dda84998de7) is pushed on the
review branch with exact remote verification. Mandatory hooks pass 443 Rust tests;
49 core, 68 mobile Rust, 23 WASM, 41 Python, 34 parity contracts, strict docs and
the unchanged storage benchmark pass as scoped in the completion log. Creation,
enrollment, real client application and the prior native timing failure remain
active; no main merge, personal installation or deployment occurred.


R75/R02/R49/desktop-main:R92/R67/R69/R73/R77 continuation: Flutter now binds saved
Google grants to the shared native catalog and offers actual Preferences discovery,
retry/rescan, pause, bounded pagination and disconnect recovery. Verified identity
must commit before profile content appears, and stale grant results cannot update
a replacement session. Final host/native/Android/Appium/Playwright evidence and
retained failures are in the latest completion entry. TODO/handover cleanup removes
repeated checkpoint prose only; creation/enrollment, account/preferences application,
credential policy, all original parity requests and the prior timing failure remain
active. Prompt review-branch shipping is recorded with the verified commit below.


R77/R75/R02/R49/desktop-main:R92 shipping: Flutter discovery code [`438682e`](https://github.com/sam-ruff/shep.so/commit/438682e277c93832a95168034b9940afe8de0cc0) is pushed
on the review branch with exact remote verification. Mandatory hooks pass 443
root/shared tests, and the scoped native/Flutter/Android/Appium/Playwright checks
are recorded in the completion log. All 40 active request entries remain in TODO;
this is progress toward profile sync, not completed enrollment or full parity.

R75/R02/R49/desktop-main:R92/R67/R69/R73/R77 continuation: Flutter now prepares and
publishes initialized profiles from a frozen paged review, preserves exact upload
requests/owned receipts and supports pause/retry without blocking mail. Shared,
native, host, Android, Appium and Flutter Playwright evidence and retained failures
are in the latest completion entry. This advances first setup; enrollment, actual
account/preferences application, credentials and ongoing sync remain active. All
40 TODO entries are retained; final review-branch shipping is recorded below.


R77/R75/R02/R49/desktop-main:R92 shipping: Flutter publication code [`184b98a`](https://github.com/sam-ruff/shep.so/commit/184b98afafcf53bc3fd7c32a497fad04746304bf) is
pushed on the review branch with exact remote verification. Mandatory hooks pass
446 root/shared Rust tests; the scoped native/Flutter/Android/Appium/Playwright,
production ARM64 and documentation evidence is in completion. All 40 active
requests remain; enrollment, real client application and full parity continue.


R75/R02/R49/desktop-main:R92/R63/R67/R69/R73/R77 continuation: reviewed Flutter
enrollment now applies credentialless accounts and eight preferences using original
history records, independent device journals, per-account receipts and protected
local preference revisions. Saved native/browser controls cover paging, details,
retry, Reconnect and independent browsing; final results and shipping belong in
[the completion log](COMPLETION.md). All 40 active requests remain. Full continuous
sync, authenticated cross-client interchange, protected credentials and remaining
platform/product parity are still required; R76 automatic replies remains a TODO.


R77/R75/R02/R49/desktop-main:R92 shipping: reviewed Flutter enrollment code
[`9d6a6c6`](https://github.com/sam-ruff/shep.so/commit/9d6a6c6df644d340c1192ba13215c298ce8ac8b0) is pushed with exact remote verification. Mandatory hooks pass 448
root/shared Rust tests; mobile, Android, browser, APK and visual evidence is in
[the completion log](COMPLETION.md). R77 prompt shipping is fulfilled for this
increment. New timing is deferred on the saturated host; the earlier performance
failure, continuous synchronization and full platform/product parity remain active.


R75/R02/R49/desktop-main:R92/R63/R73/R77 continuation: desktop Preferences now
connects the active saved Google grant to durable read-only profile discovery.
Grant/request fences, 51-profile protocol recovery and actual iced pause/browse,
retry/reopen/rescan, compact dark recovery and permission refusal have evidence;
all 121 native functional flows pass. See [the contract](agents/PROFILE_DESKTOP.md)
and the completion log. Desktop publication/enrollment/application, native
large-page controls, continuous sync and all previously recorded parity gaps stay
active. The corrected Flutter enrollment statuses preserve remaining lifecycle,
authenticated interchange and credential gaps. Shipping is recorded separately.


R08/R10/R75/R02/R49/desktop-main:R92 shipping: desktop discovery [`9e666a5`](https://github.com/sam-ruff/shep.so/commit/9e666a55582746fa59a1849ecc2d870a4c9d4b3c) is pushed
and remotely verified. Mandatory hooks pass 454 Rust tests (two personal live
checks ignored), alongside the recorded 121 native flows, 41 Python checks,
37 parity contracts and strict docs build. All 40 active requests remain.


R75/R02/R49/desktop-main:R92/R63/R73/R08/R10 continuation: desktop initial profile
publication now connects actual saved account/preferences reviews to durable
staging and tracked Drive receipts. Changed reviews, saved-preference ordering,
75-account paging, no-duplicate upload recovery after database reopen and real iced
review/pause/retry/compact controls have evidence. Full regression and shipping
are recorded in [completion](COMPLETION.md); desktop enrollment, continuous sync,
credential protection and all other active requests remain unfinished. No request
is removed by this increment.


R08/R10/R75/R02/R49/desktop-main:R92 shipping: desktop publication [`35f11ba`](https://github.com/sam-ruff/shep.so/commit/35f11ba0627621624659455dfebf6f341ea18893)
is pushed with exact remote verification. Normal hooks pass 459 Rust tests
(two personal live checks ignored); 123 native flows, 41 Python checks,
37 parity contracts, production compilation and strict docs also pass.
All 40 active requests remain. Next is reviewed desktop enrollment/application,
then continuous reconciliation and the remaining product/platform scope.


R02/R49/R75/desktop-main:R92 continuation: desktop enrollment now applies reviewed
account metadata and eight selected preferences through the real iced controls.
Independent history, paged choices, exact application receipts, reconnect guards,
newer local intent and guarded backup/restore behavior have targeted
store/protocol/controller/native evidence. R08/R10 validation and shipping status
are recorded in the completion log. All 40 requests remain active; this increment
does not deliver continuous sync, protected credential transfer, browser
application, automatic first setup or live cross-client verification.

Desktop enrollment final validation: 470 Rust tests (two opt-in personal live
checks ignored), 125 native functional flows, 41 Python checks, 37 parity
contracts, production compilation, Clippy and strict docs pass. Reviewed final
compact/reopen captures and the shipping receipt are in the completion log.

R02/R49/R75/desktop-main:R92 shipping: desktop enrollment [`8f969cd`](https://github.com/sam-ruff/shep.so/commit/8f969cd91d45ac2e4a821c927c360a86e64569cd)
is pushed and remotely verified. Normal hooks pass 470 Rust tests (two personal
live checks ignored), alongside 125 native flows, 41 Python checks, 37 parity
contracts, production compilation and strict docs. All 40 active requests remain.


R02/R49/R75/desktop-main:R92/R63/R73 continuation: desktop enrollment choices and
recoverable failures retain the current review page. Native large-list controls
cover 51 profiles and 75 accounts, second-page exclusion, guarded application,
connection details and publication cancellation. R63's badge check now uses the
current unread count after read-on-leave, preserving actual private-bus controls.
Final checks and shipping are in [completion](COMPLETION.md). No active request
is removed; continuous reconciliation and the other parity gaps remain open.


R77/R63/R02/R49/R75/desktop-main:R92 shipping: [`0b0ffcb`](https://github.com/sam-ruff/shep.so/commit/0b0ffcbf4bdb4e6501cf3d098d7691d4c9eef497) is pushed with exact remote
verification. Mandatory hooks pass 471 Rust tests; all 128 native functional
scenarios pass across the full run and corrected badge rerun. Python, parity,
production compilation and strict docs pass. The completion log retains the
original badge failure and test correction. No active request is removed.


2026-09-09 ongoing-profile continuation (desktop-main:R92 / R75 / R02 / R49):
the desktop edit ledger and bounded reconciliation runner now preserve exact
requests, atomic remote application, independent device histories, concurrent
values, scoped copy cursors and tracked upload recovery. Twelve new isolated
regressions cover those contracts; the full profile-focused run passes 41 checks.
Automatic scheduling, reviewed subscription creation, sync/conflict controls,
account/category reconciliation and Flutter/browser equivalents remain open.
Shipping and subsequent checks belong to the completion entry; all 40 active
requests remain in TODO.

The engine checkpoint [`ecd98c5`](https://github.com/sam-ruff/shep.so/commit/ecd98c59254d59b270ec0ef521aa6e70fff29bd6) is pushed and remotely verified. Normal hooks pass
483 Rust tests; 10 native profile regressions, 42 Python checks, 37 parity
contracts, production compilation and strict docs pass. Partial catalog-loss and
missing-remote-ancestry recovery need additional coverage before automatic sync
is exposed; the existing full observation-directory rebuild test retains all
remote originals. Full scheduling/control/client integration remains open.


2026-09-09 connected ongoing-profile continuation (R02/R49/R75/desktop-main:R92,
R63/R73): completed desktop reviews now create paused sync choices; explicit
master/field controls drive an authenticated background owner outside Preferences.
Recovery covers partial inventory loss and a changed Google project. Native
controls cover background application, local edits, pause/resume, lost-upload
retry and published seven-field choices. The completion entry records final
checks/shipping. Conflict resolution, remaining categories/settings/accounts and
Flutter/browser equivalents stay open. No active request is removed.


Connected-control shipping: [`13e056d`](https://github.com/sam-ruff/shep.so/commit/13e056dcc836fbf41e418de220085540fe174dbc) is pushed and remotely verified.
Mandatory hooks pass 490 Rust tests; 22 native regressions and both final sync
control flows pass, alongside 16 Flutter Rust bridge checks, 43 Python checks,
37 parity contracts, production compilation and strict docs. The completion log
retains failed-before evidence and reviewed captures. All 40 requests stay active.
