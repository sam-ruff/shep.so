use super::*;
use crate::database::Database;
use secrecy::SecretString;
use shep_mail_core::folder_actions::{
    Connection as FolderConnection, Job, Outcome, Progress, Status,
};
use shep_mail_core::providers::mail::folders::ImapFolders;

#[cfg_attr(test, mockall::automock)]
#[async_trait::async_trait]
pub(crate) trait ChangeApi: Send + Sync {
    async fn catalogue(&self) -> Result<Vec<Mailbox>>;
    async fn apply(&self, mutation: Mutation, step: Step) -> Result<Outcome>;
}

pub(crate) struct ImapChange {
    pub account: Account,
    pub password: SecretString,
}

fn shared_job(mutation: &Mutation) -> Job {
    Job {
        query_counts: None,
        revision: 0,
        label: mutation.review.plan.source.clone(),
        destination_label: None,
        id: String::new(),
        closed: false,
        review: shep_mail_core::folder_actions::Review {
            account: mutation.review.account.clone(),
            connection: mutation.review.connection.clone(),
            imap: true,
            plan: mutation.review.plan.clone(),
            cached_messages: mutation.review.messages as usize,
            affected_history: 0,
        },
        steps: mutation
            .review
            .plan
            .steps()
            .into_iter()
            .enumerate()
            .map(|(position, step)| Progress {
                position,
                step,
                status: if position < mutation.completed {
                    Status::Done
                } else {
                    Status::Queued
                },
                error: None,
            })
            .collect(),
    }
}

#[async_trait::async_trait]
impl ChangeApi for ImapChange {
    async fn catalogue(&self) -> Result<Vec<Mailbox>> {
        ImapFolders::open(&self.account, &self.password)
            .await?
            .catalog()
            .await
    }
    async fn apply(&self, mutation: Mutation, step: Step) -> Result<Outcome> {
        let mut connection = ImapFolders::open(&self.account, &self.password).await?;
        let catalogue = connection.catalog().await?;
        let absent = match shared_job(&mutation).preflight(&catalogue) {
            Ok(absent) => absent,
            Err(_) => return Ok(Outcome::Rejected("The reviewed subtree changed.".into())),
        };
        if let Step::Delete { source } = &step
            && absent.contains(source)
        {
            return Ok(Outcome::Applied);
        }
        Ok(connection.apply(&step).await)
    }
}

async fn status(db: &Database, before: &Creation, state: &str, message: &str) -> Result<Creation> {
    let mut after = before.clone();
    after.status = state.into();
    after.error = (!message.is_empty()).then(|| message.into());
    super::super::execute::transition(db, before, after).await
}

pub(crate) async fn execute(
    db: &Database,
    api: Option<&dyn ChangeApi>,
    mut job: Creation,
) -> Result<Creation> {
    let mutation = job
        .mutation
        .clone()
        .context("The reviewed change is missing.")?;
    anyhow::ensure!(
        mutation.prepared,
        "Freeze the reviewed folder membership before provider work."
    );
    if mutation.receipt.is_some() {
        if job.status == "checking" {
            if let Some(api) = api
                && api.catalogue().await.is_err()
            {
                return status(
                    db,
                    &job,
                    "repair",
                    "Could not check the server. The saved receipt and cached mail were kept.",
                )
                .await;
            }
            let mut after = job.clone();
            after
                .mutation
                .as_mut()
                .context("The reviewed change is missing.")?
                .checked = true;
            after.status = "repair".into();
            job = super::super::execute::transition(db, &job, after).await?;
        }
        return finish_receipt(db, api, job).await;
    }
    let steps = mutation.review.plan.steps();
    let step = steps
        .get(mutation.completed)
        .context("The folder change has no remaining step.")?
        .clone();
    let checking = job.status == "checking";
    anyhow::ensure!(
        checking || matches!(job.status.as_str(), "queued" | "waiting"),
        "Check this saved folder change before continuing."
    );
    if !checking {
        job = status(db, &job, "planning", "Checking the reviewed folders.").await?;
    }
    if let Some(api) = api {
        let catalogue = match api.catalogue().await {
            Ok(value) => value,
            Err(_) => {
                return status(
                    db,
                    &job,
                    if checking { "uncertain" } else { "waiting" },
                    "Could not read the folder list. Reconnect and check again.",
                )
                .await;
            }
        };
        if checking {
            let mut after = job.clone();
            after
                .mutation
                .as_mut()
                .context("The review is missing.")?
                .checked = true;
            after.status = "uncertain".into();
            after.error = Some("The server was checked. The original result is still unconfirmed; keep cached mail and stop tracking, or check again. No command will be repeated.".into());
            return super::super::execute::transition(db, &job, after).await;
        }
        if shared_job(&mutation).preflight(&catalogue).is_err() {
            return status(db,&job,"rejected","The subtree changed after review. Cancel or check this request before reviewing it again.").await;
        }
    }
    if !matches!(step, Step::Forget { .. })
        && let Some(api) = api
    {
        job = status(db, &job, "running", "Changing the reviewed folder.").await?;
        match api.apply(mutation.clone(),step.clone()).await {
            Ok(Outcome::Applied) => {},
            Ok(Outcome::Rejected(_)) => return status(db,&job,"rejected","The server refused this folder step. Review it before retrying.").await,
            Ok(Outcome::Uncertain(_)) => return status(db,&job,"uncertain","The server result is unknown. Check it; this command will not be repeated automatically.").await,
            Err(_) => return status(db,&job,"waiting","Could not connect before changing the folder. The saved request will continue after reconnection.").await,
        }
    }
    let mut after = job.clone();
    after.acknowledged = true;
    after.status = "repair".into();
    after.error = None;
    after
        .mutation
        .as_mut()
        .context("The review is missing.")?
        .receipt = Some(step);
    let saved = super::super::execute::transition(db, &job, after).await?;
    finish_receipt(db, api, saved).await
}

async fn finish_receipt(
    db: &Database,
    api: Option<&dyn ChangeApi>,
    mut job: Creation,
) -> Result<Creation> {
    let mutation = job
        .mutation
        .as_ref()
        .context("The folder review is missing.")?;
    if !mutation.observed {
        if matches!(mutation.receipt, Some(Step::Rename { .. }))
            && let Some(api) = api
        {
            let catalog =
                match api.catalogue().await {
                    Ok(value) => value,
                    Err(_) => return status(
                        db,
                        &job,
                        "repair",
                        "The rename was acknowledged. Check the destination to finish saving here.",
                    )
                    .await,
                };
            if shared_job(mutation)
                .confirm_renamed_catalog(&catalog)
                .is_err()
            {
                let mut after = job.clone();
                after
                    .mutation
                    .as_mut()
                    .context("The review is missing.")?
                    .checked = true;
                after.error = Some("The acknowledged destination differs from the review. Keep cached mail and stop tracking, or check again.".into());
                after.status = "repair".into();
                return super::super::execute::transition(db, &job, after).await;
            }
        }
        let mut after = job.clone();
        after
            .mutation
            .as_mut()
            .context("The folder review is missing.")?
            .observed = true;
        job = super::super::execute::transition(db, &job, after).await?;
    }
    db.write(move |db| repair(db, &job)).await
}
