use super::*;
use crate::database::Database;
use secrecy::SecretString;
use shep_mail_core::{
    folder_actions::creation::CreateOutcome, providers::mail::folders::ImapFolders,
};

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub(crate) trait CreationApi: Send + Sync {
    async fn plan(&self, parent: Option<String>, name: String) -> Result<Mailbox>;
    async fn inspect(&self, target: Mailbox) -> Result<Option<Mailbox>>;
    /// An error means connection setup failed before CREATE was dispatched.
    async fn create(&self, target: Mailbox) -> Result<CreateOutcome>;
}

pub(crate) struct ImapCreation {
    pub account: Account,
    pub password: SecretString,
}

#[async_trait::async_trait]
impl CreationApi for ImapCreation {
    async fn plan(&self, parent: Option<String>, name: String) -> Result<Mailbox> {
        ImapFolders::open(&self.account, &self.password)
            .await?
            .plan_folder(parent.as_deref(), &name)
            .await
    }
    async fn inspect(&self, target: Mailbox) -> Result<Option<Mailbox>> {
        ImapFolders::open(&self.account, &self.password)
            .await?
            .find_planned_folder(&target)
            .await
    }
    async fn create(&self, target: Mailbox) -> Result<CreateOutcome> {
        let mut provider = ImapFolders::open(&self.account, &self.password).await?;
        Ok(provider.create_planned_folder(&target).await)
    }
}

pub(super) async fn transition(
    db: &Database,
    before: &Creation,
    after: Creation,
) -> Result<Creation> {
    let original = before.clone();
    let desired = after.clone();
    let result = db.write(move |db| save(db, &original, &desired)).await;
    let Err(error) = result else {
        return result;
    };
    let id = before.id.clone();
    let saved = db.read(move |db| get(db, &id)).await?;
    if let Some(saved) = saved {
        let mut expected = after.clone();
        expected.revision = saved.revision;
        if saved.revision > before.revision && saved == expected {
            return Ok(saved);
        }
        if saved == *before
            && after.acknowledged
            && (!before.acknowledged
                || after
                    .mutation
                    .as_ref()
                    .is_some_and(|mutation| mutation.receipt.is_some())
                    && before
                        .mutation
                        .as_ref()
                        .is_some_and(|mutation| mutation.receipt.is_none()))
        {
            return db.write(move |db| save(db, &saved, &after)).await;
        }
    }
    Err(error)
}

async fn attention(
    db: &Database,
    before: &Creation,
    status: &str,
    message: &str,
) -> Result<Creation> {
    let mut after = before.clone();
    after.status = status.into();
    after.error = Some(message.into());
    transition(db, before, after).await
}

async fn observed(db: &Database, job: &Creation, mailbox: Mailbox) -> Result<Creation> {
    anyhow::ensure!(
        job.target.as_ref().is_some_and(
            |target| target.name == mailbox.name && target.encoding == mailbox.encoding
        ),
        "The server returned a different folder identity. Keep this request for review."
    );
    let mut after = job.clone();
    after.receipt = Some(mailbox);
    after.status = "repair".into();
    after.error = None;
    let saved = transition(db, job, after).await?;
    db.write(move |db| apply_receipt(db, &saved)).await
}

pub(crate) async fn execute(
    db: &Database,
    api: &dyn CreationApi,
    mut job: Creation,
) -> Result<Creation> {
    let checking = matches!(job.status.as_str(), "checking" | "repair");
    if job.receipt.is_some() {
        return db.write(move |db| apply_receipt(db, &job)).await;
    }
    anyhow::ensure!(
        checking || matches!(job.status.as_str(), "queued" | "waiting"),
        "This folder request needs a decision before continuing."
    );
    if !checking {
        job = attention(db, &job, "planning", "Preparing the folder on the server.").await?;
    }
    if job.target.is_none() {
        anyhow::ensure!(
            !checking,
            "This request has no saved folder identity to inspect."
        );
        let target =
            match api.plan(job.parent.clone(), job.name.clone()).await {
                Ok(target) => target,
                Err(error) => return attention(
                    db,
                    &job,
                    if error.downcast_ref::<shep_mail_core::folder_actions::creation::PlanRejected>().is_some() { "rejected" } else { "waiting" },
                    if error.downcast_ref::<shep_mail_core::folder_actions::creation::PlanRejected>().is_some() { "The folder name or parent is unavailable. Cancel this request and check its name and parent." } else { "Could not connect or read the folder namespace. This saved request will continue after reconnection." },
                )
                .await,
            };
        let mut after = job.clone();
        after.target = Some(target);
        job = transition(db, &job, after).await?;
    }
    let target = job
        .target
        .clone()
        .context("The planned folder was not saved.")?;
    match api.inspect(target.clone()).await {
        Ok(Some(mailbox)) => return observed(db, &job, mailbox).await,
        Ok(None) if checking => {
            return attention(db, &job, if job.acknowledged { "repair" } else { "rejected" },
                if job.acknowledged { "The server accepted CREATE but the folder is now missing. Check again or stop tracking this request." }
                else { "The checked folder is absent. You can retry this saved request." }).await;
        }
        Ok(None) => {}
        Err(_) => {
            return attention(
                db,
                &job,
                if checking {
                    if job.acknowledged {
                        "repair"
                    } else {
                        "uncertain"
                    }
                } else {
                    "waiting"
                },
                "Could not inspect this folder. Reconnect and check again.",
            )
            .await;
        }
    }
    job = attention(db, &job, "running", "Creating the folder.").await?;
    match api.create(target.clone()).await {
        Ok(CreateOutcome::Acknowledged) => {
            let mut after = job.clone();
            after.acknowledged = true;
            after.status = "repair".into();
            after.error = None;
            job = transition(db, &job, after).await?;
        }
        Ok(CreateOutcome::Rejected(_)) => return attention(db, &job, "rejected", "The server refused to create this folder. Check its name and permissions before retrying.").await,
        Ok(CreateOutcome::Uncertain(_)) => return attention(db, &job, "uncertain", "The server response was lost. Check the saved folder before deciding what to do.").await,
        Err(_) => return attention(db, &job, "waiting", "Could not connect before creating the folder. Reconnect to continue.").await,
    }
    match api.inspect(target).await {
        Ok(Some(mailbox)) => observed(db, &job, mailbox).await,
        _ => {
            attention(
                db,
                &job,
                "repair",
                "The server accepted the folder. Check it again to finish updating this device.",
            )
            .await
        }
    }
}
