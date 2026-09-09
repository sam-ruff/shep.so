pub(crate) mod backup_history;
mod bulk;
mod folder_actions;
mod folder_projection;
mod mail_actions;
mod mail_query;
mod move_journal;
mod notifications;
mod read_moves;
use crate::model::*;
mod connections;
mod conversations;
pub use connections::{ConnectionKind, ConnectionRef, CredentialCleanup, RemovalPreview};
mod drafts;
mod google_lifecycle;
mod outgoing;
mod profile_sync;
mod restore;
mod scratch;
mod selection;
pub(crate) mod worker;
use anyhow::Context;
pub use bulk::BulkLease;
pub use conversations::{CONVERSATION_PAGE_SIZE, ConversationPage};
pub use drafts::DraftState;
pub use folder_actions::FolderLease;
use rusqlite::{Connection, params};
pub use selection::{
    MailSelectionId, SelectedMail, SelectionChange, SelectionGroup, SelectionPage,
    SelectionSnapshot,
};
use serde::{Serialize, de::DeserializeOwned};
use std::{path::Path, sync::Arc};

#[derive(Clone)]
pub struct Store(Arc<worker::Worker>, Option<Arc<crate::cache_cipher::Key>>);

pub(crate) const DATABASE_VERSION: u32 = 4;

#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub move_pending_total: usize,
    pub accounts: Vec<Account>,
    pub account_reconnect: crate::profile_sync::join::Reconnect,
    pub calendars: Vec<CalendarSource>,
    pub preferences: Preferences,
    pub preferences_revision: u64,
    pub folders: Vec<String>,
    pub account_folders: std::collections::HashMap<String, Vec<String>>,
    pub folder_trees: std::collections::HashMap<String, Arc<crate::folders::Tree>>,
    pub drafts: Vec<Draft>,
    pub drafts_revision: u64,
    pub connections_revision: u64,
    pub credential_cleanup: usize,
    pub outgoing_pending: usize,
    pub outgoing_revision: u64,
    pub outgoing_drafts: std::collections::HashSet<String>,
    pub google_archived: std::collections::HashSet<String>,
    pub removed_google_calendars: usize,
}
impl Workspace {
    pub fn folder_label<'a>(
        &'a self,
        account: Option<&str>,
        name: &'a str,
    ) -> std::borrow::Cow<'a, str> {
        if name.eq_ignore_ascii_case("INBOX") {
            return std::borrow::Cow::Borrowed("Inbox");
        }
        let node = if let Some(account) = account {
            self.folder_trees
                .get(account)
                .and_then(|tree| tree.node(name))
        } else {
            self.accounts.iter().find_map(|account| {
                self.folder_trees
                    .get(&account.id)
                    .and_then(|tree| tree.node(name))
            })
        };
        std::borrow::Cow::Borrowed(node.map_or(name, |node| node.display_path.as_str()))
    }
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
    /// The data-root owner loads the key before opening any persisted schema.
    /// This does not migrate plaintext data or obtain keys on the UI thread.
    pub fn open_encrypted(
        path: impl AsRef<Path>,
        key: Arc<crate::cache_cipher::Key>,
    ) -> anyhow::Result<Self> {
        let connection = key.open(path.as_ref(), rusqlite::OpenFlags::default())?;
        Self::from_connection_key(connection, Some(key))
    }
    /// Independent snapshot/journal owners share immutable key material, never
    /// the cache connection. The key is absent from workspace serialization.
    pub fn connection_key(&self) -> Option<Arc<crate::cache_cipher::Key>> {
        self.1.clone()
    }
    pub fn memory() -> anyhow::Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }
    fn from_connection(conn: Connection) -> anyhow::Result<Self> {
        Self::from_connection_key(conn, None)
    }
    fn from_connection_key(
        conn: Connection,
        key: Option<Arc<crate::cache_cipher::Key>>,
    ) -> anyhow::Result<Self> {
        let scratch = scratch::attach(&conn, key.as_deref())?;
        // The initializer owns the connection. On failure it closes that handle
        // before this scope removes scratch, including on Windows.
        let conn = Self::initialize_connection(conn)?;
        Ok(Self(
            Arc::new(worker::Worker::with_scratch(conn, scratch)?),
            key,
        ))
    }
    fn initialize_connection(mut conn: Connection) -> anyhow::Result<Connection> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: u32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        anyhow::ensure!(
            version <= DATABASE_VERSION,
            "This database was created by a newer Shep version. Update Shep before opening it."
        );
        conn.execute_batch("PRAGMA main.journal_mode=WAL; PRAGMA foreign_keys=ON;
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
            CREATE TABLE IF NOT EXISTS restored_messages (
                id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE);
            CREATE TABLE IF NOT EXISTS draft_attachments (
                id TEXT PRIMARY KEY, draft TEXT NOT NULL, name TEXT NOT NULL,
                media_type TEXT NOT NULL, size INTEGER NOT NULL, data BLOB NOT NULL);
            CREATE INDEX IF NOT EXISTS draft_attachment_owner ON draft_attachments(draft);
            CREATE TABLE IF NOT EXISTS draft_inline (
                attachment TEXT PRIMARY KEY REFERENCES draft_attachments(id) ON DELETE CASCADE,
                content_id TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS draft_sent (id TEXT PRIMARY KEY, revision INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS events(id TEXT PRIMARY KEY, source TEXT NOT NULL, start INTEGER NOT NULL, data TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS event_start ON events(start);")?;
        backup_history::schema(&conn)?;
        conversations::schema(&conn)?;
        notifications::schema(&conn)?;
        connections::schema(&conn)?;
        outgoing::schema(&conn)?;
        selection::schema(&conn)?;
        folder_projection::schema(&conn)?;
        bulk::schema(&conn)?;
        move_journal::schema(&conn)?;
        read_moves::schema(&conn)?;
        folder_actions::schema(&conn)?;
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
        if version < 3 {
            let tx = conn.transaction()?;
            import_archive_schema(&tx)?;
            tx.pragma_update(None, "user_version", DATABASE_VERSION)?;
            tx.commit()?;
        }
        if version < 4 {
            conn.pragma_update(None, "user_version", DATABASE_VERSION)?;
        }
        Ok(conn)
    }
    pub async fn run<T, F>(&self, f: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    {
        self.0.run(f).await
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
        requested: impl Into<crate::preference_edits::Write>,
    ) -> anyhow::Result<PreferenceSnapshot> {
        let mut requested = requested.into();
        crate::backup::config::capture_editor(&mut requested.value);
        requested.validate()?;
        self.update_preferences_checked(move |current| {
            if crate::backup::config::locations_changed(current, &requested) {
                crate::backup::config::validate_filesystem(&requested)?;
            }
            let previous_backups = current.clone();
            let last_backup = current.last_backup;
            let backup_ready = current.backup_ready;
            let previous_target = crate::backup::BackupTarget::from_preferences(current);
            let connection = current.google_connection_id.clone();
            let lifecycle = current.google_lifecycle;
            let grant = current.google_grant.clone();
            *current = requested.merge(current);
            // These are backend-owned metadata, not user preferences.
            current.google_connection_id = connection;
            current.google_lifecycle = lifecycle;
            current.google_grant = grant;
            if (lifecycle.disconnected || !current.google_grant.access.drive_allowed())
                && current.backup_destination == BackupDestination::GoogleDrive
            {
                current.auto_backup = false;
            }
            let same_target =
                previous_target == crate::backup::BackupTarget::from_preferences(current);
            current.last_backup = if same_target { last_backup } else { None };
            current.backup_ready = same_target
                && backup_ready
                && current.backup_format == previous_backups.backup_format;
            crate::backup::config::preserve_metadata(&previous_backups, current);
            Ok(())
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
            crate::backup::config::record(current, &target, time, ready);
        })
        .await
    }
    pub async fn record_backup_format(
        &self,
        target: crate::backup::BackupTarget,
        format: crate::backup::format::Options,
        time: i64,
        ready: bool,
    ) -> anyhow::Result<PreferenceSnapshot> {
        self.update_preferences(move |current| {
            crate::backup::config::record_format(current, &target, format, time, ready)
        })
        .await
    }
    pub async fn update_preferences<F>(&self, update: F) -> anyhow::Result<PreferenceSnapshot>
    where
        F: FnOnce(&mut Preferences) + Send + 'static,
    {
        self.update_preferences_checked(move |value| {
            update(value);
            Ok(())
        })
        .await
    }
    async fn update_preferences_checked<F>(&self, update: F) -> anyhow::Result<PreferenceSnapshot>
    where
        F: FnOnce(&mut Preferences) -> anyhow::Result<()> + Send + 'static,
    {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut value: Preferences = get(&tx, "preferences")?;
            let before = value.clone();
            update(&mut value)?;
            value.validate()?;
            put(&tx, "preferences", &value)?;
            let revision = get(&tx, "preferences_revision")?;
            profile_sync::state::record_native_preferences(&tx, &before, &value, revision)?;
            tx.commit()?;
            Ok(PreferenceSnapshot { revision, value })
        })
        .await
    }
    pub async fn workspace(&self) -> anyhow::Result<Workspace> {
        self.run(|c| {
            folder_actions::prepare_local_catalogs(c)?;
            let mut folders = vec![
                "INBOX".into(),
                "Archive".into(),
                "Sent".into(),
                "Trash".into(),
            ];
            let mut stmt = c.prepare("SELECT DISTINCT folder FROM recovered_mail ORDER BY folder")?;
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
            let mut catalogs: std::collections::HashMap<String, Vec<crate::folders::Mailbox>> = get(c, "folder_catalogs")?;
            let catalog_selection: std::collections::HashMap<_, std::collections::HashMap<_,bool>> = catalogs.iter().map(|(account,catalog)| {
                let mut selection=std::collections::HashMap::new();
                for folder in catalog {
                    *selection.entry(folder.name.as_str()).or_default() |= folder.selectable;
                    *selection.entry(folder.path()).or_default() |= folder.selectable;
                }
                (account.as_str(),selection)
            }).collect();
            let mut seen: std::collections::HashMap<_,std::collections::HashSet<_>> = account_folders.iter().map(|(account,names)|(account.clone(),names.iter().cloned().collect())).collect();
            for pair in c
                .prepare("SELECT DISTINCT account,folder FROM recovered_mail ORDER BY folder")?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            {
                let (account, folder) = pair?;
                if catalog_selection.get(account.as_str()).and_then(|catalog|catalog.get(folder.as_str())) == Some(&false) {
                    continue;
                }
                if seen.entry(account.clone()).or_default().insert(folder.clone()) {
                    account_folders.entry(account).or_default().push(folder);
                }
            }
            for (account, names) in &account_folders {
                let catalog = catalogs.entry(account.clone()).or_default();
                let mut known: std::collections::HashSet<_> = catalog.iter().map(|folder|folder.name.clone()).collect();
                for name in names {
                    if known.insert(name.clone()) {
                        catalog.push(crate::folders::Mailbox::flat(name.clone()));
                    }
                }
            }
            let folder_trees = catalogs.into_iter().map(|(account, catalog)| (account, Arc::new(crate::folders::Tree::new(&catalog)))).collect();
            let drafts = drafts::snapshot(c)?;
            Ok(Workspace {
                accounts: get(c, "accounts")?,
                account_reconnect: get(c,crate::profile_sync::join::RECONNECT_KEY)?,
                calendars: get(c, "calendars")?,
                preferences: get(c, "preferences")?,
                preferences_revision: get(c, "preferences_revision")?,
                account_folders,
                folder_trees,
                folders,
                drafts: drafts.drafts,
                drafts_revision: drafts.revision,
                connections_revision: get(c, "connections_revision")?,
                outgoing_pending: outgoing::pending(c)?,
                move_pending_total: move_journal::pending(c)?,
                outgoing_revision: get(c, "outgoing_revision")?,
                google_archived: get(c, "google_archived")?,
                outgoing_drafts:c.prepare("SELECT draft FROM outgoing WHERE stage IN ('Submitting','Uncertain','Accepted')")?.query_map([],|r|r.get::<_,String>(0))?.collect::<Result<_,_>>()?,
                credential_cleanup: c.query_row(
                    "SELECT COUNT(*) FROM credential_cleanup",
                    [],
                    |r| r.get::<_, i64>(0).map(|v| v as usize),
                )?,
                removed_google_calendars: c.query_row(
                    "SELECT COUNT(*) FROM connection_tombstones WHERE google_data IS NOT NULL",
                    [],
                    |r| r.get::<_, i64>(0).map(|v| v as usize),
                )?,
            })
        })
        .await
    }
    pub async fn upsert(&self, messages: Vec<StoredMail>) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            for message in messages {
                upsert_message(&tx, &message)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn query(&self, query: MailQuery) -> anyhow::Result<MailPage> {
        self.query_folder_projection(query, None).await
    }
    pub(crate) async fn query_folder_projection(
        &self,
        query: MailQuery,
        projection: Option<(String, u64, Arc<crate::folder_actions::Review>)>,
    ) -> anyhow::Result<MailPage> {
        self.run(move |c| {
            let transaction = c.transaction()?;
            let c = &transaction;
            read_moves::prepare(c, &query.project_moves)?;
            let plan = mail_query::Plan::new(c, &query)?;
            let (total, unread) = plan.counts(c)?;
            let folder_count = if let Some((token, revision, review)) = &projection {
                folder_projection::capture(c, token, review, *revision, &query)?;
                Some(plan.affected_counts(c, token)?)
            } else { None };
            let columns = if query.project_moves.is_empty() && !move_journal::has_projection(c)? { "data,unread,starred,folder,account,0" } else { "data,unread,starred,folder,account,messages.pending_move" };
            let (sql, mut values) = plan.ordered(columns);
            values.push((PAGE_SIZE as i64).into());
            values.push((query.offset as i64).into());
            let mut stmt = c.prepare(&format!("{sql} LIMIT ? OFFSET ?"))?;
            let mut move_placeholders = std::collections::HashSet::new();
            let mut rows = stmt.query_map(rusqlite::params_from_iter(&values), |r| Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,bool>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,bool>(5)?)))?
                .map(|r| { let (data,unread,starred,folder,account,pending)=r?; let mut m:Mail=serde_json::from_str(&data)?; m.unread=unread;m.starred=starred;m.folder=folder;m.account_id=account;if pending { move_placeholders.insert(m.id.clone()); m.remote_id.clear(); } Ok(m) }).collect::<anyhow::Result<Vec<_>>>()?;
            let source = read_moves::source(c)?;
            let inbox_unread = c.prepare(&format!("SELECT account,COUNT(*) FROM {source} WHERE folder='INBOX' AND unread=1 GROUP BY account"))?
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize)))?
                .collect::<rusqlite::Result<_>>()?;
            let mut observed = std::collections::HashMap::new();
            let mut relocated = std::collections::HashMap::new();
            let mut statement = c.prepare(&format!("SELECT account,folder,unread FROM {source} WHERE id=?"))?;
            for id in query.observe {
                use rusqlite::OptionalExtension;
                let value = statement.query_row([&id], |row| Ok(MailMembership {
                    account: row.get(0)?, folder: row.get(1)?, unread: row.get(2)?,
                })).optional()?;
                if value.is_none() && relocated.len()<PAGE_SIZE {
                    let data:Option<String>=c.query_row("SELECT data FROM mail_moves WHERE source_id=? AND stage IN ('located','kept')",[&id],|r|r.get(0)).optional()?;
                    if let Some(data)=data {
                        let record:crate::mail_actions::journal::MoveRecord=serde_json::from_str(&data)?;
                        if let Some(mail)=record.resolved_mail() {
                            let data:Option<(String,bool,bool)>=c.query_row("SELECT data,unread,starred FROM messages WHERE id=?",[&mail.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
                            if let Some((data,unread,starred))=data {
                                let mut mail:Mail=serde_json::from_str(&data)?;
                                mail.unread=unread;mail.starred=starred;
                                relocated.insert(id.clone(),mail);
                            }
                        }
                    }
                }
                observed.insert(id, value);
            }
            anyhow::ensure!(query.observe_bulk.len() <= CHANNEL_CAPACITY, "Observe at most 32 mail operations at a time");
            let mut bulk_observed = std::collections::HashMap::new();
            for id in query.observe_bulk {
                use rusqlite::OptionalExtension;
                if let Some(undo) = c.query_row("SELECT undo_requested FROM bulk_jobs WHERE id=?",[&id],|r|r.get::<_,bool>(0)).optional()? {
                    bulk_observed.insert(id,undo);
                }
            }
            let mut move_recovery = std::collections::HashMap::new();
            for mail in &mut rows {
                if let Some(record)=move_journal::for_cache(c,&mail.id)? {
                    mail.remote_id.clear();
                    move_placeholders.insert(mail.id.clone());
                    move_recovery.insert(mail.id.clone(),record);
                }
            }
            let mut bulk_pending = std::collections::HashSet::new();
            for mail in &rows {
                let pending: bool = c.query_row("SELECT EXISTS(SELECT 1 FROM bulk_effects WHERE id=?)",[&mail.id],|r|r.get(0))?;
                if pending { bulk_pending.insert(mail.id.clone()); }
            }
            let page = MailPage { move_pending_total:move_journal::pending(c)?, relocated, move_recovery, move_placeholders, rows, total, unread, folder_count, inbox_unread, observed, bulk_pending, bulk_observed, bulk_placeholders: Default::default(), bulk_revision: get(c,"bulk_revision")? };
            drop(stmt);
            drop(statement);
            if projection.is_some() {
                read_moves::prepare(c, &[])?;
                transaction.commit()?;
            }
            Ok(page)
        }).await
    }
    pub async fn detail(&self, id: String) -> anyhow::Result<MailDetail> {
        self.run(move |c| {
            let (data, raw, unread, starred, folder): (String, Vec<u8>, bool, bool, String) = c
                .query_row(
                    "SELECT data,raw,unread,starred,folder FROM messages WHERE id=COALESCE(
                        (SELECT id FROM messages WHERE id=?1),
                        (SELECT CASE stage WHEN 'kept' THEN json_extract(data,'$.retained.id') ELSE json_extract(data,'$.receipt.current.id') END FROM mail_moves WHERE source_id=?1 AND stage IN ('located','kept')))",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )?;
            let mut summary: Mail = serde_json::from_str(&data)?;
            summary.unread = unread;
            summary.starred = starred;
            summary.folder = folder;
            move_journal::project_detail(c, &mut summary)?;
            let parsed = mailparse::parse_mail(&raw)?;
            let content = crate::email_content::extract(&parsed);
            let (body, attachments) = (content.text, content.attachments);
            let body_truncated = body.chars().count() > 32000;
            let body: String = body.chars().take(32000).collect();
            let (latest_body, replies) = crate::replies::split(&body);
            let remote_images = content
                .html
                .as_ref()
                .map(|h| h.remote_images.clone())
                .unwrap_or_default();
            Ok(MailDetail {
                html: content.html.map(Arc::new),
                latest_body,
                replies,
                summary,
                body,
                body_truncated,
                remote_images,
                attachments: Arc::new(attachments),
                reply: crate::compose::ReplyHeaders::parse(&parsed),
            })
        })
        .await
    }
    pub async fn patch_flags(
        &self,
        mail: Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            folder_actions::idle(&tx, &mail.account_id)?;
            let changed = tx.execute("UPDATE messages SET unread=COALESCE(?, unread),starred=COALESCE(?, starred) WHERE id=? AND account=? AND folder=?",
                params![changes.unread, changes.starred, mail.id, mail.account_id, mail.folder])?;
            anyhow::ensure!(changed == 1, "This message moved or was removed. Refresh the folder and try again.");
            tx.commit()?;
            Ok(())
        }).await
    }
    pub async fn flags(&self, mail: Mail) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            folder_actions::mail_idle(&tx, &mail.id)?;
            tx.execute(
                "UPDATE messages SET unread=?,starred=? WHERE id=?",
                params![mail.unread, mail.starred, mail.id],
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn move_local(&self, id: String, folder: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            folder_actions::mail_idle(&tx, &id)?;
            let changed = tx.execute(
                "UPDATE messages SET folder=? WHERE id=?",
                params![folder, id],
            )?;
            anyhow::ensure!(
                changed == 1,
                "This message was removed. Refresh the folder and try again."
            );
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn remove(&self, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            folder_actions::mail_idle(&tx, &id)?;
            tx.execute("DELETE FROM messages WHERE id=?", [id])?;
            tx.commit()?;
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
            let tx = c.transaction()?;
            let c = &tx;
            connections::allow(c, ConnectionKind::Account, &account.id)?;
            folder_actions::idle(c, &account.id)?;
            let mut accounts: Vec<Account> = get(c, "accounts")?;
            let previous = accounts.iter().find(|a| a.id == account.id).cloned();
            accounts.retain(|a| a.id != account.id);
            if !account.sent_folder.is_empty() {c.execute("INSERT INTO sent_folders(account,folder) VALUES(?,?) ON CONFLICT(account) DO UPDATE SET folder=excluded.folder",params![account.id,account.sent_folder])?;}
            else {c.execute("DELETE FROM sent_folders WHERE account=?",[&account.id])?;}
            profile_sync::join::reconnected(c, &account.id)?;
            accounts.push(account.clone());
            put(c, "accounts", &accounts)?;
            connections::changed(c)?;
            profile_sync::state::record_native_account_fields(c, &account, previous.as_ref())?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn save_folders(&self, account: String, folders: Vec<String>) -> anyhow::Result<()> {
        self.save_folder_catalog(
            account,
            folders
                .into_iter()
                .map(crate::folders::Mailbox::flat)
                .collect(),
        )
        .await
    }

    pub async fn save_folder_catalog(
        &self,
        account: String,
        catalog: Vec<crate::folders::Mailbox>,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let c = &tx;
            connections::allow(c, ConnectionKind::Account, &account)?;
            folder_actions::idle(c, &account)?;
            let mut mapping: std::collections::HashMap<String, Vec<String>> =
                get(c, "account_folders")?;
            let folders: Vec<String> = catalog
                .iter()
                .filter(|folder| folder.selectable)
                .map(|folder| folder.name.clone())
                .collect();
            use rusqlite::OptionalExtension;
            let sent: Option<String> = c
                .query_row(
                    "SELECT folder FROM sent_folders WHERE account=?",
                    [&account],
                    |r| r.get(0),
                )
                .optional()?;
            if sent.is_some_and(|folder| !folders.contains(&folder)) {
                c.execute("DELETE FROM sent_folders WHERE account=?", [&account])?;
            }
            mapping.insert(account.clone(), folders);
            put(c, "account_folders", &mapping)?;
            let mut catalogs: std::collections::HashMap<String, Vec<crate::folders::Mailbox>> =
                get(c, "folder_catalogs")?;
            catalogs.insert(account, catalog);
            put(c, "folder_catalogs", &catalogs)?;
            connections::changed(c)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn apply_sync(&self, item: MailSyncItem) -> anyhow::Result<()> {
        match item {
            MailSyncItem::InboxSyncStarted { account, epoch } => {
                self.begin_notification_sync(account, epoch).await
            }
            MailSyncItem::InboxSyncFinished { account, epoch } => {
                self.finish_notification_sync(account, epoch).await
            }
            MailSyncItem::Message(mail) => self.upsert(vec![mail]).await,
            MailSyncItem::Flags(flags) => {
                self.run(move |c| {
                    let tx = c.transaction()?;
                    for (id, unread, starred) in flags {
                        folder_actions::mail_idle(&tx, &id)?;
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
                    folder_actions::idle(&tx, &account)?;
                    let ids: Vec<(String, bool)> = tx
                        .prepare("SELECT id,EXISTS(SELECT 1 FROM restored_messages WHERE restored_messages.id=messages.id) FROM messages WHERE account=? AND folder=? AND id NOT LIKE '%:local-sent-%' AND id NOT LIKE '%:local-recovered-%' AND NOT EXISTS(SELECT 1 FROM mail_moves WHERE cache_id=messages.id)")?
                        .query_map(params![account, folder], |r| Ok((r.get(0)?, r.get(1)?)))?
                        .collect::<Result<_, _>>()?;
                    for (id, restored) in ids {
                        let live = live_ids.contains(&id);
                        if live && restored {
                            // A complete server listing confirmed this identity.
                            tx.execute("DELETE FROM restored_messages WHERE id=?", [id])?;
                        } else if !live && !restored {
                            // A backup may be the only remaining copy of deleted
                            // server mail. A sync must not erase that recovery.
                            tx.execute("DELETE FROM messages WHERE id=?", [id])?;
                        }
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await
            }
            MailSyncItem::Folders(account, folders) => self.save_folder_catalog(account, folders).await,
            MailSyncItem::SentFolder(account,folder)=>self.run(move |c| {
                let tx = c.transaction()?;
                let c = &tx;
                connections::allow(c,ConnectionKind::Account,&account)?;
                folder_actions::idle(c,&account)?;
                if let Some(folder)=folder { c.execute("INSERT INTO sent_folders(account,folder) VALUES(?,?) ON CONFLICT(account) DO UPDATE SET folder=excluded.folder",params![account,folder])?; }

                tx.commit()?; Ok(())
            }).await,
            MailSyncItem::SkippedLarge => Ok(()),
        }
    }
    pub async fn save_source(&self, source: CalendarSource) -> anyhow::Result<()> {
        self.save_sources(vec![source]).await
    }
    pub async fn save_sources(&self, sources: Vec<CalendarSource>) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let c = &tx;
            let mut current: Vec<CalendarSource> = get(c, "calendars")?;
            let prefs: Preferences = get(c, "preferences")?;
            let disconnected = prefs.google_lifecycle.disconnected
                || !prefs.google_grant.access.calendar_allowed();
            let mut archived: std::collections::HashSet<String> = get(c, "google_archived")?;
            for mut source in sources {
                if source.kind == CalendarKind::Google
                    && !prefs.google_grant.access.calendar_write_allowed()
                {
                    source.access = CalendarAccess::READ_ONLY;
                }
                if disconnected && source.kind == CalendarKind::Google {
                    source.access = CalendarAccess::READ_ONLY;
                    archived.insert(source.id.clone());
                }
                connections::revive(c, ConnectionKind::Calendar, &source.id)?;
                current.retain(|s| s.id != source.id);
                current.push(source);
            }
            put(c, "calendars", &current)?;
            put(c, "google_archived", &archived)?;
            connections::changed(c)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn refresh_google_sources(&self, sources: Vec<CalendarSource>) -> anyhow::Result<()> {
        anyhow::ensure!(
            sources.iter().all(|s| s.kind == CalendarKind::Google),
            "Invalid Google calendar list."
        );
        self.run(move |c| {
            let tx = c.transaction()?;
            let c = &tx;
            anyhow::ensure!(
                !get::<Preferences>(c, "preferences")?
                    .google_lifecycle
                    .disconnected
                    && get::<Preferences>(c, "preferences")?
                        .google_grant
                        .access
                        .calendar_allowed(),
                "Reconnect Google before refreshing calendars."
            );
            google_lifecycle::refresh_sources(c, sources)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn save_draft(&self, draft: Draft) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            drafts::save(&tx, draft)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn delete_draft(&self, id: String) -> anyhow::Result<DraftState> {
        self.run(move |c| {
            let tx = c.transaction()?;
            anyhow::ensure!(!id.is_empty() && id.len() <= 256, "Invalid draft identity.");
            let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE draft=? AND stage IN ('Submitting','Uncertain','Accepted'))", [&id], |r| r.get(0))?;
            anyhow::ensure!(!pending, "Review this message in Outbox before discarding its draft.");
            // A discarded identity is retired permanently, including revisions
            // captured by a file picker or autosave before the delete committed.
            tx.execute("INSERT INTO draft_sent(id,revision) VALUES(?,?) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision", params![id, i64::MAX])?;
            let mut drafts: Vec<Draft> = get(&tx, "drafts")?;
            drafts.retain(|d| d.id != id);
            put(&tx, "drafts", &drafts)?;
            tx.execute("DELETE FROM draft_attachments WHERE draft=?", [&id])?;
            if tx.execute("DELETE FROM outgoing WHERE draft=? AND stage IN ('Rejected','Released')", [&id])? > 0 {
                outgoing::changed(&tx)?;
            }
            drafts::changed(&tx)?;
            let state = drafts::snapshot(&tx)?;
            tx.commit()?;
            Ok(state)
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
            connections::allow(&tx, ConnectionKind::Calendar, &source)?;
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
            calendar_changed(&tx)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn save_event(&self, event: CalendarEvent) -> anyhow::Result<()> {
        self.run(move |c| {
            connections::allow(c, ConnectionKind::Calendar, &event.source_id)?;
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
            calendar_changed(&tx)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn delete_event(&self, source: String, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            tx.execute(
                "DELETE FROM events WHERE source=? AND json_extract(data, '$.id')=?",
                params![source, id],
            )?;
            calendar_changed(&tx)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn events(&self) -> anyhow::Result<Vec<CalendarEvent>> {
        Ok(self.calendar_snapshot().await?.1)
    }
    pub async fn calendar_snapshot(&self) -> anyhow::Result<(u64, Vec<CalendarEvent>)> {
        self.run(|c| {
            let events = c
                .prepare("SELECT data FROM events ORDER BY start LIMIT 5000")?
                .query_map([], |r| r.get::<_, String>(0))?
                .map(|r| Ok(serde_json::from_str(&r?)?))
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok((get(c, "calendar_revision")?, events))
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

pub(crate) fn import_archive_schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS imported_operations(
        import_id TEXT NOT NULL,kind TEXT NOT NULL,identity TEXT NOT NULL,data TEXT NOT NULL,
        PRIMARY KEY(import_id,kind,identity));",
    )?;
    Ok(())
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

pub(super) fn calendar_changed(c: &Connection) -> anyhow::Result<()> {
    let revision: u64 = get(c, "calendar_revision")?;
    put(
        c,
        "calendar_revision",
        &revision
            .checked_add(1)
            .context("Calendar revision overflow")?,
    )
}

fn upsert_message(c: &Connection, message: &StoredMail) -> anyhow::Result<()> {
    connections::allow(c, ConnectionKind::Account, &message.summary.account_id)?;
    folder_actions::idle(c, &message.summary.account_id)?;
    let m = &message.summary;
    c.execute("INSERT INTO messages(id,account,folder,sender,subject,body,timestamp,unread,starred,data,raw)
        VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
        ON CONFLICT(id) DO UPDATE SET unread=excluded.unread,starred=excluded.starred",
        params![m.id,m.account_id,m.folder,m.sender,m.subject,message.text,m.timestamp,m.unread,m.starred,serde_json::to_string(m)?,message.raw])?;
    conversations::index_message(c, &m.id)?;
    outgoing::reconcile(c, m)?;
    notifications::remember(c, message)?;
    Ok(())
}
