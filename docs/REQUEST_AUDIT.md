# Conversation request audit

R02/R49 follow-up: verified immutable profile records now survive polling and
restart without repeated downloads. Current complete discovery remains required.
Source `e77eabc` is pushed with 729 integrated hook executions and nine native
flows passing. Remaining change-token polling/review work stays in TODO.

9 September continuous profile checkpoint (R02/R49/R92): background receipt, local
publication and native field-generation reconciliation now have source/tests.
Source `bd50c52` is pushed; remaining interoperability/review work and the native
picker readiness limitation are recorded in the completion log and TODO. This is
not full OAuth/profile completion. R87 is delivered in integrated agent commit
`d29ce06`, with direction, pacing and native-control evidence reviewed.

9 September parallel delivery request (R93): independent agents now own isolated
worktrees, with the primary agent responsible for tested integration into main.
The full TODO goal remains active; parallelism does not waive native verification
or make partially implemented features complete.

9 September continuation: R02/R49/R92 connects after-login discovery, a persistent
opt-out, optional first-device setup and single-profile automatic import into an
untouched workspace. Multiple profiles and populated workspaces retain review;
continuous updates and protected credentials remain open. Source `acb4969` is
pushed with 30 native scenarios and 702 hook test executions passing; see the
newest completion entry.

9 September continuation: R02/R49/R92 adopts the published shared initialization
barrier through `43cdcf0f`, including native incomplete/legacy import guards,
multi-record creation/retry, safe unstarted-seed upgrade and portable Tooltips.
Source `9158b50` is pushed with 17 native scenarios and 698 mandatory hook test
executions passing. See the newest completion entry; continuous changes, automatic login enrollment and
admitted legacy recovery remain in TODO.

9 September continuation: R02/R49/R92 adds durable common values at native
enrollment, exact local change capture and verified history acknowledgments.
The upcoming continuous loop remains unfinished. The current Flutter initialization
barrier (`184b98a`) also needs adoption; desktop remains pinned to `33d222d7`.
Source `5c0e9f0` is pushed with 14 affected native scenarios and 692 mandatory
hook test executions passing; see the newest completion entry. OAuth and the shared handover remain first in TODO.

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
| R30 | Folder right-click delete/move into other folders; nested collapsible groups default collapsed | Trees delivered in 6d520e9: server delimiters/selectability, decoded labels, saved expansion, keyboard reveal and drag-hover expansion, with protocol/cache/native evidence. Backend checkpoint 5eabb52 is installed/pushed with tested mutation plans, checked provider commands and durable cache/recovery; native checkpoint 3567de2 is installed/pushed: reviewed delete/move controls, optimistic projection, local POP3 and bounded recovery/close, with seven saved native flows and final Rust/native/release evidence. Combined scopes, aggregate account choice and wider uncertainty/history lifecycle verification remain open |
| R31 | Inbox item right-click menu | Delivered baseline; reported immediate-dismiss regression now R54 |
| R32 | Multiple backup options simultaneously; good setup UX; Google Drive/S3/FTP/SFTP/local; compression and passcode encryption; restore password; rolling unreadable copies | Multiple Local/Drive checkpoint `3f00292` is being integrated, with migration, independent schedules/retention/passphrases, reviewed removal and three native scenarios verified. S3/FTP/FTPS/SFTP, optional formats, combined manual backup and final integration remain TODO; see completion. |
| R33 | Separate Contacts Preferences section | Delivered |
| R34 | Save-success toast and slightly depressed button state | Delivered, with acknowledgment-based toast and failure preservation |
| R35 | Compose in preview pane, autosave while typing, read other mail and work on several replies/drafts | Delivered e16590c: inline reply/new/forward editors, parked drafts, restart-safe recipients/files, reply quotes, normal text input, pending-save/removal guards, Find and conversation paging. Verification/shipping evidence is in the completion log; provider rekey/recovery remains R73 |
| R36 | Collapsible Drafts group, right-click delete, discard bin in draft editor; red discard confirmation button | Delivered 5aab267; collapsed preference, context/bin review, red confirmation, retirement/rollback/outgoing tests and native flows |
| R37 | Shortcut hints for buttons | Superseded by R51/R52: icon-only tooltip, primary key only and toggles |
| R38 | Render HTML like supplied examples, including the later raw-XHTML tags; preserve layout/images and selectable text | Delivered 1968e37 / 32120b4: MIME/XHTML selection, worker-rendered static HTML, native text copy, plain mode, quotes, inline/remote images, wide tables and focused-reader keyboard scrolling; 270 Rust, 20 Python and all 81 native functional scenarios pass; installed optimized build |
| R39 | Delete shortcut and optional second binding; final defaults Ctrl+D → Trash, Backspace + Delete → Archive; all remappable | Delivered in 590ab10: versioned slots/migration, conflict and native tests; older Delete-to-Trash default superseded |
| R40 | Clicking Mail while already open from another folder returns to unified/first Inbox | Delivered in 590ab10, native unified/first-account Inbox tests |
| R41 | Add Forward and Print controls in preview | Forward delivered in 9062dcf; Print delivered in b120721: complete cached MIME in Formatted/Plain mode, headers/CID images, background preparation, remappable Mod+P and browser printer/PDF selection; 289 Rust, 24 Python and 95 native functional scenarios verified (one startup-layout test rerun) |
| R42 | Ctrl+A list selection, Ctrl-click, Shift-click, checkbox Select mode beside search, bulk toolbar/keybind actions and Y/N/Enter/Esc confirmations | Delivered in b352d12, building on 2444049: native multi-selection, frozen reviews, group toolbar/keybind actions, immediate feedback, partial results and Undo; History pagination, Continue, retry and explicit uncertainty review. Real fixture process close/crash/restart and empty Inbox coverage pass. All 127 native functional scenarios, 362 Rust tests and 32 Python tests pass; optimized Linux release installed and pushed. Broader individual/group ordering and provider ambiguity remain R50/R60; independent-process coordination remains R01/R06 |
| R43 | Inbox (unread count) in sidebar | Delivered in 590ab10; cache counts ignore query/filter scope, with storage/native tests |
| R44 | Ctrl+F within email; fast search; fuzzy matching library and exact body “test” ranked first | Delivered d3a530a / c266035: library-based relevance search plus remappable Ctrl+F in formatted/plain message bodies, literal Unicode/whitespace matching, case toggle, highlighted next/previous, visible quote scope, wide-table reveal and native keyboard isolation; 277 Rust, 20 Python and all 86 native functional scenarios pass |
| R45 | Drag messages/selection from list into sidebar folders | Delivered in ca39080: single/group drops, destination outlines, hover expansion, sidebar scrolling, account rules, review/Undo, cancellation and shadow cleanup; ten saved drag scenarios and all 137 native functional flows pass; Linux release installed |
| R46 | Highlight Move target used by Enter | Delivered in 590ab10; highlighted Inbox/Enter target and native visual evidence |
| R47 | Cannot move out of A. Keep into Inbox; display Inbox rather than INBOX | In progress; local metadata confirms folders exist, native return-move and wire/logout tests exist; actual reported personal-account cause not confirmed |
| R48 | I goes to Inbox only with sidebar focus; remappable and disableable | Delivered in 590ab10, including sidebar/list focus and native disable/remap tests |
| R49 | Drive appDataFolder continuously syncs accounts and as many settings as possible; first-time offer/toggle; existing cloud setup automatically loads on another PC | In progress. First-device creation/recovery and reviewed existing-profile import are shipped (`071c6b0`), with shared-catalog discovery, safe local account IDs/reconnection and selected preference application; see COMPLETION. Automatic login prompts, linking existing workspaces, continuous changes, remaining portable settings and live interoperability remain. Credential-protection preference question pending. |
| R50 | Flagging immediately reflects UI intent before database/network save; apply same treatment elsewhere appropriate | Flags/read/same-account moves delivered 742b21e; cross-account source feedback and typed results delivered 9c907d2; 90776fa adds observed pending identities and global unread count reconciliation across page scopes; filtered destination rows, ambiguous outcomes, other controls and durable recovery remain open |
| R51 | Remove newly added tooltips from text-labeled controls; tooltips only on icons | Delivered in 590ab10, labeled controls unwrapped and native visual checks |
| R52 | Tooltip shows primary shortcut only; disable all tooltips or keyboard hints independently; searchable Preferences | Delivered in 590ab10; both tooltip toggles, primary-only hints, settings index/direct section navigation and native light/dark/compact tests |
| R53 | Sync mail becomes refresh icon at top right | Delivered in 590ab10; mouse sync/busy/navigation native tests |
| R54 | Right-click menu immediately disappears; fix and add tests | Delivered in 590ab10: background Changed no longer dismisses the menu; target refresh, mouse release, sync completion, action and dismissal tests |
| R55 | Audit whole conversation; maintain TODO.md immediately for every request; update AGENTS and remove items only when complete | Delivered: TODO.md and full audit plus immediate-tracking/removal rules in AGENTS.md; ongoing maintenance required |
| R56 | No root log files; delete accidental ones | Ongoing requirement; all current agent logs use ignored artifacts/logs |
| R57 | Preload messages, adjacent emails and next pages; WebP for image loading | Delivered baseline; maintain while large-mail/HTML work proceeds |
| R58 | Additional useful features required | Existing sender actions, outgoing recovery, conversations, connection removal and calendar discovery and forwarding and printing delivered; remaining selection refinements/settings sync remain explicit open requests |
| R60 | Immediate optimistic archive/move and app-wide reversible-action feedback; persist principle in AGENTS.md | Principle recorded; immediate flags/read/moves delivered in 742b21e and 9c907d2, with remaining reconciliation/recovery under R50/R60 |
| R61 | Raw GitHub installers for Linux/macOS/Windows, user-local default, optional system install and app menus; first install commands in README/docs | Recorded, open |
| R62 | Fix nonworking read/unread and add integration coverage for basic mail behavior | Delivered 742b21e; IMAP NO detection, selective flags, dispatcher/cache/reopen/native coverage |
| R64 | Transparent GNOME desktop icon matching system theme | Recorded, open |
| R63 | Fix shortcut × and extend native E2E coverage across functionality paths | Clear controls delivered 742b21e; native search-focus isolation and wrong-row clear regression delivered 9c907d2. Native key/click ordering and event-time field focus delivered b451777, installed/pushed: three native-widget tests, saved rapid-key baseline failure and passing native regressions. All 174 native scenarios have passing coverage across the 172/174 full run and final targeted reruns; detailed limitations are in COMPLETION. Final functionality coverage audit remains open. |
| R65 | Frequent background sync (user prioritizes rapid arrival), immediate startup check and independent manual Refresh | Delivered d4ecb21; 15-second default, saved seconds interval, independent coalescing refresh, virtual-time and native arrival/retry tests |
| R66 | Record Dungeonwalk vectoriser/remove.bg credential discovery in AGENTS.md | Delivered 742b21e; discovery pointer only, no keys copied |
| R67 | Select an inbox message, then click away to count it as read | Delivered 9c907d2: deliberate selection, immediate read-on-leave, explicit-unread protection, rollback and native navigation tests |
| R68 | Refreshing counted toast with Undo for archive, delete and move | Delivered 9c907d2 / 551f86c: immediate counted feedback and grouped Undo before/after acknowledgment, original-account/folder restoration, verified server identities and persistent failed-reversal retry; 81 native functional flows pass, including immediate toasts and Undo while mail saves remain pending |
| R59 | Investigate and fix the newly failed CI build | Delivered eea1dfb; strict local build and GitHub run 34026001754 passed |
| R69 | HTML layout moves during rendering, visual artifacts and slow readiness; investigate more pre-caching/rendering | Delivered across cd8f732, 6d83b72 and cc38af9: actual viewport loading, stale-frame isolation, retained fonts, bounded adjacent-frame preparation, stable controls, compact attachments/Find, late-image text anchoring and native Retry. All 104 native functional scenarios and 305 Rust/27 Python tests pass; release installed and pushed. Final idle-host timing remains R03/R09 |
| R70 | New-email count badges on the dock/taskbar launcher, as in the supplied GNOME screenshot | Linux shipped `90776fa`; Windows/macOS adapters integrated/pushed `1595fb3` with 751 hook executions and 12 native Linux scenarios, merged Windows and exact macOS adapter checking. Actual Windows/macOS runtime and remaining count reconciliation stay open; see completion. |
| R71 | Refresh icon looks malformed in the latest screenshot | Delivered for native Shep in cd8f732: shared Mail/Calendar SVG arrowheads corrected; normal/120% scale, light/dark and compact visual/native evidence; the browser crop was not separately reproduced |
| R72 | Highest priority: measure and greatly improve selection-to-visible HTML latency | Delivered ebddf54: reproduced the original 1.5–2.1 s worker delay; the same messages now take 47–63 ms. Corrected table reuse preserves pixels/height against the uncached corrected renderer. Eight native pixel cases pass (20 samples each; 100/50 ms gates), with 36 final native regressions and 427 Rust tests. Installed/pushed; detailed evidence and limits in COMPLETION/PERFORMANCE |
| R73 | Moved messages do not appear in the destination folder | Partial delivery in acb33c0 and a81d767, installed/pushed: immediate destination membership/cached reading, durable original-MIME/copy receipts, bounded lookup, explicit recovery/review/local-copy controls, failure/retry/restart/navigation/close coverage. All 167 native flows, 484 Rust tests plus two drawing-adapter tests and release/hooks pass. Actual adapter wire/journal, broader Undo/history/alias integration and the personal-account report remain open alongside R47/R50. |
| R74 | F5 default secondary Refresh; animate only explicitly requested refreshes | Delivered in 80f5867, installed/pushed with 2b4c480 test synchronization: conflict/clear-safe F5 migration, manual-only animation, SVG transform/clip repair, 492 Rust plus two adapter tests, and passing coverage for all 171 native scenarios across the full run and corrected rerun. Background checks remain still. |
| R75 | Mail returns to Inbox but leaves the previous folder highlighted | Delivered 3f24823: Mail clears the previous outline, focuses the list and retargets subsequent sidebar navigation to Inbox; the sidebar Inbox accelerator retains sidebar focus. Unit/native unified/per-account coverage and reviewed light/dark captures; installed and pushed |
| R76 | Match preview background to the email so white messages do not float in a dark surround | Delivered in ebddf54 and 1bf6ac9, installed/pushed: standalone and expanded conversation surfaces follow the document background, with readable controls, cached switching, refresh/scroll and light/dark/compact pixel coverage. |
| R77 | Reply editor leaves overlapping text at its bottom edge while typing | Clipping correction delivered ebddf54: direct renderer regression fails before/passes after, native long-reply typing is clean in compact light/dark. e16590c integrates the inline composer; compact light/dark native typing/repaint checks pass and screenshots were reviewed. Preserve these regressions |
| R78 | Scrolling the preview with other thread messages snaps back to the top | Delivered ebddf54: ordinary conversation refreshes preserve manual scroll; controller and saved native sync/scroll regression pass. Installed and pushed |
| R79 | Use a square selection icon for Select and adjacent copy icons in sender details | Delivered ebddf54: square list selection icon and adjacent sender copy icons, generous targets and configurable icon tooltips; native toggling and actual clipboard paste checks pass. Installed and pushed |
| R80 | Selection-mode row clicks always toggle one message while retaining other choices | Delivered ebddf54: plain selection-mode row clicks toggle one item, Shift adds ranges across pages, double-click reader preserved. Controller, modifiers, arrivals and real cross-page bulk review pass. Installed and pushed |
| R81 | Default padding and a centered reading column for plain/minimally styled email | Delivered in 1bf6ac9, installed/pushed: padded/centered plain letters and simple HTML, conservative preservation of sender layouts, rendered geometry/font-size tests and native selection/Find/full/compact pixel checks. Copy regression coordinates updated in cc20ce0. |
| R82 | Native Linux/Windows/macOS new-mail notification popups and sound, enabled by default and configurable | Installed/pushed ded5aca: separate popup/sound/details controls, persistent initial-import/identity deduplication, worker/private Linux bus/protocol tests and native preference/recovery scenarios. Windows cross-compilation passes; actual Windows/macOS delivery and macOS bundle integration remain open. |
| R83 | Preferences export of the complete SQL database, including all emails, configuration and accounts, for moving to another PC | Delivered by export `3927053` and import/profile checkpoint `93d4289`, pushed to main. Complete validated import, pending-operation review/fencing, isolated profiles, rename and next-launch selection pass Rust/native tests; see COMPLETION. Passwords require reconnection; separately protected credential sharing remains R49. |
| R84 | Search should search other folders, not just Inbox | Delivered in a81d767, installed/pushed: account-scoped search across cached folders, result locations, matching selection/relevance scopes, and storage/native search/move/bulk/clear/account/compact-layout tests. All 167 native functional scenarios pass. |
| R85 | Conserve credits: write handover.md, clean TODO and push current project changes | Completed by the handover push; verification and checkpoint identity in COMPLETION.md. Full product remains unfinished |
| R86 | Native close-to-tray; follow-up: temporarily use tray while saving even when ordinary close-to-tray is disabled, notify and quit after saving | Native tray, temporary saving notice, durable auto-quit and failure/Open recovery are implemented in the parallel lane; nine targeted Rust, 58 Python and all 26 selected native scenarios pass with reviewed WebPs. Full Windows GNU and exact macOS adapter checks pass; root integration 6728931 is pushed with32 native scenarios and745 hook executions passing. Ordinary-hide write-failure recovery ships in b8eafde with 748 hook executions and 16 native scenarios passing. Actual Windows/macOS execution and full macOS app checking remain open. See completion. |
| R87 | Refresh icon should spin more slowly and clockwise; add to TODO and push | Delivered in `d29ce06`: 2.4-second clockwise turn; renderer and seven native scenarios pass; see completion log. |
| R88 | Remove sender icons/avatars for shorter compact email-list rows, retain action buttons; unread highlight, dot and bold subject | Recorded in TODO with the supplied horizontal-row reference described; no UI implementation yet |
| R89 | Deleting a message should select the next message down and keep the list from snapping to the top | Shipped in 5a85ac3 and 91ed9a9: adjacent optimistic selection, stable scroll, previous/empty fallback and bounded page refill; six controller tests and 59 integrated native scenarios pass. Selected-account Unicode Move correction included; see completion evidence |
| R90 | Sync/close seems excessively slow; investigate and fix any bug | `34cfc71`: channel-owned account scheduling interrupts held read-only sync for read/flag writes while preserving cache commits; 528 Rust/adapter, 49 Python and 185/185 native functional tests. Additional close continuation/capacity-wait checkpoint passes 29 targeted Rust, 57 Python and 15 selected native scenarios; integrated as `2733cc6` and pushed with 725 hook executions and 23 merged native scenarios passing. Pending-save tray and remaining personal-server diagnosis stay open |
| R91 | Use channels rather than locks to manage state | Account/calendar scheduling, mail-cache connection/local leases, credential operations and profile catalog use bounded owning workers. Long database copying has its own connection/controller and does not hold the cache worker. Cache/credential cancellation/draining and profile/restart tests pass. Remaining Google lifecycle and backup-journal coordination stay in TODO; see completion/shipping evidence. |
| R92 | Next priority: full DB import/export in Settings; Google OAuth/Drive account/profile sharing; first login on either client, new/existing devices, configurable toggles; Flutter interoperability document in shep-clients; follow-up: put OAuth implementation referencing that handover at the top of TODO | Handover, parity/scenario gaps and top-priority client TODO shipped in client-branch `02c4b32`, with verified audit `59578f3`. Database transfer/local profiles are covered by R83. Native first-device creation/recovery and category controls ship in `488a9ec`, with failed-settings/close recovery in `6860f50` and isolated MCP/protocol coverage; see the newest completion entry. Existing-profile discovery/import ships in `071c6b0`, with 22 affected native scenarios. OAuth/profile implementation remains explicitly first in TODO, linking the Flutter handover; after-login discovery/enrollment ships in `acb4969`, while continuous changes, live interoperability and the credential-protection choice remain open. |


The main omissions were already present in the completion log but were not an adequate live checklist: faithful HTML, local encryption/large-mail streaming, folder trees/mutations, inline multiple drafts/discard, palette editing, multiple backup targets and continuous settings/account sync. Every request remains traceable above; TODO tracks unfinished work, including the remaining optimistic-state reconciliation and final coverage audit. Passing unrelated tests does not close a request.


9 September continuation: R02/R49/R92 now has named shared-catalog discovery and
native reviewed existing-profile import, including safe local account identities,
reconnection gating, selected settings and idempotent application. The newest
completion entry records 22 native scenarios and backend coverage. Continuous
updates, automatic login prompts, credential protection and real cross-client
Google verification remain open. R92's OAuth/handover follow-up remains first in
TODO. Source `071c6b0` is pushed; mandatory hooks passed 683 test executions. See the newest completion entry.

R90 shutdown review follow-up: matching attachment/discard/forward failures cancel
pending close; obsolete results retain newer dependencies. Three App::update tests
cover both result identities and late BulkStopped. Nine selected native scenarios
pass, including three saved failure/close/retry flows with reviewed WebPs. The
close filter passes 32 Rust tests; Python passes 57. Awaiting mandatory hooks and
root integration/shipping; native tray and personal-server diagnosis remain open.

R86 native-tray checkpoint: searchable persisted close preference, native Open/Quit,
background daemon/window ownership, temporary-saving notice and durable auto-exit,
reopening on failure/host loss, and native attachment-chooser safety are implemented.
Nine targeted Rust and 58 Python tests pass; all 26 selected native scenarios pass,
including final picker coverage and reviewed light/compact-dark/native menu WebPs. Full Windows GNU checking and exact macOS adapter checking
pass, with actual OS execution and full macOS app checking explicitly outstanding.
See the completion log; root integration/shipping and mandatory hook recording
remain pending, so R86/R90 are retained in TODO.
