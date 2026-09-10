//! One channel-driven coordinator owns scheduling state. Account operations
//! hold completion senders, never a shared mutex or mutable scheduling map.
use super::*;
use futures::{FutureExt, future::BoxFuture, stream::FuturesUnordered};
use std::{
    collections::{HashMap, VecDeque},
    future::Future,
};
use tokio::sync::oneshot;

#[derive(Clone)]
pub(super) struct Accounts(mpsc::Sender<Request>);
impl Default for Accounts {
    fn default() -> Self {
        let (sender, requests) = mpsc::channel(32);
        tokio::spawn(coordinate(requests));
        Self(sender)
    }
}

pub(super) struct Access {
    // Dropping after the actual receipt/cache commit acknowledges completion.
    _finished: oneshot::Sender<()>,
}

struct Request {
    account: String,
    interrupt: Option<oneshot::Sender<()>>,
    grant: oneshot::Sender<bool>,
    finished: oneshot::Receiver<()>,
}
struct Pending {
    ticket: u64,
    account: String,
    interrupt: Option<oneshot::Sender<()>>,
    grant: oneshot::Sender<bool>,
}
struct Active {
    ticket: u64,
    interrupt: Option<oneshot::Sender<()>>,
}

pub(super) struct Stop {
    writer: oneshot::Receiver<()>,
    owner: oneshot::Receiver<()>,
}
impl Stop {
    pub(super) async fn cancelled(&mut self) {
        tokio::select! {
            _ = &mut self.writer => {},
            _ = &mut self.owner => {},
        }
    }
}

impl Accounts {
    async fn acquire(&self, id: &str, interrupt: Option<oneshot::Sender<()>>) -> Option<Access> {
        let (finished, receipt) = oneshot::channel();
        let (grant, granted) = oneshot::channel();
        self.0
            .send(Request {
                account: id.into(),
                interrupt,
                grant,
                finished: receipt,
            })
            .await
            .expect("account coordinator stays open");
        if granted
            .await
            .expect("account coordinator acknowledges requests")
        {
            Some(Access {
                _finished: finished,
            })
        } else {
            None
        }
    }

    pub(super) async fn write(&self, id: &str) -> Access {
        self.acquire(id, None)
            .await
            .expect("writes are never skipped")
    }

    pub(super) async fn sync<F, Fut>(&self, id: &str, work: F) -> anyhow::Result<()>
    where
        F: FnOnce(Stop) -> Fut + Send + 'static,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        let accounts = self.clone();
        let id = id.to_owned();
        let (owner, mut closed) = oneshot::channel();
        // A cycle deadline/shutdown drops owner and asks the read-only work to
        // stop. This owned task still observes every in-flight cache commit.
        let task = tokio::spawn(async move {
            let (interrupt, writer) = oneshot::channel();
            let access = tokio::select! {
                biased;
                _ = &mut closed => return Ok(()),
                access = accounts.acquire(&id, Some(interrupt)) => access,
            };
            let Some(_access) = access else { return Ok(()) };
            work(Stop {
                writer,
                owner: closed,
            })
            .await
        });
        let result = task.await.context("Account sync stopped unexpectedly")?;
        drop(owner);
        result
    }
}

async fn coordinate(mut requests: mpsc::Receiver<Request>) {
    let mut active: HashMap<String, Active> = HashMap::new();
    let mut pending: VecDeque<Pending> = VecDeque::new();
    let mut finished: FuturesUnordered<BoxFuture<'static, (u64, String)>> = FuturesUnordered::new();
    let mut next = 0u64;
    let mut closed = false;
    loop {
        // Grant independent accounts immediately. Queued writes keep FIFO order;
        // speculative sync yields if any write for its account is waiting.
        let mut i = 0;
        while i < pending.len() {
            let request = &pending[i];
            let skip = request.grant.is_closed()
                || request.interrupt.is_some()
                    && pending
                        .iter()
                        .any(|other| other.account == request.account && other.interrupt.is_none());
            if skip {
                let request = pending.remove(i).unwrap();
                let _ = request.grant.send(false);
            } else if !active.contains_key(&request.account) {
                let request = pending.remove(i).unwrap();
                if request.grant.send(true).is_ok() {
                    active.insert(
                        request.account,
                        Active {
                            ticket: request.ticket,
                            interrupt: request.interrupt,
                        },
                    );
                }
            } else {
                i += 1;
            }
        }
        if closed && finished.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            Some((ticket, account)) = finished.next(), if !finished.is_empty() => {
                if active.get(&account).is_some_and(|entry| entry.ticket == ticket) {
                    active.remove(&account);
                }
                pending.retain(|request| request.ticket != ticket);
            }
            request = requests.recv(), if !closed && pending.len() < 32 => {
                let Some(request) = request else { closed = true; continue };
                if request.grant.is_closed() { continue; }
                if request.interrupt.is_some() && active.contains_key(&request.account) {
                    let _ = request.grant.send(false);
                    continue;
                }
                next = next.checked_add(1).expect("account ticket space exhausted");
                let ticket = next;
                let account = request.account.clone();
                finished.push(async move {
                    let _ = request.finished.await;
                    (ticket, account)
                }.boxed());
                if request.interrupt.is_none()
                    && let Some(sync) = active.get_mut(&request.account)
                    && let Some(interrupt) = sync.interrupt.take()
                {
                    let _ = interrupt.send(());
                }
                pending.push_back(Pending { ticket, account: request.account, interrupt: request.interrupt, grant: request.grant });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::poll;
    use std::task::Poll;

    #[tokio::test]
    async fn queued_sync_cannot_get_ahead_of_a_waiting_write() {
        let accounts = Accounts::default();
        let first = accounts.write("fixture").await;
        let mut sync = Box::pin(accounts.sync("fixture", |_| async {
            panic!("queued sync must yield before contacting the provider")
        }));
        assert!(poll!(&mut sync).is_pending());
        // Run the owned task until it has joined the lock queue.
        tokio::task::yield_now().await;
        let mut write = Box::pin(accounts.write("fixture"));
        assert!(poll!(&mut write).is_pending());
        drop(first);
        sync.await.unwrap();
        let _writer = write.await;
    }

    #[tokio::test]
    async fn cancelling_a_queued_writer_does_not_disable_future_sync() {
        let accounts = Accounts::default();
        let first = accounts.write("fixture").await;
        let mut writer = Box::pin(accounts.write("fixture"));
        assert!(poll!(&mut writer).is_pending());
        drop(writer);
        drop(first);
        let (started, start) = oneshot::channel();
        accounts
            .sync("fixture", |mut stop| async move {
                assert!(matches!(poll!(Box::pin(stop.cancelled())), Poll::Pending));
                started.send(()).unwrap();
                Ok(())
            })
            .await
            .unwrap();
        start.await.unwrap();
    }

    fn waiting(id: &str) -> (Request, oneshot::Receiver<bool>, oneshot::Sender<()>) {
        let (grant, response) = oneshot::channel();
        let (done, finished) = oneshot::channel();
        (
            Request {
                account: id.into(),
                interrupt: None,
                grant,
                finished,
            },
            response,
            done,
        )
    }

    #[tokio::test]
    async fn coordinator_bounds_pending_work_and_drains_abandoned_requests() {
        let (tx, rx) = mpsc::channel(32);
        let actor = tokio::spawn(coordinate(rx));
        let accounts = Accounts(tx.clone());
        let active = accounts.write("fixture").await;
        let mut waiting_calls = Vec::new();
        for _ in 0..32 {
            let (request, response, done) = waiting("fixture");
            tx.send(request).await.unwrap();
            waiting_calls.push((response, done));
        }
        tokio::time::timeout(Duration::from_secs(10), async {
            while tx.capacity() != 32 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        // Exactly 32 pending entries plus the bounded 32-request ingress.
        for _ in 0..32 {
            let (request, response, done) = waiting("fixture");
            assert!(tx.try_send(request).is_ok());
            waiting_calls.push((response, done));
        }
        let (request, _, _) = waiting("fixture");
        assert!(matches!(
            tx.try_send(request),
            Err(mpsc::error::TrySendError::Full(_))
        ));
        drop(waiting_calls);
        drop(active);
        drop(accounts);
        drop(tx);
        tokio::time::timeout(Duration::from_secs(10), actor)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn queued_writes_keep_order_and_an_abandoned_waiter_does_not_occupy_account() {
        let accounts = Accounts::default();
        let initial = accounts.write("fixture").await;
        let (first, accepted_first, completed_first) = waiting("fixture");
        accounts.0.send(first).await.unwrap();
        let (cancelled, accepted_cancelled, completed_cancelled) = waiting("fixture");
        accounts.0.send(cancelled).await.unwrap();
        let (last, mut accepted_last, completed_last) = waiting("fixture");
        accounts.0.send(last).await.unwrap();
        drop(accepted_cancelled);
        drop(completed_cancelled);
        drop(initial);
        assert!(accepted_first.await.unwrap());
        assert!(matches!(
            accepted_last.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(completed_first);
        assert!(accepted_last.await.unwrap());
        drop(completed_last);
        let _next = accounts.write("fixture").await;
    }
}
