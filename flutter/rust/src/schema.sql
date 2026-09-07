BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS accounts (id TEXT PRIMARY KEY, settings TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS mail (
 id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id), remote_id TEXT NOT NULL,
 folder TEXT NOT NULL, sender TEXT NOT NULL, recipient TEXT NOT NULL, subject TEXT NOT NULL,
 preview TEXT NOT NULL, timestamp INTEGER NOT NULL, unread INTEGER NOT NULL, starred INTEGER NOT NULL,
 attachment_count INTEGER NOT NULL, body TEXT NOT NULL, raw BLOB NOT NULL, moved INTEGER NOT NULL DEFAULT 0,
 UNIQUE(account_id, remote_id, folder)
);
CREATE INDEX IF NOT EXISTS mail_page ON mail(folder, timestamp DESC, id);
CREATE INDEX IF NOT EXISTS mail_account_page ON mail(account_id, folder, timestamp DESC, id);
CREATE VIRTUAL TABLE IF NOT EXISTS mail_search USING fts5(sender, subject, body, content='mail', content_rowid='rowid', tokenize='unicode61');
CREATE TRIGGER IF NOT EXISTS mail_insert AFTER INSERT ON mail BEGIN
 INSERT INTO mail_search(rowid,sender,subject,body) VALUES(new.rowid,new.sender,new.subject,new.body);
END;
CREATE TRIGGER IF NOT EXISTS mail_delete AFTER DELETE ON mail BEGIN
 INSERT INTO mail_search(mail_search,rowid,sender,subject,body) VALUES('delete',old.rowid,old.sender,old.subject,old.body);
END;
CREATE TRIGGER IF NOT EXISTS mail_edit AFTER UPDATE OF sender,subject,body ON mail BEGIN
 INSERT INTO mail_search(mail_search,rowid,sender,subject,body) VALUES('delete',old.rowid,old.sender,old.subject,old.body);
 INSERT INTO mail_search(rowid,sender,subject,body) VALUES(new.rowid,new.sender,new.subject,new.body);
END;
CREATE TABLE IF NOT EXISTS folders(account_id TEXT PRIMARY KEY REFERENCES accounts(id), names TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS drafts(id TEXT PRIMARY KEY, revision INTEGER NOT NULL, content TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS draft_files(id TEXT PRIMARY KEY, draft_id TEXT NOT NULL REFERENCES drafts(id) ON DELETE CASCADE, name TEXT NOT NULL, media_type TEXT NOT NULL, bytes BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS discarded_drafts(id TEXT PRIMARY KEY, revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS outgoing(id TEXT PRIMARY KEY, draft_id TEXT NOT NULL UNIQUE, state TEXT NOT NULL, account_id TEXT NOT NULL, message_id TEXT NOT NULL, raw BLOB NOT NULL, draft TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS move_receipts(id TEXT PRIMARY KEY REFERENCES mail(id) ON DELETE CASCADE, receipt TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS pending_moves(id TEXT PRIMARY KEY REFERENCES mail(id) ON DELETE CASCADE, destination TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS draft_file_revisions(id TEXT PRIMARY KEY REFERENCES drafts(id) ON DELETE CASCADE, revision INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS outgoing_meta(id TEXT PRIMARY KEY REFERENCES outgoing(id), created INTEGER, from_address TEXT, recovery TEXT, recovered_draft TEXT);
CREATE TABLE IF NOT EXISTS outgoing_sent(id TEXT PRIMARY KEY REFERENCES outgoing(id), account TEXT NOT NULL, state TEXT NOT NULL, folder TEXT, receipt TEXT, error TEXT, complete INTEGER NOT NULL DEFAULT 0, local_edited INTEGER NOT NULL DEFAULT 0);
CREATE INDEX IF NOT EXISTS outgoing_message_id ON outgoing(account_id,message_id);
CREATE TABLE IF NOT EXISTS mail_aliases(alias TEXT PRIMARY KEY, id TEXT NOT NULL REFERENCES mail(id) ON DELETE CASCADE, CHECK(alias!=id));
CREATE INDEX IF NOT EXISTS mail_alias_target ON mail_aliases(id);
CREATE TABLE IF NOT EXISTS discovered_sent(account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE, folder TEXT);
CREATE TABLE IF NOT EXISTS known_sent_folders(account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE, folder TEXT NOT NULL, PRIMARY KEY(account_id,folder));
CREATE VIEW IF NOT EXISTS sent_folder_names AS
 SELECT id AS account_id,json_extract(settings,'$.sent_folder') AS folder FROM accounts WHERE COALESCE(json_extract(settings,'$.sent_folder'),'')!=''
 UNION SELECT account_id,folder FROM discovered_sent WHERE folder IS NOT NULL
 UNION SELECT account_id,folder FROM known_sent_folders;
INSERT OR IGNORE INTO known_sent_folders(account_id,folder) SELECT o.account_id,s.folder FROM outgoing o JOIN outgoing_sent s ON s.id=o.id JOIN accounts a ON a.id=o.account_id WHERE (SELECT user_version FROM pragma_user_version)<6 AND s.state='saved' AND s.folder IS NOT NULL;
CREATE TABLE IF NOT EXISTS removed_accounts(id TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, cleanup INTEGER NOT NULL DEFAULT 1);
CREATE TRIGGER IF NOT EXISTS refuse_removed_account BEFORE INSERT ON accounts WHEN EXISTS(SELECT 1 FROM removed_accounts WHERE id=new.id) BEGIN SELECT RAISE(ABORT,'This account was removed. Add a new account.'); END;
CREATE TRIGGER IF NOT EXISTS refuse_removed_draft BEFORE INSERT ON drafts WHEN EXISTS(SELECT 1 FROM removed_accounts WHERE id=json_extract(new.content,'$.account_id')) BEGIN SELECT RAISE(ABORT,'This account was removed. Choose a connected account.'); END;
CREATE TRIGGER IF NOT EXISTS refuse_removed_draft_edit BEFORE UPDATE ON drafts WHEN EXISTS(SELECT 1 FROM removed_accounts WHERE id=json_extract(new.content,'$.account_id')) BEGIN SELECT RAISE(ABORT,'This account was removed. Choose a connected account.'); END;
CREATE TABLE IF NOT EXISTS credential_slots(slot TEXT PRIMARY KEY, account_id TEXT NOT NULL, settings TEXT, expected TEXT, state TEXT NOT NULL CHECK(state IN ('prepared','active','cleanup')));
CREATE TABLE IF NOT EXISTS account_credentials(account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE, slot TEXT NOT NULL UNIQUE REFERENCES credential_slots(slot));
CREATE TABLE IF NOT EXISTS draft_inline(file_id TEXT PRIMARY KEY REFERENCES draft_files(id) ON DELETE CASCADE, content_id TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS draft_forwards(draft_id TEXT PRIMARY KEY REFERENCES drafts(id) ON DELETE CASCADE, source_id TEXT NOT NULL);
PRAGMA user_version=9;
COMMIT;
