use super::*;

pub(super) fn initialize(db: &mut Connection, binding: &Binding) -> Result<Uuid> {
    let tx = db.transaction()?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 0 && version != 1 {
        return Err(crate::Error::Upgrade.into());
    }
    if version == 0 {
        // Refuse a different existing database instead of claiming its tables.
        let existing: i64 = tx.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        if existing != 0 {
            return Err(Error::Binding);
        }
        tx.execute_batch("CREATE TABLE binding(namespace TEXT NOT NULL,principal TEXT NOT NULL,profile TEXT NOT NULL,generation TEXT NOT NULL,device TEXT NOT NULL);
            CREATE TABLE state(revision INTEGER NOT NULL DEFAULT 0,operations INTEGER NOT NULL DEFAULT 0,waiting INTEGER NOT NULL DEFAULT 0,ready INTEGER NOT NULL DEFAULT 0,queued INTEGER NOT NULL DEFAULT 0,fields INTEGER NOT NULL DEFAULT 0,conflicts INTEGER NOT NULL DEFAULT 0,removed INTEGER NOT NULL DEFAULT 0);
            INSERT INTO state DEFAULT VALUES;
            CREATE TABLE operations(seq INTEGER PRIMARY KEY AUTOINCREMENT,id TEXT NOT NULL UNIQUE,device TEXT NOT NULL,raw BLOB NOT NULL,sha256 TEXT NOT NULL,request BLOB,local INTEGER NOT NULL,applied INTEGER NOT NULL DEFAULT 0,remaining INTEGER NOT NULL DEFAULT 0,uploaded INTEGER NOT NULL DEFAULT 0,file_id TEXT UNIQUE);
            CREATE INDEX ready_operations ON operations(seq) WHERE applied=0 AND remaining=0;
            CREATE INDEX upload_queue ON operations(local,uploaded,seq);
            CREATE TABLE parents(child TEXT NOT NULL REFERENCES operations(id),parent TEXT NOT NULL,PRIMARY KEY(child,parent));
            CREATE INDEX children ON parents(parent,child);
            CREATE TABLE heads(id TEXT PRIMARY KEY REFERENCES operations(id));
            CREATE TABLE removed_accounts(id TEXT PRIMARY KEY,operation TEXT NOT NULL REFERENCES operations(id));
            CREATE TABLE targets(target TEXT PRIMARY KEY,account TEXT,versions INTEGER NOT NULL DEFAULT 0,visible INTEGER NOT NULL DEFAULT 0,revision INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX account_targets ON targets(account,target);
            CREATE INDEX visible_targets ON targets(target) WHERE visible=1;
            CREATE TABLE versions(target TEXT NOT NULL REFERENCES targets(target),operation TEXT NOT NULL REFERENCES operations(id),position INTEGER NOT NULL,PRIMARY KEY(target,operation));
            PRAGMA user_version=1;")?;
        tx.execute(
            "INSERT INTO binding VALUES(?,?,?,?,?)",
            params![
                binding.namespace,
                binding.principal,
                binding.profile.to_string(),
                binding.generation.to_string(),
                Uuid::new_v4().to_string()
            ],
        )?;
    }
    let saved = tx.query_row(
        "SELECT namespace,principal,profile,generation,device FROM binding",
        [],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        },
    )?;
    if saved.0 != binding.namespace
        || saved.1 != binding.principal
        || saved.2 != binding.profile.to_string()
        || saved.3 != binding.generation.to_string()
    {
        return Err(Error::Binding);
    }
    // Connection-owned scratch must live in the keyed database. SQLite TEMP
    // files do not inherit SQLCipher protection, and a memory TEMP table would
    // grow with the complete causal history. It is empty at committed boundaries.
    tx.execute_batch("CREATE TABLE IF NOT EXISTS history_ancestors(id TEXT PRIMARY KEY,expanded INTEGER NOT NULL DEFAULT 0);
        CREATE INDEX IF NOT EXISTS history_frontier ON history_ancestors(id) WHERE expanded=0;
        DELETE FROM history_ancestors;")?;
    let device = parse_uuid(&saved.4)?;
    tx.commit()?;
    Ok(device)
}
