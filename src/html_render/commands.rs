//! Cancellation wakes blocking renderer receives even while iced retains UI state.
use tokio::sync::{mpsc, watch};
pub type Sender<T> = mpsc::Sender<T>;

pub(super) struct CancelOnDrop(watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
pub(super) fn cancellation() -> (CancelOnDrop, watch::Receiver<bool>) {
    let (tx, rx) = watch::channel(false);
    (CancelOnDrop(tx), rx)
}

pub struct Receiver<T> {
    inner: mpsc::Receiver<T>,
    cancel: watch::Sender<bool>,
    stopped: watch::Receiver<bool>,
}
pub fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let (tx, inner) = mpsc::channel(capacity);
    let (cancel, stopped) = watch::channel(false);
    (
        tx,
        Receiver {
            inner,
            cancel,
            stopped,
        },
    )
}
impl<T> Receiver<T> {
    pub(super) fn cancel_on_drop(&self) -> CancelOnDrop {
        CancelOnDrop(self.cancel.clone())
    }
    pub fn blocking_recv(&mut self) -> Option<T> {
        if *self.stopped.borrow() {
            return None;
        }
        futures::executor::block_on(async {
            tokio::select! {
                biased;
                _ = self.stopped.changed() => None,
                value = self.inner.recv() => value,
            }
        })
    }
    pub fn try_recv(&mut self) -> Result<T, mpsc::error::TryRecvError> {
        if *self.stopped.borrow() {
            return Err(mpsc::error::TryRecvError::Disconnected);
        }
        self.inner.try_recv()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn cancellation_wakes_an_idle_worker_with_a_retained_ui_sender() {
        let (tx, mut input) = channel::<()>(1);
        let cancel = input.cancel_on_drop();
        let worker = tokio::task::spawn_blocking(move || input.blocking_recv());
        drop(cancel);
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(1), worker)
                .await
                .unwrap()
                .unwrap(),
            None
        );
        assert!(tx.is_closed());
    }
}
