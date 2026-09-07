//! An isolated fictional server. Its catalog is separate from the cache so
//! lost acknowledgments and process restarts exercise the ordinary journal.
use super::*;
use crate::{
    folder_actions::{Outcome, Step},
    folders::Mailbox,
};

pub(super) struct Connection {
    store: Store,
    key: String,
    plan: Plan,
    catalog: Vec<Mailbox>,
    mode: String,
}
impl Connection {
    pub async fn open(store: &Store, job: &Job) -> anyhow::Result<Self> {
        let key = format!("fixture:folder-server:{}", job.review.account);
        let saved = store.get::<Option<Vec<Mailbox>>>(&key).await?;
        let catalog = if let Some(catalog) = saved {
            catalog
        } else {
            store
                .current_folder_catalog(job.review.account.clone())
                .await?
        };
        let mode = std::env::args()
            .find_map(|arg| arg.strip_prefix("--folder-actions=").map(str::to_owned))
            .unwrap_or_default();
        Ok(Self {
            store: store.clone(),
            key,
            plan: job.review.plan.clone(),
            catalog,
            mode,
        })
    }
}
#[async_trait::async_trait]
impl crate::folder_actions::Connection for Connection {
    async fn catalog(&mut self) -> anyhow::Result<Vec<Mailbox>> {
        Ok(self.catalog.clone())
    }
    async fn apply(&mut self, step: &Step) -> Outcome {
        if !self.mode.is_empty() {
            tokio::time::sleep(Duration::from_millis(1600)).await;
        }
        let attempted = format!("{}:attempted", self.key);
        let first = match self.store.get::<bool>(&attempted).await {
            Ok(value) => !value,
            Err(error) => return Outcome::Rejected(error.to_string()),
        };
        if let Err(error) = self.store.put(&attempted, true).await {
            return Outcome::Rejected(error.to_string());
        }
        if first && self.mode == "fail" {
            return Outcome::Rejected(
                "The fictional server refused this change. Retry to continue.".into(),
            );
        }
        self.catalog = self.plan.project(&self.catalog, std::slice::from_ref(step));
        if let Err(error) = self.store.put(&self.key, Some(self.catalog.clone())).await {
            return Outcome::Uncertain(error.to_string());
        }
        if first && self.mode == "uncertain" {
            Outcome::Uncertain("The fictional server disconnected before confirming. Check the folders before accepting the result.".into())
        } else {
            Outcome::Applied
        }
    }
}
