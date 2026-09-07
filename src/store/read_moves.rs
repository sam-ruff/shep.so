//! Per-read display projections. They never change provider identities or mail
//! content, and the read transaction rolls them back before releasing SQLite.
use super::*;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TEMP TABLE IF NOT EXISTS read_moves(
        id TEXT PRIMARY KEY, source_account TEXT NOT NULL, source_folder TEXT NOT NULL,
        account TEXT NOT NULL, folder TEXT NOT NULL, unread INTEGER NOT NULL, starred INTEGER NOT NULL);")?;
    for (view, source, pending) in [
        ("read_visible_mail", "messages", "0"),
        ("read_visible_bulk", "visible_mail", "0"),
        ("read_recovered_mail", "recovered_mail", "m.pending_move"),
        ("read_recovered_bulk", "recovered_bulk", "m.pending_move"),
    ] {
        // UNION keeps ordinary folder predicates indexable; only the small
        // modified branch needs to retrieve source rows by their primary key.
        c.execute_batch(&format!(
            "CREATE TEMP VIEW IF NOT EXISTS {view} AS
            SELECT m.rowid AS rowid,m.id,m.account,m.folder,m.sender,m.subject,m.body,m.timestamp,
                m.unread,m.starred,m.data,{pending} AS pending_move
            FROM {source} m WHERE NOT EXISTS(SELECT 1 FROM read_moves e
                WHERE e.id=m.id AND e.source_account=m.account AND e.source_folder=m.folder)
            UNION ALL
            SELECT m.rowid AS rowid,m.id,e.account,e.folder,m.sender,m.subject,m.body,m.timestamp,
                e.unread,e.starred,m.data,1 AS pending_move
            FROM read_moves e JOIN {source} m ON m.id=e.id
                AND m.account=e.source_account AND m.folder=e.source_folder"
        ))?;
    }
    Ok(())
}

pub(super) fn prepare(c: &Connection, moves: &[MailMoveProjection]) -> anyhow::Result<()> {
    anyhow::ensure!(
        moves.len() <= CHANNEL_CAPACITY * 4,
        "Too many pending message moves. Let a move finish before loading this folder."
    );
    c.execute("DELETE FROM read_moves", [])?;
    let mut statement = c.prepare("INSERT INTO read_moves(id,source_account,source_folder,account,folder,unread,starred) VALUES(?,?,?,?,?,?,?)")?;
    for change in moves {
        statement.execute(params![
            change.id,
            change.source_account,
            change.source_folder,
            change.account,
            change.folder,
            change.unread,
            change.starred
        ])?;
    }
    Ok(())
}

pub(super) fn source(c: &Connection) -> anyhow::Result<&'static str> {
    Ok(
        match (bulk::has_effects(c)?, move_journal::has_projection(c)?) {
            (false, false) => "messages",
            (true, false) => "visible_mail",
            (false, true) => "recovered_mail",
            (true, true) => "recovered_bulk",
        },
    )
}
