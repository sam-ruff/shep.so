//! One owner for the connection and local operation leases. Accepted jobs drain
//! even when their caller leaves; SQL never borrows state from the UI thread.
use anyhow::Context;
use rusqlite::Connection;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{mpsc, oneshot};

const CAPACITY: usize = 32;
type Job = Box<dyn FnOnce(&mut Owner) + Send>;

struct Owner {
    connection: Connection,
    // Drop after the connection, including when the final observer cancelled.
    _scratch: Option<tempfile::TempDir>,
    leases: HashMap<String, oneshot::Receiver<()>>,
    // Root ownership ends only after the last admitted write has drained and
    // the connection is closed, so no migration can start beneath a write.
    _guard: Option<Arc<crate::cache_cipher::ownership::Guard>>,
}

pub(crate) struct Worker {
    commands: mpsc::Sender<Job>,
}

pub(super) struct Lease {
    // Dropping a sender closes its receiver. Release does not need queue space
    // or a lock, including when a caller is cancelled before receiving its lease.
    _release: oneshot::Sender<()>,
}

impl Worker {
    #[cfg(test)]
    pub fn new(connection: Connection) -> anyhow::Result<Self> {
        Self::named(connection, "shep-mail-cache")
    }

    pub fn named(connection: Connection, name: &'static str) -> anyhow::Result<Self> {
        Self::start(connection, name, None, None)
    }

    /// An owner of a database inside a guarded data root.
    pub fn owned(
        connection: Connection,
        name: &'static str,
        guard: Option<Arc<crate::cache_cipher::ownership::Guard>>,
    ) -> anyhow::Result<Self> {
        Self::start(connection, name, None, guard)
    }

    pub fn with_scratch(
        connection: Connection,
        scratch: Option<tempfile::TempDir>,
        guard: Option<Arc<crate::cache_cipher::ownership::Guard>>,
    ) -> anyhow::Result<Self> {
        Self::start(connection, "shep-mail-cache", scratch, guard)
    }

    fn start(
        connection: Connection,
        name: &'static str,
        scratch: Option<tempfile::TempDir>,
        guard: Option<Arc<crate::cache_cipher::ownership::Guard>>,
    ) -> anyhow::Result<Self> {
        let (commands, mut input) = mpsc::channel::<Job>(CAPACITY);
        // Bundle resources before spawning so failure to create the thread
        // follows the same connection-before-scratch drop order as normal exit.
        let mut owner = Owner {
            connection,
            _scratch: scratch,
            leases: HashMap::new(),
            _guard: guard,
        };
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                while let Some(job) = input.blocking_recv() {
                    job(&mut owner);
                }
            })
            .context("Could not start the local database worker")?;
        Ok(Self { commands })
    }

    async fn request<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Owner) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Box::new(move |owner| {
                // Execute every accepted write. Cancelling the observation must
                // not cancel persistence or lose an acknowledged provider receipt.
                let _ = reply.send(job(owner));
            }))
            .await
            .map_err(|_| anyhow::anyhow!("The local database worker is unavailable."))?;
        result
            .await
            .context("The local database worker stopped before acknowledging the operation")?
    }

    pub async fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce(&mut Connection) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        self.request(move |owner| job(&mut owner.connection)).await
    }

    pub(super) async fn lease(&self, id: String) -> anyhow::Result<Lease> {
        self.request(move |owner| {
            owner.leases.retain(|_, release| {
                matches!(release.try_recv(), Err(oneshot::error::TryRecvError::Empty))
            });
            anyhow::ensure!(
                !owner.leases.contains_key(&id),
                "This mail operation is still running in another window"
            );
            let (release, released) = oneshot::channel();
            owner.leases.insert(id, released);
            Ok(Lease { _release: release })
        })
        .await
    }
}

#[cfg(test)]
mod tests;
