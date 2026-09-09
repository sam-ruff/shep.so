use super::*;
use std::path::PathBuf;
use tokio::sync::{mpsc, oneshot, watch};

type Request = Box<dyn FnOnce(&mut Journal) + Send>;
/// Exactly one connection owner and at most 32 accepted pending commands. A
/// cancelled observation never cancels an accepted write or releases its lock.
#[derive(Clone)]
pub struct Worker {
    binding: Binding,
    commands: mpsc::Sender<Request>,
    finished: watch::Receiver<bool>,
}
impl Worker {
    pub async fn open(path: PathBuf, binding: Binding) -> Result<Self> {
        let worker_binding = binding.clone();
        let (commands, mut input) = mpsc::channel::<Request>(32);
        let (started, ready) = oneshot::channel();
        let (finished, done) = watch::channel(false);
        std::thread::Builder::new()
            .name("shep-profile-history".into())
            .spawn(move || {
                if let Some(parent) = path.parent() {
                    let mut directory = std::fs::DirBuilder::new();
                    directory.recursive(true);
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::DirBuilderExt;
                        directory.mode(0o700);
                    }
                    if directory.create(parent).is_err() {
                        let _ = started.send(Err(Error::Storage));
                        return;
                    }
                }
                let mut journal = match Journal::open(&path, binding) {
                    Ok(journal) => journal,
                    Err(error) => {
                        let _ = started.send(Err(error));
                        return;
                    }
                };
                if started.send(Ok(())).is_err() {
                    return;
                }
                while let Some(request) = input.blocking_recv() {
                    request(&mut journal);
                }
                drop(journal);
                let _ = finished.send(true);
            })
            .map_err(|_| Error::Stopped)?;
        ready.await.map_err(|_| Error::Stopped)??;
        Ok(Self {
            binding: worker_binding,
            commands,
            finished: done,
        })
    }
    /// Immutable device-local binding; provider calls must independently verify it.
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub async fn request(&self, command: Command) -> Result<Reply> {
        self.submit(command)?.await.map_err(|_| Error::Stopped)?
    }
    fn submit(&self, command: Command) -> Result<oneshot::Receiver<Result<Reply>>> {
        match &command {
            Command::Import { record } if record.len() > crate::MAX_RECORD_BYTES => {
                return Err(crate::Error::TooLarge.into());
            }
            Command::Edit { edit } => {
                crate::json::encode(edit)?;
            }
            Command::Fields { after } if after.as_ref().is_some_and(|s| s.len() > 128) => {
                return Err(Error::Changed);
            }
            Command::Versions { target, .. } | Command::Value { target, .. }
                if target.len() > 128 =>
            {
                return Err(Error::Changed);
            }
            _ => {}
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .try_send(Box::new(move |journal| {
                let _ = reply.send(journal.execute(command));
            }))
            .map_err(|e| match e {
                mpsc::error::TrySendError::Full(_) => Error::Busy,
                mpsc::error::TrySendError::Closed(_) => Error::Stopped,
            })?;
        Ok(response)
    }
    /// Stop after every accepted command completes and every other clone drops.
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
    use crate::{Action, Change, SettingKey};

    #[tokio::test]
    async fn accepted_write_survives_cancelled_reply_and_full_queue_and_close_drains() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("history.sqlite");
        let binding = Binding {
            namespace: "so.shep.fixture".into(),
            principal: "drive:fixture".into(),
            profile: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        };
        let worker = Worker::open(path.clone(), binding.clone()).await.unwrap();
        let (entered, blocked) = oneshot::channel();
        let (release, held) = std::sync::mpsc::channel();
        worker
            .commands
            .try_send(Box::new(move |_| {
                let _ = entered.send(());
                held.recv().unwrap();
            }))
            .ok()
            .unwrap();
        blocked.await.unwrap();
        let frozen = LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: 0,
            changes: vec![Change {
                action: Action::Setting {
                    key: SettingKey::Appearance,
                    value: serde_json::json!("Dark"),
                },
                extra: Default::default(),
            }],
            resolutions: vec![],
        };
        let reply = worker
            .submit(Command::Edit {
                edit: frozen.clone(),
            })
            .unwrap();
        drop(reply);
        let replies = (0..31)
            .map(|_| worker.submit(Command::State).unwrap())
            .collect::<Vec<_>>();
        assert!(matches!(worker.submit(Command::State), Err(Error::Busy)));
        assert!(matches!(
            Journal::open(&path, binding.clone()),
            Err(Error::Owned)
        ));
        release.send(()).unwrap();
        for reply in replies {
            let Reply::State(state) = reply.await.unwrap().unwrap() else {
                panic!()
            };
            assert_eq!(state.operations, 1);
        }
        let Reply::State(state) = worker
            .request(Command::Edit { edit: frozen })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(state.operations, 1);
        assert_eq!(state.queued, 1);
        worker.close().await.unwrap();
        let reopened = Journal::open(&path, binding).unwrap();
        assert_eq!(reopened.state().unwrap().queued, 1);
    }
}
