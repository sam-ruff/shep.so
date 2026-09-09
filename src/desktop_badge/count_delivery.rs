//! Await each native main-queue acknowledgement before scheduling another one.
//! A busy desktop retains only the latest count in the capacity-one watch signal.
use std::future::Future;
use tokio::sync::watch;

pub(super) async fn run<F, Fut>(mut counts: watch::Receiver<u64>, mut deliver: F)
where
    F: FnMut(u64) -> Fut,
    Fut: Future<Output = ()>,
{
    loop {
        let count = *counts.borrow_and_update();
        deliver(count).await;
        if counts.changed().await.is_err() {
            deliver(0).await;
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn held_native_delivery_coalesces_latest_count_and_clears_on_close() {
        let (tx, rx) = watch::channel(3);
        let (observed, mut events) = tokio::sync::mpsc::channel(1);
        let worker = tokio::spawn(async move {
            run(rx, |count| {
                let observed = observed.clone();
                async move {
                    let (ack, done) = tokio::sync::oneshot::channel();
                    observed.send((count, ack)).await.unwrap();
                    let _ = done.await;
                }
            })
            .await;
        });
        let (initial, ack) = events.recv().await.unwrap();
        assert_eq!(initial, 3);
        for count in 4..100 {
            tx.send_replace(count);
        }
        tx.send_replace(0); // preference disabled while native delivery is held
        ack.send(()).unwrap();
        let (disabled, ack) = events.recv().await.unwrap();
        assert_eq!(disabled, 0);
        tx.send_replace(17);
        ack.send(()).unwrap();
        let (restored, ack) = events.recv().await.unwrap();
        assert_eq!(restored, 17);
        drop(tx);
        ack.send(()).unwrap();
        let (closed, ack) = events.recv().await.unwrap();
        assert_eq!(closed, 0);
        ack.send(()).unwrap();
        worker.await.unwrap();
    }
}
