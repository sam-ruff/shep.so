use super::*;
use crate::folder_actions::Connection;
use crate::folder_actions::creation::CreateOutcome;
use crate::store::{CreationJob, CreationStage};
#[cfg(test)]
mod tests;

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
trait CreationApi: Send + Sync {
    async fn plan(&self, parent: Option<String>, name: String) -> anyhow::Result<Mailbox>;
    async fn inspect(&self, target: Mailbox) -> anyhow::Result<Option<Mailbox>>;
    /// Err means opening the connection failed before CREATE was dispatched.
    async fn create(&self, target: Mailbox) -> anyhow::Result<CreateOutcome>;
    async fn catalog(&self) -> anyhow::Result<Vec<Mailbox>>;
}

struct ImapApi<'a> {
    account: &'a Account,
    password: &'a secrecy::SecretString,
}

#[async_trait::async_trait]
impl CreationApi for ImapApi<'_> {
    async fn plan(&self, parent: Option<String>, name: String) -> anyhow::Result<Mailbox> {
        providers::mail::folders::ImapFolders::open(self.account, self.password)
            .await?
            .plan_folder(parent.as_deref(), &name)
            .await
    }
    async fn inspect(&self, target: Mailbox) -> anyhow::Result<Option<Mailbox>> {
        providers::mail::folders::ImapFolders::open(self.account, self.password)
            .await?
            .find_planned_folder(&target)
            .await
    }
    async fn create(&self, target: Mailbox) -> anyhow::Result<CreateOutcome> {
        let mut connection =
            providers::mail::folders::ImapFolders::open(self.account, self.password).await?;
        Ok(connection.create_planned_folder(&target).await)
    }
    async fn catalog(&self) -> anyhow::Result<Vec<Mailbox>> {
        providers::mail::folders::ImapFolders::open(self.account, self.password)
            .await?
            .catalog()
            .await
    }
}

#[cfg(test)]
async fn execute(
    store: &Store,
    job: CreationJob,
    api: &impl CreationApi,
    stopping: &crate::lifecycle::Signal,
) -> anyhow::Result<CreationJob> {
    execute_observed(store, job, api, stopping, None).await
}

async fn execute_observed(
    store: &Store,
    mut job: CreationJob,
    api: &impl CreationApi,
    stopping: &crate::lifecycle::Signal,
    mut output: Option<&mut Output>,
) -> anyhow::Result<CreationJob> {
    if stopping.get() {
        return Ok(job);
    }
    if job.stage == CreationStage::Queued {
        if job.target.is_none() {
            let target = tokio::select! {
                _=stopping.requested()=>return Ok(job),
                result=api.plan(job.parent.clone(),job.name.clone())=>result,
            };
            let target = match target {
                Ok(target) => target,
                Err(error) => {
                    return store
                        .update_creation(
                            job,
                            CreationStage::Waiting,
                            None,
                            None,
                            Some(format!("{error:#}")),
                        )
                        .await;
                }
            };
            job = store
                .update_creation(job, CreationStage::Queued, Some(target), None, None)
                .await?;
        }
        let target = job.target.clone().context("No saved folder target")?;
        let existing = tokio::select! {
            _=stopping.requested()=>return Ok(job),
            result=api.inspect(target.clone())=>result,
        };
        match existing {
            Ok(Some(receipt)) => {
                job = store
                    .update_creation(job, CreationStage::Repair, None, Some(receipt), None)
                    .await?
            }
            Err(error) => {
                return store
                    .update_creation(
                        job,
                        CreationStage::Waiting,
                        None,
                        None,
                        Some(format!("{error:#}")),
                    )
                    .await;
            }
            Ok(None) => {
                if stopping.get() {
                    return Ok(job);
                }
                job = store
                    .update_creation(job, CreationStage::Running, None, None, None)
                    .await?;
                if let Some(output) = output.as_mut() {
                    let _ = output.send(Event::CreationChanged(job.clone())).await;
                }
                match api.create(target.clone()).await {
                    Ok(CreateOutcome::Acknowledged) => {
                        job = store
                            .update_creation(job, CreationStage::Repair, None, Some(target), None)
                            .await?
                    }
                    Ok(CreateOutcome::Rejected(error)) => {
                        return store
                            .update_creation(job, CreationStage::Rejected, None, None, Some(error))
                            .await;
                    }
                    Ok(CreateOutcome::Uncertain(error)) => {
                        return store
                            .update_creation(job, CreationStage::Uncertain, None, None, Some(error))
                            .await;
                    }
                    Err(error) => {
                        return store
                            .update_creation(
                                job,
                                CreationStage::Waiting,
                                None,
                                None,
                                Some(format!("{error:#}")),
                            )
                            .await;
                    }
                }
            }
        }
    }
    if job.stage == CreationStage::Checking {
        let Some(target) = job.target.clone() else {
            return store.update_creation(job,CreationStage::Uncertain,None,None,Some("This imported request has no observed server target. Review folders before dismissing it.".into())).await;
        };
        let result = tokio::select! {
            _=stopping.requested()=>return store.update_creation(job,CreationStage::Uncertain,None,None,Some("Folder inspection was interrupted.".into())).await,
            result=api.inspect(target)=>result,
        };
        match result {
            Ok(Some(receipt)) => {
                job = store
                    .update_creation(job, CreationStage::Repair, None, Some(receipt), None)
                    .await?
            }
            Ok(None) => return store
                .update_creation(
                    job,
                    CreationStage::Rejected,
                    None,
                    None,
                    Some(
                        "The saved folder was not found. Retry only when you want to create it."
                            .into(),
                    ),
                )
                .await,
            Err(error) => {
                return store
                    .update_creation(
                        job,
                        CreationStage::Uncertain,
                        None,
                        None,
                        Some(format!("{error:#}")),
                    )
                    .await;
            }
        }
    }
    if job.stage == CreationStage::Repair {
        if let Some(output) = output.as_mut() {
            let _ = output.send(Event::CreationChanged(job.clone())).await;
        }
        let result = tokio::select! {
            _=stopping.requested()=>return Ok(job),
            result=api.catalog()=>result,
        };
        match result {
            Ok(catalog) => return store.finish_creation_cache(job, catalog).await,
            Err(error) => {
                return store
                    .update_creation(
                        job,
                        CreationStage::Repair,
                        None,
                        None,
                        Some(format!("{error:#}")),
                    )
                    .await;
            }
        }
    }
    Ok(job)
}

impl Engine {
    pub(in crate::engine) async fn execute_creation_work(
        &self,
        id: String,
        mut output: Output,
    ) -> bool {
        let result = async {
            let job = self.store.creation_job(id.clone()).await?;
            let protocol = self.account(&job.account).await?.protocol;
            let _slot = if protocol == Protocol::Pop3 {
                None
            } else {
                Some(tokio::select! {
                    _=self.bulk_control.stopping.requested()=>return Ok(false),
                    slot=self.provider_slots.acquire()=>slot,
                })
            };
            let account_lock = tokio::select! {
                _=self.bulk_control.stopping.requested()=>return Ok(false),
                guard=self.account_exclusive(&job.account)=>guard,
            };
            let current = self.store.creation_job(id.clone()).await?;
            if current != job || self.bulk_control.stopping.get() {
                return Ok(false);
            }
            let account = self.account(&job.account).await?;
            anyhow::ensure!(
                crate::mail_actions::connection_key(&account) == job.connection,
                "The account connection changed. Review this folder request."
            );
            let saved = if account.protocol == Protocol::Pop3 {
                self.store.finish_local_creation(job.clone()).await?
            } else if self.demo {
                #[cfg(any(test, feature = "test-support"))]
                {
                    execute_observed(
                        &self.store,
                        job.clone(),
                        &PreviewApi {
                            store: &self.store,
                            account: job.account.clone(),
                        },
                        &self.bulk_control.stopping,
                        Some(&mut output),
                    )
                    .await?
                }
                #[cfg(not(any(test, feature = "test-support")))]
                {
                    anyhow::bail!("Folder creation preview is unavailable in this build.");
                }
            } else {
                let password = match self.credentials.account_password(&account, false).await {
                    Ok(password) => password,
                    Err(error) => {
                        let stage = if job.receipt.is_some() {
                            CreationStage::Repair
                        } else if job.stage == CreationStage::Checking {
                            CreationStage::Uncertain
                        } else {
                            CreationStage::Waiting
                        };
                        let saved = self
                            .store
                            .update_creation(
                                job,
                                stage,
                                None,
                                None,
                                Some(format!("Reconnect this account to continue. {error:#}")),
                            )
                            .await?;
                        self.workspace(&mut output).await?;
                        output.send(Event::CreationChanged(saved)).await?;
                        return Ok(stage != CreationStage::Repair);
                    }
                };
                execute_observed(
                    &self.store,
                    job.clone(),
                    &ImapApi {
                        account: &account,
                        password: &password,
                    },
                    &self.bulk_control.stopping,
                    Some(&mut output),
                )
                .await?
            };
            drop(account_lock);
            self.workspace(&mut output).await?;
            output.send(Event::CreationChanged(saved.clone())).await?;
            Ok::<_, anyhow::Error>(
                saved.stage != job.stage
                    || saved.target != job.target
                    || saved.receipt != job.receipt,
            )
        }
        .await;
        match result {
            Ok(changed) => changed,
            Err(error) => {
                if let Ok(job) = self.store.creation_job(id).await {
                    if matches!(
                        job.stage,
                        CreationStage::Succeeded
                            | CreationStage::Cancelled
                            | CreationStage::Dismissed
                    ) {
                        return false;
                    }
                    let stage = match job.stage {
                        CreationStage::Running | CreationStage::Checking => {
                            CreationStage::Uncertain
                        }
                        CreationStage::Queued => CreationStage::Waiting,
                        stage => stage,
                    };
                    let observed = self
                        .store
                        .update_creation(job.clone(), stage, None, None, Some(format!("{error:#}")))
                        .await
                        .unwrap_or(job);
                    let _ = output.send(Event::CreationChanged(observed)).await;
                }
                let _ = output
                    .send(Event::Error(format!(
                        "Folder creation needs attention. Open its saved request. {error:#}"
                    )))
                    .await;
                false
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
struct PreviewApi<'a> {
    store: &'a Store,
    account: String,
}

#[cfg(any(test, feature = "test-support"))]
#[async_trait::async_trait]
impl CreationApi for PreviewApi<'_> {
    async fn plan(&self, parent: Option<String>, name: String) -> anyhow::Result<Mailbox> {
        let catalog = self.catalog().await?;
        let root = Mailbox {
            delimiter: catalog.first().and_then(|m| m.delimiter),
            encoding: catalog.first().map(|m| m.encoding).unwrap_or_default(),
            ..Mailbox::flat(String::new())
        };
        let parent = parent
            .as_ref()
            .map(|path| {
                catalog
                    .iter()
                    .find(|m| &m.name == path)
                    .context("The parent folder is no longer available.")
            })
            .transpose()?;
        crate::folder_actions::creation::plan(&root, parent, &name)
    }
    async fn inspect(&self, target: Mailbox) -> anyhow::Result<Option<Mailbox>> {
        Ok(self
            .catalog()
            .await?
            .into_iter()
            .find(|m| m.name == target.name && m.selectable && !m.non_existent))
    }
    async fn create(&self, target: Mailbox) -> anyhow::Result<CreateOutcome> {
        let mode = std::env::args()
            .find_map(|a| a.strip_prefix("--folder-actions=").map(str::to_owned))
            .unwrap_or_default();
        if !mode.is_empty() {
            tokio::time::sleep(Duration::from_millis(1600)).await;
        }
        let key = format!("fixture:folder-create:{}", self.account);
        let attempted = format!("{key}:attempted");
        let first = !self.store.get::<bool>(&attempted).await?;
        self.store.put(&attempted, true).await?;
        if first && mode == "fail" {
            return Ok(CreateOutcome::Rejected(
                "The fictional server refused this folder. Review the saved request to retry."
                    .into(),
            ));
        }
        let mut catalog = self.catalog().await?;
        catalog.push(target);
        self.store.put(&key, Some(catalog)).await?;
        if first && mode == "uncertain" {
            return Ok(CreateOutcome::Uncertain(
                "The connection closed before confirming the folder.".into(),
            ));
        }
        Ok(CreateOutcome::Acknowledged)
    }
    async fn catalog(&self) -> anyhow::Result<Vec<Mailbox>> {
        match self
            .store
            .get::<Option<Vec<Mailbox>>>(&format!("fixture:folder-create:{}", self.account))
            .await?
        {
            Some(catalog) => Ok(catalog),
            None => {
                self.store
                    .current_folder_catalog(self.account.clone())
                    .await
            }
        }
    }
}
