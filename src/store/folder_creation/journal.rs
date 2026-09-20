use super::*;

const PENDING_JOBS: &str = "SELECT data FROM folder_creations INDEXED BY folder_creation_ready WHERE json_extract(data,'$.stage') IN ('queued','waiting','running','checking','repair','rejected','uncertain') ORDER BY rowid DESC LIMIT 32";
const AT_CAPACITY: &str = "SELECT EXISTS(SELECT 1 FROM folder_creations INDEXED BY folder_creation_ready WHERE json_extract(data,'$.stage') IN ('queued','waiting','running','checking','repair','rejected','uncertain') LIMIT 1 OFFSET 31)";

#[cfg(test)]
mod query_tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CreationStage {
    Queued,
    Waiting,
    Running,
    Checking,
    Repair,
    Succeeded,
    Rejected,
    Uncertain,
    Cancelled,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CreationJob {
    pub id: String,
    pub account: String,
    pub connection: String,
    pub parent: Option<String>,
    pub name: String,
    pub stage: CreationStage,
    pub target: Option<Mailbox>,
    pub receipt: Option<Mailbox>,
    #[serde(default)]
    pub provider_acknowledged: bool,
    pub error: Option<String>,
    pub revision: u64,
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    let columns = c
        .prepare("PRAGMA table_info(folder_creations)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if !columns.iter().any(|column| column == "data") {
        c.execute_batch("ALTER TABLE folder_creations ADD COLUMN data TEXT;")?;
    }
    c.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS folder_creation_id ON folder_creations(json_extract(data,'$.id'));
        CREATE INDEX IF NOT EXISTS folder_creation_ready ON folder_creations(json_extract(data,'$.stage'),json_extract(data,'$.id'));")?;
    loop {
        let legacy = c
        .prepare(
            "SELECT account,connection,request,target FROM folder_creations INDEXED BY folder_creation_ready WHERE json_extract(data,'$.stage') IS NULL AND data IS NULL LIMIT 32",
        )?
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
        if legacy.is_empty() {
            break;
        }
        for (account, connection, request, target) in legacy {
            let (parent, name) = serde_json::from_str(&request)?;
            let job = CreationJob {
                id: uuid::Uuid::new_v4().to_string(),
                account,
                connection,
                parent,
                name,
                stage: CreationStage::Uncertain,
                target: Some(serde_json::from_str(&target)?),
                receipt: None,
                provider_acknowledged: false,
                error: Some("Check this saved folder on the server before continuing.".into()),
                revision: 1,
            };
            c.execute(
                "UPDATE folder_creations SET data=? WHERE account=? AND connection=? AND request=?",
                params![
                    serde_json::to_string(&job)?,
                    job.account,
                    job.connection,
                    request
                ],
            )?;
        }
    }
    Ok(())
}

fn load(c: &Connection, id: &str) -> anyhow::Result<CreationJob> {
    let data: String = c.query_row(
        "SELECT data FROM folder_creations WHERE json_extract(data,'$.id')=?",
        [id],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&data)?)
}

pub(crate) fn pending_jobs(c: &Connection) -> anyhow::Result<Vec<CreationJob>> {
    c.prepare(PENDING_JOBS)?
        .query_map([], |r| r.get::<_, String>(0))?
        .map(|r| Ok(serde_json::from_str(&r?)?))
        .collect()
}

fn save(c: &Connection, job: &CreationJob) -> anyhow::Result<()> {
    c.execute(
        "UPDATE folder_creations SET target=?,data=? WHERE json_extract(data,'$.id')=?",
        params![
            serde_json::to_string(&job.target)?,
            serde_json::to_string(job)?,
            job.id
        ],
    )?;
    Ok(())
}

pub(crate) fn fence_import(c: &Connection, note: &str) -> anyhow::Result<()> {
    schema(c)?;
    c.execute("UPDATE folder_creations SET data=json_set(data,'$.stage','uncertain','$.error',?,'$.revision',json_extract(data,'$.revision')+1)
        WHERE json_extract(data,'$.stage') NOT IN ('succeeded','cancelled','dismissed','repair')",[note])?;
    Ok(())
}

impl Store {
    pub(crate) async fn dismiss_creation(
        &self,
        id: String,
        revision: u64,
    ) -> anyhow::Result<CreationJob> {
        let job = self.creation_job(id).await?;
        anyhow::ensure!(
            job.revision == revision && job.stage == CreationStage::Uncertain,
            "Refresh this unconfirmed folder request before stopping tracking."
        );
        self.update_creation(job,CreationStage::Dismissed,None,None,Some("Tracking stopped without confirming the server result. No server folders were deleted.".into())).await
    }
    pub(crate) async fn finish_local_creation(
        &self,
        expected: CreationJob,
    ) -> anyhow::Result<CreationJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = load(&tx, &expected.id)?;
            let account = checked_account(&tx, &job.account, &job.connection)?;
            anyhow::ensure!(
                account.protocol == Protocol::Pop3
                    && job == expected
                    && job.stage == CreationStage::Queued,
                "The local folder request changed."
            );
            let catalogs: HashMap<String, Vec<Mailbox>> = get(&tx, "folder_catalogs")?;
            let mut catalog = catalogs.get(&job.account).cloned().unwrap_or_default();
            let root = Mailbox {
                delimiter: Some('/'),
                encoding: catalog.first().map(|m| m.encoding).unwrap_or_default(),
                ..Mailbox::flat(String::new())
            };
            let parent = job
                .parent
                .as_ref()
                .map(|name| {
                    catalog
                        .iter()
                        .find(|m| &m.name == name)
                        .context("The parent folder is no longer available.")
                })
                .transpose()?;
            let created = crate::folder_actions::creation::plan(&root, parent, &job.name)?;
            if let Some(existing) = catalog.iter().find(|m| m.name == created.name) {
                anyhow::ensure!(
                    existing.selectable && !existing.non_existent,
                    "This name belongs to an unavailable folder."
                );
            } else {
                catalog.push(created.clone());
            }
            save_catalog(&tx, &job.account, catalog)?;
            job.target = Some(created.clone());
            job.receipt = Some(created);
            job.stage = CreationStage::Succeeded;
            job.error = None;
            job.revision += 1;
            save(&tx, &job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub(crate) async fn recover_creations(&self) -> anyhow::Result<()> {
        self.run(|c| {
            c.execute("UPDATE folder_creations SET data=json_set(data,'$.stage','uncertain','$.error','The folder request was interrupted. Check the saved target before continuing.','$.revision',json_extract(data,'$.revision')+1) WHERE json_extract(data,'$.stage') IN ('running','checking')",[])?;
            Ok(())
        }).await
    }
    pub(crate) async fn admit_folder_creation(
        &self,
        id: String,
        account: String,
        connection: String,
        parent: Option<String>,
        name: String,
    ) -> anyhow::Result<CreationJob> {
        uuid::Uuid::parse_str(&id)?;
        crate::folder_actions::creation::valid_path(&name)?;
        if let Some(parent) = &parent {
            crate::folder_actions::creation::valid_path(parent)?;
        }
        self.run(move |c| {
            let tx=c.transaction()?;
            checked_account(&tx,&account,&connection)?;
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM folder_creations WHERE json_extract(data,'$.id')=?)",[&id],|r|r.get(0))?;
            if exists {
                let existing=load(&tx,&id)?;
                anyhow::ensure!(existing.account==account && existing.connection==connection && existing.parent==parent && existing.name==name,"This folder request changed. Open New folder again.");
                return Ok(existing);
            }
            let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM folder_creations INDEXED BY folder_creation_ready WHERE account=? AND connection=? AND json_extract(data,'$.parent') IS ? AND json_extract(data,'$.name')=? AND json_extract(data,'$.stage') IN ('queued','waiting','running','checking','repair','rejected','uncertain'))",params![account,connection,parent,name],|r|r.get(0))?;
            anyhow::ensure!(!pending,"This folder already has a saved request. Review its progress.");
            let full:bool=tx.query_row(AT_CAPACITY,[],|r|r.get(0))?;
            anyhow::ensure!(!full,"Finish the saved folder requests before adding another.");
            let job=CreationJob{id,account,connection,parent,name,stage:CreationStage::Queued,target:None,receipt:None,provider_acknowledged:false,error:None,revision:1};
            tx.execute("INSERT INTO folder_creations(account,connection,request,target,data) VALUES(?,?,?,'null',?)",params![job.account,job.connection,job.id,serde_json::to_string(&job)?])?;
            connections::changed(&tx)?;
            tx.commit()?;
            Ok(job)
        }).await
    }

    pub(crate) async fn creation_job(&self, id: String) -> anyhow::Result<CreationJob> {
        self.run(move |c| load(c, &id)).await
    }

    pub(crate) async fn update_creation(
        &self,
        expected: CreationJob,
        stage: CreationStage,
        target: Option<Mailbox>,
        receipt: Option<Mailbox>,
        error: Option<String>,
    ) -> anyhow::Result<CreationJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = load(&tx, &expected.id)?;
            connections::allow(&tx, ConnectionKind::Account, &job.account)?;
            if !matches!(
                stage,
                CreationStage::Waiting
                    | CreationStage::Rejected
                    | CreationStage::Uncertain
                    | CreationStage::Cancelled
                    | CreationStage::Dismissed
            ) {
                checked_account(&tx, &job.account, &job.connection)?;
            }
            anyhow::ensure!(
                job == expected,
                "This folder request changed. Refresh its progress."
            );
            let allowed = job.stage == stage
                || match job.stage {
                    CreationStage::Queued => matches!(
                        stage,
                        CreationStage::Running
                            | CreationStage::Waiting
                            | CreationStage::Rejected
                            | CreationStage::Cancelled
                            | CreationStage::Repair
                    ),
                    CreationStage::Running => matches!(
                        stage,
                        CreationStage::Waiting
                            | CreationStage::Rejected
                            | CreationStage::Uncertain
                            | CreationStage::Repair
                    ),
                    CreationStage::Checking => matches!(
                        stage,
                        CreationStage::Uncertain | CreationStage::Rejected | CreationStage::Repair
                    ),
                    CreationStage::Waiting | CreationStage::Rejected => {
                        matches!(stage, CreationStage::Queued | CreationStage::Cancelled)
                    }
                    CreationStage::Uncertain => {
                        matches!(stage, CreationStage::Checking | CreationStage::Dismissed)
                    }
                    _ => false,
                };
            anyhow::ensure!(allowed, "This folder creation transition is not safe.");
            if let Some(target) = target {
                crate::folder_actions::creation::valid_path(&target.name)?;
                anyhow::ensure!(
                    job.target.as_ref().is_none_or(|saved| saved == &target),
                    "The saved folder target changed."
                );
                job.target = Some(target);
            }
            if let Some(receipt) = receipt {
                anyhow::ensure!(
                    receipt.selectable
                        && !receipt.non_existent
                        && job
                            .target
                            .as_ref()
                            .is_some_and(|target| target.name == receipt.name
                                && target.encoding == receipt.encoding),
                    "The folder receipt does not match its target."
                );
                job.receipt = Some(receipt);
                job.provider_acknowledged |= job.stage == CreationStage::Running;
            }
            job.stage = stage;
            job.error = error;
            job.revision += 1;
            save(&tx, &job)?;
            connections::changed(&tx)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }

    pub(crate) async fn decide_creation(
        &self,
        id: String,
        revision: u64,
        cancel: bool,
    ) -> anyhow::Result<CreationJob> {
        let job = self.creation_job(id).await?;
        anyhow::ensure!(
            job.revision == revision,
            "This folder request changed. Refresh its progress."
        );
        anyhow::ensure!(
            matches!(
                job.stage,
                CreationStage::Queued | CreationStage::Waiting | CreationStage::Rejected
            ) || !cancel && matches!(job.stage, CreationStage::Uncertain | CreationStage::Repair),
            "This folder request is already running."
        );
        let stage = if cancel {
            CreationStage::Cancelled
        } else if job.receipt.is_some() {
            CreationStage::Repair
        } else if job.stage == CreationStage::Uncertain {
            CreationStage::Checking
        } else {
            CreationStage::Queued
        };
        self.update_creation(job, stage, None, None, None).await
    }

    pub(crate) async fn finish_creation_cache(
        &self,
        expected: CreationJob,
        catalog: Vec<Mailbox>,
    ) -> anyhow::Result<CreationJob> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut job = load(&tx, &expected.id)?;
            checked_account(&tx, &job.account, &job.connection)?;
            anyhow::ensure!(
                job == expected && job.stage == CreationStage::Repair,
                "The folder receipt changed."
            );
            let receipt = job.receipt.as_ref().context("No saved folder receipt")?;
            anyhow::ensure!(
                catalog.iter().any(|folder| folder.name == receipt.name
                    && folder.selectable
                    && !folder.non_existent),
                "The server has not listed the acknowledged folder yet. Refresh its progress."
            );
            save_catalog(&tx, &job.account, catalog)?;
            job.stage = CreationStage::Succeeded;
            job.error = None;
            job.revision += 1;
            save(&tx, &job)?;
            tx.commit()?;
            Ok(job)
        })
        .await
    }
}
