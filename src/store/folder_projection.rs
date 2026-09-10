//! Scalar observations of one reviewed folder set. Arbitrary folder membership
//! stays in the owning cache connection's encrypted disk scratch database.
use super::*;
use crate::folder_actions::Review;
use rusqlite::OptionalExtension;

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE scratch.folder_projection_scopes(
        token TEXT PRIMARY KEY,revision INTEGER NOT NULL,job TEXT UNIQUE,query TEXT NOT NULL);
        CREATE TABLE scratch.folder_projection_folders(
        token TEXT NOT NULL REFERENCES folder_projection_scopes(token) ON DELETE CASCADE,account TEXT NOT NULL,folder TEXT NOT NULL,
        PRIMARY KEY(token,account,folder));",
    )?;
    Ok(())
}

pub(super) fn release(c: &Connection, token: &str) -> anyhow::Result<()> {
    c.execute(
        "DELETE FROM scratch.folder_projection_folders WHERE token=?",
        [token],
    )?;
    c.execute(
        "DELETE FROM scratch.folder_projection_scopes WHERE token=?",
        [token],
    )?;
    Ok(())
}

pub(super) fn capture(
    c: &Connection,
    token: &str,
    review: &Review,
    revision: u64,
    query: &MailQuery,
) -> anyhow::Result<()> {
    let revision = i64::try_from(revision)?;
    let current: Option<i64> = c.query_row(
        "SELECT MAX(revision) FROM scratch.folder_projection_scopes WHERE job IS NULL",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        current.is_none_or(|current| current < revision),
        "This folder review was superseded by a newer view."
    );
    // Cancellation cannot leave a growing series of abandoned snapshots, and
    // an older read must never remove the current review's membership.
    c.execute(
        "DELETE FROM scratch.folder_projection_scopes WHERE job IS NULL",
        [],
    )?;
    c.execute(
        "INSERT INTO scratch.folder_projection_scopes(token,revision,query) VALUES(?,?,?)",
        params![token, revision, serde_json::to_string(query)?],
    )?;
    let mut insert = c.prepare("INSERT OR IGNORE INTO scratch.folder_projection_folders(token,account,folder) VALUES(?,?,?)")?;
    for member in &review.plan.members {
        insert.execute(params![token, review.account, member.mailbox.name])?;
    }
    Ok(())
}

pub(super) fn bind(c: &Connection, token: &str, job: &str) -> anyhow::Result<()> {
    c.execute("DELETE FROM scratch.folder_projection_scopes WHERE job IS NOT NULL AND NOT EXISTS(SELECT 1 FROM folder_jobs WHERE id=job)", [])?;
    let admitted: i64 = c.query_row(
        "SELECT COUNT(*) FROM scratch.folder_projection_scopes WHERE job IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        admitted < 32,
        "Too many folder changes are pending. Wait for one to finish and try again."
    );
    anyhow::ensure!(
        c.execute(
            "UPDATE scratch.folder_projection_scopes SET job=? WHERE token=? AND job IS NULL",
            params![job, token]
        )? == 1,
        "The folder view changed. Review the folder again before confirming."
    );
    Ok(())
}

pub(super) fn counts(c: &Connection, job: &str) -> anyhow::Result<Option<(usize, usize)>> {
    let scope: Option<(String, String)> = c
        .query_row(
            "SELECT token,query FROM scratch.folder_projection_scopes WHERE job=?",
            [job],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((token, query)) = scope else {
        return Ok(None);
    };
    let query: MailQuery = serde_json::from_str(&query)?;
    read_moves::prepare(c, &query.project_moves)?;
    let plan = mail_query::Plan::new(c, &query)?;
    let result = plan.affected_counts(c, &token)?;
    read_moves::prepare(c, &[])?;
    Ok(Some(result))
}

impl Store {
    pub async fn release_folder_projection(&self, token: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            release(&tx, &token)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    #[cfg(test)]
    pub async fn folder_projection_counts(
        &self,
        job: String,
    ) -> anyhow::Result<Option<(usize, usize)>> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let result = counts(&tx, &job)?;
            tx.commit()?;
            Ok(result)
        })
        .await
    }

    /// Completion consumes the observation even when the UI is gone or its
    /// cleanup queue is full. A count failure cannot leave admitted scratch.
    pub(crate) async fn finish_folder_projection(
        &self,
        job: String,
    ) -> anyhow::Result<Option<(usize, usize)>> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let result = counts(&tx, &job);
            read_moves::prepare(&tx, &[])?;
            tx.execute(
                "DELETE FROM scratch.folder_projection_scopes WHERE job=?",
                [&job],
            )?;
            tx.commit()?;
            result
        })
        .await
    }
}

#[cfg(test)]
#[path = "folder_projection_tests.rs"]
mod tests;
