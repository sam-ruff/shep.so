//! Query-only optimistic metadata. The same SQLite snapshot supplies filtered
//! pages, counts and confirmed rollback values; pending provider work owns no
//! read connection and these projections never change persistent mail.
use crate::operations::{SUMMARY, stored_mail, summary};
use anyhow::Result;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Edit {
    folder: Option<String>,
    unread: Option<bool>,
    starred: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Scope {
    pub folder: String,
    pub account: Option<String>,
    pub query: String,
    pub filter: String,
    pub oldest: bool,
    pub projection: BTreeMap<String, Edit>,
}

pub(crate) struct Plan {
    pub prefix: &'static str,
    pub table: &'static str,
    pub conditions: String,
    pub data: String,
    pub folder: String,
    pub account: Option<String>,
    pub filter: String,
    pub words: String,
    pub aliases: BTreeMap<String, String>,
    pub confirmed: Vec<shep_mail_core::model::Mail>,
}
impl Plan {
    pub fn new(db: &Connection, scope: Scope) -> Result<Self> {
        let Scope {
            folder,
            account,
            query,
            filter,
            projection,
            ..
        } = scope;
        let mut edits = BTreeMap::new();
        let mut confirmed = Vec::new();
        let mut aliases = BTreeMap::new();
        for (id, mut edit) in projection {
            let message = stored_mail(db, &id)?;
            if id != message.id {
                aliases.insert(id, message.id.clone());
            }
            if edit
                .folder
                .as_deref()
                .is_some_and(|f| f.eq_ignore_ascii_case("Inbox"))
            {
                edit.folder = Some("INBOX".into());
            }
            anyhow::ensure!(
                edits.insert(message.id.clone(), edit).is_none(),
                "This message identity changed. Refresh the folder and retry."
            );
            confirmed.push(message);
        }
        let projected = !edits.is_empty();
        let data = serde_json::to_string(&edits)?;
        let prefix = if projected {
            "WITH projected AS (
          SELECT rowid,id,account_id,remote_id,folder,sender,recipient,subject,preview,
          timestamp,unread,starred,attachment_count,moved FROM mail
          WHERE id NOT IN (SELECT key FROM json_each(?6))
          UNION ALL SELECT m.rowid AS rowid,m.id,m.account_id,m.remote_id,
          COALESCE(json_extract(e.value,'$.folder'),m.folder) AS folder,
          m.sender,m.recipient,m.subject,m.preview,m.timestamp,
          COALESCE(json_extract(e.value,'$.unread'),m.unread) AS unread,
          COALESCE(json_extract(e.value,'$.starred'),m.starred) AS starred,
          m.attachment_count,m.moved FROM json_each(?6) e JOIN mail m ON m.id=e.key) "
        } else {
            ""
        };
        let table = if projected { "projected" } else { "mail" };
        let folder = if folder.eq_ignore_ascii_case("Inbox") {
            "INBOX".to_owned()
        } else {
            folder
        };
        let words = query
            .split_whitespace()
            .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" AND ");
        let folder_condition = if folder == "Sent" {
            "(folder=?1 OR (account_id,folder) IN (SELECT account_id,folder FROM sent_folder_names))"
        } else {
            "folder=?1"
        };
        let conditions = format!(
            "moved=0 AND {folder_condition} AND (?2 IS NULL OR account_id=?2) AND (?3!='Unread' OR unread=1) AND (?3!='Flagged' OR starred=1) AND (?4='' OR rowid IN (SELECT rowid FROM mail_search WHERE mail_search MATCH ?4))"
        );
        Ok(Self {
            prefix,
            table,
            conditions,
            data,
            folder,
            account,
            filter,
            words,
            aliases,
            confirmed,
        })
    }
    pub fn values(&self) -> Vec<rusqlite::types::Value> {
        vec![
            self.folder.clone().into(),
            self.account
                .clone()
                .map_or(rusqlite::types::Value::Null, Into::into),
            self.filter.clone().into(),
            self.words.clone().into(),
            0.into(),
            self.data.clone().into(),
        ]
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn page(
    db: &Connection,
    folder: String,
    account: Option<String>,
    query: String,
    filter: String,
    oldest: bool,
    offset: u32,
    projection: BTreeMap<String, Edit>,
) -> Result<Value> {
    let tx = db.unchecked_transaction()?;
    let Plan {
        prefix,
        table,
        conditions,
        data,
        folder,
        account,
        filter,
        words,
        mut aliases,
        confirmed,
    } = Plan::new(
        &tx,
        Scope {
            folder,
            account,
            query,
            filter,
            oldest,
            projection,
        },
    )?;
    // Keep the unprojected indexed query unchanged. The harmless bound ?5/?6
    // references give both forms one parameter contract.
    let total: i64 = tx.query_row(
        &format!(
            "{prefix}SELECT COUNT(*) FROM {table} WHERE {conditions} AND ?5>=0 AND length(?6)>=0"
        ),
        params![folder, account, filter, words, offset, data],
        |r| r.get(0),
    )?;
    let order = if oldest { "ASC" } else { "DESC" };
    let mut rows = tx.prepare(&format!("{prefix}SELECT {SUMMARY} FROM {table} WHERE {conditions} AND length(?6)>=0 ORDER BY timestamp {order},id LIMIT 50 OFFSET ?5"))?;
    let mail = rows
        .query_map(
            params![folder, account, filter, words, offset, data],
            summary,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let unread: i64 = tx.query_row(&format!("{prefix}SELECT COUNT(*) FROM {table} WHERE moved=0 AND folder='INBOX' AND unread=1 AND length(?6)>=0"), params![folder,account,filter,words,offset,data], |r|r.get(0))?;
    let mut alias_query = tx.prepare("SELECT alias,id FROM mail_aliases WHERE id=?1")?;
    for message in mail.iter().chain(confirmed.iter()) {
        for row in alias_query.query_map([&message.id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })? {
            let (alias, id) = row?;
            aliases.insert(alias, id);
        }
    }
    let mut folder_membership: BTreeMap<String, HashSet<String>> = BTreeMap::new();
    if folder == "Sent" {
        for message in &mail {
            folder_membership
                .entry(message.account_id.clone())
                .or_default()
                .insert(message.folder.clone());
        }
    }
    Ok(
        json!({"mail":mail,"total":total,"unread":unread,"aliases":aliases,"folder_membership":folder_membership,"confirmed":confirmed}),
    )
}
