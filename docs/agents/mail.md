# Mail

Use **New folder** at the bottom of the sidebar to choose an account, a parent
and a folder name. IMAP creates it on the server; POP3 creates a local folder.
An interrupted request reappears in this same form after restarting, with
Try again and Resume controls. Creating a folder keeps the current mail open.

Right-click selected text for Copy and Select all. Editable fields and drafts
also offer Cut and Paste; unavailable actions are disabled. Shift+F10 or the
Menu key opens the text menu for the focused control. Passwords cannot be copied.

- Add and edit multiple IMAP or POP3 accounts in Preferences → Accounts. The setup wizard separates identity, incoming IMAP/POP3 and outgoing SMTP settings, with SSL/TLS or STARTTLS, authentication choices, and independent connection tests. Fastmail has a preset; use its app password and full login address. Certificate verification is always enabled.
- Mail checks start when the cached workspace opens and repeat every 5 seconds by default. Change the interval in Preferences → General → Mail & performance (5–3600 seconds). Each account keeps its own schedule, so a slow account never holds back new mail from another. The refresh icon also works during a background check, queuing one follow-up check for each account. An account's checks never overlap; its slow check finishes before its next begins. Background activity does not put the manual refresh button into its busy state.
- IMAP accounts whose server offers IDLE also keep a dedicated connection watching Inbox, so new mail starts a check immediately instead of waiting for the interval. The watcher re-issues IDLE every 25 minutes, reconnects after a dropped connection with delays growing from 5 seconds to 5 minutes, and stops when the account is removed or the app closes. Servers without IDLE, other folders and POP3 accounts rely on the interval checks, which keep running for every account either way.
- Enable the unified inbox in General preferences, expand it to choose an account, or disable it for account-specific navigation. Custom folders appear under collapsible account headings. Ctrl+click folders to combine them in one view; an ordinary click selects just one. Search covers indexed sender, subject and body text across folders in the selected account scope, with prefixes and typo tolerance. Results show their folder; clearing search restores the browsing folder. Explicit combined folder views search the accounts represented by those folders. New searches use **Best match**: exact words rank ahead of typo expansions, with short relevant messages favored over weak matches. Choose another sort for the current search; clearing it restores the usual inbox sort. Filter All, Unread, Read, Flagged or Attachments; sort newest/oldest, sender or subject. Sorting is saved.
- Right-click an account folder, or focus it and press Shift+F10, to move or delete it. Moving searches for a parent folder, then reviews the affected subtree before Enter/Y confirms; N/Escape cancels. Delete is permanent and includes child folders and their mail; POP3 only deletes local copies. Folder changes shows progress and recovery after restart. Retry a rejected operation there. An unconfirmed result requires checking the server before explicitly stopping remaining changes; its cached originals are kept. Inbox is protected.
- Right-click an inbox message for open, reply, read/unread, flag, move, archive, Trash, sender copy and export actions. Shift+F10 opens the same menu, with arrows/Enter and Escape. Button tooltips show current remapped shortcuts.
- Use **Select** beside search for checkboxes, Ctrl-click to toggle rows, Shift-click for a range, or remappable Ctrl+A while the inbox list has focus. Selection spans pages. New arrivals stay unselected until you choose them with a checkbox, Ctrl-click, a Shift range or another Select All. Earlier choices are preserved. **Clear** unchecks rows; **Done** or Escape leaves selection mode. Text fields and the reader keep their normal text-selection shortcuts.
- With messages selected, the preview toolbar and mail-action shortcuts act on that group. Reviews accept Enter/Y and cancel with Escape/N. Archive, Trash and Move show immediate counted feedback with Undo, including while saving. **History** lists individual results and offers Undo/recovery. Queued group changes survive restarting; an unconfirmed server result requires review. **Continue** resumes a paused group. **Retry Undo** retries a failed reversal. After refreshing and checking the affected folders, **Accept current mail state** clears only the unconfirmed steps; those steps cannot be undone as part of the group. Other confirmed changes keep their Undo receipts. History provides separate pages for groups and message results. Default moves use each message's own account.
- Flag/unflag directly in an inbox row or the reader toolbar, with a red outline for flagged messages; mark read/unread in the toolbar. IMAP flags synchronize with the server; POP3 flags and folders are local. The Flagged sidebar view searches across folders.
- Drag the sidebar edge or the divider between inbox and reader to resize them. The layout and window dimensions persist across sessions; closing waits for pending layout saves. Pages contain 50 messages; visible rows and a small margin are rendered. Adjacent bodies and the next page preload in the background.
- Move with the visible button or `M`, search a folder by name, accent-insensitive text, a typo or an abbreviation, then press Enter to use the highlighted top match. Opt-in moves between IMAP accounts preserve the source until the destination confirms receipt; interrupted transfers are recorded to prevent blind duplicate uploads. Reply or Reply all, compose with To/Cc/Bcc, save drafts, archive, move to Trash, export original `.eml` files and save attachments through native file dialogs.
- Drag a message or selected group onto a sidebar folder. Valid destinations have an outline; the floating label names the destination and account. Hold over a collapsed account or Inbox to expand it, and scroll the sidebar while holding the message. Escape or dropping outside a folder cancels. Group drops review the entire selection, including other pages; Enter/Y confirms and Escape/N cancels. Single drops show immediate feedback with Undo. Inbox, Archive and Trash use each message's own account; explicit account folders honor the cross-account preference. The combined Sent and Flagged views are not drop destinations. POP3 supports local folder moves only. The provider checks actual server capabilities when committing.
- Attach files through the native picker; attachment chips wrap and can be removed with the mouse. Files are copied into the draft cache so moving or deleting the original does not break a saved draft. Up to 32 files / 18 MiB of attachments fit within the 25 MiB encoded-message limit.
- The forward arrow beside Reply, or remappable `F`, prepares an independent draft from the complete cached original. Recipients start empty. Original HTML, inline images and attachments survive saving/reopening; prepending a note keeps the formatting, while editing the quoted original sends the edited plain text. Preparation uses the persistence worker and never requests external images. Late results stay in Drafts when another editor is open.
- New messages and replies use the preview pane. Replies retain their original conversation below the editor and save the choice to include quoted text. Switching mail parks its reply and returning restores it; new drafts are also accessible in Drafts. Each owned session autosaves after editing pauses. Window close waits for every pending draft, and failed saves keep the window open. Older saves cannot overwrite newer edits or restore a sent or discarded draft. The counted Drafts group remembers its collapsed state. Right-click a draft or use the composer bin to review permanent deletion, including cached attachments. Enter/Y confirms; Escape/N cancels without losing unsaved edits. Failed deletion keeps the draft available for retry. Send saves a durable delivery record before contacting SMTP, then releases the composer while delivery runs in the background. Interrupted sends appear in **Outbox** and are never retried automatically. Review delivery there before returning an uncertain message to drafts or recording it as sent.
- Sent messages keep a local copy. IMAP accounts can also save to the server's Sent folder, rely on a server that saves its own copies, or keep copies locally; choose this in the SMTP setup step. Automatic discovery uses the server's Sent designation, with an optional explicit folder override. The sidebar's Sent view combines each account's actual Sent folder and local copies. POP3 keeps Sent copies locally.
- Outbox separates delivery from Sent-copy recovery. Check for an existing copy, save a missing server copy, or keep it locally without another SMTP send. A lost copy acknowledgment requires a review before another upload. Locally stored Sent copies have local flags/folders; moving them to another account requires first saving and syncing a server copy.
- Reply uses Reply-To when provided. Reply all excludes your configured addresses, deduplicates recipients and preserves message references. Its default shortcut is `Shift+R`, remappable in Preferences.
- Up/Down selects messages, Tab switches between inbox and sidebar navigation, and double-click opens a full-window reader. Close it with the button or remappable Escape shortcut.
- Related messages appear as separate, collapsible cards in the reader, including cached replies in other folders of the same account. Outside selection mode, one message opens at a time; reply, move and toolbar actions apply to that message. Long conversations page through 20 messages at a time and preload neighboring bodies. Disable **Group related messages in the reader** in General preferences to read individual messages. The inbox still lists individual emails.
- Quoted reply history within each message can be collapsed, expanded or hidden. Attachments wrap alongside Reply. Click the sender for copyable addresses. Remote images default to blocked, with message/sender/domain exceptions and Block all / Contacts / Allow all policies in Privacy preferences. Contacts are currently a manually maintained list.
- Preferences → General includes message font size and interface scaling, alongside Light, Dark and System appearance. Preferences → Shortcuts remaps every listed action and rejects duplicate bindings. Letter shortcuts do not activate inside text fields. `Mod` is Command on macOS and Control elsewhere.

IMAP checks download bodies in two lanes. Within each folder the new
messages from a metadata chunk are fetched smallest first (newest first among
equal sizes) in the usual batches, so a run of small new mail never waits
behind one large message. Anything over 1 MiB is recorded and fetched one
message at a time only after every folder's small bodies have arrived, and
messages over 25 MiB follow in the streamed staging pass. A folder's listing
is reconciled once its small bodies are in; a deferred body is not cached
yet, so it is never removed by that reconciliation. Inbox reports finished
after its last body in whichever lane carries it. If the slow lane fails part
way, the messages already received stay saved and the remaining deferred
messages are still unknown, so the next check fetches them again.

Contacts has a separate Preferences tab. Image policy and per-message/sender/domain exceptions remain under Privacy. Explicit Save buttons show a dismissible **Changes saved** toast after persistence succeeds.

Archive, Trash and Junk are logical names. A move first looks for a folder of
that exact name and otherwise uses the folder the server marks with the matching
special-use attribute, such as `Deleted Items` for Trash, so such a server never
gains a second literal folder; the sidebar's Trash and Spam entries list those
same folders. A destination that truly does not exist is created after Shep asks
for the root and hierarchy separator with `LIST "" ""`, falling back to a listing
of the destination's own reference, and servers advertising CREATE-SPECIAL-USE
receive the matching attribute at creation.

Archive and Move create a missing destination before moving mail. If an earlier
move was interrupted, Shep checks the original and destination to finish it
automatically where possible. **Retry move** is available directly in the reader;
you can keep navigating while it runs. An unresolved result keeps the cached
message readable, with further options available when the server cannot establish
which copy to use.

## Moves the server refuses

When the server definitely refuses a move (a tagged NO or BAD on the MOVE with
no COPYUID, a refused destination creation, or a server without the MOVE
capability) Shep still applies it on this device. The cached row keeps its
server identity and is shown at the destination through the same protected
projection as an unfinished move; the journal records it in the `Local` stage.
Sync leaves it alone: the source listing does not restore it, the destination
listing does not delete it, and server flag changes still reach it. Each
background check retries the server move for at most three records, at least
ten minutes apart; success gives the row its real destination identity and
clears the record, another refusal leaves it device-only, and an unconfirmed
result follows the ordinary journal rules. Undo restores the row locally and
clears the record without a server call.

Only definite refusals within one account qualify. Connection loss, timeouts,
authentication failures, an uncertain folder creation and a NO after a COPYUID
(a possible partial move) keep the message in place with an error, as before,
and cross-account transfers are never completed locally. A device-only move
does not reach other Shep clients until the server accepts it; until the
cross-device mail-state discussion settles a shared design, syncing it manually
means retrying from the reader or waiting for the automatic retry, and other
clients keep showing the message in its old folder.

Selecting an unread inbox row (or navigating to it with arrows) and then leaving it marks it read immediately. Startup selection and neighbor preloading do not. Explicit read/unread controls clear that pending reading state, so a later navigation cannot reverse a deliberate mark-unread. Failed writes restore the indicator without selecting the previous message. Archive/move waits for the read write before moving the source UID, while the list removal remains immediate.

Action toasts are generated when the UI accepts the move intent, including when it waits behind a pending read/flag. Tokens correlate failures with counted entries; acknowledgments do not recreate dismissed or expired toasts. Archive/Trash aggregate across unified accounts, while other moves group by exact account/folder. Cross-account transfers now use typed completion IDs and pending overlays, wait for confirmed source flags and retain the existing upload journal. Generic provider timeouts must not discard typed completion or cancel SQLite cleanup. Undo restores rows immediately, waits for acknowledged server identities and retains a retry control on reversal failure. Its provider receipts and exact-copy recovery are documented in the repository AGENTS.md. Broader filtered destination projection and durable recovery for individual actions remain tracked work; selected-group jobs have their own persistent journal.

Preferences → General → Colors edits separate light and dark palettes. Choose a color role, type a six-digit RGB value or pick a swatch, then Apply colors. Changes preview before applying; Undo changes cancels the pending edits and Reset palette restores the selected theme. The editor stays readable and warns about low text contrast. Palette settings persist in database exports; the current shared-profile wire format does not yet replicate these custom colors.
