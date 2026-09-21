use super::*;
use shep_mail_core::folder_actions::{Action, Plan, Step};
use shep_mail_core::folders::{NameEncoding, Tree};
pub(crate) mod execute;
#[cfg(test)]
mod tests;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Review {
    pub account: String,
    pub connection: String,
    pub plan: Plan,
    pub fingerprint: String,
    pub messages: u64,
    pub source_label: String,
    pub destination_label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Mutation {
    pub review: Review,
    pub completed: usize,
    pub receipt: Option<Step>,
    pub observed: bool,
    pub checked: bool,
    pub prepared: bool,
    pub preparing_member: usize,
    pub preparing_cursor: Option<String>,
    pub prepared_count: u64,
}

pub fn available(db: &Connection, account: &str) -> Result<()> {
    anyhow::ensure!(!db.query_row("SELECT EXISTS(SELECT 1 FROM folder_creations WHERE account_id=?1 AND mutation IS NOT NULL AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))", [account], |row| row.get::<_,bool>(0))?,
        "Finish or review this account's saved folder changes first.");
    Ok(())
}

pub fn cleanup(db: &Connection) -> Result<()> {
    let job:Option<String>=db.query_row("SELECT id FROM folder_creations INDEXED BY folder_creation_retired WHERE status IN ('succeeded','cancelled','dismissed') AND mutation IS NOT NULL AND json_extract(mutation,'$.prepared_count')>0 LIMIT 1",[],|row|row.get(0)).optional()?;
    let Some(job) = job else {
        return Ok(());
    };
    let tx = db.unchecked_transaction()?;
    tx.execute("DELETE FROM folder_change_members WHERE job=?1 AND id IN (SELECT id FROM folder_change_members WHERE job=?1 LIMIT 50)",[&job])?;
    tx.execute("UPDATE folder_creations SET mutation=json_set(mutation,'$.prepared_count',0),revision=revision+1 WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM folder_change_members WHERE job=?1)",[&job])?;
    tx.commit()?;
    Ok(())
}

fn ready(db: &Connection, account: &str) -> Result<()> {
    for query in [
        "SELECT EXISTS(SELECT 1 FROM folder_creations WHERE account_id=?1 AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))",
        "SELECT EXISTS(SELECT 1 FROM individual_mail_actions WHERE account=?1 AND status IN ('queued','running','waiting','rejected','uncertain','repair'))",
        "SELECT EXISTS(SELECT 1 FROM group_items WHERE account=?1 AND state IN ('pending','sending','undoing','reversing','failed','uncertain','undo_failed','undo_uncertain'))",
        "SELECT EXISTS(SELECT 1 FROM pending_moves p JOIN mail m ON m.id=p.id WHERE m.account_id=?1)",
        "SELECT EXISTS(SELECT 1 FROM outgoing o LEFT JOIN outgoing_sent s ON s.id=o.id LEFT JOIN outgoing_meta m ON m.id=o.id WHERE o.account_id=?1 AND (m.recovery IS NULL OR (m.recovery='marked' AND s.id IS NOT NULL)) AND COALESCE(s.complete,0)=0)",
    ] {
        anyhow::ensure!(
            !db.query_row(query, [account], |row| row.get::<_, bool>(0))?,
            "Finish or review this account's pending mail, Outbox and folder work first."
        );
    }
    Ok(())
}

pub fn catalogue(db: &Connection, account: &Account) -> Result<Vec<Mailbox>> {
    let cached: Option<String> = db
        .query_row(
            "SELECT mailboxes FROM folder_catalogues WHERE account_id=?1",
            [&account.id],
            |row| row.get(0),
        )
        .optional()?;
    if account.protocol == shep_mail_core::model::Protocol::Imap {
        return Ok(serde_json::from_str(&cached.context(
            "Refresh this account's folder list before changing it.",
        )?)?);
    }
    let names: String = db
        .query_row(
            "SELECT names FROM folders WHERE account_id=?1",
            [&account.id],
            |row| row.get(0),
        )
        .context("Refresh the local folder list.")?;
    Ok(serde_json::from_str::<Vec<String>>(&names)?
        .into_iter()
        .map(|name| Mailbox {
            delimiter: Some('/'),
            encoding: NameEncoding::Utf8,
            ..Mailbox::flat(name)
        })
        .collect())
}

pub fn review(db: &Connection, account: &str, source: &str, action: Action) -> Result<Review> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        let result = review(&tx, account, source, action)?;
        tx.commit()?;
        return Ok(result);
    }
    crate::accounts::available(db, account)?;
    ready(db, account)?;
    let settings = crate::operations::stored_account(db, account)?;
    let plan = Plan::new(&Tree::new(&catalogue(db, &settings)?), source, action)?;
    anyhow::ensure!(
        plan.members.len() <= 128,
        "Review at most 128 folders at once."
    );
    let fingerprint: String = db.query_row(
        "SELECT epoch||':'||revision FROM folder_cache_revisions WHERE account_id=?1",
        [account],
        |row| row.get(0),
    )?;
    let mut messages = 0;
    for member in &plan.members {
        anyhow::ensure!(member.mailbox.role.is_none() && !["INBOX","Archive","Sent","Drafts","Trash","Spam"].iter().any(|name|member.mailbox.name.eq_ignore_ascii_case(name)) && !db.query_row("SELECT EXISTS(SELECT 1 FROM sent_folder_names WHERE account_id=?1 AND folder=?2)", params![account, member.mailbox.name], |row| row.get::<_,bool>(0))?,
            "Special-use and Sent folders need a reviewed account migration before changing them.");
        if let Some(destination) = Plan::wire_destination(member) {
            anyhow::ensure!(
                !db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM mail WHERE account_id=?1 AND folder=?2)",
                    params![account, destination],
                    |row| row.get::<_, bool>(0)
                )?,
                "The destination contains cached mail. Review it first."
            );
        }
        messages += db.query_row(
            "SELECT count(*) FROM mail WHERE account_id=?1 AND folder=?2",
            params![account, member.mailbox.name],
            |row| row.get::<_, i64>(0),
        )? as u64;
    }
    let root = plan
        .members
        .iter()
        .find(|member| member.path == plan.source)
        .context("The reviewed root is missing.")?;
    let source_label = root
        .mailbox
        .encoding
        .display(&root.mailbox.name)
        .into_owned();
    let destination_label =
        Plan::wire_destination(root).map(|name| root.mailbox.encoding.display(&name).into_owned());
    Ok(Review {
        account: account.into(),
        connection: connection_key(&settings),
        plan,
        fingerprint,
        messages,
        source_label,
        destination_label,
    })
}

pub fn admit(db: &mut Connection, id: &str, expected: Review) -> Result<Creation> {
    uuid::Uuid::parse_str(id)?;
    let tx = db.transaction()?;
    if let Some(saved) = super::get(&tx, id)? {
        anyhow::ensure!(
            saved
                .mutation
                .as_ref()
                .is_some_and(|m| m.review == expected),
            "This request identity belongs to a different folder change."
        );
        tx.commit()?;
        return Ok(saved);
    }
    let current = review(
        &tx,
        &expected.account,
        &expected.plan.source,
        expected.plan.action.clone(),
    )?;
    anyhow::ensure!(
        current == expected,
        "The folders or their messages changed. Review them again before confirming."
    );
    let total: i64 = tx.query_row("SELECT count(*) FROM folder_creations INDEXED BY folder_creation_active WHERE status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain')", [], |row| row.get(0))?;
    anyhow::ensure!(
        total < 32,
        "Review pending folder changes before adding another."
    );
    let mutation = Mutation {
        review: expected,
        completed: 0,
        receipt: None,
        observed: false,
        checked: false,
        prepared: false,
        preparing_member: 0,
        preparing_cursor: None,
        prepared_count: 0,
    };
    tx.execute("INSERT INTO folder_creations(id,account_id,connection,name,status,created,mutation,parent) VALUES(?1,?2,?3,?4,'queued',?5,?6,?7)",params![id,mutation.review.account,mutation.review.connection,mutation.review.plan.source,chrono::Utc::now().timestamp_millis(),serde_json::to_string(&mutation)?,mutation.review.plan.parent.as_ref().map(|parent|&parent.name)])?;
    let result = super::get(&tx, id)?.context("The folder change was not saved.")?;
    tx.commit()?;
    Ok(result)
}

pub fn prepare(db: &mut Connection, job: &Creation) -> Result<Creation> {
    let tx = db.transaction()?;
    checked_account(&tx, job)?;
    let mut mutation = job
        .mutation
        .clone()
        .context("The folder review is missing.")?;
    anyhow::ensure!(
        matches!(job.status.as_str(), "queued" | "waiting") && !mutation.prepared,
        "This folder change is no longer preparing."
    );
    let fingerprint: String = tx.query_row(
        "SELECT epoch||':'||revision FROM folder_cache_revisions WHERE account_id=?1",
        [&job.account],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        fingerprint == mutation.review.fingerprint,
        "The reviewed cache changed before preparation finished. Cancel and review again."
    );
    let member = mutation
        .review
        .plan
        .members
        .get(mutation.preparing_member)
        .context("The folder preparation cursor is invalid.")?;
    let copied=tx.execute("INSERT INTO folder_change_members SELECT ?1,m.id,m.folder,m.remote_id,l.token FROM mail m INDEXED BY folder_mail_snapshot JOIN mail_lineage l ON l.id=m.id WHERE m.account_id=?2 AND m.folder=?3 AND m.id>COALESCE(?4,'') ORDER BY m.id LIMIT 50",params![job.id,job.account,member.mailbox.name,mutation.preparing_cursor])?;
    mutation.prepared_count += copied as u64;
    if copied < 50 {
        mutation.preparing_member += 1;
        mutation.preparing_cursor = None;
    } else {
        mutation.preparing_cursor = tx.query_row(
            "SELECT max(id) FROM folder_change_members WHERE job=?1 AND folder=?2",
            params![job.id, member.mailbox.name],
            |row| row.get(0),
        )?;
    }
    if mutation.preparing_member == mutation.review.plan.members.len() {
        anyhow::ensure!(
            mutation.prepared_count == mutation.review.messages,
            "The exact reviewed membership could not be frozen. Cancel and review again."
        );
        mutation.prepared = true;
    }
    let mut after = job.clone();
    after.mutation = Some(mutation);
    after.status = "queued".into();
    let result = super::save(&tx, job, &after)?;
    tx.commit()?;
    Ok(result)
}

pub fn decide(
    db: &Connection,
    before: &Creation,
    revision: i64,
    decision: &str,
) -> Result<Creation> {
    anyhow::ensure!(
        before.revision == revision,
        "This change needs a fresh observation."
    );
    let mutation = before
        .mutation
        .as_ref()
        .context("The folder review is missing.")?;
    let mut after = before.clone();
    after.status = match decision {
        "cancel"
            if mutation.completed == 0
                && mutation.receipt.is_none()
                && matches!(
                    before.status.as_str(),
                    "queued" | "waiting" | "planning" | "rejected"
                ) =>
        {
            "cancelled"
        }
        "retry" if before.status == "rejected" && mutation.receipt.is_none() => "queued",
        "check"
            if matches!(
                before.status.as_str(),
                "uncertain" | "repair" | "running" | "rejected"
            ) =>
        {
            "checking"
        }
        "dismiss"
            if matches!(before.status.as_str(), "succeeded" | "cancelled")
                || mutation.checked
                    && matches!(before.status.as_str(), "uncertain" | "repair" | "rejected") =>
        {
            "dismissed"
        }
        _ => anyhow::bail!("Check this saved folder change before deciding what to do."),
    }
    .into();
    if matches!(decision, "retry" | "check") {
        checked_account(db, before)?;
    }
    after.error = None;
    super::save(db, before, &after)
}

pub fn repair(db: &mut Connection, job: &Creation) -> Result<Creation> {
    let tx = db.transaction()?;
    let account = checked_account(&tx, job)?;
    let mut mutation = job
        .mutation
        .clone()
        .context("The folder review is missing.")?;
    let step = mutation
        .receipt
        .clone()
        .context("This folder step has no saved receipt.")?;
    anyhow::ensure!(
        mutation.observed,
        "Check the acknowledged destination folders before saving the cache."
    );
    let steps = mutation.review.plan.steps();
    anyhow::ensure!(
        steps.get(mutation.completed) == Some(&step),
        "The folder receipt does not match the current step."
    );
    let source = match &step {
        Step::Rename { .. } => None,
        Step::Delete { source } | Step::Forget { source } => Some(source.as_str()),
    };
    let records = {
        let scope = if source.is_some() {
            "f.job=?1 AND f.folder=?3"
        } else {
            "f.job=?1 AND ?3 IS NULL"
        };
        let mut statement=tx.prepare(&format!("SELECT f.id,f.folder,f.remote_id,f.lineage,COALESCE(m.account_id=?2 AND m.folder=f.folder AND m.remote_id=f.remote_id AND l.token=f.lineage,0) FROM folder_change_members f LEFT JOIN mail m ON m.id=f.id LEFT JOIN mail_lineage l ON l.id=m.id WHERE {scope} ORDER BY f.id LIMIT 50"))?;
        statement
            .query_map(params![job.id, job.account, source], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    tx.execute("INSERT INTO folder_cache_authority VALUES(?1)", [&job.id])?;
    mutation.prepared_count = mutation.prepared_count.saturating_sub(records.len() as u64);
    for (id, folder, remote_id, lineage, valid) in records {
        anyhow::ensure!(
            valid,
            "A reviewed message changed identity. Check the saved receipt before continuing."
        );
        match &step {
            Step::Rename { .. } => {
                let member = mutation
                    .review
                    .plan
                    .members
                    .iter()
                    .find(|member| member.mailbox.name == folder)
                    .context("The frozen folder mapping is missing.")?;
                let destination = Plan::wire_destination(member)
                    .context("The reviewed destination is missing.")?;
                anyhow::ensure!(
                    !tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM mail WHERE account_id=?1 AND folder=?2 AND remote_id=?3)",
                        params![job.account, destination,remote_id],
                        |row| row.get::<_, bool>(0)
                    )?,
                    "The destination cache changed. Keep the receipt for review."
                );
                tx.execute(
                    "UPDATE mail SET folder=?1 WHERE id=?2",
                    params![destination, id],
                )?;
                tx.execute(
                    "UPDATE mail_lineage SET token=?1 WHERE id=?2",
                    params![lineage, id],
                )?;
                let alias = format!("{}:{destination}:{remote_id}", job.account);
                if alias != id {
                    tx.execute(
                        "INSERT INTO mail_aliases(alias,id) VALUES(?1,?2)",
                        params![alias, id],
                    )?;
                }
            }
            Step::Delete { .. } | Step::Forget { .. } => {
                tx.execute("DELETE FROM mail WHERE id=?1", [&id])?;
            }
        }
        tx.execute(
            "DELETE FROM folder_change_members WHERE job=?1 AND id=?2",
            params![job.id, id],
        )?;
    }
    let remaining: bool = tx.query_row(
        if source.is_some() {
            "SELECT EXISTS(SELECT 1 FROM folder_change_members WHERE job=?1 AND folder=?2)"
        } else {
            "SELECT EXISTS(SELECT 1 FROM folder_change_members WHERE job=?1 AND ?2 IS NULL)"
        },
        params![job.id, source],
        |row| row.get(0),
    )?;
    if remaining {
        tx.execute("DELETE FROM folder_cache_authority WHERE job=?1", [&job.id])?;
        let mut after = job.clone();
        after.mutation = Some(mutation);
        let result = super::save(&tx, job, &after)?;
        tx.commit()?;
        return Ok(result);
    }
    let catalog = catalogue(&tx, &account)?;
    super::save_catalogue(
        &tx,
        &job.account,
        &mutation.review.plan.project(&catalog, &[step]),
    )?;
    tx.execute("DELETE FROM folder_cache_authority WHERE job=?1", [&job.id])?;
    mutation.completed += 1;
    mutation.receipt = None;
    mutation.observed = false;
    mutation.checked = false;
    let mut after = job.clone();
    after.status = if mutation.completed == steps.len() {
        "succeeded"
    } else {
        "queued"
    }
    .into();
    after.mutation = Some(mutation);
    after.error = None;
    let result = super::save(&tx, job, &after)?;
    tx.commit()?;
    Ok(result)
}
