use super::*;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const CONVERSATION_PAGE_SIZE: usize = 20;
const HEADER_LIMIT: usize = 64 * 1024;

mod ranking;

#[derive(Debug, Clone, Default)]
pub struct ConversationPage {
    pub anchor: String,
    pub rows: Vec<Mail>,
    pub offset: usize,
    pub total: usize,
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS conversation_tokens (
        account TEXT NOT NULL, token TEXT NOT NULL, group_id TEXT NOT NULL,
        PRIMARY KEY(account, token));
        CREATE INDEX IF NOT EXISTS conversation_token_group ON conversation_tokens(account,group_id);
        CREATE TABLE IF NOT EXISTS conversation_members (
            id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
            account TEXT NOT NULL, group_id TEXT NOT NULL, logical_id TEXT NOT NULL, timestamp INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS conversation_group ON conversation_members(account,group_id,timestamp,id);")?;
    ranking::schema(c)?;
    Ok(())
}

fn token(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

fn identity(id: &str, raw: &[u8]) -> (String, BTreeSet<String>) {
    let headers = mailparse::parse_headers(&raw[..raw.len().min(HEADER_LIMIT)])
        .map(|(headers, _)| headers)
        .unwrap_or_default();
    let ids = |name: &str| -> Vec<String> {
        headers
            .iter()
            .filter(|header| header.get_key_ref().eq_ignore_ascii_case(name))
            .take(4)
            .flat_map(|header| crate::compose::message_ids(&header.get_value()))
            .take(100)
            .collect()
    };
    let logical = ids("Message-ID")
        .into_iter()
        .next()
        .map(|id| token(&id))
        .unwrap_or_else(|| token(&format!("local:{id}")));
    let mut links: BTreeSet<_> = ids("References")
        .into_iter()
        .chain(ids("In-Reply-To"))
        .take(101)
        .map(|id| token(&id))
        .collect();
    links.insert(logical.clone());
    (logical, links)
}

/// Connect references even when their parent has not arrived yet. A late parent
/// or bridge merges existing components; equal subjects never connect mail.
pub(super) fn index_message(c: &Connection, id: &str) -> anyhow::Result<()> {
    let data: Option<(String, i64, Vec<u8>)> = c
        .query_row(
            "SELECT account,timestamp,substr(raw,1,?2) FROM messages WHERE id=?1
         AND NOT EXISTS(SELECT 1 FROM conversation_members WHERE id=?1)",
            params![id, HEADER_LIMIT as i64],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((account, timestamp, raw)) = data else {
        return Ok(());
    };
    let (logical, links) = identity(id, &raw);
    let mut groups = BTreeSet::new();
    for link in &links {
        if let Some(group) = c
            .query_row(
                "SELECT group_id FROM conversation_tokens WHERE account=? AND token=?",
                params![account, link],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            groups.insert(group);
        }
    }
    let group = groups.first().cloned().unwrap_or_else(|| logical.clone());
    for old in groups.iter().filter(|old| **old != group) {
        c.execute(
            "UPDATE conversation_tokens SET group_id=? WHERE account=? AND group_id=?",
            params![group, account, old],
        )?;
        c.execute(
            "UPDATE conversation_members SET group_id=? WHERE account=? AND group_id=?",
            params![group, account, old],
        )?;
    }
    for link in links {
        c.execute("INSERT INTO conversation_tokens(account,token,group_id) VALUES(?,?,?) ON CONFLICT(account,token) DO NOTHING", params![account,link,group])?;
    }
    c.execute("INSERT INTO conversation_members(id,account,group_id,logical_id,timestamp) VALUES(?,?,?,?,?)",
        params![id,account,group,logical,timestamp])?;
    Ok(())
}

impl Store {
    /// Resume legacy indexing in small transactions after the cached workspace
    /// is visible. New sync/import writes index their own messages atomically.
    pub async fn index_conversation_batch(&self) -> anyhow::Result<bool> {
        self.run(|c| {
            let tx = c.transaction()?;
            let cursor: i64 = get(&tx, "conversation_index_cursor")?;
            let rows = tx
                .prepare(
                    "SELECT rowid,id FROM messages WHERE rowid>? AND NOT EXISTS
                (SELECT 1 FROM conversation_members WHERE id=messages.id) ORDER BY rowid LIMIT 32",
                )?
                .query_map([cursor], |r| {
                    Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            for (_, id) in &rows {
                index_message(&tx, id)?;
            }
            let next = match rows.last() {
                Some((rowid, _)) => *rowid,
                None => tx.query_row("SELECT COALESCE(MAX(rowid),0) FROM messages", [], |r| {
                    r.get(0)
                })?,
            };
            put(&tx, "conversation_index_cursor", &next)?;
            tx.commit()?;
            Ok(!rows.is_empty())
        })
        .await
    }

    pub async fn conversation(
        &self,
        anchor: String,
        requested: Option<usize>,
    ) -> anyhow::Result<ConversationPage> {
        self.conversation_around(anchor, None, requested).await
    }

    pub async fn conversation_around(
        &self,
        anchor: String,
        focus: Option<String>,
        requested: Option<usize>,
    ) -> anyhow::Result<ConversationPage> {
        self.run(move |c| {
            let tx = c.transaction()?;
            index_message(&tx, &anchor)?;
            let (account, group): (String, String) = tx.query_row(
                "SELECT account,group_id FROM conversation_members WHERE id=?",
                [&anchor],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            // Keep duplicate choice and chronological rank in the owned
            // encrypted scratch database, returning one metadata page to iced.
            let (total, position) =
                ranking::capture(&tx, &account, &group, &anchor, focus.as_deref())?;
            let last = total.saturating_sub(1) / CONVERSATION_PAGE_SIZE * CONVERSATION_PAGE_SIZE;
            let offset = requested
                .unwrap_or(position / CONVERSATION_PAGE_SIZE * CONVERSATION_PAGE_SIZE)
                .min(last);
            let rows = tx
                .prepare(ranking::PAGE)?
                .query_map(params![CONVERSATION_PAGE_SIZE as i64, offset as i64], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, bool>(1)?,
                        r.get::<_, bool>(2)?,
                        r.get::<_, String>(3)?,
                    ))
                })?
                .map(|row| {
                    let (data, unread, starred, folder) = row?;
                    let mut mail: Mail = serde_json::from_str(&data)?;
                    mail.unread = unread;
                    mail.starred = starred;
                    mail.folder = folder;
                    Ok(mail)
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            ranking::clear(&tx)?;
            tx.commit()?;
            Ok(ConversationPage {
                anchor,
                rows,
                total,
                offset,
            })
        })
        .await
    }
}
