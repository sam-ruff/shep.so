use crate::model::*;
use anyhow::Context;
use rusqlite::{Connection, params};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Clone)]
pub struct Store(Arc<Mutex<Connection>>);

#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub accounts: Vec<Account>,
    pub calendars: Vec<CalendarSource>,
    pub preferences: Preferences,
    pub preferences_revision: u64,
    pub folders: Vec<String>,
    pub account_folders: std::collections::HashMap<String, Vec<String>>,
    pub drafts: Vec<Draft>,
}

#[derive(Debug, Clone, Default)]
pub struct PreferenceSnapshot {
    pub revision: u64,
    pub value: Preferences,
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        Self::from_connection(Connection::open(path)?)
    }
    pub fn memory() -> anyhow::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }
    fn from_connection(mut conn: Connection) -> anyhow::Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY, account TEXT NOT NULL, folder TEXT NOT NULL,
                sender TEXT NOT NULL, subject TEXT NOT NULL, body TEXT NOT NULL,
                timestamp INTEGER NOT NULL, unread INTEGER NOT NULL, starred INTEGER NOT NULL,
                data TEXT NOT NULL, raw BLOB NOT NULL);
            CREATE INDEX IF NOT EXISTS mail_folder_time ON messages(folder, timestamp DESC);
            CREATE INDEX IF NOT EXISTS mail_account_folder_time ON messages(account, folder, timestamp DESC);
            CREATE INDEX IF NOT EXISTS mail_folder_unread ON messages(folder,unread);
            CREATE INDEX IF NOT EXISTS mail_account_unread ON messages(account,folder,unread);
            CREATE INDEX IF NOT EXISTS mail_flagged ON messages(starred,folder);
            CREATE INDEX IF NOT EXISTS mail_sender ON messages(sender COLLATE NOCASE,timestamp DESC);
            CREATE INDEX IF NOT EXISTS mail_subject ON messages(subject COLLATE NOCASE,timestamp DESC);
            CREATE INDEX IF NOT EXISTS mail_time ON messages(timestamp DESC);
            CREATE VIRTUAL TABLE IF NOT EXISTS mail_search USING fts5(sender, subject, body, content='messages', content_rowid='rowid');
            CREATE VIRTUAL TABLE IF NOT EXISTS mail_vocab USING fts5vocab(mail_search,'row');
            CREATE TRIGGER IF NOT EXISTS messages_ai AFTER INSERT ON messages BEGIN
                INSERT INTO mail_search(rowid,sender,subject,body) VALUES(new.rowid,new.sender,new.subject,new.body); END;
            CREATE TRIGGER IF NOT EXISTS messages_ad AFTER DELETE ON messages BEGIN
                INSERT INTO mail_search(mail_search,rowid,sender,subject,body) VALUES('delete',old.rowid,old.sender,old.subject,old.body); END;
            CREATE TRIGGER IF NOT EXISTS messages_au AFTER UPDATE OF sender,subject,body ON messages BEGIN
                INSERT INTO mail_search(mail_search,rowid,sender,subject,body) VALUES('delete',old.rowid,old.sender,old.subject,old.body);
                INSERT INTO mail_search(rowid,sender,subject,body) VALUES(new.rowid,new.sender,new.subject,new.body); END;
            CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY, source TEXT NOT NULL, start INTEGER NOT NULL, data TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS event_start ON events(start);")?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 2 {
            let tx = conn.transaction()?;
            let events = tx
                .prepare("SELECT data FROM events")?
                .query_map([], |r| r.get::<_, String>(0))?
                .map(|row| Ok(serde_json::from_str::<CalendarEvent>(&row?)?))
                .collect::<anyhow::Result<Vec<_>>>()?;
            tx.execute("DELETE FROM events", [])?;
            for event in events {
                tx.execute(
                    "INSERT INTO events VALUES(?,?,?,?)",
                    params![
                        event.key(),
                        event.source_id,
                        event.start.timestamp(),
                        serde_json::to_string(&event)?
                    ],
                )?;
            }
            tx.pragma_update(None, "user_version", 2)?;
            tx.commit()?;
        }
        Ok(Self(Arc::new(Mutex::new(conn))))
    }
    pub async fn run<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    {
        let store = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = store
                .0
                .lock()
                .map_err(|_| anyhow::anyhow!("The local database is unavailable."))?;
            f(&mut conn)
        })
        .await?
    }
    pub async fn get<T: DeserializeOwned + Default + Send + 'static>(
        &self,
        key: &str,
    ) -> anyhow::Result<T> {
        let key = key.to_string();
        self.run(move |c| get(c, &key)).await
    }
    pub async fn put<T: Serialize + Send + 'static>(
        &self,
        key: &str,
        value: T,
    ) -> anyhow::Result<()> {
        let key = key.to_string();
        self.run(move |c| {
            let tx = c.transaction()?;
            put(&tx, &key, &value)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn save_preferences(
        &self,
        requested: Preferences,
    ) -> anyhow::Result<PreferenceSnapshot> {
        self.update_preferences(move |current| {
            let last_backup = current.last_backup;
            let backup_ready = current.backup_ready;
            let previous_target = crate::backup::BackupTarget::from_preferences(current);
            let connection = current.google_connection_id.clone();
            *current = requested;
            // These are backend-owned metadata, not user preferences.
            current.google_connection_id = connection;
            let same_target =
                previous_target == crate::backup::BackupTarget::from_preferences(current);
            current.last_backup = if same_target { last_backup } else { None };
            current.backup_ready = same_target && backup_ready;
        })
        .await
    }
    pub async fn record_backup(
        &self,
        target: crate::backup::BackupTarget,
        time: i64,
        ready: bool,
    ) -> anyhow::Result<PreferenceSnapshot> {
        self.update_preferences(move |current| {
            if crate::backup::BackupTarget::from_preferences(current) == target {
                current.last_backup = Some(time);
                current.backup_ready = ready;
            }
        })
        .await
    }
    pub async fn update_preferences<F>(&self, update: F) -> anyhow::Result<PreferenceSnapshot>
    where
        F: FnOnce(&mut Preferences) + Send + 'static,
    {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut value: Preferences = get(&tx, "preferences")?;
            update(&mut value);
            value.validate()?;
            put(&tx, "preferences", &value)?;
            let revision = get(&tx, "preferences_revision")?;
            tx.commit()?;
            Ok(PreferenceSnapshot { revision, value })
        })
        .await
    }
    pub async fn workspace(&self) -> anyhow::Result<Workspace> {
        self.run(|c| {
            let mut folders = vec![
                "INBOX".into(),
                "Archive".into(),
                "Sent".into(),
                "Trash".into(),
            ];
            let mut stmt = c.prepare("SELECT DISTINCT folder FROM messages ORDER BY folder")?;
            for f in stmt.query_map([], |r| r.get::<_, String>(0))? {
                let f = f?;
                if !folders.contains(&f) {
                    folders.push(f);
                }
            }
            let saved: Vec<String> = get(c, "folders")?;
            for f in saved {
                if !folders.contains(&f) {
                    folders.push(f);
                }
            }
            let mut account_folders: std::collections::HashMap<String, Vec<String>> =
                get(c, "account_folders")?;
            for pair in c
                .prepare("SELECT DISTINCT account,folder FROM messages ORDER BY folder")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            {
                let (account, folder) = pair?;
                let folders = account_folders.entry(account).or_default();
                if !folders.contains(&folder) {
                    folders.push(folder);
                }
            }
            Ok(Workspace {
                accounts: get(c, "accounts")?,
                calendars: get(c, "calendars")?,
                preferences: get(c, "preferences")?,
                preferences_revision: get(c, "preferences_revision")?,
                account_folders,
                folders,
                drafts: get(c, "drafts")?,
            })
        })
        .await
    }
    pub async fn upsert(&self, messages: Vec<StoredMail>) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            for message in messages {
                let m = &message.summary;
                tx.execute("INSERT INTO messages(id,account,folder,sender,subject,body,timestamp,unread,starred,data,raw)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                    ON CONFLICT(id) DO UPDATE SET unread=excluded.unread,starred=excluded.starred",
                    params![m.id,m.account_id,m.folder,m.sender,m.subject,message.text,m.timestamp,m.unread,m.starred,serde_json::to_string(m)?,message.raw])?;
            }
            tx.commit()?; Ok(())
        }).await
    }
    pub async fn query(&self, query: MailQuery) -> anyhow::Result<MailPage> {
        self.run(move |c| {
            let mut filters = vec!["1=1".to_string()];
            let mut values: Vec<rusqlite::types::Value> = Vec::new();
            if let Some(account) = query.account { filters.push("account=?".into()); values.push(account.into()); }
            if !query.folder.is_empty() { filters.push("folder=?".into()); values.push(query.folder.into()); }
            if query.unread_only { filters.push("unread=1".into()); }
            if query.read_only { filters.push("unread=0".into()); }
            if query.attachments_only { filters.push("json_extract(data,'$.attachment_count')>0".into()); }
            if query.starred_only { filters.push("starred=1".into()); }
            let search = crate::fuzzy::mail_query(c, &query.search)?;
            if !search.is_empty() { filters.push("rowid IN (SELECT rowid FROM mail_search WHERE mail_search MATCH ?)".into()); values.push(search.into()); }
            let condition = filters.join(" AND ");
            let total: i64 = c.query_row(&format!("SELECT COUNT(*) FROM messages WHERE {condition}"), rusqlite::params_from_iter(&values), |r| r.get(0))?;
            let unread: i64 = c.query_row(&format!("SELECT COUNT(*) FROM messages WHERE {condition} AND unread=1"), rusqlite::params_from_iter(&values), |r| r.get(0))?;
            values.push((PAGE_SIZE as i64).into()); values.push((query.offset as i64).into());
            let order = match query.sort {
                MailSort::Newest => "timestamp DESC,id",
                MailSort::Oldest => "timestamp ASC,id",
                MailSort::Sender => "sender COLLATE NOCASE,timestamp DESC,id",
                MailSort::Subject => "subject COLLATE NOCASE,timestamp DESC,id",
            };
            let mut stmt = c.prepare(&format!("SELECT data,unread,starred,folder FROM messages WHERE {condition} ORDER BY {order} LIMIT ? OFFSET ?"))?;
            let rows = stmt.query_map(rusqlite::params_from_iter(&values), |r| Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,bool>(2)?,r.get::<_,String>(3)?)))?
                .map(|r| { let (data,unread,starred,folder)=r?; let mut m:Mail=serde_json::from_str(&data)?; m.unread=unread;m.starred=starred;m.folder=folder;Ok(m) }).collect::<anyhow::Result<Vec<_>>>()?;
            Ok(MailPage { rows, total:total as usize, unread:unread as usize })
        }).await
    }
    pub async fn detail(&self, id: String) -> anyhow::Result<MailDetail> {
        self.run(move |c| {
            let (data, raw, unread, starred, folder): (String, Vec<u8>, bool, bool, String) = c
                .query_row(
                    "SELECT data,raw,unread,starred,folder FROM messages WHERE id=?",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )?;
            let mut summary: Mail = serde_json::from_str(&data)?;
            summary.unread = unread;
            summary.starred = starred;
            summary.folder = folder;
            let parsed = mailparse::parse_mail(&raw)?;
            let (body, attachments) = crate::model::content(&parsed);
            let body_truncated = body.chars().count() > 32000;
            let body: String = body.chars().take(32000).collect();
            let (latest_body, replies) = crate::replies::split(&body);
            Ok(MailDetail {
                latest_body,
                replies,
                summary,
                body,
                body_truncated,
                remote_images: crate::remote_images::extract(&parsed),
                attachments: Arc::new(attachments),
            })
        })
        .await
    }
    pub async fn flags(&self, mail: Mail) -> anyhow::Result<()> {
        self.run(move |c| {
            c.execute(
                "UPDATE messages SET unread=?,starred=? WHERE id=?",
                params![mail.unread, mail.starred, mail.id],
            )?;
            Ok(())
        })
        .await
    }
    pub async fn move_local(&self, id: String, folder: String) -> anyhow::Result<()> {
        self.run(move |c| {
            c.execute(
                "UPDATE messages SET folder=? WHERE id=?",
                params![folder, id],
            )?;
            Ok(())
        })
        .await
    }
    pub async fn remove(&self, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            c.execute("DELETE FROM messages WHERE id=?", [id])?;
            Ok(())
        })
        .await
    }
    pub async fn raw_message(&self, id: String) -> anyhow::Result<Vec<u8>> {
        self.run(move |c| {
            Ok(c.query_row("SELECT raw FROM messages WHERE id=?", [id], |r| r.get(0))?)
        })
        .await
    }
    pub async fn known(
        &self,
        account: String,
    ) -> anyhow::Result<std::collections::HashSet<String>> {
        self.run(move |c| {
            c.prepare("SELECT id FROM messages WHERE account=?")?
                .query_map([account], |r| r.get(0))?
                .collect::<Result<_, _>>()
                .map_err(Into::into)
        })
        .await
    }
    pub async fn save_account(&self, account: Account) -> anyhow::Result<()> {
        self.run(move |c| {
            let mut accounts: Vec<Account> = get(c, "accounts")?;
            accounts.retain(|a| a.id != account.id);
            accounts.push(account);
            put(c, "accounts", &accounts)
        })
        .await
    }

    pub async fn save_folders(&self, account: String, folders: Vec<String>) -> anyhow::Result<()> {
        self.run(move |c| {
            let mut mapping: std::collections::HashMap<String, Vec<String>> =
                get(c, "account_folders")?;
            mapping.insert(account, folders);
            put(c, "account_folders", &mapping)
        })
        .await
    }
    pub async fn apply_sync(&self, item: MailSyncItem) -> anyhow::Result<()> {
        match item {
            MailSyncItem::Message(mail) => self.upsert(vec![mail]).await,
            MailSyncItem::Flags(flags) => {
                self.run(move |c| {
                    let tx = c.transaction()?;
                    for (id, unread, starred) in flags {
                        tx.execute(
                            "UPDATE messages SET unread=?,starred=? WHERE id=?",
                            params![unread, starred, id],
                        )?;
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await
            }
            MailSyncItem::Reconcile {
                account,
                folder,
                live_ids,
            } => {
                self.run(move |c| {
                    let tx = c.transaction()?;
                    let ids: Vec<String> = tx
                        .prepare("SELECT id FROM messages WHERE account=? AND folder=? AND id NOT LIKE '%:local-sent-%'")?
                        .query_map(params![account, folder], |r| r.get(0))?
                        .collect::<Result<_, _>>()?;
                    for id in ids {
                        if !live_ids.contains(&id) {
                            tx.execute("DELETE FROM messages WHERE id=?", [id])?;
                        }
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await
            }
            MailSyncItem::Folders(account, folders) => self.save_folders(account, folders).await,
            MailSyncItem::SkippedLarge => Ok(()),
        }
    }
    pub async fn save_source(&self, source: CalendarSource) -> anyhow::Result<()> {
        self.run(move |c| {
            let mut sources: Vec<CalendarSource> = get(c, "calendars")?;
            sources.retain(|s| s.id != source.id);
            sources.push(source);
            put(c, "calendars", &sources)
        })
        .await
    }
    pub async fn save_draft(&self, draft: Draft) -> anyhow::Result<()> {
        self.run(move |c| {
            let mut drafts: Vec<Draft> = get(c, "drafts")?;
            drafts.retain(|d| d.id != draft.id);
            drafts.push(draft);
            put(c, "drafts", &drafts)
        })
        .await
    }
    pub async fn delete_draft(&self, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let mut drafts: Vec<Draft> = get(c, "drafts")?;
            drafts.retain(|d| d.id != id);
            put(c, "drafts", &drafts)
        })
        .await
    }
    pub async fn replace_events(
        &self,
        source: String,
        events: Vec<CalendarEvent>,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            anyhow::ensure!(
                events.iter().all(|e| e.source_id == source),
                "Calendar sync returned events from a different calendar."
            );
            let tx = c.transaction()?;
            tx.execute("DELETE FROM events WHERE source=?", [source])?;
            for e in events {
                tx.execute(
                    "INSERT OR REPLACE INTO events VALUES(?,?,?,?)",
                    params![
                        e.key(),
                        e.source_id,
                        e.start.timestamp(),
                        serde_json::to_string(&e)?
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn save_event(&self, event: CalendarEvent) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            // Also remove the legacy key on the first edit of an existing cache.
            tx.execute(
                "DELETE FROM events WHERE source=? AND json_extract(data, '$.id')=?",
                params![event.source_id, event.id],
            )?;
            tx.execute(
                "INSERT INTO events VALUES(?,?,?,?)",
                params![
                    event.key(),
                    event.source_id,
                    event.start.timestamp(),
                    serde_json::to_string(&event)?
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn delete_event(&self, source: String, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            c.execute(
                "DELETE FROM events WHERE source=? AND json_extract(data, '$.id')=?",
                params![source, id],
            )?;
            Ok(())
        })
        .await
    }
    pub async fn events(&self) -> anyhow::Result<Vec<CalendarEvent>> {
        self.run(|c| {
            c.prepare("SELECT data FROM events ORDER BY start LIMIT 5000")?
                .query_map([], |r| r.get::<_, String>(0))?
                .map(|r| Ok(serde_json::from_str(&r?)?))
                .collect()
        })
        .await
    }
    pub async fn export(&self) -> anyhow::Result<Vec<StoredMail>> {
        self.run(|c| {
            let size: i64=c.query_row("SELECT COALESCE(SUM(length(raw)),0) FROM messages",[],|r|r.get(0))?;
            anyhow::ensure!(size <= 256*1024*1024,"This vault exceeds the current 256 MiB snapshot limit. Export a smaller vault before backing up.");
            c.prepare("SELECT data,raw,body,folder,unread,starred FROM messages")?.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?
                .map(|r|{let(data,raw,text,folder,unread,starred)=r?;let mut summary:Mail=serde_json::from_str(&data)?;summary.folder=folder;summary.unread=unread;summary.starred=starred;Ok(StoredMail{summary,raw,text})}).collect()
        }).await
    }
}

fn get<T: DeserializeOwned + Default>(c: &Connection, key: &str) -> anyhow::Result<T> {
    use rusqlite::OptionalExtension;
    c.query_row("SELECT value FROM kv WHERE key=?", [key], |r| {
        r.get::<_, String>(0)
    })
    .optional()?
    .map(|s| serde_json::from_str(&s).context("Could not read saved settings"))
    .unwrap_or_else(|| Ok(T::default()))
}
fn put<T: Serialize>(c: &Connection, key: &str, value: &T) -> anyhow::Result<()> {
    c.execute(
        "INSERT INTO kv VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
        params![key, serde_json::to_string(value)?],
    )?;
    if key == "preferences" {
        let revision: u64 = get(c, "preferences_revision")?;
        put(
            c,
            "preferences_revision",
            &revision
                .checked_add(1)
                .context("Preferences revision overflow")?,
        )?;
    }
    Ok(())
}
pub fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .take(20)
        .map(|word| format!("\"{}\"*", word.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}
