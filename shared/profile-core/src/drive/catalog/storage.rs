use super::*;

pub(super) fn integer(value: impl TryInto<i64>) -> Result<i64> {
    value.try_into().map_err(|_| Error::Storage)
}
pub(super) fn optional_count(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<Option<u64>> {
    row.get::<_, Option<i64>>(index)?
        .map(|value| {
            u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
        })
        .transpose()
}

pub(super) fn directory(path: &Path) -> Result<()> {
    let mut directory = std::fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(path).map_err(|_| Error::Storage)
}
pub(super) fn private_file(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|_| Error::Storage)
}
pub(super) fn initialize(db: &mut Connection, scope: &Scope) -> Result<()> {
    let tx = db.transaction()?;
    let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != 0 && version != 1 {
        return Err(history::Error::Record(crate::Error::Upgrade).into());
    }
    if version == 0 {
        let existing: i64 = tx.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            [],
            |r| r.get(0),
        )?;
        if existing != 0 {
            return Err(Error::Binding);
        }
        tx.execute_batch("CREATE TABLE scope(namespace TEXT NOT NULL,principal TEXT NOT NULL);
            CREATE TABLE state(revision INTEGER NOT NULL DEFAULT 0,scan INTEGER NOT NULL DEFAULT 1,
                phase TEXT NOT NULL DEFAULT 'initial',full_scan INTEGER NOT NULL DEFAULT 1,
                current_token TEXT,stream_token TEXT,completed_token TEXT,completed_revision INTEGER,
                has_page INTEGER NOT NULL DEFAULT 0,page_next TEXT,page_done INTEGER NOT NULL DEFAULT 0,
                files INTEGER NOT NULL DEFAULT 0,profiles INTEGER NOT NULL DEFAULT 0,fault TEXT);
            INSERT INTO state DEFAULT VALUES;
            CREATE TABLE visited(phase TEXT NOT NULL,token TEXT NOT NULL,PRIMARY KEY(phase,token));
            CREATE TABLE listed(id TEXT PRIMARY KEY);
            CREATE TABLE pending(position INTEGER PRIMARY KEY,data TEXT NOT NULL);
            CREATE TABLE files(id TEXT PRIMARY KEY,profile TEXT NOT NULL,generation TEXT NOT NULL,
                operation TEXT NOT NULL,sha256 TEXT NOT NULL,data TEXT NOT NULL,seen_scan INTEGER NOT NULL,
                verified INTEGER NOT NULL DEFAULT 0,
                UNIQUE(profile,generation,operation));
            CREATE INDEX file_scan ON files(seen_scan);
            CREATE INDEX unverified_files ON files(id) WHERE verified=0;
            CREATE TABLE profiles(key TEXT PRIMARY KEY,summary TEXT NOT NULL,waiting INTEGER NOT NULL,ready INTEGER NOT NULL);
            CREATE INDEX ready_profiles ON profiles(key) WHERE ready>0;
            CREATE INDEX waiting_profiles ON profiles(key) WHERE waiting>0;
            PRAGMA user_version=1;")?;
        tx.execute(
            "INSERT INTO scope VALUES(?,?)",
            params![scope.namespace, scope.principal],
        )?;
    }
    let saved: (String, String) =
        tx.query_row("SELECT namespace,principal FROM scope", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?;
    if saved != (scope.namespace.clone(), scope.principal.clone()) {
        return Err(Error::Binding);
    }
    tx.commit()?;
    Ok(())
}
