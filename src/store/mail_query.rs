//! One scope/ranking plan for inbox pages and frozen bulk-selection membership.
use super::*;
use rusqlite::types::Value;

const EXACT_BODY_PRIORITY: &str = "CASE WHEN octet_length(messages.body)<=? THEN CASE WHEN trim(messages.body,char(9)||char(10)||char(13)||' ')=? COLLATE NOCASE THEN 0 ELSE 1 END ELSE 1 END";
const MATCH_TIER: &str = "shep_search_tier(mail_search.mail_search,?)";
const MATCH_SCORE: &str = "shep_search_score(mail_search.mail_search,?,0.3,2.0,1.0)";

type PageRow = (String, bool, bool, String, String, bool);
type CountedPage = (Vec<PageRow>, usize, usize);

const RANK_ORDER: &str = "priority,tier,score,timestamp DESC,id";

/// Relevance ranking for one FTS pass: the phrase, exact and expanded groups
/// share a single MATCH expression and `spec` tells the ranking functions
/// where each group's phrases sit and how many rows each phrase matches.
struct Ranking {
    expression: String,
    spec: String,
    tiered: bool,
}

pub(super) struct Plan {
    prefix: &'static str,
    from: String,
    condition: String,
    values: Vec<Value>,
    query: MailQuery,
    ranking: Option<Ranking>,
    source: &'static str,
    cache_written: bool,
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
        let mut snapshot = search_cache::Snapshot::open(c)?;
        let terms = crate::fuzzy::mail_terms(c, &query.search, |c, token| {
            snapshot.expansion(c, token, crate::fuzzy::expand_token)
        })?;
        let search = crate::fuzzy::mail_query_text(&terms);
        if search.is_empty() && !query.search.trim().is_empty() {
            filters.push("0=1".into());
        }
        let ranking = if query.sort == MailSort::Relevance && !search.is_empty() {
            Some(Self::ranking(c, &mut snapshot, &terms)?)
        } else {
            None
        };
        let cache_written = snapshot.written();
        snapshot.seal(c)?;
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
            values.push(search.into());
            from.push_str(" JOIN mail_search ON mail_search.rowid=messages.rowid");
        }
        Ok(Self {
            prefix,
            from,
            condition: filters.join(" AND "),
            values,
            query: query.clone(),
            ranking,
            source,
            cache_written,
        })
    }

    /// Statistics were measured for this plan; the enclosing transaction must
    /// commit for later queries to reuse them.
    pub fn fills_search_cache(&self) -> bool {
        self.cache_written
    }

    /// Groups in FTS5 phrase order: every exact term, then the expanded
    /// alternatives unless they repeat the exact terms. The ranking functions
    /// read the exact terms occurring adjacently as the whole phrase, so the
    /// phrase tier costs no extra FTS cursor.
    fn ranking(
        c: &Connection,
        snapshot: &mut search_cache::Snapshot,
        terms: &[crate::fuzzy::MailTerm],
    ) -> anyhow::Result<Ranking> {
        let tokens: Vec<&str> = terms.iter().map(|term| term.token.as_str()).collect();
        let literal: Vec<String> = tokens.iter().map(|token| format!("\"{token}\"")).collect();
        let expanded: Vec<String> = terms
            .iter()
            .flat_map(|term| term.alternatives.iter().cloned())
            .collect();
        let phrase = if tokens.len() > 1 {
            Some(snapshot.phrase_docs(c, &format!("\"{}\"", tokens.join(" ")))?)
        } else {
            None
        };
        let mut groups: Vec<(String, Vec<String>)> = vec![(literal.join(" AND "), literal.clone())];
        if expanded != literal {
            groups.push((crate::fuzzy::mail_query_text(terms), expanded));
        }
        let mut ends = Vec::with_capacity(groups.len());
        let mut docs = Vec::new();
        for (_, phrases) in &groups {
            for phrase in phrases {
                docs.push(snapshot.phrase_docs(c, phrase)?);
            }
            ends.push(docs.len());
        }
        let expression = if groups.len() == 1 {
            groups.swap_remove(0).0
        } else {
            groups
                .iter()
                .map(|(group, _)| format!("({group})"))
                .collect::<Vec<_>>()
                .join(" OR ")
        };
        Ok(Ranking {
            expression,
            tiered: phrase.is_some() || ends.len() > 1,
            spec: search_rank::spec(phrase, &ends, &docs),
        })
    }

    pub fn selection(c: &Connection, query: &MailQuery) -> anyhow::Result<Self> {
        let mut plan = Self::new(c, query)?;
        plan.condition
            .push_str(" AND NOT EXISTS(SELECT 1 FROM mail_moves j WHERE j.cache_id=messages.id)");
        Ok(plan)
    }

    pub fn counts(&self, c: &Connection) -> anyhow::Result<(usize, usize)> {
        Ok(c.query_row(
            &format!(
                "{}SELECT COUNT(*),COALESCE(SUM(unread),0) FROM {} WHERE {}",
                self.prefix, self.from, self.condition
            ),
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
        let Some((sql, values)) = self.counted_page_query(columns) else {
            return Ok(None);
        };
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

    /// The ranked page and its totals in one statement; `None` when the sort
    /// or search does not rank.
    pub(super) fn counted_page_query(&self, columns: &str) -> Option<(String, Vec<Value>)> {
        let ranking = self.ranking.as_ref()?;
        let offset = i64::try_from(self.query.offset).ok()?;
        let mut values = self.ranked_values(ranking, true);
        values.push((PAGE_SIZE as i64).into());
        values.push(offset.into());
        let prefix = if self.prefix.is_empty() {
            "WITH ".to_owned()
        } else {
            format!("{}, ", self.prefix.trim_end())
        };
        let tier = Self::tier(ranking);
        // Rank inside the FTS cursor before window aggregation. Only keys are
        // materialised; message metadata is read after the bounded page.
        let sql = format!(
            "{prefix}ranked_matches AS MATERIALIZED (
                SELECT messages.rowid AS rowid,messages.unread AS unread,
                    {EXACT_BODY_PRIORITY} AS priority,{tier} AS tier,
                    {MATCH_SCORE} AS score,timestamp,messages.id AS id
                FROM {} WHERE {}
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
        Some((sql, values))
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
        let (values, priority, missing, score) = match &self.ranking {
            Some(ranking) => (
                self.ranked_values(ranking, true),
                EXACT_BODY_PRIORITY,
                Self::tier(ranking),
                MATCH_SCORE,
            ),
            None => (self.values.clone(), "0", "0", "0"),
        };
        let label = match self.query.sort {
            MailSort::Sender => "messages.sender",
            MailSort::Subject => "messages.subject",
            _ => "''",
        };
        (
            format!(
                "{}INSERT INTO scratch.mail_selection_order(id,priority,missing,score,label,time)
            SELECT messages.id,{priority},{missing},{score},{label},timestamp FROM {} WHERE {}",
                self.prefix, self.from, self.condition
            ),
            values,
        )
    }

    fn tier(ranking: &Ranking) -> &'static str {
        if ranking.tiered { MATCH_TIER } else { "0" }
    }

    /// Bindings for a ranking statement: the combined MATCH expression replaces
    /// the expanded one, and the priority/spec bindings sit before the scope
    /// bindings when the ranking columns precede WHERE, after them otherwise.
    fn ranked_values(&self, ranking: &Ranking, rank_first: bool) -> Vec<Value> {
        let mut scope = self.values.clone();
        if let Some(last) = scope.last_mut() {
            *last = ranking.expression.clone().into();
        }
        let query = self.query.search.trim();
        // octet_length checks stored size without loading long bodies.
        // Whole-body equality makes a short exact reply rank first.
        let mut rank: Vec<Value> = vec![
            (query.len().saturating_add(8) as i64).into(),
            query.to_owned().into(),
            ranking.spec.clone().into(),
        ];
        if ranking.tiered {
            rank.push(ranking.spec.clone().into());
        }
        let prefixed = usize::from(!self.prefix.is_empty());
        let tail = scope.split_off(prefixed);
        if rank_first {
            scope.extend(rank);
            scope.extend(tail);
        } else {
            scope.extend(tail);
            scope.extend(rank);
        }
        scope
    }

    /// `columns` is a static projection supplied by our store methods, not input.
    pub fn ordered(&self, columns: &str) -> (String, Vec<Value>) {
        let (values, order) = match (&self.ranking, self.query.sort) {
            (Some(ranking), _) => (
                self.ranked_values(ranking, false),
                format!(
                    "{EXACT_BODY_PRIORITY},{},{MATCH_SCORE},timestamp DESC,id",
                    Self::tier(ranking)
                ),
            ),
            (None, MailSort::Relevance | MailSort::Newest) => {
                (self.values.clone(), "timestamp DESC,id".into())
            }
            (None, MailSort::Oldest) => (self.values.clone(), "timestamp ASC,id".into()),
            (None, MailSort::Sender) => (
                self.values.clone(),
                "messages.sender COLLATE NOCASE,timestamp DESC,id".into(),
            ),
            (None, MailSort::Subject) => (
                self.values.clone(),
                "messages.subject COLLATE NOCASE,timestamp DESC,id".into(),
            ),
        };
        (
            format!(
                "{}SELECT {columns} FROM {} WHERE {} ORDER BY {order}",
                self.prefix, self.from, self.condition
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

    fn fixture_store() -> Store {
        let store = Store::memory().unwrap();
        let mails = [
            "Architecture plans for milestone 17",
            "Architecture planning for milestone 170",
            "Milestones",
        ]
        .into_iter()
        .enumerate()
        .map(|(index, body)| {
            parse_mail(
                "fixture",
                &index.to_string(),
                "INBOX",
                format!("Subject: Notes\r\n\r\n{body}").into_bytes(),
                false,
                false,
            )
            .unwrap()
        })
        .collect();
        futures::executor::block_on(store.upsert(mails)).unwrap();
        store
    }

    #[tokio::test]
    async fn relevance_ranks_multi_term_searches_in_one_fts_pass() {
        let store = fixture_store();
        store
            .run(|c| {
                for search in [
                    "milestone 17",
                    "milestnoe 17",
                    "architecture plans milestone 17",
                ] {
                    for folders in [
                        None,
                        Some(vec![FolderSelection {
                            account: Some("fixture".into()),
                            folder: "INBOX".into(),
                            sent_only: false,
                        }]),
                    ] {
                        let query = MailQuery {
                            search: search.into(),
                            sort: MailSort::Relevance,
                            folders,
                            ..Default::default()
                        };
                        let plan = Plan::new(c, &query)?;
                        let (page, page_values) = plan
                            .counted_page_query("messages.id")
                            .expect("relevance search pages inside SQLite");
                        let (ordered, ordered_values) = plan.ordered("messages.id");
                        let (selection, selection_values) = plan.selection_order();
                        for (label, sql, values) in [
                            ("page", page, page_values),
                            ("ordered", format!("{ordered} LIMIT 50"), ordered_values),
                            ("selection", selection, selection_values),
                        ] {
                            let steps = plan_steps(c, &sql, &values)?;
                            let joined = steps.join("\n");
                            // One FTS cursor ranks every candidate. A relation
                            // per tier would rescan the index and join through
                            // automatic indexes; guard that across upgrades.
                            let fts: Vec<_> = steps
                                .iter()
                                .filter(|step| step.contains("mail_search"))
                                .collect();
                            assert_eq!(fts.len(), 1, "{search} {label}: {joined}");
                            assert!(
                                fts[0].starts_with("SCAN mail_search VIRTUAL TABLE"),
                                "{search} {label}: {joined}"
                            );
                            for forbidden in ["AUTOMATIC", "LEFT-JOIN"] {
                                assert!(!joined.contains(forbidden), "{search} {label}: {joined}");
                            }
                            // Only the page statement materialises anything: the
                            // bounded set of ranked keys for the window counts.
                            let materialised = steps
                                .iter()
                                .filter(|step| step.contains("MATERIALIZE"))
                                .count();
                            assert_eq!(
                                materialised,
                                usize::from(label == "page"),
                                "{search} {label}: {joined}"
                            );
                            if label == "selection" {
                                assert!(!joined.contains("TEMP B-TREE"), "{search}: {joined}");
                            }
                            c.prepare(&sql)?
                                .query_map(rusqlite::params_from_iter(&values), |_| Ok(()))?
                                .count();
                        }
                        c.execute("DELETE FROM scratch.mail_selection_order", [])?;
                    }
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn ranking_spec_lists_phrase_exact_and_expanded_groups_once() {
        let store = fixture_store();
        store
            .run(|c| {
                let spec = |search: &str| -> anyhow::Result<(String, String, bool)> {
                    let plan = Plan::new(
                        c,
                        &MailQuery {
                            search: search.into(),
                            sort: MailSort::Relevance,
                            ..Default::default()
                        },
                    )?;
                    let ranking = plan.ranking.expect("relevance search ranks");
                    Ok((ranking.expression, ranking.spec, ranking.tiered))
                };
                assert_eq!(
                    spec("architecture 17")?,
                    (
                        "\"architecture\" AND \"17\"".into(),
                        "0|2|2,1".into(),
                        true
                    ),
                    "exact terms carry the phrase tier without a second group"
                );
                assert_eq!(
                    spec("17")?,
                    ("\"17\"".into(), "-|1|1".into(), false),
                    "a single exact term ranks in one group"
                );
                assert_eq!(
                    spec("milestone")?,
                    (
                        "(\"milestone\") OR ((\"milestone\"* OR \"milestones\"))".into(),
                        "-|1,3|2,3,1".into(),
                        true
                    ),
                    "a longer vocabulary term keeps the prefix in the expanded group"
                );
                let (expression, spec, tiered) = spec("milestnoe 17")?;
                assert_eq!(
                    expression,
                    "(\"milestnoe\" AND \"17\") OR ((\"milestnoe\" OR \"milestone\" OR \"milestones\") AND (\"17\"))"
                );
                assert_eq!(spec, "0|2,6|0,1,0,2,1,1");
                assert!(tiered);
                Ok(())
            })
            .await
            .unwrap();
    }
}
