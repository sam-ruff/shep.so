//! Test-support view of what the cache holds for one query: the page the store
//! would return, its counts, every folder's totals and the pending-write totals.
//! It reads through the ordinary page path in one transaction so native flows
//! can compare the displayed list against the database after each action.
use super::*;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FolderCount {
    pub account: String,
    pub folder: String,
    pub total: usize,
    pub unread: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct StoreTruth {
    /// Row ids of the requested page, in the store's order.
    pub page_ids: Vec<String>,
    pub total: usize,
    pub unread: usize,
    /// Unread Inbox messages per account, omitting accounts with none.
    pub inbox_unread: std::collections::BTreeMap<String, usize>,
    /// Every account/folder with at least one message, sorted.
    pub folders: Vec<FolderCount>,
    /// Journal records still holding a cache row.
    pub pending_moves: usize,
    /// Display effects of bulk jobs not yet reconciled.
    pub bulk_effects: usize,
    /// Acknowledged writes the session ledger still holds.
    pub ledger_entries: usize,
}

impl Store {
    /// The store's answer for `query` without UI projections, plus the folder
    /// and pending-write totals from the same snapshot.
    pub async fn truth(&self, mut query: MailQuery) -> anyhow::Result<StoreTruth> {
        query.project_moves.clear();
        query.observe.clear();
        query.observe_bulk.clear();
        self.run(move |c| {
            let page = read_page(c, query, None)?;
            let source = read_moves::source(c)?;
            let folders = c
                .prepare(&format!(
                    "SELECT account,folder,COUNT(*),COALESCE(SUM(unread),0) FROM {source} GROUP BY account,folder ORDER BY account,folder"
                ))?
                .query_map([], |r| {
                    Ok(FolderCount {
                        account: r.get(0)?,
                        folder: r.get(1)?,
                        total: r.get::<_, i64>(2)? as usize,
                        unread: r.get::<_, i64>(3)? as usize,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let count = |sql: &str| -> anyhow::Result<usize> {
                Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))? as usize)
            };
            Ok(StoreTruth {
                page_ids: page.rows.iter().map(|mail| mail.id.clone()).collect(),
                total: page.total,
                unread: page.unread,
                inbox_unread: page.inbox_unread,
                folders,
                pending_moves: page.move_pending_total,
                bulk_effects: count("SELECT COUNT(*) FROM bulk_effects")?,
                ledger_entries: count("SELECT COUNT(*) FROM scratch.write_ledger")?,
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(uid: &str, folder: &str, unread: bool) -> StoredMail {
        parse_mail(
            "fixture",
            uid,
            folder,
            format!("From: a@example.test\r\nSubject: {folder} {uid}\r\n\r\nbody").into_bytes(),
            unread,
            false,
        )
        .expect("fixture mail parses")
    }

    #[tokio::test]
    async fn truth_reports_the_page_folder_totals_and_no_pending_writes() -> anyhow::Result<()> {
        let store = Store::memory()?;
        store
            .upsert(vec![
                mail("1", "INBOX", true),
                mail("2", "INBOX", false),
                mail("3", "Archive", true),
            ])
            .await?;
        let query = MailQuery {
            folder: "INBOX".into(),
            ..Default::default()
        };
        let page = store.query(query.clone()).await?;
        let truth = store.truth(query).await?;
        let ids: Vec<_> = page.rows.iter().map(|m| m.id.clone()).collect();
        assert_eq!(truth.page_ids, ids);
        assert_eq!((truth.total, truth.unread), (2, 1));
        assert_eq!(truth.inbox_unread.get("fixture"), Some(&1));
        assert_eq!(
            truth.folders,
            vec![
                FolderCount {
                    account: "fixture".into(),
                    folder: "Archive".into(),
                    total: 1,
                    unread: 1
                },
                FolderCount {
                    account: "fixture".into(),
                    folder: "INBOX".into(),
                    total: 2,
                    unread: 1
                },
            ]
        );
        assert_eq!(
            (
                truth.pending_moves,
                truth.bulk_effects,
                truth.ledger_entries
            ),
            (0, 0, 0)
        );
        Ok(())
    }

    #[tokio::test]
    async fn truth_ignores_ui_projections_and_follows_local_moves() -> anyhow::Result<()> {
        let store = Store::memory()?;
        store
            .upsert(vec![mail("1", "INBOX", true), mail("2", "INBOX", true)])
            .await?;
        let first = store
            .query(MailQuery {
                folder: "INBOX".into(),
                ..Default::default()
            })
            .await?
            .rows[0]
            .clone();
        let query = MailQuery {
            folder: "INBOX".into(),
            project_moves: vec![MailMoveProjection {
                id: first.id.clone(),
                source_account: first.account_id.clone(),
                source_folder: "INBOX".into(),
                account: first.account_id.clone(),
                folder: "Archive".into(),
                unread: true,
                starred: false,
            }],
            observe: vec![first.id.clone()],
            ..Default::default()
        };
        let truth = store.truth(query.clone()).await?;
        assert_eq!(truth.total, 2, "a projection is not a store fact");
        store.move_local(first.id.clone(), "Archive".into()).await?;
        let truth = store.truth(query).await?;
        assert_eq!(truth.total, 1);
        assert_eq!(truth.page_ids.len(), 1);
        assert_ne!(truth.page_ids[0], first.id);
        assert_eq!(truth.inbox_unread.get("fixture"), Some(&1));
        Ok(())
    }
}
