# Mobile folder changes

Flutter saves folder requests before reading credentials or taking provider
capacity. The form closes after local admission. A pending, nonselectable entry
shows the requested name beneath its captured account and parent; it is never a
physical Move destination.

## Ownership and recovery

`flutter/rust/src/folders.rs` owns schema 24's requests and catalogue metadata.
It uses the existing SQLite writer, account exclusion and shared provider slots.
Busy owners return immediately, leaving work queued or waiting. The foreground
controller retains one executing request per account and resumes saved work.
The journal allows 32 active requests and returns at most 50 activity rows.

The shared IMAP transport discovers the namespace and encoding. The exact target
is saved before CREATE. Offline namespace discovery remains Waiting; a typed
invalid name or parent result is Rejected. Its acknowledgement is saved before inspection and
cache publication; a failed read after acknowledgement is local repair, not a
failed CREATE. A saved observation can finish cache repair without credentials
or provider capacity. POP3 folders use local storage only.

An unknown CREATE is never automatically repeated. Check server takes account
ownership and performs inspection only. Checked absence permits an explicit
retry if no acknowledgement exists. An acknowledged but missing folder remains
under review. Cancel prevents an unsent request; Stop tracking neither deletes a
server folder nor changes its recorded outcome. Restart preserves queued work,
repairs known receipts and expires unconfirmed dispatch authority.

Incoming connection identity, credential activation and imported-account
reconnect guards are checked before provider access. Removal reviews include
folder requests and their revisions, then delete local requests/catalogues with
the account. An active CREATE holds the existing account fence through its
receipt. No folder metadata or secrets are added to portable profile records.

## Evidence and limits

Manage folders uses the shared checked Rename, Move and Delete plan. A review
freezes at most 128 exact folder identities and an account cache revision.
Confirmation saves the request locally; metadata freezing and cache repair each
process at most 50 messages per transaction through the same owner. Pending
logical names never become mail destinations. Definite rejection restores the
unapplied tree projection; partial acknowledgements remain recorded.

Pending mail, Outbox, folder and Sent-role work excludes conflicting admission.
Saved changes fence provider claims, account edits and ordinary cache writes.
Credential-only reconnect remains available. An acknowledged RENAME retains
message UIDs and lineage only after the exact destination catalogue is checked.
Unknown results never authorise replay or guessed UID mapping. Check server can
enable stopping tracking while retaining cached mail and the recorded outcome.
Folder Undo and cross-account moves are not offered.

`folders/changes/tests.rs` exercises reviewed admission, bounded preparation and
repair, provider refusal/uncertainty, partial deletion, cache failure, restart,
account removal and conflicting writes. `folder_changes_test.dart` covers
accessible review controls, queued cancellation and compact light/dark layouts.
The actual bridge also executes POP3 rename, move and delete without credentials.

`folders/tests.rs` covers local admission under held capacity, provider refusal,
lost response, receipt/cache failure, actual database reopen, changed accounts,
imported reconnect guards and removal. `folder_native_repository_test.dart`
exercises the real bridge and locked-credential POP3 path.
`folder_creation_test.dart` uses visible controls for admission, lost replies,
recovery and the pending sidebar, with compact light/dark images.

Live IMAP, Android and Apple execution of these folder controls remain open.
Complete mobile folder trees and large-catalogue performance remain separate
work. The existing generic sync permit order is
unchanged; folder work uses nonblocking ownership acquisition to avoid adding
another wait cycle.
