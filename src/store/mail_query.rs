//! One scope/ranking plan for inbox pages and frozen bulk-selection membership.
use super::*;
use rusqlite::types::Value;

const EXACT_BODY_PRIORITY: &str = "CASE WHEN octet_length(messages.body)<=? THEN CASE WHEN trim(messages.body,char(9)||char(10)||char(13)||' ')=? COLLATE NOCASE THEN 0 ELSE 1 END ELSE 1 END";
const MATCH_TIER: &str = "CASE WHEN phrase_matches.rowid IS NOT NULL THEN 0 WHEN exact_matches.rowid IS NOT NULL THEN 1 ELSE 2 END";
const MATCH_SCORE: &str =
    "COALESCE(phrase_matches.rank,exact_matches.rank,bm25(mail_search.mail_search,0.3,2.0,1.0))";
const MATCH_JOINS: &str = " LEFT JOIN exact_matches ON exact_matches.rowid=messages.rowid LEFT JOIN phrase_matches ON phrase_matches.rowid=messages.rowid";

type PageRow = (String, bool, bool, String, String, bool);
type CountedPage = (Vec<PageRow>, usize, usize);

const RANK_ORDER: &str = "priority,tier,score,timestamp DESC,id";

struct Ranking {
    prefix: String,
    value_index: usize,
    joins: &'static str,
    tier: &'static str,
    score: &'static str,
}

pub(super) struct Plan {
    prefix: &'static str,
    from: String,
    condition: String,
    values: Vec<Value>,
    query: MailQuery,
    search: String,
    source: &'static str,
}

/// Special-use folders that logical names such as `Trash` stand for, per
/// account, so a view of `Trash` also lists `(\Trash) "Deleted Items"`.
fn role_aliases(
    c: &Connection,
    selections: &[(Option<&str>, &str)],
) -> anyhow::Result<Vec<FolderSelection>> {
    let logical = selections
        .iter()
        .any(|(_, folder)| crate::folders::FolderRole::for_logical_name(folder).is_some());
    if !logical {
        return Ok(Vec::new());
    }
    let catalogs: std::collections::HashMap<String, Vec<crate::folders::Mailbox>> =
        get(c, "folder_catalogs")?;
    let mut aliases = Vec::new();
    for (account, folder) in selections {
        for (id, catalog) in &catalogs {
            if account.is_some_and(|account| account != id) {
                continue;
            }
            for name in crate::folders::role_aliases(catalog, folder) {
                let alias = FolderSelection {
                    account: Some(id.clone()),
                    folder: name.to_owned(),
                    sent_only: false,
                };
                if !aliases.contains(&alias) {
                    aliases.push(alias);
                }
            }
        }
    }
    aliases.sort_by(|a, b| (&a.account, &a.folder).cmp(&(&b.account, &b.folder)));
    Ok(aliases)
}

fn with_role_aliases(
    c: &Connection,
    folders: &[FolderSelection],
) -> anyhow::Result<Vec<FolderSelection>> {
    let selections: Vec<_> = folders
        .iter()
        .filter(|f| !f.sent_only)
        .map(|f| (f.account.as_deref(), f.folder.as_str()))
        .collect();
    let mut expanded = folders.to_vec();
    for alias in role_aliases(c, &selections)? {
        if !expanded.contains(&alias) {
            expanded.push(alias);
        }
    }
    Ok(expanded)
}

/// A condition over `alias` and the values it binds, in order.
pub(super) type Filter = (String, Vec<Value>);

/// Hides mail whose projected or physical account has a committed removal.
///
/// The cache table only needs the removed accounts that still own mail, so a
/// completed cleanup adds no per-row work and page counts stay on covering
/// indexes. Projected views can show a pending move under its destination
/// account, so they check every removal with uncorrelated subqueries that run
/// once per statement. Run the returned condition in the same read transaction.
pub(super) fn removed_accounts_filter(
    c: &Connection,
    alias: &str,
    source: &str,
) -> anyhow::Result<Option<Filter>> {
    const REMOVED: &str = "SELECT id FROM connection_tombstones WHERE kind='account'";
    if source != "messages" {
        let removed: bool = c.query_row(&format!("SELECT EXISTS({REMOVED})"), [], |r| r.get(0))?;
        if !removed {
            return Ok(None);
        }
        let physical = format!(
            "SELECT physical.id FROM main.messages physical WHERE physical.account IN ({REMOVED})"
        );
        return Ok(Some((
            format!("{alias}.account NOT IN ({REMOVED}) AND {alias}.id NOT IN ({physical})"),
            Vec::new(),
        )));
    }
    let owners = c
        .prepare(&format!(
            "{REMOVED} AND EXISTS(SELECT 1 FROM main.messages m WHERE m.account=connection_tombstones.id)"
        ))?
        .query_map([], |r| r.get::<_, String>(0).map(Value::from))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if owners.is_empty() {
        return Ok(None);
    }
    let placeholders = vec!["?"; owners.len()].join(",");
    Ok(Some((
        format!("{alias}.account NOT IN ({placeholders})"),
        owners,
    )))
}

/// Unread Inbox mail per account across the whole cache, for launcher badges.
pub(super) fn inbox_unread_query(c: &Connection, source: &str) -> anyhow::Result<Filter> {
    let (removed, values) = removed_accounts_filter(c, "m", source)?
        .map(|(filter, values)| (format!(" AND {filter}"), values))
        .unwrap_or_default();
    Ok((
        format!(
            "SELECT account,COUNT(*) FROM {source} m WHERE folder='INBOX' AND unread=1{removed} GROUP BY account"
        ),
        values,
    ))
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
                values.push(serde_json::to_string(&with_role_aliases(c, folders)?)?.into());
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
                let aliases =
                    role_aliases(c, &[(query.account.as_deref(), query.folder.as_str())])?;
                if aliases.is_empty() {
                    filters.push("folder=?".into());
                    values.push(query.folder.clone().into());
                } else {
                    filters.push("(folder=? OR (account,folder) IN (SELECT json_extract(value,'$.account'),json_extract(value,'$.folder') FROM json_each(?)))".into());
                    values.push(query.folder.clone().into());
                    values.push(serde_json::to_string(&aliases)?.into());
                }
            }
            ""
        };
        if !query.exclude_folders.is_empty() {
            filters.push("NOT EXISTS(SELECT 1 FROM json_each(?) e WHERE json_extract(e.value,'$.account')=messages.account AND json_extract(e.value,'$.folder')=messages.folder)".into());
            values.push(serde_json::to_string(&query.exclude_folders)?.into());
        }
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
        // Bindings follow filter order; the search binding comes last.
        if let Some((removed, bindings)) = removed_accounts_filter(c, "messages", source)? {
            filters.push(removed);
            values.extend(bindings);
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
            source,
        })
    }

    pub fn selection(c: &Connection, query: &MailQuery) -> anyhow::Result<Self> {
        let mut plan = Self::new(c, query)?;
        plan.condition
            .push_str(" AND NOT EXISTS(SELECT 1 FROM mail_moves j WHERE j.cache_id=messages.id)");
        Ok(plan)
    }

    fn counts_query(&self) -> String {
        format!(
            "{}SELECT COUNT(*),COALESCE(SUM(unread),0) FROM {} WHERE {}",
            self.prefix, self.from, self.condition
        )
    }

    pub fn counts(&self, c: &Connection) -> anyhow::Result<(usize, usize)> {
        Ok(c.query_row(
            &self.counts_query(),
            rusqlite::params_from_iter(&self.values),
            |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    row.get::<_, i64>(1)? as usize,
                ))
            },
        )?)
    }

    pub fn counted_page(
        &self,
        c: &Connection,
        columns: &str,
    ) -> anyhow::Result<Option<CountedPage>> {
        if self.query.sort != MailSort::Relevance || self.search.is_empty() {
            return Ok(None);
        }
        let Ok(offset) = i64::try_from(self.query.offset) else {
            return Ok(None);
        };
        let mut values = self.values.clone();
        let ranking = self.relevance_relations(&mut values);
        values.splice(
            ranking.value_index..ranking.value_index,
            [
                (self.query.search.trim().len().saturating_add(8) as i64).into(),
                self.query.search.trim().to_owned().into(),
            ],
        );
        values.push((PAGE_SIZE as i64).into());
        values.push(offset.into());
        let prefix = if ranking.prefix.is_empty() {
            "WITH ".to_owned()
        } else {
            format!("{}, ", ranking.prefix.trim_end())
        };
        let Ranking {
            joins, tier, score, ..
        } = ranking;
        // Rank inside the FTS cursor before window aggregation. Only keys are
        // materialised; message metadata is read after the bounded page.
        let sql = format!(
            "{prefix}ranked_matches AS MATERIALIZED (
                SELECT messages.rowid AS rowid,messages.unread AS unread,
                    {EXACT_BODY_PRIORITY} AS priority,{tier} AS tier,
                    {score} AS score,timestamp,messages.id AS id
                FROM {}{joins} WHERE {}
            ), counted_matches AS (
                SELECT rowid,priority,tier,score,timestamp,id,
                    COUNT(*) OVER() AS total,COALESCE(SUM(unread) OVER(),0) AS unread_total
                FROM ranked_matches
                ORDER BY {RANK_ORDER} LIMIT ? OFFSET ?
            )
            SELECT {columns},page.total,page.unread_total
            FROM {} AS messages JOIN counted_matches AS page ON page.rowid=messages.rowid
            ORDER BY page.priority,page.tier,page.score,page.timestamp DESC,page.id",
            self.from, self.condition, self.source
        );
        let mut statement = c.prepare(&sql)?;
        let mut cursor = statement.query(rusqlite::params_from_iter(&values))?;
        let mut rows = Vec::with_capacity(PAGE_SIZE);
        let mut counts = None;
        while let Some(row) = cursor.next()? {
            counts = Some((
                usize::try_from(row.get::<_, i64>(6)?)?,
                usize::try_from(row.get::<_, i64>(7)?)?,
            ));
            rows.push((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ));
        }
        Ok(counts.map(|(total, unread)| (rows, total, unread)))
    }

    pub fn page(
        &self,
        c: &Connection,
        columns: &str,
        total: usize,
    ) -> anyhow::Result<Vec<PageRow>> {
        let expected = total.saturating_sub(self.query.offset).min(PAGE_SIZE);
        if expected == 0 {
            return Ok(Vec::new());
        }
        let read = |(sql, mut values): (String, Vec<Value>)| -> anyhow::Result<Vec<PageRow>> {
            values.push((PAGE_SIZE as i64).into());
            values.push((self.query.offset as i64).into());
            Ok(c.prepare(&format!("{sql} LIMIT ? OFFSET ?"))?
                .query_map(rusqlite::params_from_iter(&values), |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?)
        };
        read(self.ordered(columns))
    }

    pub(super) fn affected_query(&self, token: &str) -> (String, Vec<Value>) {
        let mut values = self.values.clone();
        values.push(token.to_owned().into());
        (
            format!(
                "{}SELECT COUNT(*),COALESCE(SUM(messages.unread),0) FROM {} WHERE {} AND EXISTS(SELECT 1 FROM scratch.folder_projection_folders AS targets WHERE targets.token=? AND targets.account=messages.account AND targets.folder=messages.folder)",
                self.prefix, self.from, self.condition
            ),
            values,
        )
    }

    pub fn affected_counts(&self, c: &Connection, token: &str) -> anyhow::Result<(usize, usize)> {
        let (sql, values) = self.affected_query(token);
        Ok(
            c.query_row(&sql, rusqlite::params_from_iter(&values), |row| {
                Ok((
                    row.get::<_, i64>(0)? as usize,
                    row.get::<_, i64>(1)? as usize,
                ))
            })?,
        )
    }

    /// Populate an already-indexed scratch table without an unbounded sort.
    /// Keep these keys identical to `ordered`; query/selection equivalence tests
    /// cover every sort, literal/fuzzy search, folder scope and ranking tie.
    pub fn selection_order(&self) -> (String, Vec<Value>) {
        let mut values = self.values.clone();
        let mut from = self.from.clone();
        let mut prefix = self.prefix.to_owned();
        let (priority, missing, score) =
            if self.query.sort == MailSort::Relevance && !self.search.is_empty() {
                let ranking = self.relevance_relations(&mut values);
                prefix = ranking.prefix;
                from.push_str(ranking.joins);
                values.splice(
                    ranking.value_index..ranking.value_index,
                    [
                        (self.query.search.trim().len().saturating_add(8) as i64).into(),
                        self.query.search.trim().to_owned().into(),
                    ],
                );
                (EXACT_BODY_PRIORITY, ranking.tier, ranking.score)
            } else {
                ("0", "0", "0")
            };
        let label = match self.query.sort {
            MailSort::Sender => "messages.sender",
            MailSort::Subject => "messages.subject",
            _ => "''",
        };
        (
            format!(
                "{}INSERT INTO scratch.mail_selection_order(id,priority,missing,score,label,time)
            SELECT messages.id,{priority},{missing},{score},{label},timestamp FROM {from} WHERE {}",
                prefix, self.condition
            ),
            values,
        )
    }

    fn relevance_relations(&self, values: &mut Vec<Value>) -> Ranking {
        let literal = crate::fuzzy::literal_query(&self.query.search);
        let phrase = crate::fuzzy::phrase_query(&self.query.search);
        let literal_grouped = literal
            .split(" AND ")
            .map(|term| format!("({term})"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let index = usize::from(!self.prefix.is_empty());
        if self.search == literal_grouped {
            if literal == phrase {
                return Ranking {
                    prefix: self.prefix.to_owned(),
                    value_index: index,
                    joins: "",
                    tier: "0",
                    score: "bm25(mail_search.mail_search,0.3,2.0,1.0)",
                };
            }
            let relation = "phrase_matches AS MATERIALIZED (SELECT rowid,rank FROM mail_search(?, 'bm25(0.3, 2.0, 1.0)')) ";
            values.insert(index, phrase.into());
            return Ranking {
                prefix: if self.prefix.is_empty() {
                    format!("WITH {relation}")
                } else {
                    format!("{}, {relation}", self.prefix.trim_end())
                },
                value_index: index + 1,
                joins: " LEFT JOIN phrase_matches ON phrase_matches.rowid=messages.rowid",
                tier: "CASE WHEN phrase_matches.rowid IS NULL THEN 1 ELSE 0 END",
                score: "COALESCE(phrase_matches.rank,bm25(mail_search.mail_search,0.3,2.0,1.0))",
            };
        }
        let mut bindings = vec![literal.clone().into()];
        // Materialise each FTS relation once, avoiding a full ranked scan per
        // candidate. A single term shares its literal relation with the phrase.
        let phrase_relation = if literal == phrase {
            "phrase_matches AS (SELECT rowid,rank FROM exact_matches)"
        } else {
            bindings.push(phrase.into());
            "phrase_matches AS MATERIALIZED (SELECT rowid,rank FROM mail_search(?, 'bm25(0.3, 2.0, 1.0)'))"
        };
        let relations = format!(
            "exact_matches AS MATERIALIZED (SELECT rowid,rank FROM mail_search(?, 'bm25(0.3, 2.0, 1.0)')), {phrase_relation} "
        );
        let prefix = if self.prefix.is_empty() {
            format!("WITH {relations}")
        } else {
            format!("{}, {relations}", self.prefix.trim_end())
        };
        let select_index = index + bindings.len();
        values.splice(index..index, bindings);
        Ranking {
            prefix,
            value_index: select_index,
            joins: MATCH_JOINS,
            tier: MATCH_TIER,
            score: MATCH_SCORE,
        }
    }

    /// `columns` is a static projection supplied by our store methods, not input.
    pub fn ordered(&self, columns: &str) -> (String, Vec<Value>) {
        let mut values = self.values.clone();
        let mut from = self.from.to_owned();
        let mut prefix = self.prefix.to_owned();
        let order = match self.query.sort {
            MailSort::Relevance if !self.search.is_empty() => {
                let ranking = self.relevance_relations(&mut values);
                prefix = ranking.prefix;
                from.push_str(ranking.joins);
                // octet_length checks stored size without loading long bodies.
                // Whole-body equality makes a short exact reply rank first.
                values.push((self.query.search.trim().len().saturating_add(8) as i64).into());
                values.push(self.query.search.trim().to_owned().into());
                format!(
                    "{EXACT_BODY_PRIORITY},{},{},timestamp DESC,id",
                    ranking.tier, ranking.score
                )
            }
            MailSort::Relevance | MailSort::Newest => "timestamp DESC,id".into(),
            MailSort::Oldest => "timestamp ASC,id".into(),
            MailSort::Sender => "messages.sender COLLATE NOCASE,timestamp DESC,id".into(),
            MailSort::Subject => "messages.subject COLLATE NOCASE,timestamp DESC,id".into(),
        };
        (
            format!(
                "{}SELECT {columns} FROM {from} WHERE {} ORDER BY {order}",
                prefix, self.condition
            ),
            values,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::folders::{FolderRole, Mailbox};

    fn stored(account: &str, folder: &str, subject: &str) -> StoredMail {
        parse_mail(
            account,
            subject,
            folder,
            format!("Subject: {subject}\r\n\r\nbody").into_bytes(),
            false,
            false,
        )
        .unwrap()
    }

    #[tokio::test]
    async fn logical_folder_views_include_each_accounts_special_use_folder() {
        let store = Store::memory().unwrap();
        for (account, catalog) in [
            (
                "stalwart",
                vec![
                    Mailbox::flat("INBOX".into()),
                    Mailbox {
                        role: Some(FolderRole::Trash),
                        ..Mailbox::flat("Deleted Items".into())
                    },
                    Mailbox {
                        role: Some(FolderRole::Junk),
                        ..Mailbox::flat("Junk Mail".into())
                    },
                ],
            ),
            (
                "literal",
                vec![
                    Mailbox::flat("INBOX".into()),
                    Mailbox::flat("Trash".into()),
                    Mailbox::flat("Deleted Items".into()),
                ],
            ),
        ] {
            store
                .save_folder_catalog(account.into(), catalog)
                .await
                .unwrap();
        }
        store
            .upsert(vec![
                stored("stalwart", "Deleted Items", "binned"),
                stored("stalwart", "Junk Mail", "spam"),
                stored("stalwart", "INBOX", "kept"),
                stored("literal", "Trash", "literal-binned"),
                stored("literal", "Deleted Items", "plain-folder"),
            ])
            .await
            .unwrap();
        let subjects = |page: crate::model::MailPage| {
            let mut subjects: Vec<_> = page.rows.into_iter().map(|row| row.subject).collect();
            subjects.sort();
            subjects
        };
        let trash = store
            .query(MailQuery {
                folder: "Trash".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(subjects(trash), ["binned", "literal-binned"]);
        let scoped = store
            .query(MailQuery {
                account: Some("stalwart".into()),
                folder: "Junk".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(subjects(scoped), ["spam"]);
        let combined = store
            .query(MailQuery {
                folders: Some(vec![
                    FolderSelection {
                        account: None,
                        folder: "Trash".into(),
                        sent_only: false,
                    },
                    FolderSelection {
                        account: Some("stalwart".into()),
                        folder: "INBOX".into(),
                        sent_only: false,
                    },
                ]),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(combined.total, 3);
        assert_eq!(subjects(combined), ["binned", "kept", "literal-binned"]);
        let physical = store
            .query(MailQuery {
                folder: "Deleted Items".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(subjects(physical), ["binned", "plain-folder"]);
    }

    fn plan_steps(c: &Connection, sql: &str, values: &[Value]) -> anyhow::Result<Vec<String>> {
        Ok(c.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
            .query_map(rusqlite::params_from_iter(values), |r| {
                r.get::<_, String>(3)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    #[tokio::test]
    async fn page_counts_check_removed_accounts_once_per_statement() -> anyhow::Result<()> {
        let store = Store::memory()?;
        let unread = |account: &str, subject: &str| {
            parse_mail(
                account,
                subject,
                "INBOX",
                format!("Subject: {subject}\r\n\r\nbody").into_bytes(),
                true,
                false,
            )
        };
        store
            .upsert(vec![unread("kept", "kept")?, unread("removed", "hidden")?])
            .await?;
        let inbox = MailQuery {
            folder: "INBOX".into(),
            ..Default::default()
        };
        let query = inbox.clone();
        store
            .run(move |c| {
                // A per-row removal check turned the 100,000-message page
                // count from a covering index scan into a table walk.
                let covered = |c: &Connection| -> anyhow::Result<()> {
                    let plan = Plan::new(c, &query)?;
                    let (badge, values) = inbox_unread_query(c, "messages")?;
                    for steps in [
                        plan_steps(c, &plan.counts_query(), &plan.values)?,
                        plan_steps(c, &badge, &values)?,
                    ] {
                        assert!(steps.iter().any(|s| s.contains("COVERING INDEX")), "{steps:?}");
                        assert!(!steps.iter().any(|s| s.contains("SUBQUERY")), "{steps:?}");
                    }
                    Ok(())
                };
                covered(c)?;
                let remove = |account: &str| {
                    c.execute(
                        "INSERT INTO connection_tombstones(kind,id,revision) VALUES('account',?,1)",
                        [account],
                    )
                };
                // A completed cleanup leaves only the tombstone behind.
                remove("cleaned")?;
                assert!(removed_accounts_filter(c, "messages", "messages")?.is_none());
                remove("removed")?;
                covered(c)?;
                let plan = Plan::new(c, &query)?;
                let (ordered, values) = plan.ordered("messages.id");
                let (badge, badge_values) = inbox_unread_query(c, "visible_mail")?;
                let (projected, _) = removed_accounts_filter(c, "messages", "visible_mail")?
                    .ok_or_else(|| anyhow::anyhow!("a projected view checks every removal"))?;
                for steps in [
                    plan_steps(c, &format!("{ordered} LIMIT 50"), &values)?,
                    plan_steps(c, &badge, &badge_values)?,
                    plan_steps(
                        c,
                        &format!("SELECT COUNT(*) FROM visible_mail AS messages WHERE folder='INBOX' AND {projected}"),
                        &[],
                    )?,
                ] {
                    assert!(!steps.iter().any(|s| s.contains("CORRELATED")), "{steps:?}");
                }
                Ok(())
            })
            .await?;
        let page = store.query(inbox).await?;
        assert_eq!(page.total, 1);
        assert_eq!(page.rows[0].account_id, "kept");
        assert_eq!(page.inbox_unread.len(), 1);
        assert_eq!(page.inbox_unread.get("kept"), Some(&1));
        Ok(())
    }

    #[tokio::test]
    async fn relevance_does_not_rescan_a_literal_fts_relation_per_candidate() {
        let store = Store::memory().unwrap();
        store
            .upsert(vec![
                parse_mail(
                    "fixture",
                    "1",
                    "INBOX",
                    b"Subject: Milestones\r\n\r\n17".to_vec(),
                    false,
                    false,
                )
                .unwrap(),
            ])
            .await
            .unwrap();
        store
            .run(|c| {
                for folders in [
                    None,
                    Some(vec![FolderSelection {
                        account: Some("fixture".into()),
                        folder: "INBOX".into(),
                        sent_only: false,
                    }]),
                ] {
                    let query = MailQuery {
                        search: "milestone 17".into(),
                        sort: MailSort::Relevance,
                        folders,
                        ..Default::default()
                    };
                    let plan = Plan::new(c, &query)?;
                    let (sql, values) = plan.ordered("messages.id");
                    let steps = c
                        .prepare(&format!("EXPLAIN QUERY PLAN {sql} LIMIT 50"))?
                        .query_map(rusqlite::params_from_iter(&values), |r| {
                            r.get::<_, String>(3)
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    // A virtual-table LEFT JOIN re-runs FTS filtering/ranking for
                    // every outer row. Guard that expensive plan across upgrades;
                    // search/selection tests separately verify exact ordered values.
                    assert!(
                        !steps
                            .iter()
                            .any(|s| s.contains("VIRTUAL TABLE") && s.contains("LEFT-JOIN")),
                        "{steps:?}"
                    );
                    for relation in ["exact_matches", "phrase_matches"] {
                        assert!(
                            steps
                                .iter()
                                .any(|s| s.contains(relation) && s.contains("INDEX")),
                            "{relation}: {steps:?}"
                        );
                    }
                }
                Ok(())
            })
            .await
            .unwrap();
    }
}
