use super::*;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS mail_lineage(
            id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
            lineage TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS mail_lineage_origin ON mail_lineage(lineage);
        CREATE TABLE IF NOT EXISTS mail_lineage_alias(alias TEXT PRIMARY KEY,lineage TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS mail_lineage_alias_target ON mail_lineage_alias(lineage);
        CREATE TABLE IF NOT EXISTS mail_identity_history(
            id TEXT NOT NULL,account TEXT NOT NULL,folder TEXT NOT NULL,remote TEXT NOT NULL,lineage TEXT NOT NULL,
            PRIMARY KEY(id,account,folder,remote,lineage));
        CREATE INDEX IF NOT EXISTS mail_identity_history_account ON mail_identity_history(account);
        INSERT OR IGNORE INTO mail_lineage SELECT id,lower(hex(randomblob(16))) FROM messages;
        CREATE TRIGGER IF NOT EXISTS mail_lineage_insert AFTER INSERT ON messages BEGIN
            INSERT OR REPLACE INTO mail_lineage VALUES(new.id,lower(hex(randomblob(16))));
            DELETE FROM bulk_effects WHERE id=new.id; END;
        CREATE TRIGGER IF NOT EXISTS mail_lineage_replace AFTER UPDATE OF account,folder,data,raw ON messages
        WHEN old.account!=new.account OR old.folder!=new.folder OR old.raw!=new.raw
            OR json_extract(old.data,'$.remote_id') IS NOT json_extract(new.data,'$.remote_id') BEGIN
            UPDATE mail_lineage SET lineage=lower(hex(randomblob(16))) WHERE id=new.id;
            DELETE FROM bulk_effects WHERE id=new.id; END;
        CREATE TRIGGER IF NOT EXISTS mail_lineage_delete AFTER DELETE ON messages BEGIN
            DELETE FROM bulk_effects WHERE id=old.id; END;",
    )?;
    Ok(())
}

pub(super) fn get(c: &Connection, id: &str) -> anyhow::Result<String> {
    Ok(
        c.query_row("SELECT lineage FROM mail_lineage WHERE id=?", [id], |r| {
            r.get(0)
        })?,
    )
}

pub(super) fn inherit(c: &Connection, destination: &str, lineage: &str) -> anyhow::Result<()> {
    let previous = get(c, destination)?;
    if previous != lineage {
        c.execute(
            "UPDATE mail_lineage_alias SET lineage=? WHERE lineage=?",
            params![lineage, previous],
        )?;
        c.execute("INSERT INTO mail_lineage_alias(alias,lineage) VALUES(?,?) ON CONFLICT(alias) DO UPDATE SET lineage=excluded.lineage", params![previous,lineage])?;
        c.execute(
            "UPDATE bulk_admissions SET lineage=? WHERE lineage=?",
            params![lineage, previous],
        )?;
        c.execute(
            "INSERT INTO bulk_field_owners(lineage,field,sequence)
            SELECT ?1,field,sequence FROM bulk_field_owners WHERE lineage=?2
            ON CONFLICT(lineage,field) DO UPDATE SET sequence=max(sequence,excluded.sequence)",
            params![lineage, previous],
        )?;
        c.execute("DELETE FROM bulk_field_owners WHERE lineage=?", [&previous])?;
        c.execute("UPDATE bulk_items SET status='cancelled',error='A newer decision owns these fields. This change was not sent.'
            WHERE status='queued' AND (job,position) IN (SELECT a.job,a.position FROM bulk_admissions a
                WHERE a.lineage=? AND NOT EXISTS(SELECT 1 FROM bulk_field_owners o WHERE o.sequence=a.sequence))", [lineage])?;
    }
    anyhow::ensure!(
        c.execute(
            "UPDATE mail_lineage SET lineage=? WHERE id=?",
            params![lineage, destination]
        )? == 1,
        "The acknowledged destination is unavailable"
    );
    Ok(())
}

pub(super) fn remember(c: &Connection, source: &Mail, lineage: &str) -> anyhow::Result<()> {
    c.execute(
        "INSERT OR IGNORE INTO mail_identity_history VALUES(?,?,?,?,?)",
        params![
            source.id,
            source.account_id,
            source.folder,
            source.remote_id,
            lineage
        ],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_acknowledged_relocation_preserves_identity_continuity() -> anyhow::Result<()> {
        let store = Store::memory()?;
        let message = parse_mail(
            "work",
            "1.7",
            "INBOX",
            b"Subject: Test\r\n\r\nBody".to_vec(),
            true,
            false,
        )?;
        let original = message.summary.clone();
        store.upsert(vec![message]).await?;
        let id = original.id.clone();
        let lineage = store.run(move |c| get(c, &id)).await?;
        let mut destination = original.clone();
        destination.folder = "Archive".into();
        store.relocate_mail(original, destination.clone()).await?;
        let id = destination.id.clone();
        assert_eq!(store.run(move |c| get(c, &id)).await?, lineage);
        let id = destination.id.clone();
        store
            .run(move |c| {
                c.execute(
                    "UPDATE messages SET data=json_set(data,'$.remote_id','2.9') WHERE id=?",
                    [&id],
                )?;
                Ok(())
            })
            .await?;
        let id = destination.id;
        assert_ne!(store.run(move |c| get(c, &id)).await?, lineage);
        Ok(())
    }
}
