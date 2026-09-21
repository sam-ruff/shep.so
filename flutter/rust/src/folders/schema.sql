BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS folder_creations (
 id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id),
 connection TEXT NOT NULL, parent TEXT, name TEXT NOT NULL,
 status TEXT NOT NULL, target TEXT, receipt TEXT,
 acknowledged INTEGER NOT NULL DEFAULT 0, error TEXT,
 revision INTEGER NOT NULL DEFAULT 1, created INTEGER NOT NULL, mutation TEXT
);
CREATE INDEX IF NOT EXISTS folder_creation_ready ON folder_creations(created,id)
 WHERE status IN ('queued','waiting');
CREATE INDEX IF NOT EXISTS folder_creation_active ON folder_creations(account_id,created,id)
 WHERE status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain');
CREATE INDEX IF NOT EXISTS folder_creation_history ON folder_creations(created DESC,id)
 WHERE status IN ('succeeded','cancelled');
CREATE INDEX IF NOT EXISTS folder_creation_retired ON folder_creations(id)
 WHERE status IN ('succeeded','cancelled','dismissed') AND mutation IS NOT NULL AND json_extract(mutation,'$.prepared_count')>0;
CREATE TABLE IF NOT EXISTS folder_catalogues (
 account_id TEXT PRIMARY KEY REFERENCES accounts(id), mailboxes TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS folder_change_members (
 job TEXT NOT NULL REFERENCES folder_creations(id) ON DELETE CASCADE,
 id TEXT NOT NULL, folder TEXT NOT NULL, remote_id TEXT NOT NULL, lineage TEXT NOT NULL,
 PRIMARY KEY(job,id)
);
CREATE INDEX IF NOT EXISTS folder_change_members_folder ON folder_change_members(job,folder,id);
CREATE TABLE IF NOT EXISTS folder_cache_authority(job TEXT PRIMARY KEY REFERENCES folder_creations(id) ON DELETE CASCADE);
CREATE TABLE IF NOT EXISTS folder_cache_revisions(account_id TEXT PRIMARY KEY REFERENCES accounts(id) ON DELETE CASCADE,epoch TEXT NOT NULL,revision INTEGER NOT NULL DEFAULT 0);
INSERT OR IGNORE INTO folder_cache_revisions(account_id,epoch) SELECT id,lower(hex(randomblob(16))) FROM accounts;
CREATE INDEX IF NOT EXISTS folder_mail_snapshot ON mail(account_id,folder,id);
CREATE TRIGGER IF NOT EXISTS folder_revision_account AFTER INSERT ON accounts BEGIN INSERT OR IGNORE INTO folder_cache_revisions(account_id,epoch) VALUES(new.id,lower(hex(randomblob(16)))); END;
CREATE TRIGGER IF NOT EXISTS folder_revision_insert AFTER INSERT ON mail BEGIN UPDATE folder_cache_revisions SET revision=revision+1 WHERE account_id=new.account_id; END;
CREATE TRIGGER IF NOT EXISTS folder_revision_update AFTER UPDATE ON mail BEGIN UPDATE folder_cache_revisions SET revision=revision+1 WHERE account_id IN (old.account_id,new.account_id); END;
CREATE TRIGGER IF NOT EXISTS folder_revision_delete AFTER DELETE ON mail BEGIN UPDATE folder_cache_revisions SET revision=revision+1 WHERE account_id=old.account_id; END;
CREATE TRIGGER IF NOT EXISTS folder_fence_mail_insert BEFORE INSERT ON mail
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account_id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
AND NOT EXISTS(SELECT 1 FROM folder_cache_authority)
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before changing this account mail.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_mail_update BEFORE UPDATE ON mail
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id IN (old.account_id,new.account_id) AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
AND NOT EXISTS(SELECT 1 FROM folder_cache_authority)
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before changing this account mail.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_mail_delete BEFORE DELETE ON mail
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=old.account_id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
AND NOT EXISTS(SELECT 1 FROM folder_cache_authority)
AND NOT EXISTS(SELECT 1 FROM removed_accounts WHERE id=old.account_id)
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before changing this account mail.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_action_insert BEFORE INSERT ON individual_mail_actions
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before changing this account mail.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_group_insert BEFORE INSERT ON group_items
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before changing this account mail.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_group_update BEFORE UPDATE ON group_items
WHEN new.state IN ('pending','sending','undoing','reversing') AND EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before changing this account mail.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_outgoing BEFORE INSERT ON outgoing
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account_id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before sending from this account.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_catalogue_update BEFORE UPDATE ON folder_catalogues
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account_id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
AND NOT EXISTS(SELECT 1 FROM folder_cache_authority)
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before refreshing this catalogue.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_outgoing_update BEFORE UPDATE ON outgoing
WHEN new.state IN ('queued','submitting') AND EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account_id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before sending from this account.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_action_update BEFORE UPDATE ON individual_mail_actions
WHEN new.status IN ('queued','running','waiting') AND EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before changing this account mail.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_names_update BEFORE UPDATE ON folders
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account_id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
AND NOT EXISTS(SELECT 1 FROM folder_cache_authority)
BEGIN SELECT RAISE(ABORT,'Review saved folder changes before refreshing these folders.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_account_settings BEFORE UPDATE OF settings ON accounts
WHEN new.settings!=old.settings AND EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Finish or stop tracking saved folder changes before editing this account.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_sent_role BEFORE INSERT ON known_sent_folders
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account_id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Finish saved folder changes before changing Sent roles.'); END;
CREATE TRIGGER IF NOT EXISTS folder_fence_discovered_sent BEFORE INSERT ON discovered_sent
WHEN EXISTS(SELECT 1 FROM folder_creations WHERE account_id=new.account_id AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))
BEGIN SELECT RAISE(ABORT,'Finish saved folder changes before changing Sent roles.'); END;
PRAGMA user_version=24;
COMMIT;
