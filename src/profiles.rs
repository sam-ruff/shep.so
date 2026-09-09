//! Local profile identities are device-owned. Google profile IDs and imported
//! database values must never select an OS credential namespace or cache path.
use crate::{credentials::Scope, store::worker::Worker};
use anyhow::Context;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

const APPLICATION_ID: i32 = 0x5348_5052;
pub const CATALOG_FILE: &str = "profiles.sqlite";
pub(crate) const IMPORT_MARKER_KEY: &str = "profile_import_ready_v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Id {
    Legacy,
    Imported(uuid::Uuid),
}
impl Id {
    pub fn key(self) -> String {
        match self {
            Self::Legacy => "legacy".into(),
            Self::Imported(id) => id.to_string(),
        }
    }
    pub fn scope(self) -> Scope {
        match self {
            Self::Legacy => Scope::Legacy,
            Self::Imported(id) => Scope::Profile(id),
        }
    }
    fn parse(value: &str) -> anyhow::Result<Self> {
        if value == "legacy" {
            return Ok(Self::Legacy);
        }
        let id = uuid::Uuid::parse_str(value).context("The saved profile identity is invalid")?;
        anyhow::ensure!(
            !id.is_nil() && id.to_string() == value,
            "The saved profile identity is invalid"
        );
        Ok(Self::Imported(id))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub id: Id,
    pub name: String,
    pub ready: bool,
}
#[derive(Debug, Clone)]
pub struct Page {
    pub offset: u64,
    pub revision: u64,
    pub active: Id,
    pub total: u64,
    pub rows: Vec<Profile>,
}

#[derive(Debug, Clone)]
pub enum Request {
    List {
        offset: u64,
    },
    Rename {
        id: Id,
        revision: u64,
        name: String,
        offset: u64,
    },
    Activate {
        id: Id,
        revision: u64,
        offset: u64,
    },
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub current: Profile,
    pub page: Page,
    pub warning: Option<String>,
}
#[derive(Debug, Default)]
pub struct Recovery {
    pub found: u64,
    pub warnings: u64,
    pub first_warning: Option<String>,
}
impl Recovery {
    fn warn(&mut self, error: impl std::fmt::Display) {
        self.warnings += 1;
        if self.first_warning.is_none() {
            self.first_warning = Some(error.to_string());
        }
    }
    pub fn message(&self) -> Option<String> {
        self.first_warning.as_ref().map(|error|format!("{} profile files need attention. They were kept; restore or repair the affected copy, then refresh. {error}",self.warnings))
    }
}
#[derive(Clone)]
pub(crate) struct Session {
    pub catalog: Catalog,
    pub current: Id,
}
impl Session {
    pub async fn snapshot(&self, offset: u64) -> anyhow::Result<Snapshot> {
        let current = self.current;
        let current = self
            .catalog
            .worker
            .run(move |c| profile(c, current))
            .await?;
        Ok(Snapshot {
            current,
            page: self.catalog.page(offset).await?,
            warning: None,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ImportMarker {
    pub version: u8,
    pub local_profile: uuid::Uuid,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone)]
pub struct Catalog {
    root: PathBuf,
    legacy_filename: String,
    worker: Arc<Worker>,
    key: Option<Arc<crate::cache_cipher::Key>>,
}
impl Catalog {
    /// Call on a background worker. Existing legacy installations keep their
    /// cache filename and keychain entries until an explicit profile switch.
    pub fn open(root: &Path, legacy_filename: &str) -> anyhow::Result<Self> {
        Self::open_with_key(root, legacy_filename, None)
    }

    /// The owning bootstrap must admit and retain the device key before opening
    /// any catalog or profile file. This never migrates an existing plain file.
    pub fn open_encrypted(
        root: &Path,
        legacy_filename: &str,
        key: Arc<crate::cache_cipher::Key>,
    ) -> anyhow::Result<Self> {
        Self::open_with_key(root, legacy_filename, Some(key))
    }

    fn open_with_key(
        root: &Path,
        legacy_filename: &str,
        key: Option<Arc<crate::cache_cipher::Key>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !legacy_filename.eq_ignore_ascii_case(CATALOG_FILE)
                && !legacy_filename.eq_ignore_ascii_case("backup-uploads.sqlite"),
            "The legacy cache cannot use a profile catalog or backup journal filename"
        );
        anyhow::ensure!(
            Path::new(legacy_filename)
                .file_name()
                .is_some_and(|name| name == legacy_filename),
            "Use a plain legacy cache filename"
        );
        std::fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        }
        let mut connection = crate::cache_cipher::open(
            key.as_deref(),
            &root.join(CATALOG_FILE),
            rusqlite::OpenFlags::default(),
        )?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let application: i32 = tx.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if application == 0 {
            let objects: i64 =
                tx.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
            anyhow::ensure!(
                objects == 0,
                "The profile catalog filename is occupied by another database. Keep that file and choose a different name for it before reopening Shep."
            );
            tx.execute_batch("CREATE TABLE profiles(id TEXT PRIMARY KEY,name TEXT NOT NULL,ready INTEGER NOT NULL DEFAULT 0);
                CREATE TABLE selection(singleton INTEGER PRIMARY KEY CHECK(singleton=1),revision INTEGER NOT NULL,active TEXT NOT NULL REFERENCES profiles(id));
                INSERT INTO profiles VALUES('legacy','My mail',1);
                INSERT INTO selection VALUES(1,0,'legacy');")?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", 1)?;
        } else {
            anyhow::ensure!(
                application == APPLICATION_ID,
                "This is not a Shep profile catalog"
            );
            let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            anyhow::ensure!(version == 1, "Update Shep to open this profile catalog");
        }
        tx.commit()?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON;",
        )?;
        Ok(Self {
            root,
            legacy_filename: legacy_filename.into(),
            worker: Arc::new(Worker::named(connection, "shep-profiles")?),
            key,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn path(&self, id: Id) -> PathBuf {
        match id {
            Id::Legacy => self.root.join(&self.legacy_filename),
            Id::Imported(id) => self
                .root
                .join("profiles")
                .join(id.to_string())
                .join("shep.sqlite"),
        }
    }

    pub async fn page(&self, offset: u64) -> anyhow::Result<Page> {
        self.worker.run(move |c| {
            let tx = c.transaction()?;
            let (revision, active) = selection(&tx)?;
            let total: i64 = tx.query_row("SELECT count(*) FROM profiles", [], |r| r.get(0))?;
            let mut statement = tx.prepare("SELECT id,name,ready FROM profiles ORDER BY name COLLATE NOCASE,id LIMIT 50 OFFSET ?")?;
            let mut rows = statement.query([i64::try_from(offset)?])?;
            let mut profiles = Vec::new();
            while let Some(row) = rows.next()? {
                profiles.push(Profile { id: Id::parse(&row.get::<_, String>(0)?)?, name: row.get(1)?, ready: row.get(2)? });
            }
            Ok(Page { offset, revision, active, total: u64::try_from(total)?, rows: profiles })
        }).await
    }

    pub async fn active(&self) -> anyhow::Result<(u64, Profile)> {
        self.worker
            .run(|c| {
                let tx = c.transaction()?;
                let (revision, id) = selection(&tx)?;
                Ok((revision, profile(&tx, id)?))
            })
            .await
    }

    pub(crate) async fn open_active(
        self,
        demo: bool,
    ) -> anyhow::Result<(crate::store::Store, Session)> {
        let (_, profile) = self.active().await?;
        anyhow::ensure!(
            profile.ready,
            "Finish importing the selected profile before opening it"
        );
        let path = self.path(profile.id);
        let id = profile.id;
        let key = self.key.clone();
        let store = tokio::task::spawn_blocking(move || {
            if let Id::Imported(id) = id {
                verify_import_marker(&path, id, key.as_deref())?;
            }
            #[cfg(feature = "test-support")]
            if demo {
                return crate::test_support::workspace::open(Some(&path));
            }
            let _ = demo;
            match key {
                Some(key) => crate::store::Store::open_encrypted(path, key),
                None => crate::store::Store::open(path),
            }
        })
        .await??;
        Ok((
            store,
            Session {
                catalog: self,
                current: id,
            },
        ))
    }

    pub(crate) async fn reserve(&self, id: uuid::Uuid, name: String) -> anyhow::Result<Profile> {
        anyhow::ensure!(!id.is_nil(), "A new profile needs a fresh local identity");
        let name = name_checked(name)?;
        self.worker
            .run(move |c| {
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let inserted = tx.execute(
                    "INSERT OR IGNORE INTO profiles(id,name,ready) VALUES(?,?,0)",
                    params![id.to_string(), name],
                )?;
                if inserted > 0 {
                    bump(&tx)?;
                }
                let value = profile(&tx, Id::Imported(id))?;
                tx.commit()?;
                Ok(value)
            })
            .await
    }

    /// Called only after the exact validated copy, imported-operation fences and
    /// marker are durable. It cannot adopt an arbitrary selected database file.
    pub(crate) async fn finish(&self, id: uuid::Uuid) -> anyhow::Result<Profile> {
        let path = self.path(Id::Imported(id));
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || verify_import_marker(&path, id, key.as_deref()))
            .await??;
        self.worker
            .run(move |c| {
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                anyhow::ensure!(
                    tx.query_row(
                        "SELECT ready FROM profiles WHERE id=?",
                        [id.to_string()],
                        |r| r.get::<_, bool>(0)
                    )
                    .optional()?
                    .is_some(),
                    "This import no longer exists"
                );
                if tx.execute(
                    "UPDATE profiles SET ready=1 WHERE id=? AND ready=0",
                    [id.to_string()],
                )? > 0
                {
                    bump(&tx)?;
                }
                let value = profile(&tx, Id::Imported(id))?;
                tx.commit()?;
                Ok(value)
            })
            .await
    }

    pub async fn rename(&self, id: Id, revision: u64, name: String) -> anyhow::Result<()> {
        let name = name_checked(name)?;
        self.worker
            .run(move |c| {
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                expected(&tx, revision)?;
                anyhow::ensure!(
                    tx.execute(
                        "UPDATE profiles SET name=? WHERE id=?",
                        params![name, id.key()]
                    )? == 1,
                    "This profile no longer exists"
                );
                bump(&tx)?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    pub async fn activate(&self, id: Id, revision: u64) -> anyhow::Result<()> {
        let path = self.path(id);
        let key = self.key.clone();
        tokio::task::spawn_blocking(move || match id {
            Id::Imported(id) => verify_import_marker(&path, id, key.as_deref()).map(|_| ()),
            Id::Legacy => {
                anyhow::ensure!(path.is_file(), "The original profile database is missing. Restore it before opening this profile.");
                Ok(())
            }
        }).await??;
        self.worker
            .run(move |c| {
                let tx = c.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                expected(&tx, revision)?;
                anyhow::ensure!(
                    profile(&tx, id)?.ready,
                    "Finish importing this profile before opening it"
                );
                tx.execute(
                    "UPDATE selection SET active=? WHERE singleton=1",
                    [id.key()],
                )?;
                bump(&tx)?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// A completed copy may outlive its registration reply or a process crash.
    /// Discover only canonical, device-owned profile directories. Each metadata
    /// record crosses a bounded channel; never collect every database in memory.
    pub async fn recover_imports(&self) -> anyhow::Result<Recovery> {
        let directory = self.root.join("profiles");
        let key = self.key.clone();
        let (output, mut input) = tokio::sync::mpsc::channel(8);
        let scan = tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let entries = match std::fs::read_dir(directory) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error.into()),
            };
            for entry in entries {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    continue;
                }
                let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                    continue;
                };
                let Ok(Id::Imported(id)) = Id::parse(&name) else {
                    continue;
                };
                let path = entry.path().join("shep.sqlite");
                // An empty directory can remain if the process stopped before
                // the no-overwrite file publication. It is not a profile.
                if !path.try_exists()? {
                    continue;
                }
                let item = verify_import_marker(&path, id, key.as_deref())
                    .map(|marker| (id, marker.name))
                    .map_err(|error| format!("{}: {error:#}", path.display()));
                if output.blocking_send(item).is_err() {
                    break;
                }
            }
            Ok(())
        });
        let mut recovery = Recovery::default();
        while let Some(item) = input.recv().await {
            match item {
                Ok((id, name)) => {
                    let result = async {
                        self.reserve(id, name).await?;
                        self.finish(id).await
                    }
                    .await;
                    match result {
                        Ok(_) => recovery.found += 1,
                        Err(error) => recovery.warn(error),
                    }
                }
                Err(error) => recovery.warn(error),
            }
        }
        scan.await??;
        Ok(recovery)
    }
}

pub(crate) fn name_checked(name: String) -> anyhow::Result<String> {
    let name = name.trim();
    anyhow::ensure!(
        !name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control),
        "Give this profile a name of 1–80 characters"
    );
    Ok(name.into())
}
fn selection(c: &Connection) -> anyhow::Result<(u64, Id)> {
    let (revision, active): (i64, String) = c.query_row(
        "SELECT revision,active FROM selection WHERE singleton=1",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    Ok((u64::try_from(revision)?, Id::parse(&active)?))
}
fn expected(c: &Connection, revision: u64) -> anyhow::Result<()> {
    anyhow::ensure!(
        selection(c)?.0 == revision,
        "Profiles changed in another window. Refresh the list and try again."
    );
    Ok(())
}
fn bump(c: &Connection) -> anyhow::Result<()> {
    let next = selection(c)?
        .0
        .checked_add(1)
        .context("Profile revision exhausted")?;
    c.execute(
        "UPDATE selection SET revision=? WHERE singleton=1",
        [i64::try_from(next)?],
    )?;
    Ok(())
}
fn profile(c: &Connection, id: Id) -> anyhow::Result<Profile> {
    let (name, ready) = c
        .query_row(
            "SELECT name,ready FROM profiles WHERE id=?",
            [id.key()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .context("This profile no longer exists")?;
    Ok(Profile { id, name, ready })
}
fn verify_import_marker(
    path: &Path,
    id: uuid::Uuid,
    key: Option<&crate::cache_cipher::Key>,
) -> anyhow::Result<ImportMarker> {
    anyhow::ensure!(
        path.parent()
            .context("The profile has no directory")?
            .symlink_metadata()?
            .file_type()
            .is_dir(),
        "The imported profile must use an ordinary directory"
    );
    anyhow::ensure!(
        path.symlink_metadata()?.file_type().is_file(),
        "The imported database must be an ordinary file in its profile directory"
    );
    let connection =
        crate::cache_cipher::open(key, path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.set_db_config(
        rusqlite::config::DbConfig::SQLITE_DBCONFIG_TRUSTED_SCHEMA,
        false,
    )?;
    let value: String = connection.query_row(
        "SELECT value FROM kv WHERE key=?",
        [IMPORT_MARKER_KEY],
        |r| r.get(0),
    )?;
    let marker: ImportMarker = serde_json::from_str(&value)?;
    anyhow::ensure!(
        marker.version == 1 && marker.local_profile == id,
        "This file is not the prepared import for this profile"
    );
    Ok(marker)
}

#[cfg(test)]
mod tests;

mod export_guard;
