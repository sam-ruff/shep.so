//! Device-local discovery sessions. Only the verified Google transport can open
//! a production session; reviewed publication retains that same identity.
pub(crate) mod creation;
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use shep_profile_core::drive::{
    Drive,
    catalog::{Discovery, Scope, State},
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::{Mutex, Semaphore};
use uuid::Uuid;

#[async_trait]
trait Remote: Send + Sync {
    fn scope(&self) -> Scope;
    async fn advance(&self, catalog: &Discovery) -> Result<State>;
    async fn publish(
        &self,
        worker: &shep_profile_core::history::Worker,
        catalog: &Discovery,
    ) -> Result<Option<Uuid>>;
}
#[async_trait]
impl Remote for Drive {
    fn scope(&self) -> Scope {
        Scope {
            namespace: self.namespace().into(),
            principal: self.principal().into(),
        }
    }
    async fn advance(&self, catalog: &Discovery) -> Result<State> {
        Ok(catalog.advance(self).await?)
    }
    async fn publish(
        &self,
        worker: &shep_profile_core::history::Worker,
        catalog: &Discovery,
    ) -> Result<Option<Uuid>> {
        Ok(self.upload_next_tracked(worker, catalog).await?)
    }
}
struct Session {
    id: Uuid,
    remote: Arc<dyn Remote>,
    catalog: Discovery,
    advancing: Arc<Semaphore>,
}
#[derive(Default)]
struct Slots {
    opening: Option<Uuid>,
    active: Option<Arc<Session>>,
}
pub(crate) struct Runtime {
    opening: Arc<Semaphore>,
    slots: Mutex<Slots>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            opening: Arc::new(Semaphore::new(1)),
            slots: Mutex::default(),
        }
    }
}
#[derive(Serialize)]
pub(crate) struct Opened {
    session: Uuid,
    scope: Scope,
    state: State,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Command {
    State,
    Profiles { after: Option<String> },
    Advance,
    Retry { expected_revision: u64 },
    Refresh { expected_revision: u64, full: bool },
}
fn changed() -> anyhow::Error {
    anyhow::anyhow!("Profile discovery changed. Reopen Profiles and sync before continuing.")
}
impl Runtime {
    async fn begin(&self, id: Uuid) -> Result<()> {
        if id.is_nil() {
            return Err(changed());
        }
        let mut slots = self.slots.lock().await;
        if slots.opening.is_some() || slots.active.is_some() {
            bail!("Profile discovery is already open. Close that session before reconnecting.");
        }
        slots.opening = Some(id);
        Ok(())
    }
    async fn abandon(&self, id: Uuid) {
        let mut slots = self.slots.lock().await;
        if slots.opening == Some(id) {
            slots.opening = None;
        }
    }
    pub async fn open(
        &self,
        path: &Path,
        id: Uuid,
        token: String,
        namespace: String,
        expected_principal: Option<String>,
    ) -> Result<Opened> {
        let _permit = self
            .opening
            .clone()
            .try_acquire_owned()
            .context("Google profile connection is busy. Retry shortly.")?;
        self.begin(id).await?;
        let result = async {
            let drive = Drive::connect(
                SecretString::from(token),
                namespace,
                expected_principal.as_deref(),
            )
            .await?;
            self.activate(path, id, Arc::new(drive)).await
        }
        .await;
        if result.is_err() {
            self.abandon(id).await;
        }
        result
    }
    async fn activate(&self, path: &Path, id: Uuid, remote: Arc<dyn Remote>) -> Result<Opened> {
        if self.slots.lock().await.opening != Some(id) {
            return Err(changed());
        }
        let scope = remote.scope();
        let mut root = path.as_os_str().to_owned();
        root.push(".profile-discovery");
        let path = PathBuf::from(root).join(format!("{}.sqlite", scope.storage_key()?));
        let catalog = Discovery::open(path, scope.clone()).await?;
        let state = catalog.state().await?;
        let mut slots = self.slots.lock().await;
        if slots.opening != Some(id) {
            drop(slots);
            catalog.close().await?;
            return Err(changed());
        }
        slots.opening = None;
        slots.active = Some(Arc::new(Session {
            id,
            remote,
            catalog,
            advancing: Arc::new(Semaphore::new(1)),
        }));
        Ok(Opened {
            session: id,
            scope,
            state,
        })
    }
    async fn session(&self, id: Uuid) -> Result<Arc<Session>> {
        self.slots
            .lock()
            .await
            .active
            .as_ref()
            .filter(|s| s.id == id)
            .cloned()
            .ok_or_else(changed)
    }
    pub async fn run(&self, id: Uuid, command: Command) -> Result<Value> {
        let session = self.session(id).await?;
        let catalog = &session.catalog;
        let result: Result<Value> = async {
            Ok(match command {
                Command::State => serde_json::to_value(catalog.state().await?)?,
                Command::Profiles { after } => {
                    serde_json::to_value(catalog.profiles(after).await?)?
                }
                Command::Advance => {
                    let _permit = session
                        .advancing
                        .clone()
                        .try_acquire_owned()
                        .context("Profile discovery is busy. Retry shortly.")?;
                    serde_json::to_value(session.remote.advance(catalog).await?)?
                }
                Command::Retry { expected_revision } => {
                    serde_json::to_value(catalog.retry(expected_revision).await?)?
                }
                Command::Refresh {
                    expected_revision,
                    full,
                } => serde_json::to_value(catalog.refresh(expected_revision, full).await?)?,
            })
        }
        .await;
        // Data AND errors from an old grant cannot update a replacement screen.
        self.session(id).await?;
        result
    }
    pub async fn creation(
        &self,
        db: &crate::database::Database,
        id: Uuid,
        command: creation::Command,
    ) -> Result<Value> {
        let session = self.session(id).await?;
        let _permit = if command.reads() {
            None
        } else {
            Some(
                session
                    .advancing
                    .clone()
                    .try_acquire_owned()
                    .context("Profile work is busy. Retry shortly.")?,
            )
        };
        if command.needs_discovery() {
            let state = session.catalog.state().await?;
            anyhow::ensure!(
                state.phase == shep_profile_core::drive::catalog::Phase::Complete
                    && state.error.is_none(),
                "Finish or retry discovery before publishing a profile."
            );
        }
        let result = creation::run(
            db,
            session.remote.scope(),
            session.remote.as_ref(),
            &session.catalog,
            command,
        )
        .await;
        self.session(id).await?;
        result
    }
    pub async fn close(&self, id: Uuid) -> Result<()> {
        let mut slots = self.slots.lock().await;
        if slots.opening == Some(id) {
            slots.opening = None;
        }
        let session = if slots.active.as_ref().is_some_and(|s| s.id == id) {
            slots.active.take()
        } else {
            None
        };
        drop(slots);
        if let Some(session) = session {
            let catalog = session.catalog.clone();
            drop(session);
            // Other accepted requests retain the old owner until they finish.
            // The active slot is already empty, and late observations are fenced.
            catalog.close().await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
