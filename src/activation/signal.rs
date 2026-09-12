use std::{
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::watch;

const CLOSING: u64 = 1 << 63;

#[derive(Debug, Clone)]
pub struct Signal(Arc<Inner>);

#[derive(Debug)]
struct Inner {
    requested: AtomicU64,
    acknowledged: AtomicU64,
    requests: watch::Sender<u64>,
    acknowledgments: watch::Sender<u64>,
}

impl Default for Signal {
    fn default() -> Self {
        Self(Arc::new(Inner {
            requested: AtomicU64::new(0),
            acknowledged: AtomicU64::new(0),
            requests: watch::channel(0).0,
            acknowledgments: watch::channel(0).0,
        }))
    }
}

impl Hash for Signal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        Arc::as_ptr(&self.0).hash(state);
    }
}

impl Signal {
    pub(crate) fn request(&self) -> Option<u64> {
        let previous = self
            .0
            .requested
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                (value < CLOSING - 1).then_some(value + 1)
            })
            .ok()?;
        let generation = previous + 1;
        self.0
            .requests
            .send_modify(|current| *current = (*current).max(generation));
        Some(generation)
    }

    pub(crate) fn acknowledge(&self, generation: u64) {
        self.0.acknowledged.fetch_max(generation, Ordering::SeqCst);
        self.0
            .acknowledgments
            .send_modify(|current| *current = (*current).max(generation));
    }

    /// Closing and accepting Open compete for the same atomic state.
    pub(crate) fn try_close(&self) -> bool {
        let acknowledged = self.0.acknowledged.load(Ordering::SeqCst);
        self.0
            .requested
            .compare_exchange(acknowledged, CLOSING, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
            || self.0.requested.load(Ordering::SeqCst) == CLOSING
    }

    pub(super) async fn acknowledged(&self, generation: u64) -> bool {
        self.0
            .acknowledgments
            .subscribe()
            .wait_for(|value| *value >= generation)
            .await
            .is_ok()
    }
}

pub(crate) fn subscription(
    signal: &Option<Signal>,
) -> impl iced::futures::Stream<Item = u64> + use<> {
    let signal = signal.clone();
    iced::stream::channel(1, async move |mut output| {
        use iced::futures::SinkExt;
        if let Some(signal) = signal {
            let mut requests = signal.0.requests.subscribe();
            loop {
                let generation = *requests.borrow_and_update();
                if generation > signal.0.acknowledged.load(Ordering::SeqCst)
                    && output.send(generation).await.is_err()
                {
                    return;
                }
                if requests.changed().await.is_err() {
                    return;
                }
            }
        }
        std::future::pending::<()>().await;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_open_prevents_terminal_close() {
        let signal = Signal::default();
        let first = signal.request().expect("open accepted");
        let last = signal.request().expect("second open accepted");
        signal.acknowledge(first);
        assert!(!signal.try_close());
        signal.acknowledge(last);
        assert!(signal.try_close());
        assert_eq!(signal.request(), None);
    }

    #[test]
    fn terminal_close_rejects_new_open() {
        let signal = Signal::default();
        assert!(signal.try_close());
        assert_eq!(signal.request(), None);
    }
}
