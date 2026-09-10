//! One owner holds the Google token vault. Requests run in FIFO order on a
//! dedicated thread; an admitted refresh or sign-in finishes and persists even
//! when its caller is cancelled, so a rotated refresh token is never lost.
use super::tokens::{CredentialStore, State};
use futures::future::BoxFuture;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

const CAPACITY: usize = 32;
type Job = Box<dyn for<'a> FnOnce(&'a mut State, &'a Backend) -> BoxFuture<'a, ()> + Send>;

/// Everything a token operation needs besides the vault state itself.
pub(super) struct Backend {
    pub(super) http: reqwest::Client,
    pub(super) credentials: Arc<dyn CredentialStore>,
    pub(super) token_endpoint: url::Url,
}

#[derive(Clone)]
pub(super) struct Owner {
    commands: mpsc::Sender<Job>,
    start_error: Option<Arc<String>>,
}

impl Owner {
    pub(super) fn start(backend: Backend) -> Self {
        let (commands, mut input) = mpsc::channel::<Job>(CAPACITY);
        let started = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
            .and_then(|runtime| {
                std::thread::Builder::new()
                    .name("shep-google-tokens".into())
                    .spawn(move || {
                        let mut state = State::default();
                        // Every admitted job runs to completion before the next
                        // one starts or the thread exits, including after the
                        // last handle is dropped.
                        runtime.block_on(async {
                            while let Some(job) = input.recv().await {
                                job(&mut state, &backend).await;
                            }
                        });
                    })
                    .map(drop)
                    .map_err(|error| error.to_string())
            });
        Self {
            commands,
            start_error: started.err().map(|error| {
                Arc::new(format!(
                    "Could not start the Google token service: {error}. Reopen Shep."
                ))
            }),
        }
    }

    pub(super) async fn run<T, F>(&self, job: F) -> anyhow::Result<T>
    where
        T: Send + 'static,
        F: for<'a> FnOnce(&'a mut State, &'a Backend) -> BoxFuture<'a, anyhow::Result<T>>
            + Send
            + 'static,
    {
        if let Some(error) = &self.start_error {
            anyhow::bail!("{error}");
        }
        let (reply, result) = oneshot::channel();
        self.commands
            .send(Box::new(move |state, backend| {
                Box::pin(async move {
                    let _ = reply.send(job(state, backend).await);
                })
            }))
            .await
            .map_err(|_| {
                anyhow::anyhow!("The Google token service stopped. Reopen Shep and retry.")
            })?;
        result.await.map_err(|_| {
            anyhow::anyhow!("The Google token service stopped before confirming the operation. Reopen Shep and check the connection.")
        })?
    }
}
