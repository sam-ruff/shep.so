//! One scope/ranking plan for inbox pages and frozen bulk-selection membership.
use super::*;
use rusqlite::types::Value;

pub(super) struct Plan {
    prefix: &'static str,
    from: String,
    condition: String,
    values: Vec<Value>,
    query: MailQuery,
    search: String,
}

impl Plan {
    pub fn new(c: &Connection, query: &MailQuery) -> anyhow::Result<Self> {
        let scope = query.search_scope();
        let query = scope.as_ref();
        let mut filters = vec!["1=1".to_string()];
        let mut values = Vec::new();
        let sent = "((folder='Sent' AND (id LIKE '%:local-sent-%' OR account NOT IN (SELECT account FROM sent_folders))) OR (account,folder) IN (SELECT account,folder FROM sent_folders))";
        let prefix = if let Some(folders) = &query.folders {
            if folders.iter().all(|f| f.folder.is_empty() && !f.sent_only) {
                if !folders.iter().any(|f| f.account.is_none()) {
                    filters.push("account IN (SELECT value FROM json_each(?))".into());
                    values.push(
                        serde_json::to_string(
                            &folders
                                .iter()
                                .filter_map(|f| f.account.as_deref())
                                .collect::<Vec<_>>(),
                        )?
                        .into(),
                    );
                }
                ""
            } else {
                // A single bound JSON value avoids SQLite parameter/expression limits.
                values.push(serde_json::to_string(folders)?.into());
                filters.push(format!("((account,folder) IN (SELECT account,folder FROM selected_folders WHERE account IS NOT NULL AND NOT sent_only) OR folder IN (SELECT folder FROM selected_folders WHERE account IS NULL AND NOT sent_only) OR ({sent} AND EXISTS(SELECT 1 FROM selected_folders s WHERE s.sent_only AND (s.account IS NULL OR s.account=messages.account))))"));
                "WITH selected_folders AS (SELECT json_extract(value,'$.account') AS account,json_extract(value,'$.folder') AS folder,json_extract(value,'$.sent_only') AS sent_only FROM json_each(?)) "
            }
        } else {
            if let Some(account) = &query.account {
                filters.push("account=?".into());
                values.push(account.clone().into());
            }
            if query.sent_only {
                filters.push(sent.into());
            } else if !query.folder.is_empty() {
                filters.push("folder=?".into());
                values.push(query.folder.clone().into());
            }
            ""
        };
        if query.unread_only {
            filters.push("unread=1".into());
        }
        if query.read_only {
            filters.push("unread=0".into());
        }
        if query.attachments_only {
            filters.push("json_extract(data,'$.attachment_count')>0".into());
        }
        if query.starred_only {
            filters.push("starred=1".into());
        }
        let search = crate::fuzzy::mail_query(c, &query.search)?;
        if search.is_empty() && !query.search.trim().is_empty() {
            filters.push("0=1".into());
        }
        let mut source = read_moves::source(c)?;
        if !query.project_moves.is_empty() {
            source = match source {
                "messages" => "read_visible_mail",
                "visible_mail" => "read_visible_bulk",
                "recovered_mail" => "read_recovered_mail",
                _ => "read_recovered_bulk",
            };
        }
        let mut from = format!("{source} AS messages");
        if !search.is_empty() {
            filters.push("mail_search.mail_search MATCH ?".into());
            values.push(search.clone().into());
            from.push_str(" JOIN mail_search ON mail_search.rowid=messages.rowid");
        }
        Ok(Self {
            prefix,
            from,
            condition: filters.join(" AND "),
            values,
            query: query.clone(),
            search,
        })
    }

    pub fn selection(c: &Connection, query: &MailQuery) -> anyhow::Result<Self> {
        let mut plan = Self::new(c, query)?;
        plan.condition
            .push_str(" AND NOT EXISTS(SELECT 1 FROM mail_moves j WHERE j.cache_id=messages.id)");
        Ok(plan)
    }

    pub fn counts(&self, c: &Connection) -> anyhow::Result<(usize, usize)> {
        let (total, unread): (i64, i64) = c.query_row(
            &format!(
                "{}SELECT COUNT(*),COALESCE(SUM(unread),0) FROM {} WHERE {}",
                self.prefix, self.from, self.condition
            ),
            rusqlite::params_from_iter(&self.values),
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((total as usize, unread as usize))
    }

    /// `columns` is a static projection supplied by our store methods, not input.
    pub fn ordered(&self, columns: &str) -> (String, Vec<Value>) {
        let mut values = self.values.clone();
        let mut from = self.from.to_owned();
        let order = match self.query.sort {
            MailSort::Relevance if !self.search.is_empty() => {
                // Score the literal match separately so rare typo alternatives
                // cannot outrank an otherwise stronger exact match.
                from.push_str(" LEFT JOIN mail_search(?, 'bm25(0.3, 2.0, 1.0)') AS exact_matches ON exact_matches.rowid=messages.rowid");
                values.insert(
                    usize::from(!self.prefix.is_empty()),
                    crate::fuzzy::literal_query(&self.query.search).into(),
                );
                // octet_length checks stored size without loading long bodies.
                // Whole-body equality makes a short exact reply rank first.
                values.push((self.query.search.trim().len().saturating_add(8) as i64).into());
                values.push(self.query.search.trim().to_owned().into());
                "CASE WHEN octet_length(messages.body)<=? THEN CASE WHEN trim(messages.body,char(9)||char(10)||char(13)||' ')=? COLLATE NOCASE THEN 0 ELSE 1 END ELSE 1 END,exact_matches.rowid IS NULL,COALESCE(exact_matches.rank,bm25(mail_search.mail_search,0.3,2.0,1.0)),timestamp DESC,id"
            }
            MailSort::Relevance | MailSort::Newest => "timestamp DESC,id",
            MailSort::Oldest => "timestamp ASC,id",
            MailSort::Sender => "messages.sender COLLATE NOCASE,timestamp DESC,id",
            MailSort::Subject => "messages.subject COLLATE NOCASE,timestamp DESC,id",
        };
        (
            format!(
                "{}SELECT {columns} FROM {from} WHERE {} ORDER BY {order}",
                self.prefix, self.condition
            ),
            values,
        )
    }
}
