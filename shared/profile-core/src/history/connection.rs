//! Platform clients own keys; the shared protocol owns connection lifetime.
use super::Result;
use rusqlite::Connection;
use std::{path::Path, sync::Arc};

type Open = dyn Fn(&Path) -> Result<Connection> + Send + Sync;

/// An owned connection initializer, invoked only after the normal file lock is
/// acquired and before any schema access. Failure never falls back to an
/// unconfigured connection. Clients may capture a zeroizing local key here;
/// key retrieval belongs outside the callback and outside the UI thread.
#[derive(Clone)]
pub struct ConnectionFactory(Arc<Open>);

impl Default for ConnectionFactory {
    fn default() -> Self {
        Self::new(|path| Ok(Connection::open(path)?))
    }
}

impl ConnectionFactory {
    pub fn new(open: impl Fn(&Path) -> Result<Connection> + Send + Sync + 'static) -> Self {
        Self(Arc::new(open))
    }

    pub(crate) fn open(&self, path: &Path) -> Result<Connection> {
        (self.0)(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Binding, Error, Journal, Worker};

    fn binding() -> Binding {
        Binding {
            namespace: "so.shep.fixture".into(),
            principal: "drive:fixture".into(),
            profile: uuid::Uuid::new_v4(),
            generation: uuid::Uuid::new_v4(),
        }
    }

    #[test]
    fn initialization_precedes_schema_and_preserves_configuration_and_ownership() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let binding = binding();
        let (sent, received) = std::sync::mpsc::sync_channel(4);
        let connections = ConnectionFactory::new(move |path| {
            let c = Connection::open(path)?;
            let objects: i64 =
                c.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))?;
            sent.send(objects).unwrap();
            c.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA cache_size=-1234;")?;
            Ok(c)
        });
        let journal = Journal::open_with(&path, binding.clone(), &connections).unwrap();
        assert_eq!(received.recv().unwrap(), 0);
        assert_eq!(
            journal
                .db
                .query_row("PRAGMA temp_store", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            journal
                .db
                .query_row("PRAGMA cache_size", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            -1234
        );
        assert!(matches!(
            Journal::open_with(&path, binding.clone(), &connections),
            Err(Error::Owned)
        ));
        assert!(received.try_recv().is_err());
        drop(journal);
        let _reopened = Journal::open_with(&path, binding, &connections).unwrap();
        assert!(received.recv().unwrap() > 0);
    }

    #[tokio::test]
    async fn failed_initializer_never_opens_plaintext_and_can_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let binding = binding();
        let failed = ConnectionFactory::new(|_| Err(Error::Storage));
        assert!(matches!(
            Worker::open_with(path.clone(), binding.clone(), failed).await,
            Err(Error::Storage)
        ));
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
        let worker = Worker::open(path, binding).await.unwrap();
        worker.close().await.unwrap();
    }
}
