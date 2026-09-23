use super::*;
use crate::store::{ReadyWork, Work, WorkPage};
use futures::{FutureExt, future::BoxFuture, stream::FuturesUnordered};

impl Engine {
    pub(super) async fn run_action_owner(
        self,
        mut input: mpsc::Receiver<Command>,
        mut output: Output,
    ) {
        let mut running: FuturesUnordered<BoxFuture<'_, (usize, Work, bool)>> =
            FuturesUnordered::new();
        let mut leases = std::collections::HashMap::<String, Arc<crate::store::BulkLease>>::new();
        let mut active = Vec::<(usize, ReadyWork)>::new();
        let mut after: [String; 6] = Default::default();
        let mut sweep = Sweep::default();
        let mut blocked = [false; 6];
        let mut storage_backoff = [None::<tokio::time::Instant>; 6];
        let mut next_domain = 0;
        let mut progressed = false;
        let mut scanning = false;
        let mut input_open = true;
        let mut stopped = false;
        loop {
            let mut scan_pending = false;
            if !self.bulk_control.stopping.get() {
                stopped = false;
                for domain in 0..6 {
                    if let Some(until) = storage_backoff[domain] {
                        if until <= tokio::time::Instant::now() {
                            storage_backoff[domain] = None;
                            blocked[domain] = false;
                            after[domain].clear();
                            sweep.restart(domain);
                        } else {
                            blocked[domain] = true;
                        }
                    }
                }
                if !blocked.iter().all(|blocked| *blocked) {
                    match self.store.expire_action_backoff().await {
                        Ok(expired) => progressed |= expired,
                        Err(error) => {
                            blocked = [true; 6];
                            storage_backoff =
                                [Some(tokio::time::Instant::now() + Duration::from_secs(2)); 6];
                            let _ = output.send(Event::Error(format!("Could not update pending action retries. Refresh their history to retry. {error:#}"))).await;
                        }
                    }
                }
                if progressed && !scanning {
                    after = Default::default();
                    sweep = Sweep::default();
                    progressed = false;
                }
                while running.len() < dispatch::NETWORK_CONCURRENCY + 1 {
                    let cache_only = running.len() >= dispatch::NETWORK_CONCURRENCY;
                    let mut found = None;
                    for repair_only in [true, false]
                        .into_iter()
                        .filter(|repair| *repair || !cache_only)
                    {
                        for offset in 0..6 {
                            let domain = (next_domain + offset) % 6;
                            if blocked[domain] || sweep.exhausted(domain, repair_only) {
                                continue;
                            }
                            let occupied = active
                                .iter()
                                .flat_map(|(_, work)| work.accounts.iter().cloned())
                                .collect();
                            let ids = active
                                .iter()
                                .filter(|(d, _)| *d == domain)
                                .map(|(_, work)| work.work.key())
                                .collect();
                            #[cfg(test)]
                            self.bulk_control
                                .scans
                                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            match self
                                .store
                                .scan_action_work(
                                    domain,
                                    after[domain].clone(),
                                    occupied,
                                    ids,
                                    repair_only,
                                )
                                .await
                            {
                                Ok(WorkPage::Ready(work)) => {
                                    found = Some((domain, work));
                                    break;
                                }
                                Ok(WorkPage::More(cursor)) => {
                                    after[domain] = cursor;
                                    scan_pending = true;
                                }
                                Ok(WorkPage::Done) => sweep.finish(domain, repair_only),
                                Err(error) => {
                                    blocked[domain] = true;
                                    let _ = output.send(Event::Error(format!("Could not read pending actions. Refresh their history to retry. {error:#}"))).await;
                                }
                            }
                        }
                        if found.is_some() || scan_pending {
                            break;
                        }
                    }
                    let Some((domain, ready)) = found else {
                        break;
                    };
                    after[domain].clone_from(&ready.cursor);
                    next_domain = (domain + 1) % 6;
                    let activity = self.bulk_control.active.enter();
                    let lease = if let Work::Mail { id, .. } = &ready.work {
                        match leases.get(id).cloned() {
                            Some(lease) => Some(lease),
                            None => {
                                let acquired = async {
                                    let lease = Arc::new(self.store.bulk_lease(id.clone()).await?);
                                    self.store.resume_bulk(&lease).await?;
                                    Ok::<_, anyhow::Error>(lease)
                                }
                                .await;
                                match acquired {
                                    Ok(lease) => {
                                        leases.insert(id.clone(), lease.clone());
                                        Some(lease)
                                    }
                                    Err(error) => {
                                        // A competing owner must not be mistaken for an abandoned step.
                                        if self
                                            .store
                                            .action_work_progress(domain, id.clone(), false)
                                            .await
                                            .is_err()
                                        {
                                            blocked[domain] = true;
                                            storage_backoff[domain] = Some(
                                                tokio::time::Instant::now()
                                                    + Duration::from_secs(2),
                                            );
                                        }
                                        let _ = output
                                            .send(Event::BulkFinished(
                                                id.clone(),
                                                Err(format!("{error:#}")),
                                            ))
                                            .await;
                                        continue;
                                    }
                                }
                            }
                        }
                    } else {
                        None
                    };
                    active.push((domain, ready.clone()));
                    let engine = &self;
                    let output = output.clone();
                    running.push(
                        async move {
                            let _activity = activity;
                            let work = ready.work.clone();
                            let progressed =
                                engine.execute_ready_work(ready.work, lease, output).await;
                            (domain, work, progressed)
                        }
                        .boxed(),
                    );
                }
            }
            if running.is_empty()
                && !stopped
                && let Some(event) = self.bulk_control.stopped_event()
            {
                stopped = true;
                let _ = output.send(event).await;
            }
            if running.is_empty()
                && !input_open
                && (self.bulk_control.stopping.get() || !scan_pending && !progressed)
            {
                break;
            }
            let mut retry_at = if self.bulk_control.stopping.get()
                || blocked[2]
                || running.len() >= dispatch::NETWORK_CONCURRENCY
            {
                None
            } else {
                let occupied = active
                    .iter()
                    .flat_map(|(_, work)| work.accounts.iter().cloned())
                    .collect();
                self.store
                    .next_unreserved_calendar_retry(occupied)
                    .await
                    .ok()
                    .flatten()
            };
            if !self.bulk_control.stopping.get()
                && !blocked.iter().all(|blocked| *blocked)
                && running.len() < dispatch::NETWORK_CONCURRENCY
            {
                let blocked_domains = blocked
                    .iter()
                    .enumerate()
                    .filter_map(|(domain, blocked)| blocked.then_some(domain))
                    .collect();
                match self.store.next_action_backoff(blocked_domains).await {
                    Ok(Some(at)) => retry_at = Some(retry_at.map_or(at, |current| current.min(at))),
                    Ok(None) => {}
                    Err(error) => {
                        blocked = [true; 6];
                        storage_backoff =
                            [Some(tokio::time::Instant::now() + Duration::from_secs(2)); 6];
                        retry_at = None;
                        let _ = output.send(Event::Error(format!("Could not read pending action retries. Refresh their history to retry. {error:#}"))).await;
                    }
                }
            }
            let storage_deadline = if self.bulk_control.stopping.get() {
                None
            } else {
                storage_backoff.into_iter().flatten().min()
            };
            if scan_pending {
                scanning = true;
            } else if running.len() < dispatch::NETWORK_CONCURRENCY {
                scanning = false;
            }
            tokio::select! {
                _ = tokio::task::yield_now(), if (scan_pending || progressed && !scanning) && !self.bulk_control.stopping.get() => {}
                completion = running.next(), if !running.is_empty() => {
                    if let Some((domain, finished, changed)) = completion {
                        let id = finished.id().to_owned();
                        active.retain(|(d, work)| *d != domain || work.work.key() != finished.key());
                        if domain == 0 && !active.iter().any(|(d, work)| *d == 0 && work.work.id() == id) {
                            leases.remove(&id);
                        }
                        progressed |= changed;
                        if !self.bulk_control.stopping.get()
                            && let Err(error) = self.store.action_work_progress(domain,id,changed).await
                        {
                            blocked[domain] = true;
                            storage_backoff[domain] = Some(tokio::time::Instant::now()+Duration::from_secs(2));
                            let _ = output.send(Event::Error(format!("Could not retain an action retry delay. Refresh its history to retry. {error:#}"))).await;
                        }
                    }
                }
                command = input.recv(), if input_open => {
                    // Take every queued wake at once; they ask for the same rescan.
                    let mut next = command;
                    loop {
                        match next {
                            Some(Command::BulkRun(id)) => {
                                blocked = [false; 6];
                                // A job with a running item is rescanned when that item finishes.
                                progressed |= id.is_empty() || !leases.contains_key(&id);
                            }
                            Some(_) => { let _ = output.send(Event::Error("Unexpected action-owner command".into())).await; }
                            None => { input_open = false; break; }
                        }
                        next = match input.try_recv() {
                            Ok(command) => Some(command),
                            Err(mpsc::error::TryRecvError::Empty) => break,
                            Err(mpsc::error::TryRecvError::Disconnected) => None,
                        };
                    }
                }
                _ = async {
                    let provider_deadline = retry_at.map(|at|tokio::time::Instant::now()+Duration::from_secs(at.saturating_sub(chrono::Utc::now().timestamp()).clamp(0,300) as u64));
                    let deadline = provider_deadline.into_iter().chain(storage_deadline).min();
                    match deadline {
                        Some(at) => tokio::time::sleep_until(at).await,
                        None => std::future::pending::<()>().await,
                    }
                } => { progressed = true; }
            }
        }
    }

    async fn execute_ready_work(
        &self,
        work: Work,
        lease: Option<Arc<crate::store::BulkLease>>,
        mut output: Output,
    ) -> bool {
        if self.bulk_control.stopping.get() {
            return false;
        }
        match work {
            Work::Mail { id, position } => {
                self.execute_bulk_work(id, Some(position), 1, lease, &mut output)
                    .await
            }
            Work::Calendar(id) => self.execute_calendar_work(id, &mut output).await,
            Work::Creation(id) => self.execute_creation_work(id, output).await,
            Work::Removal(id) => self.execute_removal_work(id, output).await,
            Work::Folder(id) => {
                let before = self
                    .store
                    .folder_job(id.clone())
                    .await
                    .ok()
                    .map(|j| (j.revision, j.closed));
                self.execute_folder_job(id.clone(), output).await;
                self.store
                    .folder_job(id)
                    .await
                    .is_ok_and(|j| Some((j.revision, j.closed)) != before)
            }
            Work::Outgoing(id) => {
                let before = self
                    .store
                    .outgoing_info(id.clone())
                    .await
                    .ok()
                    .map(|j| j.delivery);
                if let Err(error) = self.submit_outgoing(&id, &mut output).await {
                    let _ = output
                        .send(Event::Error(format!(
                            "An outgoing message needs attention in Outbox. {error:#}"
                        )))
                        .await;
                }
                self.store
                    .outgoing_info(id)
                    .await
                    .is_ok_and(|j| Some(j.delivery) != before)
            }
        }
    }
}

/// Domains whose scan reached its end during the current sweep. Occupancy only
/// grows until a completion or wake restarts the sweep, so a finished domain
/// cannot gain eligible work before then and need not be read again.
#[derive(Default)]
struct Sweep {
    finished: [[bool; 2]; 6],
}

impl Sweep {
    fn exhausted(&self, domain: usize, repair_only: bool) -> bool {
        self.finished[domain][usize::from(repair_only)]
    }
    fn finish(&mut self, domain: usize, repair_only: bool) {
        self.finished[domain][usize::from(repair_only)] = true;
    }
    fn restart(&mut self, domain: usize) {
        self.finished[domain] = [false; 2];
    }
}

#[cfg(test)]
mod tests {
    use super::Sweep;

    #[test]
    fn a_finished_domain_is_skipped_per_pass_until_restarted() {
        let mut sweep = Sweep::default();
        sweep.finish(3, false);
        assert!(sweep.exhausted(3, false));
        assert!(
            !sweep.exhausted(3, true),
            "repair and ordinary passes end separately"
        );
        assert!(!sweep.exhausted(2, false));
        sweep.finish(3, true);
        sweep.restart(3);
        assert!(!sweep.exhausted(3, false) && !sweep.exhausted(3, true));
    }
}
