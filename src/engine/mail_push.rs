//! Keeps one IMAP IDLE watcher per account so a server push makes that
//! account due at once. Interval polling stays the fallback for every account.

use super::*;
use crate::providers::mail::push::{Push, WatchEnd};
use mail_sync::SyncTarget;
use std::{
    collections::HashMap,
    future::Future,
    sync::atomic::{AtomicBool, Ordering},
};
use tokio::sync::watch;

/// Reconnect delays double from the first to the cap and reset once IDLE
/// connects again.
const FIRST_RETRY: Duration = Duration::from_secs(5);
const MAX_RETRY: Duration = Duration::from_secs(300);

pub(super) type Stop = watch::Receiver<bool>;

/// Resolves once the watcher should close its connection.
pub(super) async fn stopped(mut stop: Stop) {
    let _ = stop.wait_for(|stopped| *stopped).await;
}

/// Where a watcher reports progress for one target.
#[derive(Clone)]
pub(super) struct Sink {
    key: String,
    changed: mpsc::UnboundedSender<String>,
    connected: Arc<AtomicBool>,
}

impl Sink {
    pub(super) fn push(&self, push: Push) {
        match push {
            Push::Connected => self.connected.store(true, Ordering::SeqCst),
            Push::Changed => {
                let _ = self.changed.send(self.key.clone());
            }
        }
    }
}

/// Runs the watcher for one target until it is stopped or the server turns
/// out to lack IDLE, reconnecting after failures with growing delays.
pub(super) async fn supervise<T, W, WF>(
    target: T,
    watch: W,
    changed: mpsc::UnboundedSender<String>,
    stop: Stop,
) where
    T: SyncTarget,
    W: Fn(T, Sink, Stop) -> WF,
    WF: Future<Output = anyhow::Result<WatchEnd>>,
{
    let mut delay = FIRST_RETRY;
    loop {
        let connected = Arc::new(AtomicBool::new(false));
        let sink = Sink {
            key: target.key().to_owned(),
            changed: changed.clone(),
            connected: connected.clone(),
        };
        match watch(target.clone(), sink, stop.clone()).await {
            Ok(WatchEnd::Stopped) | Ok(WatchEnd::Unsupported) => return,
            Err(error) => tracing::warn!("{} mail watcher failed: {error:#}", target.name()),
        }
        if connected.load(Ordering::SeqCst) {
            delay = FIRST_RETRY;
        }
        let wait = delay;
        delay = (delay * 2).min(MAX_RETRY);
        tokio::select! {
            biased;
            _ = stopped(stop.clone()) => return,
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// A running supervisor and the connection settings it was started with.
struct Running {
    connection: String,
    stop: watch::Sender<bool>,
}

/// The running supervisors, one per listed target.
#[derive(Default)]
pub(super) struct Watchers {
    running: HashMap<String, Running>,
    tasks: tokio::task::JoinSet<()>,
}

impl Watchers {
    /// Starts a supervisor for each newly listed target and stops the
    /// supervisors of targets no longer listed. A target whose connection
    /// settings changed gets a fresh supervisor after the old one is told to
    /// log out.
    pub(super) fn reconcile<T, W, WF>(
        &mut self,
        targets: &[T],
        watch: &W,
        changed: &mpsc::UnboundedSender<String>,
    ) where
        T: SyncTarget,
        W: Fn(T, Sink, Stop) -> WF + Clone + Send + 'static,
        WF: Future<Output = anyhow::Result<WatchEnd>> + Send + 'static,
    {
        self.running.retain(|key, running| {
            let current = targets
                .iter()
                .any(|target| target.key() == key && target.connection() == running.connection);
            if !current {
                let _ = running.stop.send(true);
            }
            current
        });
        for target in targets {
            if self.running.contains_key(target.key()) {
                continue;
            }
            let (stop, receiver) = watch::channel(false);
            self.running.insert(
                target.key().to_owned(),
                Running {
                    connection: target.connection().to_owned(),
                    stop,
                },
            );
            let watch = watch.clone();
            let changed = changed.clone();
            let target = target.clone();
            self.tasks
                .spawn(supervise(target, watch, changed, receiver));
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Waits for one supervisor to finish; call only while some are running.
    pub(super) async fn reap(&mut self) {
        let _ = self.tasks.join_next().await;
    }

    /// Signals every watcher and gives their sessions a moment to log out.
    pub(super) async fn shutdown(mut self) {
        for running in self.running.values() {
            let _ = running.stop.send(true);
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), async {
            while self.tasks.join_next().await.is_some() {}
        })
        .await;
    }
}
