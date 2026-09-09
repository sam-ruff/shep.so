use super::*;
use std::sync::Arc;
use tokio::sync::{Semaphore, mpsc, oneshot, watch};

type Request = Box<dyn FnOnce(&mut Catalog) + Send>;
/// One owning thread and 32 accepted commands. Observations remain available
/// while one provider step is pending. Accepted SQL finishes after cancellation.
#[derive(Clone)]
pub struct Discovery {
    scope: Scope,
    commands: mpsc::Sender<Request>,
    finished: watch::Receiver<bool>,
    advancing: Arc<Semaphore>,
}
impl Discovery {
    pub async fn open(path: PathBuf, scope: Scope) -> Result<Self> {
        Self::open_with(path, scope, history::ConnectionFactory::default()).await
    }
    /// The same factory keys the catalog and every nested observation journal.
    pub async fn open_with(
        path: PathBuf,
        scope: Scope,
        connections: history::ConnectionFactory,
    ) -> Result<Self> {
        scope.validate()?;
        let binding = scope.clone();
        let (commands, mut requests) = mpsc::channel::<Request>(32);
        let (started, ready) = oneshot::channel();
        let (finished, done) = watch::channel(false);
        std::thread::Builder::new()
            .name("shep-profile-discovery".into())
            .spawn(move || {
                let mut catalog = match Catalog::open_with(&path, binding, connections) {
                    Ok(catalog) => catalog,
                    Err(error) => {
                        let _ = started.send(Err(error));
                        return;
                    }
                };
                if started.send(Ok(())).is_err() {
                    return;
                }
                while let Some(request) = requests.blocking_recv() {
                    request(&mut catalog);
                }
                drop(catalog);
                let _ = finished.send(true);
            })
            .map_err(|_| Error::Stopped)?;
        ready.await.map_err(|_| Error::Stopped)??;
        Ok(Self {
            scope,
            commands,
            finished: done,
            advancing: Arc::new(Semaphore::new(1)),
        })
    }
    async fn call<T: Send + 'static>(
        &self,
        action: impl FnOnce(&mut Catalog) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (send, reply) = oneshot::channel();
        self.commands
            .try_send(Box::new(move |catalog| {
                let _ = send.send(action(catalog));
            }))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => Error::Busy,
                mpsc::error::TrySendError::Closed(_) => Error::Stopped,
            })?;
        reply.await.map_err(|_| Error::Stopped)?
    }
    pub fn scope(&self) -> &Scope {
        &self.scope
    }
    pub async fn state(&self) -> Result<State> {
        self.call(|catalog| catalog.state()).await
    }
    pub async fn profiles(&self, after: Option<String>) -> Result<Vec<Profile>> {
        if after.as_ref().is_some_and(|key| key.len() > 73) {
            return Err(Error::Changed);
        }
        self.call(move |catalog| catalog.profiles(after.as_deref()))
            .await
    }
    /// Restarting retains known file identities and remote history. A changed
    /// revision fences the previous HTTP response without waiting for that read.
    pub async fn refresh(&self, expected_revision: u64, full: bool) -> Result<State> {
        self.call(move |catalog| catalog.refresh(expected_revision, full))
            .await
    }
    pub async fn retry(&self, expected_revision: u64) -> Result<State> {
        self.call(move |catalog| catalog.retry(expected_revision))
            .await
    }
    pub async fn advance(&self, drive: &Drive) -> Result<State> {
        self.scope.check(drive)?;
        let _permit = self
            .advancing
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let (revision, work) = self.call(|catalog| catalog.work()).await?;
        enum Action {
            Start(String),
            Files(Page),
            Changes(ChangePage),
            Accept {
                position: u64,
                file: File,
                record: String,
            },
            Drain {
                profile: Uuid,
                generation: Uuid,
            },
            Advance,
        }
        // Only GETs occur here. Cancelling one leaves its saved page unchanged;
        // accepted storage commands retain ownership through their commit.
        let action: std::result::Result<Action, crate::drive::Error> = match work {
            Work::Start => drive.start_page_token().await.map(Action::Start),
            Work::List(token) => drive.list_page(token.as_deref()).await.map(Action::Files),
            Work::Changes(token) => drive.changes_page(&token).await.map(Action::Changes),
            Work::Download { position, file } => {
                drive.download(&file).await.map(|record| Action::Accept {
                    position,
                    file,
                    record,
                })
            }
            Work::Drain {
                profile,
                generation,
            } => Ok(Action::Drain {
                profile,
                generation,
            }),
            Work::Advance => Ok(Action::Advance),
            Work::Done => return self.state().await,
        };
        let action = match action {
            Ok(action) => action,
            Err(error) => {
                let message = error.to_string();
                self.call(move |catalog| catalog.record_error(revision, &message))
                    .await?;
                return Err(error.into());
            }
        };
        self.call(move |catalog| {
            catalog.step(revision, |catalog| match action {
                Action::Start(token) => catalog.start(token),
                Action::Files(page) => catalog.stage_files(page),
                Action::Changes(page) => catalog.stage_changes(page),
                Action::Accept {
                    position,
                    file,
                    record,
                } => catalog.accept(position, file, record),
                Action::Drain {
                    profile,
                    generation,
                } => catalog.drain(profile, generation),
                Action::Advance => catalog.advance_page(),
            })
        })
        .await
    }
    /// The Drive transport calls this only after checking the exact remote bytes.
    /// Save the discovery receipt before acknowledging the local upload journal.
    pub(crate) async fn accept_upload(&self, file: File, record: String) -> Result<State> {
        self.call(move |catalog| catalog.accept_upload(file, record))
            .await
    }
    /// Drain accepted work once every other discovery handle has been dropped.
    pub async fn close(self) -> Result<()> {
        let Self {
            commands,
            mut finished,
            ..
        } = self;
        drop(commands);
        while !*finished.borrow() {
            finished.changed().await.map_err(|_| Error::Stopped)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope {
        Scope {
            namespace: "so.shep.fixture".into(),
            principal: "drive:fixture-owner".into(),
        }
    }

    #[tokio::test]
    async fn cancelled_observers_and_a_full_queue_keep_accepted_progress_and_drain_ownership() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite");
        let discovery = Discovery::open(path.clone(), scope()).await.unwrap();
        let (entered, held) = oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        discovery
            .commands
            .try_send(Box::new(move |_| {
                entered.send(()).unwrap();
                gate.recv().unwrap();
            }))
            .ok()
            .unwrap();
        held.await.unwrap();
        let (send, cancelled) = oneshot::channel();
        discovery
            .commands
            .try_send(Box::new(move |catalog| {
                let _ = send.send(catalog.step(0, |c| c.start("durable-start".into())));
            }))
            .ok()
            .unwrap();
        drop(cancelled);
        let mut observations = Vec::new();
        for _ in 0..31 {
            let (send, reply) = oneshot::channel();
            observations.push(reply);
            discovery
                .commands
                .try_send(Box::new(move |catalog| {
                    let _ = send.send(catalog.state());
                }))
                .ok()
                .unwrap();
        }
        assert!(matches!(discovery.state().await, Err(Error::Busy)));
        assert!(matches!(Catalog::open(&path, scope()), Err(Error::Owned)));
        release.send(()).unwrap();
        for reply in observations {
            assert_eq!(reply.await.unwrap().unwrap().phase, Phase::Files);
        }
        discovery.close().await.unwrap();
        let reopened = Discovery::open(path, scope()).await.unwrap();
        assert_eq!(reopened.state().await.unwrap().phase, Phase::Files);
        reopened.close().await.unwrap();
    }

    #[tokio::test]
    async fn scope_and_canonical_path_ownership_survive_reopening() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite");
        let discovery = Discovery::open(path.clone(), scope()).await.unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::{fs::PermissionsExt, fs::symlink};
            let alias = directory.path().join("alias.sqlite");
            symlink(&path, &alias).unwrap();
            assert!(matches!(
                Discovery::open(alias, scope()).await,
                Err(Error::Owned)
            ));
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
                0
            );
        }
        discovery.close().await.unwrap();
        let mut wrong = scope();
        wrong.principal = "drive:another-owner".into();
        assert!(matches!(
            Discovery::open(path.clone(), wrong).await,
            Err(Error::Binding)
        ));
        let reopened = Discovery::open(path, scope()).await.unwrap();
        assert_eq!(reopened.state().await.unwrap().revision, 0);
        assert!(matches!(
            reopened.refresh(u64::MAX, true).await,
            Err(Error::Changed)
        ));
        reopened.close().await.unwrap();
    }

    #[test]
    fn an_independent_process_cannot_claim_the_catalog() {
        const VARIABLE: &str = "SHEP_DISCOVERY_LOCK_FIXTURE";
        if let Some(path) = std::env::var_os(VARIABLE) {
            assert!(matches!(
                Catalog::open(Path::new(&path), scope()),
                Err(Error::Owned)
            ));
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("catalog.sqlite");
        let catalog = Catalog::open(&path, scope()).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "drive::catalog::worker::tests::an_independent_process_cannot_claim_the_catalog",
                "--nocapture",
            ])
            .env(VARIABLE, &path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        drop(catalog);
        assert!(Catalog::open(&path, scope()).is_ok());
    }
}
