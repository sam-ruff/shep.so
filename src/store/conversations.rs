use super::*;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const CONVERSATION_PAGE_SIZE: usize = 20;
const HEADER_LIMIT: usize = 64 * 1024;

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
                "SELECT account,group_id FROM conversation_members WHERE id=?", [&anchor], |r| Ok((r.get(0)?,r.get(1)?)))?;
            // Show one copy per Message-ID. The actual selected mailbox copy wins
            // so every action still targets its correct account/folder/remote UID.
            // Window functions materialize their input: never include raw MIME
            // or full body text when ranking the metadata for a conversation.
            let query = "WITH copies AS (
                SELECT m.id,m.data,m.unread,m.starred,m.folder,m.timestamp,
                    ROW_NUMBER() OVER (PARTITION BY t.logical_id ORDER BY
                    CASE WHEN m.id=?3 OR m.id=?4 THEN 0 WHEN m.folder='INBOX' THEN 1 WHEN m.folder='Sent' THEN 2 ELSE 3 END,m.id) AS copy
                FROM conversation_members t JOIN messages m ON m.id=t.id WHERE t.account=?1 AND t.group_id=?2
            ), ordered AS (
                SELECT *,ROW_NUMBER() OVER (ORDER BY timestamp,id)-1 AS position FROM copies WHERE copy=1
            ) ";
            let values = params![account,group,anchor,focus];
            let (total, position): (i64, i64) = tx.query_row(&format!("{query} SELECT COUNT(*),COALESCE(MAX(CASE WHEN id=?4 THEN position END),MAX(CASE WHEN id=?3 THEN position END),0) FROM ordered"), values,
                |r| Ok((r.get(0)?,r.get(1)?)))?;
            let last = (total as usize).saturating_sub(1) / CONVERSATION_PAGE_SIZE * CONVERSATION_PAGE_SIZE;
            let offset = requested.unwrap_or(position as usize / CONVERSATION_PAGE_SIZE * CONVERSATION_PAGE_SIZE).min(last);
            let rows = tx.prepare(&format!("{query} SELECT data,unread,starred,folder FROM ordered ORDER BY position LIMIT ?5 OFFSET ?6"))?
                .query_map(params![account,group,anchor,focus,CONVERSATION_PAGE_SIZE as i64,offset as i64], |r| Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?,r.get::<_,bool>(2)?,r.get::<_,String>(3)?)))?
                .map(|row| { let (data,unread,starred,folder)=row?; let mut mail:Mail=serde_json::from_str(&data)?; mail.unread=unread; mail.starred=starred; mail.folder=folder; Ok(mail) })
                .collect::<anyhow::Result<Vec<_>>>()?;
            tx.commit()?;
            Ok(ConversationPage { anchor, rows, total:total as usize, offset })
        }).await
    }
}
