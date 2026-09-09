//! One owner for the connection and local operation leases. Accepted jobs drain
//! even when their caller leaves; SQL never borrows state from the UI thread.
use anyhow::Context;
use rusqlite::Connection;
use std::collections::HashMap;
use tokio::sync::{mpsc, oneshot};

const CAPACITY: usize = 32;
type Job = Box<dyn FnOnce(&mut Owner) + Send>;

struct Owner {
    connection: Connection,
    leases: HashMap<String, oneshot::Receiver<()>>,
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
    pub fn new(connection: Connection) -> anyhow::Result<Self> {
        Self::named(connection, "shep-mail-cache")
    }

    pub fn named(connection: Connection, name: &'static str) -> anyhow::Result<Self> {
        let (commands, mut input) = mpsc::channel::<Job>(CAPACITY);
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let mut owner = Owner {
                    connection,
                    leases: HashMap::new(),
                };
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
