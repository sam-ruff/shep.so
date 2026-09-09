//! One active profile-history owner per native workspace. This API stages and
//! inspects metadata only; it cannot apply accounts, access credentials or send
//! provider commands. Authentication/enrollment must supply the binding later.
use anyhow::{Context, Result};
use serde_json::Value;
use shep_profile_core::history::{Binding, Command, Worker};
use std::{path::Path, sync::Arc};
use tokio::sync::{Mutex, Semaphore};

pub(crate) struct Runtime {
    admitted: Arc<Semaphore>,
    active: Mutex<Option<(Binding, Worker)>>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            admitted: Arc::new(Semaphore::new(1)),
            active: Mutex::new(None),
        }
    }
}
impl Runtime {
    pub async fn run(&self, path: &Path, binding: Binding, command: Command) -> Result<Value> {
        // MobileProfile owns its request task through completion even when Dart
        // leaves. This permit cannot be released while accepted SQL still runs.
        let _permit = self
            .admitted
            .clone()
            .try_acquire_owned()
            .context("Profile history is busy. Retry the same request shortly.")?;
        let key = binding.storage_key()?;
        let mut active = self.active.lock().await;
        if !active.as_ref().is_some_and(|(old, _)| old == &binding) {
            if let Some((_, old)) = active.take() {
                old.close().await?;
            }
            let file = path
                .with_extension("profile-history")
                .join(format!("{key}.sqlite"));
            let worker = Worker::open(file, binding.clone()).await?;
            *active = Some((binding, worker));
        }
        let reply = active.as_ref().unwrap().1.request(command).await?;
        Ok(serde_json::to_value(reply)?)
    }
    pub async fn close(&self, binding: Binding) -> Result<()> {
        let _permit =
            self.admitted.clone().try_acquire_owned().context(
                "Profile history is busy. Wait for the current request before closing it.",
            )?;
        let mut active = self.active.lock().await;
        if active
            .as_ref()
            .is_some_and(|(current, _)| current != &binding)
        {
            return Err(shep_profile_core::history::Error::Changed.into());
        }
        if let Some((_, worker)) = active.take() {
            worker.close().await?;
        }
        Ok(())
    }
}
