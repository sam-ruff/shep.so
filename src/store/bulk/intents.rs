use super::*;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS bulk_admissions(
            sequence INTEGER PRIMARY KEY,
            job TEXT NOT NULL,position INTEGER NOT NULL,lineage TEXT NOT NULL,original TEXT NOT NULL,
            predecessor INTEGER REFERENCES bulk_admissions(sequence) ON DELETE SET NULL,
            UNIQUE(job,position),
            FOREIGN KEY(job,position) REFERENCES bulk_items(job,position) ON DELETE CASCADE);
        CREATE INDEX IF NOT EXISTS bulk_admission_lineage ON bulk_admissions(lineage,sequence);
        CREATE INDEX IF NOT EXISTS bulk_item_unconfirmed_identity ON bulk_items(id)
            WHERE status IN ('running','uncertain');
        CREATE TABLE IF NOT EXISTS bulk_field_owners(
            lineage TEXT NOT NULL,field TEXT NOT NULL,sequence INTEGER NOT NULL
                REFERENCES bulk_admissions(sequence) ON DELETE CASCADE,
            PRIMARY KEY(lineage,field));",
    )?;
    Ok(())
}

pub(super) fn admit(c: &Connection, job: &str, action: &Action) -> anyhow::Result<()> {
    c.execute(
        "INSERT INTO bulk_admissions(job,position,lineage,original,predecessor)
         SELECT i.job,i.position,l.lineage,i.original,
             (SELECT max(sequence) FROM bulk_admissions WHERE lineage=l.lineage)
         FROM bulk_items i JOIN mail_lineage l ON l.id=i.id
         WHERE i.job=? AND i.status='queued' ORDER BY i.position",
        [job],
    )?;
    let fields: &[&str] = match action {
        Action::Move { .. } => &["location"],
        Action::Flags(flags) => match (flags.unread.is_some(), flags.starred.is_some()) {
            (true, true) => &["unread", "starred"],
            (true, false) => &["unread"],
            (false, true) => &["starred"],
            (false, false) => &[],
        },
    };
    for field in fields {
        c.execute(
            "INSERT INTO bulk_field_owners(lineage,field,sequence)
             SELECT lineage,?,sequence FROM bulk_admissions WHERE job=?
             ON CONFLICT(lineage,field) DO UPDATE SET sequence=excluded.sequence",
            params![field, job],
        )?;
    }
    c.execute("UPDATE bulk_items SET status='cancelled',error='A newer decision owns these fields. This change was not sent.'
        WHERE status='queued' AND (job,position) IN (
            SELECT a.job,a.position FROM bulk_admissions a
            WHERE a.lineage IN (SELECT lineage FROM bulk_admissions WHERE job=?1)
            AND NOT EXISTS(SELECT 1 FROM bulk_field_owners o WHERE o.sequence=a.sequence))", [job])?;
    Ok(())
}

pub(super) fn prepare(c: &Connection, item: &mut Item) -> anyhow::Result<()> {
    if item.undo && !matches!(item.receipt, Some(Receipt::Flags { .. })) {
        return Ok(());
    }
    let current: Option<(String, String)> = c
        .query_row(
            "SELECT m.id,json_set(m.data,'$.account_id',m.account,'$.folder',m.folder,
            '$.unread',json(CASE m.unread WHEN 1 THEN 'true' ELSE 'false' END),
            '$.starred',json(CASE m.starred WHEN 1 THEN 'true' ELSE 'false' END))
         FROM bulk_admissions a JOIN mail_lineage l ON l.lineage=a.lineage
         JOIN messages m ON m.id=l.id WHERE a.job=? AND a.position=?",
            params![item.job, item.position as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let bound: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM bulk_admissions WHERE job=? AND position=?)",
        params![item.job, item.position as i64],
        |r| r.get(0),
    )?;
    if !bound {
        return Ok(());
    }
    let (id, data) = current.context("This message was replaced. Refresh before trying again.")?;
    item.id = id;
    item.original = Some(serde_json::from_str(&data)?);
    c.execute(
        "UPDATE bulk_items SET id=?,original=? WHERE job=? AND position=?",
        params![item.id, data, item.job, item.position as i64],
    )?;
    Ok(())
}

pub(super) fn owns(c: &Connection, item: &Item, field: &str) -> anyhow::Result<bool> {
    // Older jobs retain their strict identity checks until they have a receipt.
    Ok(c.query_row(
        "SELECT NOT EXISTS(SELECT 1 FROM bulk_admissions WHERE job=?1 AND position=?2)
         OR EXISTS(SELECT 1 FROM bulk_admissions a JOIN bulk_field_owners o
             ON o.sequence=a.sequence WHERE a.job=?1 AND a.position=?2 AND o.field=?3)",
        params![item.job, item.position as i64, field],
        |r| r.get(0),
    )?)
}

pub(super) fn refresh(c: &Connection, job: &str) -> anyhow::Result<()> {
    refresh_scope(c, job, None)
}

pub(super) fn refresh_item(c: &Connection, item: &Item) -> anyhow::Result<()> {
    refresh_scope(c, &item.job, Some(item.position as i64))
}

pub(super) fn refresh_scope(
    c: &Connection,
    job: &str,
    position: Option<i64>,
) -> anyhow::Result<()> {
    if !c.query_row(
        "SELECT EXISTS(SELECT 1 FROM bulk_admissions WHERE job=?1 AND (?2 IS NULL OR position=?2))",
        params![job, position],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(());
    }
    c.execute(
        "DELETE FROM bulk_effects WHERE id IN (
            SELECT l.id FROM mail_lineage l JOIN bulk_admissions a ON a.lineage=l.lineage WHERE a.job=?1 AND (?2 IS NULL OR a.position=?2))
            OR (job=?1 AND (?2 IS NULL OR position=?2))",
        params![job,position],
    )?;
    c.execute(
        "INSERT INTO bulk_effects(id,job,position,account,folder,unread,starred)
         SELECT l.id,a.job,a.position,
           max(CASE WHEN o.field='location' AND i.status IN ('queued','running','repair')
               AND (i.undo=1 OR j.undo_requested=0)
               THEN CASE i.undo WHEN 1 THEN json_extract(i.original,'$.account_id')
                    ELSE json_extract(j.action,'$.Move.account') END END),
           max(CASE WHEN o.field='location' AND i.status IN ('queued','running','repair')
               AND (i.undo=1 OR j.undo_requested=0)
               THEN CASE i.undo WHEN 1 THEN json_extract(i.original,'$.folder')
                    ELSE json_extract(j.action,'$.Move.folder') END END),
           max(CASE WHEN o.field='unread' AND i.status IN ('queued','running','repair')
               AND (i.undo=1 OR j.undo_requested=0)
               THEN CASE i.undo WHEN 1 THEN json_extract(i.receipt,'$.Flags.before.unread')
                    ELSE json_extract(j.action,'$.Flags.unread') END END),
           max(CASE WHEN o.field='starred' AND i.status IN ('queued','running','repair')
               AND (i.undo=1 OR j.undo_requested=0)
               THEN CASE i.undo WHEN 1 THEN json_extract(i.receipt,'$.Flags.before.starred')
                    ELSE json_extract(j.action,'$.Flags.starred') END END)
         FROM bulk_field_owners o JOIN bulk_admissions a ON a.sequence=o.sequence
         JOIN bulk_items i ON i.job=a.job AND i.position=a.position
         JOIN bulk_jobs j ON j.id=a.job JOIN mail_lineage l ON l.lineage=a.lineage
         WHERE i.status IN ('queued','running','repair','uncertain')
           AND a.lineage IN (SELECT lineage FROM bulk_admissions WHERE job=?1 AND (?2 IS NULL OR position=?2))
         GROUP BY l.id",
        params![job,position],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn successive_field_reservations_keep_disjoint_and_same_value_intent()
    -> anyhow::Result<()> {
        let store = Store::memory()?;
        let message = parse_mail(
            "work",
            "1.1",
            "INBOX",
            b"Subject: Intent\r\n\r\nBody".to_vec(),
            true,
            false,
        )?;
        let mail = message.summary.clone();
        store.upsert(vec![message]).await?;
        store.run(move |c| {
            let actions = [
                Action::Flags(crate::mail_actions::Flags { unread: Some(false), starred: Some(true) }),
                Action::Flags(crate::mail_actions::Flags { unread: Some(false), starred: None }),
            ];
            for (position, action) in actions.iter().enumerate() {
                let job = format!("intent-{position}");
                c.execute("INSERT INTO bulk_jobs(id,action,source,created) VALUES(?,?,?,0)", params![job, serde_json::to_string(action)?, mail.id])?;
                c.execute("INSERT INTO bulk_items(job,position,id,original,status) VALUES(?,0,?,?,'queued')", params![job, mail.id, serde_json::to_string(&mail)?])?;
                admit(c, &job, action)?;
            }
            let item = parse_item("intent-0", (0, mail.id.clone(), Some(serde_json::to_string(&mail)?), false, "queued".into(), None, None))?;
            assert!(!owns(c, &item, "unread")?);
            assert!(owns(c, &item, "starred")?);
            c.execute("UPDATE bulk_items SET status='failed' WHERE job='intent-1'", [])?;
            assert!(!owns(c, &item, "unread")?, "A rejected newer decision must not resurrect an older value");
            Ok(())
        }).await
    }
}
