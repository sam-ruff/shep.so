use super::*;
use crate::backup::Snapshot;
use secrecy::SecretString;
use std::collections::{HashMap, HashSet};

pub(crate) struct Restored {
    pub messages: usize,
    pub credentials: Vec<(String, SecretString)>,
    pub kept_connections: usize,
}

impl Store {
    /// One additive transaction: existing mail, settings, passwords and drafts
    /// win over older copies. Keychain work starts only after this has committed.
    pub(crate) async fn restore_snapshot(&self, snapshot: Snapshot) -> anyhow::Result<Restored> {
        let snapshot = tokio::task::spawn_blocking(move || snapshot.prepare_restore()).await??;
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut accounts: Vec<Account> = get(&tx, "accounts")?;
            let mut calendars: Vec<CalendarSource> = get(&tx, "calendars")?;
            let mut slots = HashMap::new();
            for account in &accounts {
                slots.insert(account.id.clone(), (false, account.id.clone()));
                // Reserve even currently unused SMTP slots against aliases.
                slots.insert(format!("{}:smtp", account.id), (false, account.id.clone()));
            }
            for source in &calendars {
                slots.insert(source.id.clone(), (true, source.id.clone()));
            }
            let mut allowed = HashSet::new();
            let mut kept_connections = 0;
            for account in snapshot.accounts {
                connections::revive(&tx, ConnectionKind::Account, &account.id)?;
                for key in [&account.id, &format!("{}:smtp", account.id)] {
                    anyhow::ensure!(slots.get(key).is_none_or(|owner| owner == &(false, account.id.clone())), "An account in this backup conflicts with an existing credential owner. No local data was changed.");
                }
                if let Some(current) = accounts.iter().find(|current| current.id == account.id) {
                    let mut connection = account.clone();
                    connection.name.clone_from(&current.name);
                    if &connection != current {
                        kept_connections += 1;
                        continue;
                    }
                } else {
                    accounts.push(account.clone());
                }
                allowed.insert(account.id.clone());
                if account.smtp_separate_password {
                    allowed.insert(format!("{}:smtp", account.id));
                }
            }
            for source in snapshot.calendars {
                connections::revive(&tx, ConnectionKind::Calendar, &source.id)?;
                anyhow::ensure!(slots.get(&source.id).is_none_or(|owner| owner == &(true, source.id.clone())), "A calendar in this backup conflicts with an existing credential owner. No local data was changed.");
                if let Some(current) = calendars.iter().find(|current| current.id == source.id) {
                    let mut connection = source.clone();
                    connection.name.clone_from(&current.name);
                    if &connection != current {
                        kept_connections += 1;
                        continue;
                    }
                } else {
                    calendars.push(source.clone());
                }
                if source.kind == CalendarKind::CalDav {
                    allowed.insert(source.id);
                }
            }
            put(&tx, "accounts", &accounts)?;
            let prefs: Preferences = get(&tx, "preferences")?;
            if prefs.google_lifecycle.disconnected || !prefs.google_grant.access.calendar_allowed() {
                let mut archived: HashSet<String> = get(&tx, "google_archived")?;
                for source in &mut calendars {
                    if source.kind == CalendarKind::Google {
                        source.access = CalendarAccess::READ_ONLY;
                        archived.insert(source.id.clone());
                    }
                }
                put(&tx, "google_archived", &archived)?;
            }
            if !prefs.google_grant.access.calendar_write_allowed() {
                for source in &mut calendars { if source.kind == CalendarKind::Google { source.access = CalendarAccess::READ_ONLY; } }
            }
            put(&tx, "calendars", &calendars)?;
            connections::changed(&tx)?;
            let mut messages = 0;
            for message in snapshot.messages {
                let m = &message.summary;
                use rusqlite::OptionalExtension;
                let existing: Option<(String, String)> = tx.query_row(
                    "SELECT account,json_extract(data,'$.remote_id') FROM messages WHERE id=?", [&m.id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                ).optional()?;
                anyhow::ensure!(existing.as_ref().is_none_or(|(account, remote)| account == &m.account_id && remote == &m.remote_id), "A message in this backup conflicts with a different cached message. No local data was changed.");
                // A previous restore, read/flag or local move must not be undone.
                let added = tx.execute("INSERT INTO messages(id,account,folder,sender,subject,body,timestamp,unread,starred,data,raw)
                    VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(id) DO NOTHING",
                    params![m.id,m.account_id,m.folder,m.sender,m.subject,message.text,m.timestamp,m.unread,m.starred,serde_json::to_string(m)?,message.raw])?;
                if added > 0 {
                    conversations::index_message(&tx, &m.id)?;
                    tx.execute("INSERT INTO restored_messages(id) VALUES(?)", [&m.id])?;
                    messages += added;
                }
            }
            tx.commit()?;
            Ok(Restored {
                messages,
                credentials: snapshot.credentials.into_iter()
                    .filter(|(id, _)| allowed.contains(id))
                    .map(|(id, secret)| (id, SecretString::from(secret))).collect(),
                kept_connections,
            })
        }).await
    }
}
