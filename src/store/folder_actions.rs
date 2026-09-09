//! Durable folder commands and atomic cache migrations. MIME stays in SQLite;
//! the executor owns a lease and records acknowledgment before applying metadata.
use super::*;
use crate::folder_actions::{Action, Job, Outcome, Plan, Progress, Review, Status, Step};
use crate::folders::{Mailbox, Tree};
use rusqlite::OptionalExtension;
use std::collections::{HashMap, HashSet};

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS folder_jobs(
        id TEXT PRIMARY KEY,account TEXT NOT NULL,review TEXT NOT NULL,closed INTEGER NOT NULL DEFAULT 0,created INTEGER NOT NULL);
        CREATE UNIQUE INDEX IF NOT EXISTS folder_account_work ON folder_jobs(account) WHERE closed=0;
        CREATE TABLE IF NOT EXISTS folder_steps(
        job TEXT NOT NULL REFERENCES folder_jobs(id) ON DELETE CASCADE,position INTEGER NOT NULL,
        step TEXT NOT NULL,status TEXT NOT NULL,error TEXT,PRIMARY KEY(job,position));")?;
    let has_revision = c
        .prepare("PRAGMA table_info(folder_jobs)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == "revision");
    if !has_revision {
        c.execute_batch("ALTER TABLE folder_jobs ADD COLUMN revision INTEGER NOT NULL DEFAULT 0;")?;
    }
    c.execute_batch("CREATE TRIGGER IF NOT EXISTS folder_step_insert_revision AFTER INSERT ON folder_steps BEGIN UPDATE folder_jobs SET revision=revision+1 WHERE id=NEW.job; END;
        CREATE TRIGGER IF NOT EXISTS folder_step_update_revision AFTER UPDATE ON folder_steps BEGIN UPDATE folder_jobs SET revision=revision+1 WHERE id=NEW.job; END;")?;
    Ok(())
}

pub(super) fn idle(c: &Connection, account: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !c.query_row(
            "SELECT EXISTS(SELECT 1 FROM folder_jobs WHERE account=? AND closed=0)",
            [account],
            |r| r.get::<_, bool>(0)
        )?,
        "A folder change is unfinished for this account. Review it before making another mail change."
    );
    Ok(())
}

pub(super) fn mail_idle(c: &Connection, id: &str) -> anyhow::Result<()> {
    if let Some(account) = c
        .query_row("SELECT account FROM messages WHERE id=?", [id], |r| {
            r.get::<_, String>(0)
        })
        .optional()?
    {
        idle(c, &account)?;
    }
    Ok(())
}

pub(super) fn account_review(
    c: &Connection,
    account: &str,
    digest: &mut sha2::Sha256,
) -> anyhow::Result<usize> {
    use sha2::Digest;
    let mut statement = c.prepare("SELECT json_array(j.id,j.review,j.closed,s.position,s.step,s.status,s.error),j.closed FROM folder_jobs j JOIN folder_steps s ON s.job=j.id WHERE j.account=? ORDER BY j.id,s.position")?;
    let mut rows = statement.query([account])?;
    let mut pending = 0;
    while let Some(row) = rows.next()? {
        let data: String = row.get(0)?;
        digest.update((data.len() as u64).to_le_bytes());
        digest.update(data);
        if !row.get::<_, bool>(1)? {
            pending += 1;
        }
    }
    Ok(pending)
}

fn catalog(c: &Connection, account: &str) -> anyhow::Result<Vec<Mailbox>> {
    let mut catalogs: HashMap<String, Vec<Mailbox>> = get(c, "folder_catalogs")?;
    catalogs
        .remove(account)
        .context("Refresh this account's folder list before changing folders.")
}

/// POP3 has no remote folder namespace. Initialize local folders and upgrade
/// simple legacy local names without reinterpreting literal slash names or a
/// reviewed operation's namespace. Never recreate folders the user deleted.
pub(super) fn prepare_local_catalogs(c: &mut Connection) -> anyhow::Result<()> {
    let accounts: Vec<Account> = get(c, "accounts")?;
    if !accounts.iter().any(|a| a.protocol == Protocol::Pop3) {
        return Ok(());
    }
    let tx = c.transaction()?;
    let mut catalogs: HashMap<String, Vec<Mailbox>> = get(&tx, "folder_catalogs")?;
    let mut names: HashMap<String, Vec<String>> = get(&tx, "account_folders")?;
    let mut changed = false;
    for account in accounts.iter().filter(|a| a.protocol == Protocol::Pop3) {
        if idle(&tx, &account.id).is_err() {
            continue;
        }
        let current = catalogs.entry(account.id.clone()).or_default();
        let before = current.clone();
        if current.is_empty() {
            let mut initial = names.get(&account.id).cloned().unwrap_or_default();
            if initial.is_empty() {
                initial.extend(["INBOX", "Archive", "Sent", "Trash"].map(str::to_owned));
            }
            *current = initial.into_iter().map(Mailbox::flat).collect();
        }
        // New imports and local message moves can introduce a folder after the
        // first catalog was created. A committed delete removed that mail too,
        // so this does not resurrect an empty deleted folder.
        let cached = tx
            .prepare("SELECT DISTINCT folder FROM messages WHERE account=?")?
            .query_map([&account.id], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for name in cached {
            if !current.iter().any(|m| m.name == name) {
                current.push(Mailbox::flat(name));
            }
        }
        for mailbox in current.iter_mut() {
            if mailbox.delimiter.is_none() && !mailbox.name.contains('/') {
                mailbox.delimiter = Some('/');
            }
        }
        if *current != before {
            changed = true;
            names.insert(
                account.id.clone(),
                current
                    .iter()
                    .filter(|m| m.selectable)
                    .map(|m| m.name.clone())
                    .collect(),
            );
        }
    }
    if changed {
        put(&tx, "folder_catalogs", &catalogs)?;
        put(&tx, "account_folders", &names)?;
        connections::changed(&tx)?;
    }
    tx.commit()?;
    Ok(())
}

fn review(c: &Connection, account: &str, source: &str, action: Action) -> anyhow::Result<Review> {
    connections::allow(c, ConnectionKind::Account, account)?;
    let config: Account = get::<Vec<Account>>(c, "accounts")?
        .into_iter()
        .find(|a| a.id == account)
        .context("This account was removed.")?;
    let plan = Plan::new(&Tree::new(&catalog(c, account)?), source, action)?;
    let mut cached_messages = 0;
    let mut names = HashSet::new();
    for member in &plan.members {
        names.insert(member.mailbox.name.as_str());
        names.insert(member.path.as_str());
    }
    for name in &names {
        cached_messages += c.query_row(
            "SELECT COUNT(*) FROM messages WHERE account=? AND folder=?",
            params![account, name],
            |r| r.get::<_, i64>(0),
        )? as usize;
    }
    let affected_history = c.query_row("SELECT COUNT(*) FROM bulk_items WHERE (json_extract(original,'$.account_id')=?1 AND json_extract(original,'$.folder') IN (SELECT value FROM json_each(?2)))
        OR (json_extract(receipt,'$.Move.account')=?1 AND json_extract(receipt,'$.Move.folder') IN (SELECT value FROM json_each(?2)))", params![account,serde_json::to_string(&names)?], |r| r.get::<_, i64>(0))? as usize;
    for member in &plan.members {
        if let Some(destination) = &member.destination {
            anyhow::ensure!(
                !c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE account=? AND folder=?)",
                    params![account, destination],
                    |r| r.get::<_, bool>(0)
                )?,
                "The destination contains cached mail. Choose another folder or recover those cached copies before moving this folder."
            );
        }
    }
    Ok(Review {
        account: account.into(),
        connection: crate::mail_actions::connection_key(&config),
        imap: config.protocol == Protocol::Imap,
        plan,
        cached_messages,
        affected_history,
    })
}

fn ready(c: &Connection, account: &str) -> anyhow::Result<()> {
    use sha2::Digest;
    let (_, pending) = bulk::account_review(c, account, &mut sha2::Sha256::new())?;
    anyhow::ensure!(
        pending == 0
            && connections::transfers(c, account)?.is_empty()
            && move_journal::account_review(c, account, &mut sha2::Sha256::new())? == 0,
        "Finish or review the account's pending mail changes before changing folders."
    );
    anyhow::ensure!(!c.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE account=? AND stage NOT IN ('Complete','Released','Rejected'))", [account], |r| r.get::<_, bool>(0))?,
        "Finish the account's outgoing messages before changing folders.");
    Ok(())
}

fn job(c: &Connection, id: &str) -> anyhow::Result<Job> {
    let (review, closed, revision): (String, bool, i64) = c.query_row(
        "SELECT review,closed,revision FROM folder_jobs WHERE id=?",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let steps = c
        .prepare(
            "SELECT position,step,status,error FROM folder_steps WHERE job=? ORDER BY position",
        )?
        .query_map([id], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get(3)?,
            ))
        })?
        .map(|row| {
            let (position, step, status, error) = row?;
            Ok(Progress {
                position: usize::try_from(position)?,
                step: serde_json::from_str(&step)?,
                status: serde_json::from_str(&status)?,
                error,
            })
        })
        .collect::<anyhow::Result<_>>()?;
    let review: Review = serde_json::from_str(&review)?;
    let root = review
        .plan
        .members
        .iter()
        .find(|m| m.path == review.plan.source)
        .context("The saved folder review has no source folder")?;
    let label = root
        .mailbox
        .encoding
        .display(&review.plan.source)
        .into_owned();
    let destination_label =
        Plan::wire_destination(root).map(|p| root.mailbox.encoding.display(&p).into_owned());
    Ok(Job {
        query_counts: None,
        revision: u64::try_from(revision)?,
        label,
        destination_label,
        id: id.into(),
        review,
        steps,
        closed,
    })
}

pub struct FolderLease {
    store: Store,
    id: String,
    _lease: BulkLease,
}
impl FolderLease {
    fn check(&self, store: &Store) -> anyhow::Result<String> {
        anyhow::ensure!(
            Arc::ptr_eq(&self.store.0, &store.0),
            "Use the folder lease for this mail cache."
        );
        Ok(self.id.clone())
    }
}

impl Store {
    pub async fn current_folder_catalog(&self, account: String) -> anyhow::Result<Vec<Mailbox>> {
        self.run(move |c| catalog(c, &account)).await
    }
    pub async fn ensure_folder_idle(&self, account: String) -> anyhow::Result<()> {
        self.run(move |c| idle(c, &account)).await
    }
    pub async fn folder_review(
        &self,
        account: String,
        source: String,
        action: Action,
    ) -> anyhow::Result<Review> {
        self.run(move |c| {
            idle(c, &account)?;
            ready(c, &account)?;
            review(c, &account, &source, action)
        })
        .await
    }
    pub async fn start_folder_change(&self, id: String, expected: Review) -> anyhow::Result<Job> {
        self.start_folder_change_scoped(id, expected, None).await
    }
    pub(crate) async fn start_folder_change_scoped(
        &self,
        id: String,
        expected: Review,
        projection: Option<String>,
    ) -> anyhow::Result<Job> {
        self.run(move |c| {
            let tx = c.transaction()?;
            if tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM folder_jobs WHERE id=?)",
                [&id],
                |r| r.get::<_, bool>(0),
            )? {
                let current = job(&tx, &id)?;
                anyhow::ensure!(
                    current.review == expected,
                    "This folder operation belongs to another review."
                );
                return Ok(current);
            }
            idle(&tx, &expected.account)?;
            ready(&tx, &expected.account)?;
            let current = review(
                &tx,
                &expected.account,
                &expected.plan.source,
                expected.plan.action.clone(),
            )?;
            anyhow::ensure!(
                current == expected,
                "The folders or their contents changed. Review the updated scope before continuing."
            );
            tx.execute(
                "INSERT INTO folder_jobs(id,account,review,created) VALUES(?,?,?,?)",
                params![
                    id,
                    expected.account,
                    serde_json::to_string(&expected)?,
                    chrono::Utc::now().timestamp_millis()
                ],
            )?;
            if let Some(token) = &projection {
                super::folder_projection::bind(&tx, token, &id)?;
            }
            for (position, step) in expected.plan.steps().iter().enumerate() {
                tx.execute(
                    "INSERT INTO folder_steps(job,position,step,status) VALUES(?,?,?,?)",
                    params![
                        id,
                        position as i64,
                        serde_json::to_string(step)?,
                        serde_json::to_string(&Status::Queued)?
                    ],
                )?;
            }
            connections::changed(&tx)?;
            let current = job(&tx, &id)?;
            tx.commit()?;
            Ok(current)
        })
        .await
    }
    pub async fn folder_job(&self, id: String) -> anyhow::Result<Job> {
        self.run(move |c| job(c, &id)).await
    }
    pub async fn next_pending_folder(&self, after: String) -> anyhow::Result<Option<String>> {
        self.run(move |c| {
            Ok(c.query_row(
                "SELECT id FROM folder_jobs WHERE closed=0 AND id>? ORDER BY id LIMIT 1",
                [after],
                |r| r.get(0),
            )
            .optional()?)
        })
        .await
    }
    pub async fn folder_jobs(&self, offset: usize) -> anyhow::Result<Vec<Job>> {
        self.run(move |c| {
            c.prepare(
                "SELECT id FROM folder_jobs ORDER BY closed,created DESC,id LIMIT 20 OFFSET ?",
            )?
            .query_map([i64::try_from(offset)?], |r| r.get::<_, String>(0))?
            .map(|id| job(c, &id?))
            .collect()
        })
        .await
    }
    pub async fn folder_lease(&self, id: String) -> anyhow::Result<FolderLease> {
        Ok(FolderLease {
            _lease: self.bulk_lease(format!("folder:{id}")).await?,
            store: self.clone(),
            id,
        })
    }
    /// Only the lease holder may classify a previous executor's interrupted step.
    /// Queued work remains queued; a missing acknowledgment never authorizes replay.
    pub async fn recover_folder_change(&self, lease: &FolderLease) -> anyhow::Result<Job> {
        let id = lease.check(self)?;
        self.run(move |c| {
            let tx = c.transaction()?;
            let imap = job(&tx, &id)?.review.imap;
            tx.execute("UPDATE folder_steps SET status=?,error=? WHERE job=? AND status=?", params![serde_json::to_string(&if imap { Status::Uncertain } else { Status::Queued })?,imap.then_some("Shep closed before recording the server's answer. Check the folders before continuing."),id,serde_json::to_string(&Status::Running)?])?;
            let current = job(&tx, &id)?; tx.commit()?; Ok(current)
        }).await
    }
    pub async fn claim_folder_step(&self, lease: &FolderLease) -> anyhow::Result<Option<Progress>> {
        let id = lease.check(self)?;
        self.run(move |c| {
            let tx = c.transaction()?;
            let current = job(&tx, &id)?;
            if current.closed {
                return Ok(None);
            }
            connections::allow(&tx, ConnectionKind::Account, &current.review.account)?;
            let config = get::<Vec<Account>>(&tx, "accounts")?
                .into_iter()
                .find(|a| a.id == current.review.account)
                .context("This account was removed.")?;
            anyhow::ensure!(
                crate::mail_actions::connection_key(&config) == current.review.connection,
                "This account's server changed. Review the unfinished folder change."
            );
            ready(&tx, &current.review.account)?;
            if current.steps.iter().any(|step| {
                matches!(
                    step.status,
                    Status::Running | Status::Acknowledged | Status::Rejected | Status::Uncertain
                )
            }) {
                return Ok(None);
            }
            let Some(mut next) = current
                .steps
                .into_iter()
                .find(|step| step.status == Status::Queued)
            else {
                return Ok(None);
            };
            tx.execute(
                "UPDATE folder_steps SET status=? WHERE job=? AND position=?",
                params![
                    serde_json::to_string(&Status::Running)?,
                    id,
                    next.position as i64
                ],
            )?;
            next.status = Status::Running;
            tx.commit()?;
            Ok(Some(next))
        })
        .await
    }
    pub async fn record_folder_outcome(
        &self,
        lease: &FolderLease,
        position: usize,
        outcome: Outcome,
    ) -> anyhow::Result<Job> {
        let id = lease.check(self)?;
        self.run(move |c| {
            let tx = c.transaction()?;
            let (status, error) = match outcome { Outcome::Applied => (Status::Acknowledged, None), Outcome::Rejected(error) => (Status::Rejected, Some(error)), Outcome::Uncertain(error) => (Status::Uncertain, Some(error)) };
            anyhow::ensure!(tx.execute("UPDATE folder_steps SET status=?,error=? WHERE job=? AND position=? AND status=?", params![serde_json::to_string(&status)?,error,id,position as i64,serde_json::to_string(&Status::Running)?])? == 1, "This folder result is no longer current.");
            let current = job(&tx, &id)?; tx.commit()?; Ok(current)
        }).await
    }
    pub async fn commit_folder_step(
        &self,
        lease: &FolderLease,
        position: usize,
    ) -> anyhow::Result<Job> {
        let id = lease.check(self)?;
        self.run(move |c| {
            let tx = c.transaction()?;
            let current = job(&tx, &id)?;
            let step = current
                .steps
                .get(position)
                .context("This folder step is missing.")?;
            if step.status == Status::Done {
                return Ok(current);
            }
            anyhow::ensure!(
                step.status == Status::Acknowledged,
                "The server has not acknowledged this folder change."
            );
            connections::allow(&tx, ConnectionKind::Account, &current.review.account)?;
            commit(&tx, &current.review, &step.step)?;
            tx.execute(
                "UPDATE folder_steps SET status=?,error=NULL WHERE job=? AND position=?",
                params![serde_json::to_string(&Status::Done)?, id, position as i64],
            )?;
            close_if_finished(&tx, &id)?;
            connections::changed(&tx)?;
            let current = job(&tx, &id)?;
            tx.commit()?;
            Ok(current)
        })
        .await
    }
    /// A definite rejection may be retried after a fresh provider preflight.
    pub async fn retry_folder_change(&self, lease: &FolderLease) -> anyhow::Result<Job> {
        let id = lease.check(self)?;
        self.run(move |c| {
            let tx = c.transaction()?;
            let current = job(&tx, &id)?;
            anyhow::ensure!(
                !current.closed
                    && !current.steps.iter().any(|step| matches!(
                        step.status,
                        Status::Uncertain | Status::Running | Status::Acknowledged
                    )),
                "Review the unconfirmed result before continuing."
            );
            tx.execute(
                "UPDATE folder_steps SET status=?,error=NULL WHERE job=? AND status=?",
                params![
                    serde_json::to_string(&Status::Queued)?,
                    id,
                    serde_json::to_string(&Status::Rejected)?
                ],
            )?;
            let current = job(&tx, &id)?;
            tx.commit()?;
            Ok(current)
        })
        .await
    }
    /// Explicit acceptance never invents a receipt. Completed cache changes stay
    /// completed; all remaining destructive work is retired without execution.
    pub async fn stop_folder_change(
        &self,
        lease: &FolderLease,
        accept_uncertainty: bool,
    ) -> anyhow::Result<Job> {
        let id = lease.check(self)?;
        self.run(move |c| {
            let tx = c.transaction()?;
            let current = job(&tx, &id)?;
            anyhow::ensure!(
                !current
                    .steps
                    .iter()
                    .any(|step| matches!(step.status, Status::Running | Status::Acknowledged)),
                "Wait for the current folder result to be saved."
            );
            anyhow::ensure!(
                accept_uncertainty
                    || !current
                        .steps
                        .iter()
                        .any(|step| step.status == Status::Uncertain),
                "Confirm that you checked the server's folder state first."
            );
            for step in current.steps.iter().filter(|step| {
                matches!(
                    step.status,
                    Status::Queued | Status::Rejected | Status::Uncertain
                )
            }) {
                let status = if step.status == Status::Uncertain {
                    Status::Accepted
                } else {
                    Status::Cancelled
                };
                tx.execute(
                    "UPDATE folder_steps SET status=?,error=? WHERE job=? AND position=?",
                    params![
                        serde_json::to_string(&status)?,
                        "Remaining folder changes stopped. No further server command will be sent.",
                        id,
                        step.position as i64
                    ],
                )?;
            }
            close_if_finished(&tx, &id)?;
            connections::changed(&tx)?;
            let current = job(&tx, &id)?;
            tx.commit()?;
            Ok(current)
        })
        .await
    }
}

fn close_if_finished(c: &Connection, id: &str) -> anyhow::Result<()> {
    if job(c, id)?.steps.iter().all(|step| {
        matches!(
            step.status,
            Status::Done | Status::Accepted | Status::Cancelled
        )
    }) {
        c.execute(
            "UPDATE folder_jobs SET closed=1,revision=revision+1 WHERE id=? AND closed=0",
            [id],
        )?;
    }
    Ok(())
}

fn commit(c: &Connection, review: &Review, step: &Step) -> anyhow::Result<()> {
    let account = &review.account;
    let mut changes: HashMap<String, Option<String>> = HashMap::new();
    for member in &review.plan.members {
        let applies = match step {
            Step::Rename { .. } => true,
            Step::Delete { source } | Step::Forget { source } => source == &member.mailbox.name,
        };
        if applies {
            changes.insert(member.path.clone(), member.destination.clone());
            changes.insert(member.mailbox.name.clone(), Plan::wire_destination(member));
        }
    }
    for (source, destination) in &changes {
        if let Some(destination) = destination {
            move_cache(c, account, source, destination, review.imap)?;
        } else {
            c.execute(
                "DELETE FROM messages WHERE account=? AND folder=?",
                params![account, source],
            )?;
        }
    }
    migrate_history(c, review, &changes)?;
    let mut catalogs: HashMap<String, Vec<Mailbox>> = get(c, "folder_catalogs")?;
    let previous = catalogs
        .get(account)
        .context("This account's folder catalog is missing.")?;
    let catalog = review.plan.project(previous, std::slice::from_ref(step));
    let mut folders: HashMap<String, Vec<String>> = get(c, "account_folders")?;
    folders.insert(
        account.clone(),
        catalog
            .iter()
            .filter(|mailbox| mailbox.selectable)
            .map(|mailbox| mailbox.name.clone())
            .collect(),
    );
    catalogs.insert(account.clone(), catalog);
    put(c, "folder_catalogs", &catalogs)?;
    put(c, "account_folders", &folders)?;
    put(
        c,
        "folders",
        &folders
            .values()
            .flatten()
            .collect::<std::collections::BTreeSet<_>>(),
    )?;
    let mut preferences: Preferences = get(c, "preferences")?;
    if let Some(expanded) = preferences.expanded_folders.get_mut(account) {
        *expanded = expanded
            .iter()
            .filter_map(|path| match changes.get(path) {
                Some(destination) => destination.clone(),
                None => Some(path.clone()),
            })
            .collect();
        put(c, "preferences", &preferences)?;
    }
    let mut accounts: Vec<Account> = get(c, "accounts")?;
    for config in accounts.iter_mut().filter(|config| &config.id == account) {
        if let Some(destination) = changes.get(&config.sent_folder) {
            config.sent_folder = destination.clone().unwrap_or_default();
        }
    }
    put(c, "accounts", &accounts)?;
    let sent: Option<String> = c
        .query_row(
            "SELECT folder FROM sent_folders WHERE account=?",
            [account],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(folder) = sent
        && let Some(destination) = changes.get(&folder)
    {
        if let Some(destination) = destination {
            c.execute(
                "UPDATE sent_folders SET folder=? WHERE account=?",
                params![destination, account],
            )?;
        } else {
            c.execute("DELETE FROM sent_folders WHERE account=?", [account])?;
        }
    }
    let mut after = String::new();
    loop {
        let row: Option<(String, String)> = c.query_row("SELECT attempt,data FROM outgoing WHERE account=? AND attempt>? ORDER BY attempt LIMIT 1", params![account,after], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((attempt, data)) = row else { break };
        after.clone_from(&attempt);
        let mut info: crate::outgoing::OutgoingInfo = serde_json::from_str(&data)?;
        if let Some(folder) = &info.folder
            && let Some(destination) = changes.get(folder)
        {
            info.folder.clone_from(destination);
            c.execute(
                "UPDATE outgoing SET data=? WHERE attempt=?",
                params![serde_json::to_string(&info)?, attempt],
            )?;
        }
    }
    outgoing::changed(c)?;
    Ok(())
}

fn remap_mail(
    mail: &mut Mail,
    account: &str,
    changes: &HashMap<String, Option<String>>,
    imap: bool,
) -> bool {
    if mail.account_id != account {
        return false;
    }
    let Some(destination) = changes.get(&mail.folder) else {
        return false;
    };
    if let Some(destination) = destination {
        mail.folder.clone_from(destination);
        if imap && !mail.is_local_copy() {
            mail.id = format!("{account}:{destination}:{}", mail.remote_id);
        }
    }
    true
}

fn move_cache(
    c: &Connection,
    account: &str,
    source: &str,
    destination: &str,
    imap: bool,
) -> anyhow::Result<()> {
    // Keyset iteration does not load all message metadata or any raw MIME into a
    // folder job. Copy the existing conversation identity, even without Message-ID.
    let mut after = String::new();
    loop {
        let row: Option<(String, String, bool, bool)> = c.query_row("SELECT id,data,unread,starred FROM messages WHERE account=? AND folder=? AND id>? ORDER BY id LIMIT 1", params![account,source,after], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
        let Some((id, data, unread, starred)) = row else {
            break;
        };
        after.clone_from(&id);
        let mut mail: Mail = serde_json::from_str(&data)?;
        mail.unread = unread;
        mail.starred = starred;
        mail.folder = source.into();
        remap_mail(
            &mut mail,
            account,
            &[(source.into(), Some(destination.into()))].into(),
            imap,
        );
        if id == mail.id {
            c.execute(
                "UPDATE messages SET folder=?,data=? WHERE id=?",
                params![destination, serde_json::to_string(&mail)?, id],
            )?;
        } else {
            anyhow::ensure!(
                !c.query_row(
                    "SELECT EXISTS(SELECT 1 FROM messages WHERE id=?)",
                    [&mail.id],
                    |r| r.get::<_, bool>(0)
                )?,
                "A cached message already uses the destination identity. The acknowledged folder change needs cache recovery."
            );
            c.execute("INSERT INTO messages(id,account,folder,sender,subject,body,timestamp,unread,starred,data,raw) SELECT ?1,account,?2,sender,subject,body,timestamp,unread,starred,?3,raw FROM messages WHERE id=?4", params![mail.id,destination,serde_json::to_string(&mail)?,id])?;
            c.execute(
                "INSERT INTO restored_messages(id) SELECT ?1 FROM restored_messages WHERE id=?2",
                params![mail.id, id],
            )?;
            c.execute("INSERT INTO conversation_members(id,account,group_id,logical_id,timestamp) SELECT ?1,account,group_id,logical_id,timestamp FROM conversation_members WHERE id=?2", params![mail.id,id])?;
            c.execute("DELETE FROM messages WHERE id=?", [&id])?;
        }
    }
    Ok(())
}

fn migrate_history(
    c: &Connection,
    review: &Review,
    changes: &HashMap<String, Option<String>>,
) -> anyhow::Result<()> {
    type HistoryRow = (String, i64, String, Option<String>, Option<String>);
    let mut after = (String::new(), -1_i64);
    loop {
        let row: Option<HistoryRow> = c.query_row("SELECT job,position,id,original,receipt FROM bulk_items WHERE (job,position)>(?1,?2) AND
            (json_extract(original,'$.account_id')=?3 OR json_extract(receipt,'$.Move.account')=?3) ORDER BY job,position LIMIT 1", params![after.0,after.1,review.account], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        let Some((job, position, mut id, original, receipt)) = row else {
            break;
        };
        after = (job.clone(), position);
        let mut original: Option<Mail> = original
            .map(|data| serde_json::from_str(&data))
            .transpose()?;
        let mut receipt: Option<crate::bulk::Receipt> = receipt
            .map(|data| serde_json::from_str(&data))
            .transpose()?;
        let mut affected = false;
        if let Some(original) = &mut original {
            let old = original.id.clone();
            affected |= remap_mail(original, &review.account, changes, review.imap);
            if id == old {
                id.clone_from(&original.id);
            }
        }
        if let Some(crate::bulk::Receipt::Move(receipt)) = &mut receipt {
            if receipt.account == review.account
                && let Some(destination) = changes.get(&receipt.folder)
            {
                affected = true;
                if let Some(destination) = destination {
                    receipt.folder.clone_from(destination);
                }
            }
            if let Some(current) = &mut receipt.current {
                let old = current.id.clone();
                affected |= remap_mail(current, &review.account, changes, review.imap);
                if id == old {
                    id.clone_from(&current.id);
                }
            }
        }
        if affected {
            if matches!(review.plan.action, Action::Delete) {
                c.execute("UPDATE bulk_items SET status='cancelled',error='The folder was deleted. This history item no longer offers Undo.' WHERE job=? AND position=?", params![job,position])?;
            } else {
                c.execute(
                    "UPDATE bulk_items SET id=?,original=?,receipt=? WHERE job=? AND position=?",
                    params![
                        id,
                        original
                            .map(|mail| serde_json::to_string(&mail))
                            .transpose()?,
                        receipt
                            .map(|receipt| serde_json::to_string(&receipt))
                            .transpose()?,
                        job,
                        position
                    ],
                )?;
            }
        }
    }
    let revision: u64 = get(c, "bulk_revision")?;
    put(
        c,
        "bulk_revision",
        &revision
            .checked_add(1)
            .context("Mail history revision exhausted")?,
    )?;
    Ok(())
}
