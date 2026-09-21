//! Read-only observations of existing action owners. No payloads or execution claims.
use super::*;
use rusqlite::OptionalExtension;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Domain {
    Mail,
    Moves,
    Folders,
    FolderCreations,
    Calendar,
    Outbox,
    Accounts,
    Removals,
    Credentials,
    Backups,
    Profiles,
}

impl Domain {
    pub fn label(self) -> &'static str {
        match self {
            Self::Mail => "Mail changes",
            Self::Moves => "Mail recovery",
            Self::Folders => "Folder changes",
            Self::FolderCreations => "New folders",
            Self::Calendar => "Calendar changes",
            Self::Outbox => "Outbox",
            Self::Accounts => "Account connections",
            Self::Removals => "Connection removals",
            Self::Credentials => "Device credentials",
            Self::Backups => "Backups",
            Self::Profiles => "Profiles and sync",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub domain: Domain,
    pub pending: bool,
    pub attention: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub entries: Vec<Entry>,
    pub targets: Vec<Target>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    FolderCreation { id: String, revision: u64 },
    Backup { id: String, outcome: String },
}

#[derive(Debug, Clone)]
pub enum Recovery {
    FolderCreation(super::CreationJob),
    Backup(crate::backup::history::Entry),
}

const CREATION_TARGET: &str = "SELECT json_extract(data,'$.id'),json_extract(data,'$.revision') FROM folder_creations INDEXED BY folder_creation_ready WHERE json_extract(data,'$.stage') IN ('waiting','repair','rejected','uncertain') LIMIT 1";
const CREATION_PENDING_TARGET: &str = "SELECT json_extract(data,'$.id'),json_extract(data,'$.revision') FROM folder_creations INDEXED BY folder_creation_ready WHERE json_extract(data,'$.stage') IN ('queued','running','checking') LIMIT 1";
const BACKUP_TARGET: &str = "SELECT id,outcome FROM backup_history INDEXED BY activity_backup_status WHERE outcome IN ('\"Unfinished\"','\"NeedsReview\"','\"Failed\"','\"SavedWithWarning\"') LIMIT 1";

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch("CREATE INDEX IF NOT EXISTS activity_bulk_status ON bulk_totals(status,count) WHERE count>0;
        CREATE INDEX IF NOT EXISTS activity_folder_status ON folder_steps(status,job);
        CREATE INDEX IF NOT EXISTS activity_folder_open ON folder_jobs(closed,id) WHERE closed=0;
        CREATE INDEX IF NOT EXISTS activity_backup_status ON backup_history(outcome);
        CREATE INDEX IF NOT EXISTS activity_outgoing_error ON outgoing(stage) WHERE CASE WHEN json_valid(data) THEN json_extract(data,'$.error') IS NOT NULL ELSE 0 END;
        CREATE INDEX IF NOT EXISTS activity_move_status ON mail_moves(stage) WHERE cache_id IS NOT NULL;")?;
    Ok(())
}

const QUERIES: &[(Domain, &str, &str)] = &[
    (
        Domain::Mail,
        "SELECT EXISTS(SELECT 1 FROM bulk_totals WHERE status IN ('queued','running','repair') AND count>0)",
        "SELECT EXISTS(SELECT 1 FROM bulk_totals WHERE status IN ('failed','uncertain','repair') AND count>0) OR EXISTS(SELECT 1 FROM bulk_totals t WHERE status IN ('queued','running','repair') AND count>0 AND EXISTS(SELECT 1 FROM bulk_jobs j WHERE j.id=t.job AND j.paused=1))",
    ),
    (
        Domain::Moves,
        "SELECT EXISTS(SELECT 1 FROM mail_moves WHERE stage IN ('started','copied','committed','local') AND cache_id IS NOT NULL)",
        "SELECT EXISTS(SELECT 1 FROM mail_moves WHERE stage IN ('started','copied','committed','local') AND cache_id IS NOT NULL)",
    ),
    (
        Domain::Folders,
        "SELECT EXISTS(SELECT 1 FROM folder_jobs j WHERE j.closed=0 AND EXISTS(SELECT 1 FROM folder_steps s WHERE s.job=j.id AND s.status IN ('\"Queued\"','\"Running\"','\"Acknowledged\"')))",
        "SELECT EXISTS(SELECT 1 FROM folder_jobs j WHERE j.closed=0 AND EXISTS(SELECT 1 FROM folder_steps s WHERE s.job=j.id AND s.status IN ('\"Rejected\"','\"Uncertain\"','\"Acknowledged\"')))",
    ),
    (
        Domain::FolderCreations,
        "SELECT EXISTS(SELECT 1 FROM folder_creations WHERE json_extract(data,'$.stage') IN ('queued','waiting','running','checking','repair'))",
        "SELECT EXISTS(SELECT 1 FROM folder_creations WHERE json_extract(data,'$.stage') IN ('waiting','repair','rejected','uncertain'))",
    ),
    (
        Domain::Calendar,
        "SELECT EXISTS(SELECT 1 FROM calendar_actions WHERE status IN ('queued','waiting','running','repair'))",
        "SELECT EXISTS(SELECT 1 FROM calendar_actions WHERE status IN ('waiting','repair','rejected','uncertain'))",
    ),
    (
        Domain::Outbox,
        "SELECT EXISTS(SELECT 1 FROM outgoing WHERE stage IN ('Preparing','Queued','Submitting','Accepted'))",
        "SELECT EXISTS(SELECT 1 FROM outgoing WHERE stage IN ('Uncertain','Rejected')) OR EXISTS(SELECT 1 FROM outgoing WHERE stage IN ('Preparing','Accepted') AND CASE WHEN json_valid(data) THEN json_extract(data,'$.error') IS NOT NULL ELSE 0 END)",
    ),
    (
        Domain::Accounts,
        "SELECT EXISTS(SELECT 1 FROM account_setup_attempts WHERE json_extract(data,'$.stage') IN ('Admitted','Staged','Checked'))",
        "SELECT EXISTS(SELECT 1 FROM account_setup_attempts WHERE json_extract(data,'$.stage') IN ('Failed','Interrupted'))",
    ),
    (
        Domain::Removals,
        "SELECT EXISTS(SELECT 1 FROM connection_tombstones WHERE json_extract(pending,'$.stage') IN ('queued','cleanup'))",
        "SELECT EXISTS(SELECT 1 FROM connection_tombstones WHERE json_extract(pending,'$.stage') IN ('failed','cleanup'))",
    ),
    (
        Domain::Credentials,
        "SELECT EXISTS(SELECT 1 FROM credential_cleanup)",
        "SELECT EXISTS(SELECT 1 FROM credential_cleanup) OR EXISTS(SELECT 1 FROM kv WHERE key='preferences' AND json_extract(value,'$.google_lifecycle.cleanup_pending')=1)",
    ),
    (
        Domain::Backups,
        "SELECT EXISTS(SELECT 1 FROM backup_history WHERE outcome='\"Unfinished\"')",
        "SELECT EXISTS(SELECT 1 FROM backup_history WHERE outcome IN ('\"Unfinished\"','\"NeedsReview\"','\"Failed\"','\"SavedWithWarning\"'))",
    ),
    (
        Domain::Profiles,
        "SELECT EXISTS(SELECT 1 FROM kv WHERE key='profile_replication_v1' AND (json_type(value,'$.pending')='object' OR json_extract(value,'$.deferred')!='{}')) OR EXISTS(SELECT 1 FROM kv WHERE key='profile_enrollment_v1' AND json_extract(value,'$.selection.ready')=0)",
        "SELECT EXISTS(SELECT 1 FROM kv WHERE key='profile_replication_v1' AND json_extract(value,'$.deferred')!='{}')",
    ),
];

impl Store {
    pub async fn activity_recovery(&self, target: Target) -> anyhow::Result<Recovery> {
        self.run(move |c| match target {
            Target::FolderCreation { id, revision } => {
                let raw: String = c
                    .query_row(
                        "SELECT data FROM folder_creations WHERE json_extract(data,'$.id')=?",
                        [id],
                        |row| row.get(0),
                    )
                    .optional()?
                    .context("Folder activity changed. Refresh Activity before reviewing it.")?;
                let job: super::CreationJob = serde_json::from_str(&raw)?;
                connections::allow(c, ConnectionKind::Account, &job.account)?;
                anyhow::ensure!(
                    job.revision == revision
                        && matches!(
                            job.stage,
                            super::CreationStage::Waiting
                                | super::CreationStage::Repair
                                | super::CreationStage::Rejected
                                | super::CreationStage::Uncertain
                                | super::CreationStage::Queued
                                | super::CreationStage::Running
                                | super::CreationStage::Checking
                        ),
                    "Folder activity changed. Refresh Activity before reviewing it."
                );
                Ok(Recovery::FolderCreation(job))
            }
            Target::Backup { id, outcome } => {
                let raw: String = c
                    .query_row(
                        "SELECT data FROM backup_history WHERE id=? AND outcome=?",
                        params![id, outcome],
                        |row| row.get(0),
                    )
                    .optional()?
                    .context("Backup activity changed. Refresh Activity before reviewing it.")?;
                let entry: crate::backup::history::Entry = serde_json::from_str(&raw)?;
                anyhow::ensure!(
                    entry.outcome.attention(),
                    "Backup activity changed. Refresh Activity."
                );
                Ok(Recovery::Backup(entry))
            }
        })
        .await
    }

    pub async fn activity(&self) -> anyhow::Result<Snapshot> {
        self.run(|c| {
            let tx = c.transaction()?;
            let mut entries = Vec::with_capacity(QUERIES.len());
            for &(domain, pending, attention) in QUERIES {
                entries.push(Entry {
                    domain,
                    pending: tx.query_row(pending, [], |row| row.get(0))?,
                    attention: tx.query_row(attention, [], |row| row.get(0))?,
                });
            }
            let mut targets = Vec::with_capacity(2);
            for query in [CREATION_TARGET, CREATION_PENDING_TARGET] {
                if let Some(target) = tx
                    .query_row(query, [], |row| {
                        let revision: i64 = row.get(1)?;
                        Ok(Target::FolderCreation {
                            id: row.get(0)?,
                            revision: u64::try_from(revision).map_err(|_| {
                                rusqlite::Error::IntegralValueOutOfRange(1, revision)
                            })?,
                        })
                    })
                    .optional()?
                {
                    targets.push(target);
                    break;
                }
            }
            if let Some(target) = tx
                .query_row(BACKUP_TARGET, [], |row| {
                    Ok(Target::Backup {
                        id: row.get(0)?,
                        outcome: row.get(1)?,
                    })
                })
                .optional()?
            {
                targets.push(target);
            }
            tx.commit()?;
            Ok(Snapshot { entries, targets })
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounded_queries(c: &Connection) -> anyhow::Result<()> {
        for sql in [CREATION_TARGET, CREATION_PENDING_TARGET, BACKUP_TARGET] {
            let mut statement = c.prepare(sql)?;
            let mut rows = statement.query([])?;
            let _ = rows.next()?;
            drop(rows);
            assert_eq!(
                statement.get_status(rusqlite::StatementStatus::FullscanStep),
                0,
                "{sql}"
            );
            assert!(
                statement.get_status(rusqlite::StatementStatus::VmStep) < 1000,
                "{sql}"
            );
        }
        for &(_, pending, attention) in QUERIES {
            for sql in [pending, attention] {
                let mut statement = c.prepare(sql)?;
                let _: bool = statement.query_row([], |row| row.get(0))?;
                assert_eq!(
                    statement.get_status(rusqlite::StatementStatus::FullscanStep),
                    0,
                    "{sql}"
                );
                assert!(
                    statement.get_status(rusqlite::StatementStatus::VmStep) < 1000,
                    "{sql}"
                );
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn observation_finds_old_attention_without_loading_completed_history()
    -> anyhow::Result<()> {
        let store = Store::memory()?;
        store.run(|c| {
            c.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<100000)
                INSERT INTO bulk_jobs(id,action,source,created) SELECT printf('%08d',i),'{}','{}',i FROM n;
                INSERT INTO bulk_totals(job,status,undo,count) SELECT id,'done',0,1 FROM bulk_jobs;
                INSERT INTO calendar_actions(id,origin,source,status,data) SELECT id,id,'source','succeeded','{}' FROM bulk_jobs;
                INSERT INTO backup_history(id,target,started,outcome,data) SELECT id,'target',created,'\"Saved\"','{}' FROM bulk_jobs;
                INSERT INTO folder_jobs(id,account,review,closed,created) SELECT id,id,'{}',1,created FROM bulk_jobs;
                INSERT INTO folder_steps(job,position,step,status) SELECT id,0,'{}','\"Done\"' FROM bulk_jobs;
                INSERT INTO folder_creations(account,connection,request,target,data) SELECT id,'fixture',id,'null',json_object('id',id,'revision',1,'stage','succeeded') FROM bulk_jobs;
                UPDATE bulk_jobs SET paused=1;
                UPDATE folder_steps SET status='\"Uncertain\"';")?;
            bounded_queries(c)?;
            c.execute_batch("INSERT INTO bulk_totals VALUES('00000001','uncertain',0,1);
                UPDATE calendar_actions SET status='uncertain' WHERE id='00000001';
                UPDATE folder_steps SET status='\"Rejected\"' WHERE job='00000001';
                UPDATE folder_jobs SET closed=0 WHERE id='00000001';
                UPDATE backup_history SET outcome='\"NeedsReview\"' WHERE id='00000001';")?;
            bounded_queries(c)?;
            Ok(())
        }).await?;
        let snapshot = store.activity().await?;
        assert_eq!(snapshot.entries.len(), 11);
        for domain in [
            Domain::Mail,
            Domain::Calendar,
            Domain::Folders,
            Domain::Backups,
        ] {
            assert!(
                snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.domain == domain && entry.attention)
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn exact_creation_target_survives_workspace_page_and_rejects_newer_or_removed_work()
    -> anyhow::Result<()> {
        let store = Store::memory()?;
        store.run(|c| {
            for i in 0..36 {
                let job = super::super::CreationJob {
                    id: format!("job-{i:02}"), account:"account".into(), connection:"fixture".into(), parent:None,
                    name: format!("Folder {i}"), stage: if i == 0 { super::super::CreationStage::Uncertain } else { super::super::CreationStage::Queued },
                    target:None, receipt:None, provider_acknowledged:false, error:None, revision:1,
                };
                c.execute("INSERT INTO folder_creations(account,connection,request,target,data) VALUES('account','fixture',?,'null',?)", params![job.id,serde_json::to_string(&job)?])?;
            }
            bounded_queries(c)?;
            assert!(!folder_creation::pending_jobs(c)?.iter().any(|job| job.id == "job-00"));
            Ok(())
        }).await?;
        let target = store
            .activity()
            .await?
            .targets
            .into_iter()
            .find(|target| matches!(target, Target::FolderCreation { .. }))
            .context("Exact creation target")?;
        assert_eq!(
            target,
            Target::FolderCreation {
                id: "job-00".into(),
                revision: 1
            }
        );
        assert!(
            matches!(store.activity_recovery(target.clone()).await?, Recovery::FolderCreation(job) if job.id == "job-00")
        );
        store.run(|c| { c.execute("UPDATE folder_creations SET data=json_set(data,'$.revision',2) WHERE request='job-00'", [])?; Ok(()) }).await?;
        assert!(store.activity_recovery(target).await.is_err());
        let current = Target::FolderCreation {
            id: "job-00".into(),
            revision: 2,
        };
        store.run(|c| { c.execute("INSERT INTO connection_tombstones(kind,id,revision) VALUES('account','account',1)", [])?; Ok(()) }).await?;
        assert!(store.activity_recovery(current).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn exact_backup_target_preserves_its_destination_and_rejects_resolved_history()
    -> anyhow::Result<()> {
        let store = Store::memory()?;
        let entry = crate::backup::history::Entry::new(
            crate::backup::BackupTarget::Local("owned-destination".into()),
            "Owned backup".into(),
            Default::default(),
        );
        store.write_backup_history(entry.clone()).await?;
        let target = store
            .activity()
            .await?
            .targets
            .into_iter()
            .find(|target| matches!(target, Target::Backup { .. }))
            .context("Exact backup target")?;
        assert!(
            matches!(store.activity_recovery(target.clone()).await?, Recovery::Backup(current) if current.id == entry.id && current.target == entry.target)
        );
        let mut resolved = entry;
        resolved.outcome = crate::backup::history::Outcome::Recovered;
        store.write_backup_history(resolved).await?;
        assert!(store.activity_recovery(target).await.is_err());
        Ok(())
    }

    #[tokio::test]
    async fn closed_folder_steps_and_retired_moves_do_not_resurface() -> anyhow::Result<()> {
        let store = Store::memory()?;
        store.run(|c| {
            c.execute_batch("INSERT INTO folder_jobs(id,account,review,closed,created) VALUES('closed','account','{}',1,0);
                INSERT INTO folder_steps(job,position,step,status) VALUES('closed',0,'{}','\"Uncertain\"'),('closed',1,'{}','\"Rejected\"'),('closed',2,'{}','\"Acknowledged\"');
                INSERT INTO mail_moves(token,source_id,source_account,destination_account,stage,data) VALUES('kept','one','account','account','kept','{}'),('located','two','account','account','located','{}');")?;
            Ok(())
        }).await?;
        for entry in store.activity().await?.entries {
            if matches!(entry.domain, Domain::Folders | Domain::Moves) {
                assert!(!entry.pending && !entry.attention);
            }
        }
        Ok(())
    }

    #[tokio::test]
    async fn read_does_not_recover_claim_or_hide_running_work() -> anyhow::Result<()> {
        let store = Store::memory()?;
        store.run(|c| {
            c.execute("INSERT INTO calendar_actions(id,origin,source,status,data) VALUES('held','event','calendar','running','{}')", [])?;
            Ok(())
        }).await?;
        for _ in 0..2 {
            assert!(
                store
                    .activity()
                    .await?
                    .entries
                    .iter()
                    .any(|entry| entry.domain == Domain::Calendar
                        && entry.pending
                        && !entry.attention)
            );
        }
        store
            .run(|c| {
                let status: String = c.query_row(
                    "SELECT status FROM calendar_actions WHERE id='held'",
                    [],
                    |row| row.get(0),
                )?;
                assert_eq!(status, "running");
                Ok(())
            })
            .await
    }

    #[tokio::test]
    async fn schema_ten_upgrade_preserves_attention_and_receipt_bytes() -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("activity.db");
        let store = Store::open(&path)?;
        store.run(|c| {
            c.execute("INSERT INTO calendar_actions(id,origin,source,status,data) VALUES('old','event','calendar','uncertain','{\"receipt\":\"retained exact bytes\"}')", [])?;
            c.execute_batch("DROP INDEX activity_bulk_status; DROP INDEX activity_folder_status; DROP INDEX activity_folder_open; DROP INDEX activity_backup_status; DROP INDEX activity_move_status; DROP INDEX activity_outgoing_error; PRAGMA user_version=10;")?;
            Ok(())
        }).await?;
        drop(store);
        let reopened = Store::open(&path)?;
        assert!(
            reopened
                .activity()
                .await?
                .entries
                .iter()
                .any(|entry| entry.domain == Domain::Calendar && entry.attention)
        );
        reopened
            .run(|c| {
                assert_eq!(
                    c.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))?,
                    11
                );
                assert_eq!(
                    c.query_row(
                        "SELECT data FROM calendar_actions WHERE id='old'",
                        [],
                        |row| row.get::<_, String>(0)
                    )?,
                    "{\"receipt\":\"retained exact bytes\"}"
                );
                Ok(())
            })
            .await
    }
}
