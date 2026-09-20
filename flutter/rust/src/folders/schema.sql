BEGIN IMMEDIATE;
CREATE TABLE IF NOT EXISTS folder_creations (
 id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id),
 connection TEXT NOT NULL, parent TEXT, name TEXT NOT NULL,
 status TEXT NOT NULL, target TEXT, receipt TEXT,
 acknowledged INTEGER NOT NULL DEFAULT 0, error TEXT,
 revision INTEGER NOT NULL DEFAULT 1, created INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS folder_creation_ready ON folder_creations(created,id)
 WHERE status IN ('queued','waiting');
CREATE INDEX IF NOT EXISTS folder_creation_active ON folder_creations(account_id,created,id)
 WHERE status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain');
CREATE INDEX IF NOT EXISTS folder_creation_history ON folder_creations(created DESC,id)
 WHERE status IN ('succeeded','cancelled');
CREATE TABLE IF NOT EXISTS folder_catalogues (
 account_id TEXT PRIMARY KEY REFERENCES accounts(id), mailboxes TEXT NOT NULL
);
PRAGMA user_version=23;
COMMIT;
