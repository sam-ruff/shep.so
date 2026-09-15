//! One channel-driven coordinator owns scheduling state. Account operations
//! hold completion senders, never a shared mutex or mutable scheduling map.
//!
//! A read-only sync and a mail write for the same account run side by side:
//! each has its own connection and the write ledger reconciles their cache
//! effects afterwards. Only exclusive lifecycle work (account settings,
//! removal, restore, folder structure) interrupts a running sync and waits.
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

enum Kind {
    /// A mail action on its own connection; runs beside a sync.
    Write,
    /// Needs the account to itself; interrupts a read-only sync and waits.
    Exclusive,
    /// A read-only download; never starts while another one runs.
    Sync(oneshot::Sender<()>),
}
impl Kind {
    fn is_sync(&self) -> bool {
        matches!(self, Kind::Sync(_))
    }
    fn is_exclusive(&self) -> bool {
        matches!(self, Kind::Exclusive)
    }
}

struct Request {
    account: String,
    kind: Kind,
    grant: oneshot::Sender<bool>,
    finished: oneshot::Receiver<()>,
}
struct Pending {
    ticket: u64,
    account: String,
    kind: Kind,
    grant: oneshot::Sender<bool>,
}
struct ActiveSync {
    ticket: u64,
    interrupt: Option<oneshot::Sender<()>>,
}
struct ActiveMutation {
    ticket: u64,
    exclusive: bool,
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
    async fn acquire(&self, id: &str, kind: Kind) -> Option<Access> {
        let (finished, receipt) = oneshot::channel();
        let (grant, granted) = oneshot::channel();
        self.0
            .send(Request {
                account: id.into(),
                kind,
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

    /// A mail action. Granted as soon as no other write holds the account,
    /// even while a read-only sync is downloading.
    pub(super) async fn write(&self, id: &str) -> Access {
        self.acquire(id, Kind::Write)
            .await
            .expect("writes are never skipped")
    }

    /// Lifecycle work that must not overlap a download. A running read-only
    /// sync is asked to stop and this waits until its cache commits settle.
    pub(super) async fn exclusive(&self, id: &str) -> Access {
        self.acquire(id, Kind::Exclusive)
            .await
            .expect("exclusive requests are never skipped")
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
                access = accounts.acquire(&id, Kind::Sync(interrupt)) => access,
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
    let mut syncs: HashMap<String, ActiveSync> = HashMap::new();
    let mut mutations: HashMap<String, ActiveMutation> = HashMap::new();
    let mut pending: VecDeque<Pending> = VecDeque::new();
    let mut finished: FuturesUnordered<BoxFuture<'static, (u64, String)>> = FuturesUnordered::new();
    let mut next = 0u64;
    let mut closed = false;
    loop {
        // Writes and exclusive requests keep FIFO order per account. A sync is
        // granted beside a write, refused while another sync runs and yields
        // to exclusive work instead of starting a download it would interrupt.
        let mut blocked: Vec<String> = Vec::new();
        let mut i = 0;
        while i < pending.len() {
            let request = &pending[i];
            let account = &request.account;
            let exclusive_waiting = mutations
                .get(account)
                .is_some_and(|active| active.exclusive)
                || pending
                    .iter()
                    .any(|other| other.account == *account && other.kind.is_exclusive());
            let skip = request.grant.is_closed()
                || request.kind.is_sync() && (syncs.contains_key(account) || exclusive_waiting);
            if skip {
                let request = pending.remove(i).unwrap();
                let _ = request.grant.send(false);
                continue;
            }
            let ready = match &request.kind {
                Kind::Sync(_) => true,
                Kind::Write => !mutations.contains_key(account) && !blocked.contains(account),
                Kind::Exclusive => {
                    !mutations.contains_key(account)
                        && !blocked.contains(account)
                        && !syncs.contains_key(account)
                }
            };
            if !ready {
                if !request.kind.is_sync() {
                    blocked.push(account.clone());
                }
                i += 1;
                continue;
            }
            let request = pending.remove(i).unwrap();
            if request.grant.send(true).is_err() {
                continue;
            }
            match request.kind {
                Kind::Sync(interrupt) => {
                    syncs.insert(
                        request.account,
                        ActiveSync {
                            ticket: request.ticket,
                            interrupt: Some(interrupt),
                        },
                    );
                }
                kind => {
                    mutations.insert(
                        request.account,
                        ActiveMutation {
                            ticket: request.ticket,
                            exclusive: kind.is_exclusive(),
                        },
                    );
                }
            }
        }
        if closed && finished.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            Some((ticket, account)) = finished.next(), if !finished.is_empty() => {
                if syncs.get(&account).is_some_and(|entry| entry.ticket == ticket) {
                    syncs.remove(&account);
                }
                if mutations.get(&account).is_some_and(|entry| entry.ticket == ticket) {
                    mutations.remove(&account);
                }
                pending.retain(|request| request.ticket != ticket);
            }
            request = requests.recv(), if !closed && pending.len() < 32 => {
                let Some(request) = request else { closed = true; continue };
                if request.grant.is_closed() { continue; }
                next = next.checked_add(1).expect("account ticket space exhausted");
                let ticket = next;
                let account = request.account.clone();
                finished.push(async move {
                    let _ = request.finished.await;
                    (ticket, account)
                }.boxed());
                if request.kind.is_exclusive()
                    && let Some(sync) = syncs.get_mut(&request.account)
                    && let Some(interrupt) = sync.interrupt.take()
                {
                    let _ = interrupt.send(());
                }
                pending.push_back(Pending { ticket, account: request.account, kind: request.kind, grant: request.grant });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::poll;
    use std::task::Poll;

    /// A sync whose provider stays held until `release` fires, reporting
    /// whether the coordinator asked it to stop. Returns once it holds the
    /// account.
    async fn held_sync(
        accounts: &Accounts,
        release: oneshot::Receiver<()>,
    ) -> (
        tokio::task::JoinHandle<anyhow::Result<()>>,
        oneshot::Receiver<bool>,
    ) {
        let (report, interrupted) = oneshot::channel();
        let (started, start) = oneshot::channel();
        let accounts = accounts.clone();
        let task = tokio::spawn(async move {
            accounts
                .sync("fixture", |mut stop| async move {
                    started.send(()).unwrap();
                    let stopped = tokio::select! {
                        _ = stop.cancelled() => true,
                        _ = release => false,
                    };
                    report.send(stopped).unwrap();
                    Ok(())
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(10), start)
            .await
            .unwrap()
            .unwrap();
        (task, interrupted)
    }

    #[tokio::test]
    async fn write_is_granted_while_a_sync_holds_the_provider() {
        let accounts = Accounts::default();
        let (release, held) = oneshot::channel();
        let (sync, interrupted) = held_sync(&accounts, held).await;
        let write = tokio::time::timeout(Duration::from_secs(10), accounts.write("fixture"))
            .await
            .expect("a write must not wait for the download");
        drop(write);
        release.send(()).unwrap();
        assert!(
            !interrupted.await.unwrap(),
            "a write must not interrupt sync"
        );
        sync.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn writes_keep_fifo_order_beside_a_live_sync() {
        let accounts = Accounts::default();
        let (release, held) = oneshot::channel();
        let (sync, _interrupted) = held_sync(&accounts, held).await;
        let first = accounts.write("fixture").await;
        let mut second = Box::pin(accounts.write("fixture"));
        assert!(poll!(&mut second).is_pending());
        tokio::task::yield_now().await;
        let mut third = Box::pin(accounts.write("fixture"));
        assert!(poll!(&mut third).is_pending());
        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(10), second)
            .await
            .unwrap();
        assert!(poll!(&mut third).is_pending());
        drop(second);
        tokio::time::timeout(Duration::from_secs(10), third)
            .await
            .unwrap();
        release.send(()).unwrap();
        sync.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn a_second_sync_is_refused_while_one_runs_even_beside_a_write() {
        let accounts = Accounts::default();
        let (release, held) = oneshot::channel();
        let (sync, _interrupted) = held_sync(&accounts, held).await;
        let _write = accounts.write("fixture").await;
        accounts
            .sync("fixture", |_| async {
                panic!("a second download must not start")
            })
            .await
            .unwrap();
        release.send(()).unwrap();
        sync.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn sync_starts_beside_an_active_write() {
        let accounts = Accounts::default();
        let _write = accounts.write("fixture").await;
        let (started, start) = oneshot::channel();
        accounts
            .sync("fixture", |_| async move {
                started.send(()).unwrap();
                Ok(())
            })
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(10), start)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn exclusive_interrupts_the_sync_and_waits_for_it_to_settle() {
        let accounts = Accounts::default();
        let (_release, held) = oneshot::channel();
        let (sync, interrupted) = held_sync(&accounts, held).await;
        let exclusive =
            tokio::time::timeout(Duration::from_secs(10), accounts.exclusive("fixture"))
                .await
                .unwrap();
        assert!(
            interrupted.await.unwrap(),
            "exclusive work must stop the download"
        );
        sync.await.unwrap().unwrap();
        drop(exclusive);
    }

    #[tokio::test]
    async fn queued_sync_cannot_get_ahead_of_a_waiting_exclusive_request() {
        let accounts = Accounts::default();
        let first = accounts.exclusive("fixture").await;
        let mut sync = Box::pin(accounts.sync("fixture", |_| async {
            panic!("queued sync must yield before contacting the provider")
        }));
        assert!(poll!(&mut sync).is_pending());
        // Run the owned task until it has joined the lock queue.
        tokio::task::yield_now().await;
        let mut exclusive = Box::pin(accounts.exclusive("fixture"));
        assert!(poll!(&mut exclusive).is_pending());
        drop(first);
        sync.await.unwrap();
        let _next = exclusive.await;
    }

    #[tokio::test]
    async fn cancelling_a_queued_writer_does_not_disable_future_sync() {
        let accounts = Accounts::default();
        let first = accounts.exclusive("fixture").await;
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
                kind: Kind::Write,
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
