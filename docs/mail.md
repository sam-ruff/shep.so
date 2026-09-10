# Mail

## Connect your accounts

Go to **Preferences → Accounts**. Add an IMAP or POP3 account, then enter its outgoing SMTP settings. You can test each connection during setup. Fastmail has a preset; use your full email address and an app password.

## Find and read messages

The unified inbox brings your accounts together. Search sender, subject or message text across folders in the selected accounts. Results show their folder; clearing search returns to the folder you were browsing. Filter for unread, flagged or attachment-bearing mail.

Refresh with the top-right icon, `Ctrl+R` or `F5`. The icon spins for manual refreshes; automatic background checks leave it still. Both shortcuts can be changed or disabled in Preferences.

Simple letters use a padded, centered reading column. Formatted messages keep their sender layouts, and the surrounding message card follows the email’s background.

Folder groups start collapsed. Click a chevron to show subfolders; click a folder name to read its mail. A container that cannot hold mail only expands. Shep remembers which groups you opened. With the sidebar focused, Left/Right moves through the tree and expands or collapses groups; Enter opens the focused folder. Ctrl-click combines folders. Dragging mail over a closed group opens it so you can drop into a subfolder.

Click a message to read it. Moving on to another message, folder or tab marks it read. An explicit **Mark as unread** remains unread until you choose to read it again. Double-click for a full-window reader. Related messages appear as collapsible cards; disable grouping in **Preferences → General** if you prefer individual messages.

HTML mail retains its layout, fonts, tables and inline images. Use **Plain text** for the text alternative. Select and copy text in either view; quoted text follows your reply-history preference. Use `Ctrl+F` or the reader’s search icon to find text in the open message. Enter and Shift+Enter move between matches; **Aa** matches case. Escape closes the find bar.

Drag the pane edges to give your inbox or reader more room.

Use the printer icon beside Forward, or `Ctrl+P`, to open your default browser’s print dialog. Choose a printer or save as PDF. Printing uses the complete cached message in the current Formatted/Plain text mode, including quoted history and attachment names. Only inline images and external images already loaded under your image preferences are included. Close the browser tab when finished. Remap or disable Print in Shortcuts; the macOS default is Command+P.

## Reply and organize

Use the visible message controls or right-click a message to reply, flag, move, archive, send to Trash or export it. Use `M` to move, `Ctrl+D` to send to Trash, and `Backspace` or `Delete` to archive. With the sidebar focused, `I` returns to Inbox. Change either shortcut slot, or disable the sidebar key, in **Preferences → Shortcuts**. On macOS, Command replaces Ctrl for the defaults.

Use the forward arrow beside Reply, or `F`, to create a new draft with the original message and attachments. Enter the new recipients before sending. A note above the quoted original preserves its HTML and inline images; editing the quoted original sends your edited text as plain text. Forward can be remapped or disabled in Shortcuts.

New messages and replies open in the preview pane. Replies keep the original conversation below the editor; **Include original message** controls whether quoted text is sent with your reply. Switch messages to work on several replies, then return to resume the matching draft. Collapse a draft with its chevron, or close the editor with × or Escape.

Drafts save automatically and appear in the collapsible **Drafts** group. Right-click a draft or use the bin in its editor to discard it. Review the draft and any attached files before confirming; **Keep draft** or Escape cancels. Attached files are copied into the draft, so moving the originals will not break it. Interrupted sends appear in **Outbox** for review; they are not retried automatically.

## Adjust your reading preferences

Choose appearance, text size and quoted-history display in **Preferences → General**. External images are blocked by default; manage exceptions in **Privacy** and trusted addresses in **Contacts**.

New-mail popups and sound are enabled by default. Search Preferences for **Notifications** to turn either off, hide sender/subject details, or try a test notification. Alerts cover newly received unread Inbox mail across your accounts; the first import and repeated syncs stay quiet. Shep must be running, and your operating system’s notification permissions and sound settings still apply.

See [current limits](limits.md) for provider restrictions and size limits.

Select email text with the mouse and copy it normally. Search Preferences to jump to a settings section. Icon tooltips can show the primary shortcut; both tooltips and their key hints can be turned off.

Archive, delete and move show a toast immediately while saving continues. Repeating an action increases the count and restarts its six-second display time; archive/delete counts continue across accounts in a unified inbox. A failed action restores the message and removes its count, with the error remaining visible. Use **Undo** on the toast to restore the messages, including while the original move is still pending. If reversal fails, the toast offers Retry Undo.
