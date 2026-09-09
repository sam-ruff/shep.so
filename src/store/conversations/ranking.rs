//! One metadata row per logical message, ordered by an encrypted on-disk index.
//! Window functions would materialize the whole conversation in SQLite TEMP.
use super::*;

const MEMBERS: &str = "SELECT t.logical_id,m.id,m.folder,m.timestamp
    FROM conversation_members t JOIN messages m ON m.id=t.id
    WHERE t.account=?1 AND t.group_id=?2";

const CHOOSE: &str = "INSERT INTO scratch.conversation_choices(logical_id,id,priority,timestamp)
    VALUES(?1,?2,?3,?4) ON CONFLICT(logical_id) DO UPDATE
    SET id=excluded.id,priority=excluded.priority,timestamp=excluded.timestamp
    WHERE (excluded.priority,excluded.id)<(conversation_choices.priority,conversation_choices.id)";

const POSITION: &str = "SELECT COUNT(*) FROM scratch.conversation_choices
    WHERE (timestamp,id)<(?1,?2)";

pub(super) const PAGE: &str = "SELECT m.data,m.unread,m.starred,m.folder
    FROM scratch.conversation_choices c INDEXED BY conversation_choices_order
    CROSS JOIN messages m ON m.id=c.id
    ORDER BY c.timestamp,c.id LIMIT ?1 OFFSET ?2";

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS scratch.conversation_choices (
        logical_id TEXT PRIMARY KEY,id TEXT NOT NULL UNIQUE,
        priority INTEGER NOT NULL,timestamp INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS scratch.conversation_choices_order
        ON conversation_choices(timestamp,id);",
    )?;
    Ok(())
}

pub(super) fn clear(c: &Connection) -> anyhow::Result<()> {
    c.execute("DELETE FROM scratch.conversation_choices", [])?;
    Ok(())
}

pub(super) fn capture(
    c: &Connection,
    account: &str,
    group: &str,
    anchor: &str,
    focus: Option<&str>,
) -> anyhow::Result<(usize, usize)> {
    clear(c)?;
    let mut members = c.prepare(MEMBERS)?;
    let mut choices = c.prepare(CHOOSE)?;
    let mut rows = members.query(params![account, group])?;
    while let Some(row) = rows.next()? {
        let logical: String = row.get(0)?;
        let id: String = row.get(1)?;
        let folder: String = row.get(2)?;
        let time: i64 = row.get(3)?;
        // Preserve existing mailbox-copy precedence and binary ID tie-breaking.
        let priority = if id == anchor || focus == Some(id.as_str()) {
            0
        } else if folder == "INBOX" {
            1
        } else if folder == "Sent" {
            2
        } else {
            3
        };
        choices.execute(params![logical, id, priority, time])?;
    }
    drop(rows);
    drop(members);
    drop(choices);
    let total: i64 = c.query_row(
        "SELECT COUNT(*) FROM scratch.conversation_choices",
        [],
        |r| r.get(0),
    )?;
    let mut position = None;
    for id in focus.into_iter().chain(std::iter::once(anchor)) {
        if let Some(time) = c
            .query_row(
                "SELECT timestamp FROM scratch.conversation_choices WHERE id=?",
                [id],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
        {
            position = Some(c.query_row(POSITION, params![time, id], |r| r.get::<_, i64>(0))?);
            break;
        }
    }
    Ok((
        usize::try_from(total)?,
        usize::try_from(position.unwrap_or(0))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plans(c: &Connection) -> anyhow::Result<()> {
        for (sql, values) in [
            (
                MEMBERS,
                vec![
                    rusqlite::types::Value::Text("fixture".into()),
                    rusqlite::types::Value::Text("thread".into()),
                ],
            ),
            (PAGE, vec![20.into(), 120.into()]),
            (
                POSITION,
                vec![100.into(), rusqlite::types::Value::Text("id".into())],
            ),
        ] {
            let plan = c
                .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?
                .query_map(rusqlite::params_from_iter(values), |r| {
                    r.get::<_, String>(3)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
                .join("\n");
            for forbidden in ["TEMP B-TREE", "MATERIALIZE", "AUTOMATIC"] {
                assert!(!plan.contains(forbidden), "{plan}");
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn ranking_plans_use_owned_indexes_for_plain_and_keyed_stores() {
        let dir = tempfile::tempdir().unwrap();
        for keyed in [false, true] {
            let path = dir.path().join(format!("{keyed}.sqlite"));
            let store = if keyed {
                Store::open_encrypted(
                    &path,
                    std::sync::Arc::new(crate::cache_cipher::Key::generate().unwrap()),
                )
                .unwrap()
            } else {
                Store::open(&path).unwrap()
            };
            store.run(|c| plans(c)).await.unwrap();
        }
    }

    #[tokio::test]
    async fn ranking_clear_and_transaction_rollback_remove_owned_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open_encrypted(
            dir.path().join("keyed.sqlite"),
            std::sync::Arc::new(crate::cache_cipher::Key::generate().unwrap()),
        )
        .unwrap();
        store
            .run(|c| {
                c.execute_batch("CREATE TABLE scratch.rollback_probe(value BLOB);
                    INSERT INTO scratch.rollback_probe VALUES(x'010203');
                    PRAGMA scratch.cache_size=-64;")?;
                let tx = c.transaction()?;
                // Exceed the page cache so rollback covers real encrypted disk
                // writes with OFF durability, rather than only cached pages.
                tx.execute_batch("DELETE FROM scratch.rollback_probe;
                    WITH RECURSIVE sequence(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM sequence WHERE i<128)
                    INSERT INTO scratch.rollback_probe SELECT zeroblob(32768) FROM sequence;")?;
                tx.execute(CHOOSE, params!["logical", "fictional-id", 0, 1])?;
                assert_eq!(
                    tx.query_row(
                        "SELECT count(*) FROM scratch.conversation_choices",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    1
                );
                tx.rollback()?;
                assert_eq!(c.query_row("SELECT count(*) FROM scratch.rollback_probe",[],|r|r.get::<_,i64>(0))?,1);
                assert_eq!(c.query_row("SELECT hex(value) FROM scratch.rollback_probe",[],|r|r.get::<_,String>(0))?,"010203");
                assert_eq!(
                    c.query_row(
                        "SELECT count(*) FROM scratch.conversation_choices",
                        [],
                        |r| r.get::<_, i64>(0)
                    )?,
                    0
                );
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn encrypted_ranking_matches_duplicate_focus_and_chronological_page_contract() {
        let dir = tempfile::tempdir().unwrap();
        let key = std::sync::Arc::new(crate::cache_cipher::Key::generate().unwrap());
        let path = dir.path().join("keyed.sqlite");
        let store = Store::open_encrypted(&path, key.clone()).unwrap();
        let mut messages = Vec::new();
        for i in 0..512 {
            for folder in ["INBOX", "Sent", "Archive"] {
                let mut mail = parse_mail("fixture", &i.to_string(), folder,
                    format!("From: Fictional <writer@example.test>\r\nSubject: Thread {i}\r\nMessage-ID: <{i}@example.test>\r\nReferences: <root@example.test>\r\n\r\nFictional body.").into_bytes(), i % 2 == 0, i % 3 == 0).unwrap();
                mail.summary.timestamp = i / 3; // Tied timestamps require ID order.
                messages.push(mail);
            }
        }
        store.upsert(messages).await.unwrap();
        for (anchor, focus, requested) in [
            ("fixture:Archive:256", None, None),
            ("fixture:Archive:256", Some("fixture:Sent:4"), None),
            ("fixture:Archive:256", Some("fixture:INBOX:256"), None),
            ("fixture:INBOX:511", Some("missing"), None),
            ("fixture:Sent:128", None, Some(0)),
            ("fixture:Sent:128", None, Some(usize::MAX)),
        ] {
            let a = anchor.to_string();
            let f = focus.map(str::to_string);
            // An independent legacy-window oracle on this bounded fixture
            // preserves duplicate and focus semantics during the storage change.
            let expected = store.run(move |c| {
                let query = "WITH copies AS (
                    SELECT m.id,m.timestamp,ROW_NUMBER() OVER (PARTITION BY t.logical_id ORDER BY
                    CASE WHEN m.id=?1 OR m.id=?2 THEN 0 WHEN m.folder='INBOX' THEN 1 WHEN m.folder='Sent' THEN 2 ELSE 3 END,m.id) AS copy
                    FROM conversation_members t JOIN messages m ON m.id=t.id WHERE t.account='fixture'
                ), ordered AS (SELECT id,ROW_NUMBER() OVER (ORDER BY timestamp,id)-1 AS position FROM copies WHERE copy=1) ";
                let (total, position): (usize,usize) = c.query_row(&format!("{query}SELECT COUNT(*),COALESCE(MAX(CASE WHEN id=?2 THEN position END),MAX(CASE WHEN id=?1 THEN position END),0) FROM ordered"),params![a,f],|r|Ok((r.get::<_,i64>(0)? as usize,r.get::<_,i64>(1)? as usize)))?;
                let last = total.saturating_sub(1) / CONVERSATION_PAGE_SIZE * CONVERSATION_PAGE_SIZE;
                let offset = requested.unwrap_or(position / CONVERSATION_PAGE_SIZE * CONVERSATION_PAGE_SIZE).min(last);
                let rows = c.prepare(&format!("{query}SELECT id FROM ordered ORDER BY position LIMIT ?3 OFFSET ?4"))?
                    .query_map(params![a,f,CONVERSATION_PAGE_SIZE as i64,offset as i64],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                Ok((total,offset,rows))
            }).await.unwrap();
            let page = store
                .conversation_around(anchor.into(), focus.map(str::to_string), requested)
                .await
                .unwrap();
            assert_eq!(
                (
                    page.total,
                    page.offset,
                    page.rows.into_iter().map(|m| m.id).collect::<Vec<_>>()
                ),
                expected
            );
            store
                .run(|c| {
                    plans(c)?;
                    assert_eq!(
                        c.query_row(
                            "SELECT count(*) FROM scratch.conversation_choices",
                            [],
                            |r| r.get::<_, i64>(0)
                        )?,
                        0
                    );
                    Ok(())
                })
                .await
                .unwrap();
        }
        drop(store);
        let reopened = Store::open_encrypted(path, key).unwrap();
        let page = reopened
            .conversation("fixture:INBOX:511".into(), None)
            .await
            .unwrap();
        assert_eq!(page.total, 512);
        assert!(page.rows.len() <= CONVERSATION_PAGE_SIZE);
    }
}
