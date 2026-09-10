//! Only test-support demo launches can open this explicitly marked fixture cache.
use crate::store::Store;
use anyhow::Context;
use std::{fs::OpenOptions, path::Path};

const APPLICATION_ID: i32 = 0x5348_5054;

pub fn open(path: Option<&Path>) -> anyhow::Result<Store> {
    let Some(path) = path else {
        return Store::memory();
    };
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => {
            let connection = rusqlite::Connection::open(path)?;
            connection.pragma_update(None, "application_id", APPLICATION_ID)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            anyhow::ensure!(
                !path.symlink_metadata()?.file_type().is_symlink(),
                "A fixture cache cannot be a symbolic link"
            );
            let connection = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let marker: i32 =
                connection.pragma_query_value(None, "application_id", |r| r.get(0))?;
            anyhow::ensure!(
                marker == APPLICATION_ID,
                "Refusing to open a non-fixture database in demo mode"
            );
        }
        Err(error) => return Err(error).context("Could not create the fixture cache"),
    }
    Store::open(path)
}

pub fn from_arguments() -> anyhow::Result<Store> {
    open(path_from_arguments()?.as_deref())
}

pub fn path_from_arguments() -> anyhow::Result<Option<std::path::PathBuf>> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if !arguments.iter().any(|a| a == "--persist-demo") {
        return Ok(None);
    }
    let state = arguments
        .windows(2)
        .find(|a| a[0] == "--test-state")
        .map(|a| Path::new(&a[1]))
        .context("Persistent demo requires --test-state in an owned fixture directory")?;
    let directory = state
        .parent()
        .context("The fixture state needs a parent directory")?;
    Ok(Some(directory.join("fixture.sqlite")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{MailQuery, parse_mail};

    #[tokio::test]
    async fn fixture_restart_preserves_mail_and_never_opens_an_unmarked_database() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("fixture.sqlite");
        let store = open(Some(&path)).unwrap();
        let message = parse_mail(
            "fixture",
            "1",
            "INBOX",
            b"Subject: Persisted\r\n\r\nFixture".to_vec(),
            true,
            false,
        )
        .unwrap();
        let id = message.summary.id.clone();
        store.upsert(vec![message]).await.unwrap();
        store.put("fixture_seeded", true).await.unwrap();
        drop(store);
        let reopened = open(Some(&path)).unwrap();
        assert!(reopened.get::<bool>("fixture_seeded").await.unwrap());
        assert_eq!(
            reopened.query(MailQuery::default()).await.unwrap().rows[0].id,
            id
        );
        let personal = directory.path().join("unmarked.sqlite");
        let c = rusqlite::Connection::open(&personal).unwrap();
        c.execute_batch("CREATE TABLE precious(value); INSERT INTO precious VALUES('untouched');")
            .unwrap();
        assert!(open(Some(&personal)).is_err());
        let tables: i32 = c
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            tables, 1,
            "The fixture opener must not migrate an unmarked cache"
        );
        assert_eq!(
            c.query_row("SELECT value FROM precious", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "untouched"
        );
    }
}
