//! Folder controls use local queues; the durable executor shares the mail-group
//! close barrier. Exact server paths and protocol handling stay out of iced.
use super::*;
use crate::{
    folder_actions::{Connection, Job, Plan, Review, Status},
    folders::Tree,
};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Request {
    Options(u64, String, String),
    Review(u64, String, String, crate::folder_actions::Action),
    Start(String, Arc<Review>),
    History(u64, usize),
    Retry(String),
    Stop(String, bool),
}
impl Request {
    pub(super) fn is_read(&self) -> bool {
        matches!(
            self,
            Self::Options(..) | Self::Review(..) | Self::History(..)
        )
    }
}
#[derive(Debug, Clone)]
pub struct Destination {
    pub path: Option<String>,
    pub label: String,
}
#[derive(Debug, Clone)]
pub enum Event {
    Options(u64, Result<Arc<Vec<Destination>>, String>),
    Review(u64, Result<Arc<Preview>, String>),
    Started(String, Result<Arc<Job>, String>),
    Update(Arc<Job>),
    Finished(String, Result<Arc<Job>, String>),
    History(u64, Result<Arc<Vec<Job>>, String>),
}

#[derive(Debug, Clone)]
pub struct Preview {
    pub review: Arc<Review>,
    pub tree: Arc<Tree>,
    /// Projected paths still browse the original cached folder until committed.
    pub originals: HashMap<String, String>,
}

impl Engine {
    pub(super) async fn folder_command(
        &self,
        request: Request,
        mut output: Output,
    ) -> anyhow::Result<()> {
        let event = match request {
            Request::Options(serial, account, source) => {
                let result = async {
                    self.store.ensure_folder_idle(account.clone()).await?;
                    let catalog = self.store.current_folder_catalog(account).await?;
                    tokio::task::spawn_blocking(move || {
                        let tree = Tree::new(&catalog);
                        Plan::new(&tree, &source, crate::folder_actions::Action::Delete)?;
                        let candidates = std::iter::once(None)
                            .chain(tree.nodes.iter().map(|n| Some(n.path.clone())));
                        Ok::<_, anyhow::Error>(
                            candidates
                                .filter_map(|path| {
                                    Plan::new(
                                        &tree,
                                        &source,
                                        crate::folder_actions::Action::Move {
                                            parent: path.clone(),
                                        },
                                    )
                                    .ok()?;
                                    let label = path
                                        .as_deref()
                                        .and_then(|p| tree.node(p))
                                        .map(|n| {
                                            if n.path.eq_ignore_ascii_case("INBOX") {
                                                "Inbox".into()
                                            } else {
                                                n.display_path.clone()
                                            }
                                        })
                                        .unwrap_or_else(|| "Account root".into());
                                    Some(Destination { path, label })
                                })
                                .collect::<Vec<_>>(),
                        )
                    })
                    .await?
                }
                .await
                .map(Arc::new)
                .map_err(|e: anyhow::Error| format!("{e:#}"));
                Event::Options(serial, result)
            }
            Request::Review(serial, account, source, action) => Event::Review(
                serial,
                self.folder_preview(account, source, action)
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}")),
            ),
            Request::Start(id, review) => {
                let result = self
                    .store
                    .start_folder_change(id.clone(), (*review).clone())
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"));
                if result.is_ok() {
                    self.bulk_control.stopping.set(false);
                }
                Event::Started(id, result)
            }
            Request::History(serial, offset) => Event::History(
                serial,
                self.store
                    .folder_jobs(offset)
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}")),
            ),
            Request::Retry(id) => {
                let result = async {
                    let lease = self.store.folder_lease(id.clone()).await?;
                    let job = self.store.recover_folder_change(&lease).await?;
                    // Acknowledged commands only need cache reconciliation, never replay.
                    let job = if job.steps.iter().any(|s| s.status == Status::Acknowledged) {
                        job
                    } else {
                        self.store.retry_folder_change(&lease).await?
                    };
                    self.bulk_control.stopping.set(false);
                    Ok::<_, anyhow::Error>(job)
                }
                .await
                .map(Arc::new)
                .map_err(|e| format!("{e:#}"));
                Event::Started(id, result)
            }
            Request::Stop(id, accept) => {
                let result = async {
                    let lease = self.store.folder_lease(id.clone()).await?;
                    self.store.stop_folder_change(&lease, accept).await
                }
                .await
                .map(Arc::new)
                .map_err(|e| format!("{e:#}"));
                Event::Finished(id, result)
            }
        };
        output.send(super::Event::Folder(event)).await?;
        Ok(())
    }
    pub(super) async fn folder_preview(
        &self,
        account: String,
        source: String,
        action: crate::folder_actions::Action,
    ) -> anyhow::Result<Preview> {
        let review = self
            .store
            .folder_review(account.clone(), source, action)
            .await?;
        let catalog = self.store.current_folder_catalog(account).await?;
        tokio::task::spawn_blocking(move || {
            review.plan.revalidate(&catalog)?;
            let tree = Tree::new(&review.plan.project(&catalog, &review.plan.steps()));
            let originals = review
                .plan
                .members
                .iter()
                .filter_map(|member| {
                    Some((Plan::wire_destination(member)?, member.mailbox.name.clone()))
                })
                .collect();
            Ok(Preview {
                review: Arc::new(review),
                tree: Arc::new(tree),
                originals,
            })
        })
        .await?
    }

    pub(super) async fn drain_folder_jobs(&self, mut output: Output) {
        let mut after = String::new();
        while !self.bulk_control.stopping.get() {
            match self.store.next_pending_folder(after.clone()).await {
                Ok(Some(id)) => {
                    after = id.clone();
                    self.execute_folder_job(id, output.clone()).await;
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = output.send(super::Event::Error(format!(
                        "Could not read unfinished folder changes. Open Folder history to retry. {error:#}"
                    ))).await;
                    break;
                }
            }
        }
    }

    async fn execute_folder_job(&self, id: String, mut output: Output) {
        self.bulk_control.active.set(true);
        if self.bulk_control.stopping.get() {
            self.bulk_control.active.set(false);
            let _ = output.send(super::Event::BulkStopped).await;
            return;
        }
        let result = self.perform_folder_job(&id, &mut output).await;
        // One workspace update includes the committed catalog, Sent mapping and
        // expansion preferences. A later read failure never repeats wire work.
        let result = match self.workspace(&mut output).await {
            Ok(()) => result,
            Err(error) => Err(error.context("The folder result was saved, but its view could not be refreshed. Open Folder history to retry.")),
        }.map(Arc::new).map_err(|error| format!("{error:#}"));
        let failed = result.as_ref().map_or(true, |job| {
            job.steps
                .iter()
                .any(|step| matches!(step.status, Status::Rejected | Status::Uncertain))
        });
        let _ = output
            .send(super::Event::Folder(Event::Finished(id, result)))
            .await;
        self.bulk_control.active.set(false);
        if failed {
            self.bulk_control.stopping.set(false);
        } else if self.bulk_control.stopping.get() {
            let _ = output.send(super::Event::BulkStopped).await;
        }
    }

    async fn perform_folder_job(&self, id: &str, output: &mut Output) -> anyhow::Result<Job> {
        let lease = self.store.folder_lease(id.to_owned()).await?;
        let job = self.store.recover_folder_change(&lease).await?;
        output
            .send(super::Event::Folder(Event::Update(Arc::new(job.clone()))))
            .await?;
        if job.closed
            || job
                .steps
                .iter()
                .any(|step| matches!(step.status, Status::Uncertain | Status::Rejected))
        {
            return Ok(job);
        }
        let permissions = async {
            let slot = self.provider_slots.acquire().await;
            let account = self.account_access(&job.review.account).await;
            (slot, account)
        };
        let stop = self.bulk_control.stopping.requested();
        let (_slot, _account_lock) = tokio::select! {
            biased;
            _ = stop => return Ok(job),
            permissions = permissions => permissions,
        };
        let account = self.account(&job.review.account).await?;
        anyhow::ensure!(
            crate::mail_actions::connection_key(&account) == job.review.connection,
            "This account's server changed. Reconnect the original server to finish the folder change."
        );
        let mut connection: Option<Box<dyn Connection>> =
            if self.demo && account.protocol == Protocol::Imap {
                #[cfg(any(test, feature = "test-support"))]
                {
                    Some(Box::new(
                        preview::Connection::open(&self.store, &job).await?,
                    ))
                }
                #[cfg(not(any(test, feature = "test-support")))]
                {
                    anyhow::bail!("Folder preview is unavailable in this build.");
                }
            } else if account.protocol == Protocol::Imap {
                let password = self.credentials.read(&account.id).await?;
                Some(Box::new(
                    providers::mail::folders::ImapFolders::open(&account, &password).await?,
                ))
            } else {
                None
            };
        let (progress, observation) = tokio::sync::watch::channel(Arc::new(job));
        let connection = connection
            .as_mut()
            .map(|value| value.as_mut() as &mut dyn Connection);
        let result = crate::folder_actions::runner::run(
            &self.store,
            &lease,
            connection,
            &self.bulk_control.stopping,
            Some(&progress),
        );
        tokio::pin!(result);
        let mut interval = tokio::time::interval(Duration::from_millis(100));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                result = &mut result => return result,
                _ = interval.tick() => {
                    // A full UI channel drops only this optional observation;
                    // execution and its durable acknowledgment keep progressing.
                    let _ = output.try_send(super::Event::Folder(Event::Update(observation.borrow().clone())));
                }
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
mod preview;

#[cfg(test)]
mod tests;
