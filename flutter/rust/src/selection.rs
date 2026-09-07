//! Captured query membership lives in a dedicated connection's TEMP tables.
//! Only counts, groups and one observed page cross the Flutter bridge.
use crate::paging::{Plan, Scope};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Capture {
        id: String,
        revision: u64,
        scope: Scope,
        all: bool,
    },
    Change {
        id: String,
        expected: u64,
        change: Change,
        scope: Scope,
    },
    Observe {
        id: String,
    },
    Freeze {
        id: String,
        expected: u64,
        target: String,
    },
    Page {
        id: String,
        expected: u64,
        after: Option<u64>,
    },
    Release {
        id: String,
    },
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
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
    Clear,
}
pub(crate) fn schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "PRAGMA foreign_keys=ON;
        CREATE TEMP TABLE selection_sessions(id TEXT PRIMARY KEY,revision INTEGER NOT NULL,
          frozen INTEGER NOT NULL,scope TEXT NOT NULL);
        CREATE TEMP TABLE selection_rows(session TEXT NOT NULL REFERENCES selection_sessions(id)
          ON DELETE CASCADE,id TEXT NOT NULL,position INTEGER NOT NULL,selected INTEGER NOT NULL,
          PRIMARY KEY(session,id),UNIQUE(session,position));
        CREATE INDEX temp.selection_chosen ON selection_rows(session,selected,position);",
    )?;
    Ok(())
}
fn token(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "Invalid selection identity. Select the messages again."
    );
    Ok(())
}
fn version(db: &Connection, id: &str) -> Result<(u64, bool)> {
    token(id)?;
    db.query_row(
        "SELECT revision,frozen FROM selection_sessions WHERE id=?",
        [id],
        |r| Ok((r.get::<_, i64>(0)? as u64, r.get(1)?)),
    )
    .context("This selection is no longer available. Select the messages again.")
}
fn canonical(db: &Connection, id: &str) -> Result<String> {
    Ok(db.query_row(
        "SELECT COALESCE((SELECT id FROM mail_aliases WHERE alias=?1),?1)",
        [id],
        |r| r.get(0),
    )?)
}
fn position(db: &Connection, session: &str, id: &str) -> Result<i64> {
    db.query_row(
        "SELECT position FROM selection_rows WHERE session=? AND id=?",
        params![session, id],
        |r| r.get(0),
    )
    .context(
        "This message is outside the captured selection. Select all again to include new arrivals.",
    )
}
fn reconcile(db: &Connection, session: &str) -> Result<()> {
    let mut after = String::new();
    loop {
        let changed = db.prepare("SELECT s.id,a.id FROM selection_rows s JOIN mail_aliases a ON a.alias=s.id WHERE session=?1 AND s.id>?2 ORDER BY s.id LIMIT 50")?
            .query_map(params![session,after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some((last, _)) = changed.last() else {
            break;
        };
        after = last.clone();
        for (old, target) in changed {
            if old == target {
                continue;
            }
            let original: Option<(i64, bool)> = db
                .query_row(
                    "SELECT position,selected FROM selection_rows WHERE session=? AND id=?",
                    params![session, old],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((ordinal, selected)) = original else {
                continue;
            };
            let existing: Option<(i64, bool)> = db
                .query_row(
                    "SELECT position,selected FROM selection_rows WHERE session=? AND id=?",
                    params![session, target],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            db.execute(
                "DELETE FROM selection_rows WHERE session=? AND id=?",
                params![session, old],
            )?;
            if let Some((next, chosen)) = existing {
                db.execute(
                    "UPDATE selection_rows SET position=?,selected=? WHERE session=? AND id=?",
                    params![next.min(ordinal), chosen || selected, session, target],
                )?;
            } else {
                db.execute(
                    "INSERT INTO selection_rows VALUES(?,?,?,?)",
                    params![session, target, ordinal, selected],
                )?;
            }
        }
    }
    Ok(())
}
fn snapshot(db: &Connection, id: &str, observed: &[String]) -> Result<Value> {
    let (revision, frozen) = version(db, id)?;
    let (total, selected): (i64, i64) = db.query_row(
        "SELECT COUNT(*),COALESCE(SUM(selected),0) FROM selection_rows WHERE session=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let (available, unread, starred): (i64,i64,i64) = db.query_row("SELECT COUNT(*),COALESCE(SUM(m.unread),0),COALESCE(SUM(m.starred),0) FROM selection_rows s JOIN mail m ON m.id=s.id WHERE s.session=? AND s.selected=1 AND m.moved=0",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let groups = db.prepare("SELECT m.account_id,m.folder,COUNT(*),SUM(m.unread),SUM(m.starred) FROM selection_rows s JOIN mail m ON m.id=s.id WHERE s.session=? AND s.selected=1 AND m.moved=0 GROUP BY m.account_id,m.folder ORDER BY m.account_id,m.folder")?
        .query_map([id],|r|Ok(json!({"account":r.get::<_,String>(0)?,"folder":r.get::<_,String>(1)?,"total":r.get::<_,i64>(2)?,"unread":r.get::<_,i64>(3)?,"starred":r.get::<_,i64>(4)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut positions = BTreeMap::new();
    let mut visible = BTreeSet::new();
    let mut aliases = BTreeMap::new();
    for original in observed {
        let target = canonical(db, original)?;
        if target != *original {
            aliases.insert(original.clone(), target.clone());
        }
        let row: Option<(i64,bool,bool)> = db.query_row("SELECT s.position,s.selected,EXISTS(SELECT 1 FROM mail WHERE id=s.id AND moved=0) FROM selection_rows s WHERE session=? AND id=?",params![id,target],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        if let Some((ordinal, chosen, present)) = row {
            positions.insert(target.clone(), ordinal);
            if chosen && present {
                visible.insert(target);
            }
        }
    }
    Ok(
        json!({"id":id,"revision":revision,"frozen":frozen,"total":total,"selected":selected,"available":available,"unread":unread,"starred":starred,"groups":groups,"visible":visible,"positions":positions,"aliases":aliases}),
    )
}
pub(crate) fn run(db: &mut Connection, command: Command, observed: Vec<String>) -> Result<Value> {
    ensure!(observed.len() <= 50, "Observe one mail page at a time.");
    let tx = db.transaction()?;
    let id = match command {
        Command::Capture {
            id,
            revision,
            scope,
            all,
        } => {
            token(&id)?;
            ensure!(revision <= i64::MAX as u64, "Invalid selection revision.");
            let old: Option<(i64, bool)> = tx
                .query_row(
                    "SELECT revision,frozen FROM selection_sessions WHERE id=?",
                    [&id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((old, frozen)) = old {
                ensure!(
                    !frozen && revision > old as u64,
                    "The selection changed. Select the messages again."
                );
            }
            let plan = Plan::new(&tx, scope.clone())?;
            tx.execute("DELETE FROM selection_sessions WHERE id=?", [&id])?;
            tx.execute(
                "INSERT INTO selection_sessions VALUES(?,?,0,?)",
                params![id, revision as i64, serde_json::to_string(&scope)?],
            )?;
            let order = if scope.oldest { "ASC" } else { "DESC" };
            let mut values = plan.values();
            values.extend([id.clone().into(), (all as i64).into()]);
            tx.execute(&format!("{}INSERT INTO selection_rows SELECT ?7,id,row_number() OVER(ORDER BY timestamp {order},id)-1,?8 FROM {} WHERE {} AND ?5>=0 AND length(?6)>=0",plan.prefix,plan.table,plan.conditions),params_from_iter(values))?;
            id
        }
        Command::Change {
            id,
            expected,
            change,
            scope,
        } => {
            let (revision, frozen) = version(&tx, &id)?;
            ensure!(
                !frozen && expected == revision && revision < i64::MAX as u64,
                "The selection changed. Select the messages again."
            );
            let saved: String = tx.query_row(
                "SELECT scope FROM selection_sessions WHERE id=?",
                [&id],
                |r| r.get(0),
            )?;
            let mut original: Scope = serde_json::from_str(&saved)?;
            original.projection.clear();
            let mut current = scope.clone();
            current.projection.clear();
            ensure!(
                serde_json::to_value(original)? == serde_json::to_value(current)?,
                "The mailbox view changed. Select its messages again."
            );
            reconcile(&tx, &id)?;
            match change {
                Change::Set {
                    id: mail,
                    selected,
                    clear_others,
                } => {
                    let mail = canonical(&tx, &mail)?;
                    if position(&tx, &id, &mail).is_err() {
                        // Explicitly choosing a new arrival extends this capture;
                        // passive observation never selects or imports arrivals.
                        let plan = Plan::new(&tx, scope)?;
                        let mut values = plan.values();
                        values.push(mail.clone().into());
                        let matches: bool=tx.query_row(&format!("{}SELECT EXISTS(SELECT 1 FROM {} WHERE {} AND id=?7 AND ?5>=0 AND length(?6)>=0)",plan.prefix,plan.table,plan.conditions),params_from_iter(values),|r|r.get(0))?;
                        ensure!(matches, "This message is outside the current mailbox view.");
                        tx.execute("INSERT INTO selection_rows SELECT ?1,?2,COALESCE(MAX(position),-1)+1,0 FROM selection_rows WHERE session=?1",params![id,mail])?;
                    }
                    if clear_others {
                        tx.execute(
                            "UPDATE selection_rows SET selected=0 WHERE session=?",
                            [&id],
                        )?;
                    }
                    tx.execute(
                        "UPDATE selection_rows SET selected=? WHERE session=? AND id=?",
                        params![selected, id, mail],
                    )?;
                }
                Change::Range {
                    anchor,
                    target,
                    additive,
                } => {
                    let a = position(&tx, &id, &canonical(&tx, &anchor)?)?;
                    let b = position(&tx, &id, &canonical(&tx, &target)?)?;
                    if !additive {
                        tx.execute(
                            "UPDATE selection_rows SET selected=0 WHERE session=?",
                            [&id],
                        )?;
                    }
                    tx.execute("UPDATE selection_rows SET selected=1 WHERE session=? AND position BETWEEN ? AND ?",params![id,a.min(b),a.max(b)])?;
                }
                Change::Clear => {
                    tx.execute(
                        "UPDATE selection_rows SET selected=0 WHERE session=?",
                        [&id],
                    )?;
                }
            }
            tx.execute(
                "UPDATE selection_sessions SET revision=revision+1 WHERE id=?",
                [&id],
            )?;
            id
        }
        Command::Observe { id } => {
            version(&tx, &id)?;
            reconcile(&tx, &id)?;
            id
        }
        Command::Freeze {
            id,
            expected,
            target,
        } => {
            let (revision, _) = version(&tx, &id)?;
            ensure!(
                revision == expected,
                "The selection changed. Review it again."
            );
            token(&target)?;
            reconcile(&tx, &id)?;
            tx.execute("INSERT INTO selection_sessions SELECT ?,0,1,scope FROM selection_sessions WHERE id=?",params![target,id])?;
            tx.execute("INSERT INTO selection_rows SELECT ?,id,position,1 FROM selection_rows WHERE session=? AND selected=1",params![target,id])?;
            target
        }
        Command::Page {
            id,
            expected,
            after,
        } => {
            let (revision, _) = version(&tx, &id)?;
            ensure!(
                revision == expected,
                "The selection changed. Read its first page again."
            );
            reconcile(&tx, &id)?;
            let after = after.map(i64::try_from).transpose()?.unwrap_or(-1);
            let rows=tx.prepare("SELECT s.position,s.id,m.account_id,m.folder,m.unread,m.starred FROM selection_rows s JOIN mail m ON m.id=s.id WHERE s.session=? AND s.selected=1 AND m.moved=0 AND s.position>? ORDER BY s.position LIMIT 50")?
                .query_map(params![id,after],|r|Ok(json!({"position":r.get::<_,i64>(0)?,"id":r.get::<_,String>(1)?,"account":r.get::<_,String>(2)?,"folder":r.get::<_,String>(3)?,"unread":r.get::<_,bool>(4)?,"starred":r.get::<_,bool>(5)?})))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let next_after = (rows.len() == 50).then(|| rows.last().unwrap()["position"].clone());
            tx.commit()?;
            return Ok(json!({"revision":revision,"rows":rows,"next_after":next_after}));
        }
        Command::Release { id } => {
            token(&id)?;
            tx.execute("DELETE FROM selection_sessions WHERE id=?", [id])?;
            tx.commit()?;
            return Ok(Value::Null);
        }
    };
    let result = snapshot(&tx, &id, &observed)?;
    tx.commit()?;
    Ok(result)
}
