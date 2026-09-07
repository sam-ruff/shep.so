//! Process-local selection snapshots. Keep arbitrary-sized membership in SQLite,
//! returning only counts and bounded metadata pages to the native UI.
use super::*;
use rusqlite::OptionalExtension;
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MailSelectionId(uuid::Uuid);

impl Default for MailSelectionId {
    fn default() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}

impl std::fmt::Display for MailSelectionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone)]
pub enum SelectionChange {
    Set {
        id: String,
        selected: bool,
        clear_others: bool,
    },
    Range {
        anchor: String,
        target: String,
        additive: bool,
    },
    All,
    Clear,
}

#[derive(Debug, Clone)]
pub struct SelectionGroup {
    pub account: String,
    pub folder: String,
    pub total: usize,
    pub unread: usize,
}
#[derive(Debug, Clone)]
pub struct SelectionSnapshot {
    pub id: MailSelectionId,
    pub revision: u64,
    pub frozen: bool,
    /// Number of messages in the captured inbox/search scope.
    pub total: usize,
    /// Exact selected identities, including any that have since disappeared.
    pub selected: usize,
    pub available: usize,
    pub unread: usize,
    pub starred: usize,
    pub accounts: BTreeMap<String, usize>,
    pub groups: Vec<SelectionGroup>,
    /// Selected, still-available IDs among one requested visible page.
    pub visible: HashSet<String>,
    /// Captured ordinals for observed rows, including unselected rows.
    pub positions: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Clone)]
pub struct SelectedMail {
    pub position: u64,
    pub mail: Mail,
}

#[derive(Debug, Clone)]
pub struct SelectionPage {
    pub revision: u64,
    pub rows: Vec<SelectedMail>,
    pub next_after: Option<u64>,
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TEMP TABLE mail_selections(
        id TEXT PRIMARY KEY, revision INTEGER NOT NULL, frozen INTEGER NOT NULL, query TEXT);
        CREATE TEMP TABLE mail_selection_rows(
        selection TEXT NOT NULL REFERENCES mail_selections(id) ON DELETE CASCADE,
        id TEXT NOT NULL, position INTEGER NOT NULL, selected INTEGER NOT NULL,
        PRIMARY KEY(selection,id), UNIQUE(selection,position));
        CREATE INDEX temp.mail_selection_chosen ON mail_selection_rows(selection,selected,position);")?;
    Ok(())
}

fn version(c: &Connection, id: MailSelectionId) -> anyhow::Result<(u64, bool)> {
    Ok(c.query_row(
        "SELECT revision,frozen FROM temp.mail_selections WHERE id=?",
        [id.to_string()],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
    )?)
}

fn position(c: &Connection, selection: MailSelectionId, id: &str) -> anyhow::Result<i64> {
    c.query_row(
        "SELECT position FROM temp.mail_selection_rows WHERE selection=? AND id=?",
        params![selection.to_string(), id],
        |r| r.get(0),
    )
    .context("This message is outside the captured selection. Select the messages again.")
}

/// Explicit gestures may include arrivals; passive observations never grow the
/// selected set. Rebase ranks to the current query and retain every chosen ID,
/// including messages that disappeared while the user was choosing a range.
fn rebase(c: &Connection, source: MailSelectionId, endpoints: &[&str]) -> anyhow::Result<()> {
    let key = source.to_string();
    let query: String = c.query_row(
        "SELECT query FROM temp.mail_selections WHERE id=?",
        [&key],
        |r| r.get(0),
    )?;
    let query: MailQuery = serde_json::from_str(&query)?;
    let candidate = MailSelectionId::default().to_string();
    c.execute(
        "INSERT INTO temp.mail_selections(id,revision,frozen) VALUES(?,0,0)",
        [&candidate],
    )?;
    let (sql, values) = mail_query::Plan::selection(c, &query)?.ordered("messages.id AS id");
    let mut bindings = vec![candidate.clone().into()];
    bindings.extend(values);
    bindings.push(key.clone().into());
    let count = c.execute(&format!("INSERT INTO temp.mail_selection_rows(selection,id,position,selected)
        SELECT ?,ordered.id,row_number() OVER ()-1,COALESCE(chosen.selected,0)
        FROM ({sql}) AS ordered LEFT JOIN temp.mail_selection_rows chosen ON chosen.selection=? AND chosen.id=ordered.id"),
        rusqlite::params_from_iter(bindings))?;
    // Validate against the current query before retaining unavailable choices.
    for endpoint in endpoints {
        let found: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM temp.mail_selection_rows WHERE selection=? AND id=?)",
            params![candidate, endpoint],
            |r| r.get(0),
        )?;
        anyhow::ensure!(
            found,
            "This message is no longer in this view. Your previous selection is kept."
        );
    }
    c.execute("INSERT INTO temp.mail_selection_rows(selection,id,position,selected)
        SELECT ?,id,?+row_number() OVER (ORDER BY position)-1,1 FROM temp.mail_selection_rows previous
        WHERE selection=? AND selected=1 AND NOT EXISTS(SELECT 1 FROM temp.mail_selection_rows current WHERE current.selection=? AND current.id=previous.id)",
        params![candidate,i64::try_from(count)?,key,candidate])?;
    c.execute(
        "DELETE FROM temp.mail_selection_rows WHERE selection=?",
        [&key],
    )?;
    c.execute("INSERT INTO temp.mail_selection_rows SELECT ?,id,position,selected FROM temp.mail_selection_rows WHERE selection=?", params![key,candidate])?;
    c.execute("DELETE FROM temp.mail_selections WHERE id=?", [&candidate])?;
    Ok(())
}

fn snapshot(
    c: &Connection,
    id: MailSelectionId,
    visible: &[String],
) -> anyhow::Result<SelectionSnapshot> {
    anyhow::ensure!(
        visible.len() <= PAGE_SIZE,
        "Observe at most one inbox page at a time."
    );
    let (revision, frozen) = version(c, id)?;
    let key = id.to_string();
    let (total, selected): (i64, i64) = c.query_row(
        "SELECT COUNT(*),COALESCE(SUM(selected),0) FROM temp.mail_selection_rows WHERE selection=?",
        [&key],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (available, unread, starred): (i64, i64, i64) = c.query_row(
        "SELECT COUNT(*),COALESCE(SUM(m.unread),0),COALESCE(SUM(m.starred),0)
        FROM temp.mail_selection_rows s JOIN selectable_mail m ON m.id=s.id WHERE s.selection=? AND s.selected=1",
        [&key], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let accounts = c
        .prepare(
            "SELECT m.account,COUNT(*) FROM temp.mail_selection_rows s
        JOIN selectable_mail m ON m.id=s.id WHERE s.selection=? AND s.selected=1 GROUP BY m.account",
        )?
        .query_map([&key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as usize))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let groups = c.prepare("SELECT m.account,m.folder,COUNT(*),SUM(m.unread) FROM temp.mail_selection_rows s JOIN selectable_mail m ON m.id=s.id WHERE s.selection=? AND s.selected=1 GROUP BY m.account,m.folder")?
        .query_map([&key],|r|Ok(SelectionGroup { account:r.get(0)?,folder:r.get(1)?,total:r.get::<_,i64>(2)? as usize,unread:r.get::<_,i64>(3)? as usize }))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut observed = HashSet::new();
    let mut statement = c.prepare(
        "SELECT EXISTS(SELECT 1 FROM temp.mail_selection_rows s
        JOIN selectable_mail m ON m.id=s.id WHERE s.selection=? AND s.id=? AND s.selected=1)",
    )?;
    for mail in visible {
        if statement.query_row(params![key, mail], |r| r.get::<_, bool>(0))? {
            observed.insert(mail.clone());
        }
    }
    let mut positions = std::collections::HashMap::new();
    for mail in visible {
        if let Some(position) = c
            .query_row(
                "SELECT position FROM temp.mail_selection_rows WHERE selection=? AND id=?",
                params![key, mail],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            positions.insert(mail.clone(), position as u64);
        }
    }
    Ok(SelectionSnapshot {
        id,
        revision,
        frozen,
        total: total as usize,
        selected: selected as usize,
        available: available as usize,
        unread: unread as usize,
        starred: starred as usize,
        accounts,
        groups,
        visible: observed,
        positions,
    })
}

impl Store {
    /// Capture the complete ordered query, ignoring its page offset and action
    /// observations. A later capture with a newer revision replaces this scope.
    /// Callers own the token lifecycle and release abandoned selections.
    pub async fn capture_selection(
        &self,
        id: MailSelectionId,
        revision: u64,
        query: MailQuery,
        all: bool,
        visible: Vec<String>,
    ) -> anyhow::Result<SelectionSnapshot> {
        self.run(move |c| {
            anyhow::ensure!(
                revision <= i64::MAX as u64,
                "Selection revision is invalid."
            );
            let tx = c.transaction()?;
            let key = id.to_string();
            let old: Option<(u64, bool)> = tx
                .query_row(
                    "SELECT revision,frozen FROM temp.mail_selections WHERE id=?",
                    [&key],
                    |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
                )
                .optional()?;
            if let Some((old, frozen)) = old {
                anyhow::ensure!(
                    !frozen && revision > old,
                    "The selection changed. Select the messages again."
                );
            }
            tx.execute("DELETE FROM temp.mail_selections WHERE id=?", [&key])?;
            anyhow::ensure!(query.project_moves.is_empty(), "Pending display moves cannot be used as a selection scope. Wait for the move to finish.");
            let mut scope = query.clone();
            scope.offset = 0;
            scope.observe.clear();
            scope.observe_bulk.clear();
            tx.execute(
                "INSERT INTO temp.mail_selections(id,revision,frozen,query) VALUES(?,?,0,?)",
                params![key, revision as i64, serde_json::to_string(&scope)?],
            )?;
            let (sql, values) = mail_query::Plan::selection(&tx, &query)?.ordered("messages.id AS id");
            // The ordered subquery feeds a window scan, preventing flattening
            // from dropping its order. IDs/ranks stay in SQL; MIME is not read.
            let mut bindings = vec![key.into(), (all as i64).into()];
            bindings.extend(values);
            tx.execute(
                &format!(
                    "INSERT INTO temp.mail_selection_rows(selection,id,position,selected)
                SELECT ?,id,row_number() OVER ()-1,? FROM ({sql}) AS ordered"
                ),
                rusqlite::params_from_iter(bindings),
            )?;
            let result = snapshot(&tx, id, &visible)?;
            tx.commit()?;
            Ok(result)
        })
        .await
    }

    /// Atomic compare-and-change protects a newer user intent from stale work.
    pub async fn change_selection(
        &self,
        id: MailSelectionId,
        expected: u64,
        change: SelectionChange,
        visible: Vec<String>,
    ) -> anyhow::Result<SelectionSnapshot> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (revision, frozen) = version(&tx, id)?;
            anyhow::ensure!(!frozen && revision == expected && revision < i64::MAX as u64,
                "The selection changed. Select the messages again.");
            let key = id.to_string();
            match change {
                SelectionChange::Set { id: mail, selected, clear_others } => {
                    let captured: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM temp.mail_selection_rows WHERE selection=? AND id=?)",
                        params![key,mail], |r|r.get(0))?;
                    if !captured {
                        rebase(&tx, id, &[&mail])?;
                    }
                    position(&tx, id, &mail)?;
                    if clear_others {
                        tx.execute("UPDATE temp.mail_selection_rows SET selected=0 WHERE selection=?", [&key])?;
                    }
                    tx.execute("UPDATE temp.mail_selection_rows SET selected=? WHERE selection=? AND id=?",
                        params![selected,key,mail])?;
                }
                SelectionChange::Range { anchor, target, additive } => {
                    rebase(&tx, id, &[&anchor, &target])?;
                    let a = position(&tx,id,&anchor)?;
                    let b = position(&tx,id,&target)?;
                    if !additive {
                        tx.execute("UPDATE temp.mail_selection_rows SET selected=0 WHERE selection=?", [&key])?;
                    }
                    tx.execute("UPDATE temp.mail_selection_rows SET selected=1 WHERE selection=? AND position BETWEEN ? AND ?",
                        params![key,a.min(b),a.max(b)])?;
                }
                SelectionChange::All | SelectionChange::Clear => {
                    tx.execute("UPDATE temp.mail_selection_rows SET selected=? WHERE selection=?",
                        params![matches!(change,SelectionChange::All),key])?;
                }
            }
            tx.execute("UPDATE temp.mail_selections SET revision=? WHERE id=?", params![(revision+1) as i64,key])?;
            let result = snapshot(&tx,id,&visible)?;
            tx.commit()?;
            Ok(result)
        }).await
    }

    pub async fn selection_snapshot(
        &self,
        id: MailSelectionId,
        visible: Vec<String>,
    ) -> anyhow::Result<SelectionSnapshot> {
        self.run(move |c| {
            let tx = c.transaction()?;
            snapshot(&tx, id, &visible)
        })
        .await
    }

    /// A confirmation owns an immutable copy, independent of subsequent inbox
    /// selection, recapture, new arrivals and sorting/filter changes.
    pub async fn freeze_selection(
        &self,
        source: MailSelectionId,
        expected: u64,
    ) -> anyhow::Result<SelectionSnapshot> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (revision, _) = version(&tx, source)?;
            anyhow::ensure!(revision == expected, "The selection changed. Review it again.");
            let id = MailSelectionId::default();
            tx.execute("INSERT INTO temp.mail_selections(id,revision,frozen) VALUES(?,0,1)", [id.to_string()])?;
            tx.execute("INSERT INTO temp.mail_selection_rows(selection,id,position,selected)
                SELECT ?,id,position,1 FROM temp.mail_selection_rows WHERE selection=? AND selected=1",
                params![id.to_string(),source.to_string()])?;
            let result = snapshot(&tx,id,&[])?;
            tx.commit()?;
            Ok(result)
        }).await
    }

    pub async fn selected_mail_page(
        &self,
        id: MailSelectionId,
        expected: u64,
        after: Option<u64>,
    ) -> anyhow::Result<SelectionPage> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (revision, _) = version(&tx, id)?;
            anyhow::ensure!(
                revision == expected,
                "The selection changed. Read its first page again."
            );
            let cursor = after.map(i64::try_from).transpose()?.unwrap_or(-1);
            let rows = tx
                .prepare(
                    "SELECT s.position,m.data,m.unread,m.starred,m.folder
                FROM temp.mail_selection_rows s JOIN selectable_mail m ON m.id=s.id
                WHERE s.selection=? AND s.selected=1 AND s.position>? ORDER BY s.position LIMIT ?",
                )?
                .query_map(params![id.to_string(), cursor, PAGE_SIZE as i64], |r| {
                    Ok((
                        r.get::<_, i64>(0)? as u64,
                        r.get::<_, String>(1)?,
                        r.get::<_, bool>(2)?,
                        r.get::<_, bool>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })?
                .map(|r| {
                    let (position, data, unread, starred, folder) = r?;
                    let mut mail: Mail = serde_json::from_str(&data)?;
                    mail.unread = unread;
                    mail.starred = starred;
                    mail.folder = folder;
                    Ok(SelectedMail { position, mail })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            let next_after = (rows.len() == PAGE_SIZE).then(|| rows.last().unwrap().position);
            Ok(SelectionPage {
                revision,
                rows,
                next_after,
            })
        })
        .await
    }

    /// Removes only this process-local snapshot; never edits mail or folders.
    pub async fn release_selection(&self, id: MailSelectionId) -> anyhow::Result<()> {
        self.run(move |c| {
            c.execute(
                "DELETE FROM temp.mail_selections WHERE id=?",
                [id.to_string()],
            )?;
            Ok(())
        })
        .await
    }
}
