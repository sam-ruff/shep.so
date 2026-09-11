//! One coordinator per lifecycle lane owns admission for shared provider work
//! and exclusive lifecycle changes. Holders release by dropping a one-shot
//! sender; abandoned requests leave the queue without occupying the lane.
use super::*;
use futures::{FutureExt, future::BoxFuture, stream::FuturesUnordered};
use std::collections::VecDeque;
use tokio::sync::oneshot;

const CAPACITY: usize = 32;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Shared,
    Exclusive,
}

pub(super) struct Access {
    // Dropping after the holder's final store commit acknowledges completion.
    _finished: oneshot::Sender<()>,
}

struct Request {
    mode: Mode,
    grant: oneshot::Sender<()>,
    finished: oneshot::Receiver<()>,
}
struct Pending {
    ticket: u64,
    mode: Mode,
    grant: oneshot::Sender<()>,
}

#[derive(Clone)]
pub(super) struct Lane(mpsc::Sender<Request>);
impl Default for Lane {
    fn default() -> Self {
        let (sender, requests) = mpsc::channel(CAPACITY);
        tokio::spawn(coordinate(requests));
        Self(sender)
    }
}

impl Lane {
    /// Shared access for provider work that must not overlap a lifecycle change.
    pub(super) async fn read(&self) -> Access {
        self.acquire(Mode::Shared).await
    }
    /// Exclusive access for login, disconnect, cleanup and connection changes.
    pub(super) async fn write(&self) -> Access {
        self.acquire(Mode::Exclusive).await
    }

    async fn acquire(&self, mode: Mode) -> Access {
        let (finished, receipt) = oneshot::channel();
        let (grant, granted) = oneshot::channel();
        // The coordinator outlives every handle and drains before exiting, so a
        // live caller never observes a closed lane.
        self.0
            .send(Request {
                mode,
                grant,
                finished: receipt,
            })
            .await
            .expect("lifecycle coordinator stays open");
        granted
            .await
            .expect("lifecycle coordinator acknowledges requests");
        Access {
            _finished: finished,
        }
    }

    #[cfg(test)]
    pub(super) async fn is_held(&self) -> bool {
        tokio::time::timeout(Duration::from_millis(200), self.write())
            .await
            .is_err()
    }
}

async fn coordinate(mut requests: mpsc::Receiver<Request>) {
    let mut shared = 0usize;
    let mut exclusive = false;
    let mut pending: VecDeque<Pending> = VecDeque::new();
    let mut finished: FuturesUnordered<BoxFuture<'static, (u64, Mode)>> = FuturesUnordered::new();
    let mut active: Vec<u64> = Vec::new();
    let mut next = 0u64;
    let mut closed = false;
    loop {
        // Strict FIFO: a shared request behind a waiting exclusive one waits, so
        // a disconnect cannot starve while provider work keeps arriving.
        while let Some(head) = pending.front() {
            if head.grant.is_closed() {
                pending.pop_front();
                continue;
            }
            let admissible = match head.mode {
                Mode::Shared => !exclusive,
                Mode::Exclusive => !exclusive && shared == 0,
            };
            if !admissible {
                break;
            }
            let head = pending.pop_front().expect("front was present");
            if head.grant.send(()).is_ok() {
                match head.mode {
                    Mode::Shared => shared += 1,
                    Mode::Exclusive => exclusive = true,
                }
                active.push(head.ticket);
            }
        }
        if closed && finished.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            Some((ticket, mode)) = finished.next(), if !finished.is_empty() => {
                if let Some(index) = active.iter().position(|t| *t == ticket) {
                    active.swap_remove(index);
                    match mode {
                        Mode::Shared => shared -= 1,
                        Mode::Exclusive => exclusive = false,
                    }
                }
                pending.retain(|request| request.ticket != ticket);
            }
            request = requests.recv(), if !closed && pending.len() < CAPACITY => {
                let Some(request) = request else { closed = true; continue };
                if request.grant.is_closed() { continue; }
                next = next.checked_add(1).expect("lifecycle ticket space exhausted");
                let ticket = next;
                let mode = request.mode;
                finished.push(async move {
                    let _ = request.finished.await;
                    (ticket, mode)
                }.boxed());
                pending.push_back(Pending { ticket, mode, grant: request.grant });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::poll;

    #[tokio::test]
    async fn shared_holders_block_exclusive_and_later_shared_waits_behind_it() {
        let lane = Lane::default();
        let first = lane.read().await;
        let second = lane.read().await;
        let mut write = Box::pin(lane.write());
        assert!(poll!(&mut write).is_pending());
        tokio::task::yield_now().await;
        let mut late = Box::pin(lane.read());
        assert!(poll!(&mut late).is_pending());
        drop(first);
        tokio::task::yield_now().await;
        assert!(poll!(&mut write).is_pending());
        assert!(poll!(&mut late).is_pending());
        drop(second);
        let writer = write.await;
        tokio::task::yield_now().await;
        assert!(poll!(&mut late).is_pending());
        drop(writer);
        let _late = late.await;
    }

    #[tokio::test]
    async fn abandoned_requests_leave_the_queue_and_do_not_occupy_the_lane() {
        let lane = Lane::default();
        let holder = lane.write().await;
        let mut waiting = Box::pin(lane.write());
        assert!(poll!(&mut waiting).is_pending());
        tokio::task::yield_now().await;
        drop(waiting);
        let mut reader = Box::pin(lane.read());
        assert!(poll!(&mut reader).is_pending());
        drop(holder);
        let reader = reader.await;
        assert!(lane.is_held().await);
        drop(reader);
        assert!(!lane.is_held().await);
    }

    #[tokio::test]
    async fn a_failing_holder_releases_on_drop_and_the_next_request_proceeds() {
        let lane = Lane::default();
        let failed: anyhow::Result<()> = async {
            let _access = lane.write().await;
            anyhow::bail!("provider failure")
        }
        .await;
        assert!(failed.is_err());
        let _next = tokio::time::timeout(Duration::from_secs(5), lane.write())
            .await
            .expect("lane released after the failed holder dropped");
    }

    #[tokio::test]
    async fn pending_work_is_bounded_and_granted_in_order() {
        let (tx, rx) = mpsc::channel(CAPACITY);
        let actor = tokio::spawn(coordinate(rx));
        let lane = Lane(tx.clone());
        let holder = lane.write().await;
        let mut waiting = Vec::new();
        for _ in 0..CAPACITY {
            let (grant, granted) = oneshot::channel();
            let (done, finished) = oneshot::channel();
            tx.send(Request {
                mode: Mode::Exclusive,
                grant,
                finished,
            })
            .await
            .unwrap();
            waiting.push((granted, done));
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            while tx.capacity() != CAPACITY {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("coordinator admits up to its pending bound");
        let (grant, mut overflow) = oneshot::channel();
        let (overflow_done, finished) = oneshot::channel();
        tx.send(Request {
            mode: Mode::Exclusive,
            grant,
            finished,
        })
        .await
        .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(
            tx.capacity(),
            CAPACITY - 1,
            "the 33rd request waits in the channel"
        );
        drop(holder);
        for (granted, done) in waiting {
            granted.await.unwrap();
            assert!(poll!(&mut overflow).is_pending());
            drop(done);
        }
        overflow.await.unwrap();
        drop(lane);
        drop(tx);
        tokio::task::yield_now().await;
        assert!(
            !actor.is_finished(),
            "the granted overflow holder is drained first"
        );
        drop(overflow_done);
        tokio::time::timeout(Duration::from_secs(5), actor)
            .await
            .expect("coordinator drains and exits after the last handle")
            .unwrap();
    }

    #[tokio::test]
    async fn close_waits_for_the_active_holder_before_the_coordinator_exits() {
        let (tx, rx) = mpsc::channel(CAPACITY);
        let actor = tokio::spawn(coordinate(rx));
        let lane = Lane(tx);
        let access = lane.read().await;
        drop(lane);
        tokio::task::yield_now().await;
        assert!(!actor.is_finished());
        drop(access);
        tokio::time::timeout(Duration::from_secs(5), actor)
            .await
            .expect("coordinator exits once the holder releases")
            .unwrap();
    }
}
