//! Saved CONDSTORE folder states. A state promises that every cached message
//! in the folder shows the server's flags as of `modseq`, or changed after it,
//! so a later check can fetch only newer flag changes.
use super::*;
use crate::providers::mail::condstore::{FolderState, Resume};

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS folder_modseqs(
            account TEXT NOT NULL, folder TEXT NOT NULL,
            validity INTEGER NOT NULL, modseq INTEGER NOT NULL,
            PRIMARY KEY(account,folder));",
    )?;
    Ok(())
}

pub(super) fn load(c: &Connection, account: &str) -> anyhow::Result<Resume> {
    c.prepare_cached("SELECT folder,validity,modseq FROM folder_modseqs WHERE account=?")?
        .query_map([account], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .map(|row| {
            let (folder, validity, modseq) = row?;
            let modseq = u64::try_from(modseq).context("Invalid saved mailbox state")?;
            Ok((folder, FolderState { validity, modseq }))
        })
        .collect()
}

/// Saves a folder's state from a check that began at `epoch`. A local write
/// that outranks that check's listings may have hidden a server change older
/// than the new state, so the previous state is kept until a later check.
pub(super) fn save(
    c: &Connection,
    account: &str,
    folder: &str,
    state: Option<FolderState>,
    epoch: Option<SyncEpoch>,
) -> anyhow::Result<()> {
    connections::allow(c, ConnectionKind::Account, account)?;
    let Some(state) = state else {
        c.execute(
            "DELETE FROM folder_modseqs WHERE account=? AND folder=?",
            params![account, folder],
        )?;
        return Ok(());
    };
    if let Some(epoch) = epoch {
        let prefix = format!("{account}:{folder}:");
        let entries = write_ledger::entries(c, account)?;
        if write_ledger::holds_folder_state(&entries, &prefix, epoch) {
            return Ok(());
        }
    }
    let modseq = i64::try_from(state.modseq).context("Invalid mailbox state")?;
    c.execute(
        "INSERT INTO folder_modseqs(account,folder,validity,modseq) VALUES(?,?,?,?)
         ON CONFLICT(account,folder) DO UPDATE SET validity=excluded.validity,modseq=excluded.modseq",
        params![account, folder, state.validity, modseq],
    )?;
    Ok(())
}

/// Drops states for folders the server no longer lists as selectable.
pub(super) fn retain(c: &Connection, account: &str, folders: &[String]) -> anyhow::Result<()> {
    let saved: Vec<String> = c
        .prepare_cached("SELECT folder FROM folder_modseqs WHERE account=?")?
        .query_map([account], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for folder in saved.iter().filter(|folder| !folders.contains(folder)) {
        c.execute(
            "DELETE FROM folder_modseqs WHERE account=? AND folder=?",
            params![account, folder],
        )?;
    }
    Ok(())
}

/// Forgets every state of an account whose cached flags may no longer come
/// from its server, such as after restoring a backup or removing it.
pub(super) fn forget(c: &Connection, account: &str) -> anyhow::Result<()> {
    c.execute("DELETE FROM folder_modseqs WHERE account=?", [account])?;
    Ok(())
}

impl Store {
    pub async fn folder_modseqs(&self, account: String) -> anyhow::Result<Resume> {
        self.run(move |c| load(c, &account)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INBOX: FolderState = FolderState {
        validity: 12,
        modseq: 120,
    };

    fn state(folder: &str, state: Option<FolderState>) -> MailSyncItem {
        MailSyncItem::FolderState {
            account: "work".into(),
            folder: folder.into(),
            state,
        }
    }

    async fn saved(store: &Store) -> Resume {
        store.folder_modseqs("work".into()).await.expect("load")
    }

    #[tokio::test]
    async fn saved_states_survive_reopening_and_can_be_forgotten() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("cache.sqlite");
        let store = Store::open(&path).expect("store");
        for folder in ["INBOX", "Archive"] {
            store
                .apply_sync(state(folder, Some(INBOX)))
                .await
                .expect("save");
        }
        drop(store);
        let store = Store::open(&path).expect("reopen");
        let reopened = saved(&store).await;
        assert_eq!(reopened.get("INBOX"), Some(&INBOX));
        assert_eq!(reopened.len(), 2);
        assert!(
            store
                .folder_modseqs("other".into())
                .await
                .expect("load")
                .is_empty()
        );
        store
            .apply_sync(state("INBOX", None))
            .await
            .expect("forget");
        assert_eq!(saved(&store).await.len(), 1, "NOMODSEQ forgets one folder");
        store
            .save_folders("work".into(), vec!["INBOX".into()])
            .await
            .expect("catalog");
        assert!(
            saved(&store).await.is_empty(),
            "a folder no longer listed is pruned"
        );
    }

    #[tokio::test]
    async fn an_outranking_local_write_keeps_the_previous_state() {
        let store = Store::memory().expect("store");
        let old = FolderState {
            validity: 12,
            modseq: 90,
        };
        store
            .apply_sync(state("INBOX", Some(old)))
            .await
            .expect("save");
        let epoch = store.sync_epoch().await.expect("epoch");
        let now = chrono::Utc::now().timestamp();
        store
            .run(move |c| {
                write_ledger::record_acknowledged(
                    c,
                    "work",
                    "work:INBOX:12.6",
                    WriteKind::Flags,
                    now,
                )
            })
            .await
            .expect("acknowledged write");
        for folder in ["Archive", "INBOX"] {
            store
                .apply_sync_since(state(folder, Some(INBOX)), Some(epoch))
                .await
                .expect("apply");
        }
        let held = saved(&store).await;
        assert_eq!(held.get("INBOX"), Some(&old), "the listing may be stale");
        assert_eq!(held.get("Archive"), Some(&INBOX));
        let later = store.sync_epoch().await.expect("later epoch");
        store
            .apply_sync_since(state("INBOX", Some(INBOX)), Some(later))
            .await
            .expect("advance");
        assert_eq!(
            saved(&store).await.get("INBOX"),
            Some(&INBOX),
            "a check that began after the write advances"
        );
    }

    #[tokio::test]
    async fn vanished_mail_keeps_the_same_protections_as_a_complete_listing() {
        let store = Store::memory().expect("store");
        let mail = |remote: &str| {
            parse_mail(
                "work",
                remote,
                "INBOX",
                format!("Message-ID: <{remote}@example.test>\r\nSubject: Vanished\r\n\r\nBody")
                    .into_bytes(),
                true,
                false,
            )
            .expect("mail")
        };
        let [gone, restored, pending, moved_in, other]: [StoredMail; 5] =
            ["12.1", "12.2", "12.3", "12.4", "12.5"].map(mail);
        let id = |mail: &StoredMail| mail.summary.id.clone();
        let ids = [&gone, &restored, &pending, &moved_in, &other].map(id);
        store
            .upsert(vec![
                gone.clone(),
                restored.clone(),
                pending.clone(),
                moved_in.clone(),
                other.clone(),
            ])
            .await
            .expect("cache");
        let epoch = store.sync_epoch().await.expect("epoch");
        let (restored_id, pending_id, moved_id) = (id(&restored), id(&pending), id(&moved_in));
        let now = chrono::Utc::now().timestamp();
        store
            .run(move |c| {
                c.execute("INSERT INTO restored_messages(id) VALUES(?)", [&restored_id])?;
                c.execute(
                    "INSERT INTO mail_moves(token,source_id,source_account,destination_account,cache_id,stage,data)
                     VALUES('token',?1,'work','work',?1,'Started','{}')",
                    [&pending_id],
                )?;
                write_ledger::record_acknowledged(c, "work", &moved_id, WriteKind::MovedIn, now)
            })
            .await
            .expect("protections");
        store
            .apply_sync_since(
                MailSyncItem::Vanished {
                    account: "work".into(),
                    folder: "INBOX".into(),
                    ids: ids[..4].to_vec(),
                },
                Some(epoch),
            )
            .await
            .expect("vanished");
        assert!(store.mail_metadata(id(&gone)).await.is_err(), "removed");
        for kept in [&restored, &pending, &moved_in, &other] {
            assert!(store.mail_metadata(id(kept)).await.is_ok(), "{}", id(kept));
        }
        let wrong_folder = MailSyncItem::Vanished {
            account: "work".into(),
            folder: "Archive".into(),
            ids: vec![id(&other)],
        };
        store.apply_sync(wrong_folder).await.expect("other folder");
        assert!(
            store.mail_metadata(id(&other)).await.is_ok(),
            "only the named folder"
        );
    }

    #[tokio::test]
    async fn schema_eleven_caches_gain_the_table_and_start_without_states() {
        let directory = tempfile::tempdir().expect("directory");
        let path = directory.path().join("cache.sqlite");
        let store = Store::open(&path).expect("store");
        store
            .run(|c| {
                c.execute_batch("DROP TABLE folder_modseqs; PRAGMA user_version=11;")?;
                Ok(())
            })
            .await
            .expect("downgrade");
        drop(store);
        let store = Store::open(&path).expect("upgrade");
        assert!(saved(&store).await.is_empty());
        store
            .apply_sync(state("INBOX", Some(INBOX)))
            .await
            .expect("save");
        let version: u32 = store
            .run(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
            .await
            .expect("version");
        assert_eq!(version, DATABASE_VERSION);
    }
}
