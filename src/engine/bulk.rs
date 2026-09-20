//! Durable actions share a bounded owner with independent account progress.
//! UI commands, reads, sync, and Undo intent use their independent queues.
use super::*;
use crate::bulk::{Action, Item, Job, Receipt};
mod scheduler;

#[derive(Default)]
pub(super) struct Control {
    pub stopping: crate::lifecycle::Signal,
    pub active: crate::lifecycle::Activity,
    pub stop_generation: std::sync::atomic::AtomicU64,
}

impl Control {
    pub(super) fn stopped_event(&self) -> Option<Event> {
        let generation = self
            .stop_generation
            .load(std::sync::atomic::Ordering::Acquire);
        (self.stopping.get() && !self.active.get()).then_some(Event::BulkStopped(generation))
    }
}

impl Engine {
    pub(super) async fn run_bulk_queue(self, input: mpsc::Receiver<Command>, output: Output) {
        self.run_action_owner(input, output).await;
    }

    #[cfg(test)]
    async fn execute_bulk_job(&self, id: String, mut output: Output) {
        self.execute_bulk_work(id, None, usize::MAX, &mut output)
            .await;
    }

    async fn execute_bulk_work(
        &self,
        id: String,
        position: Option<u64>,
        limit: usize,
        output: &mut Output,
    ) -> bool {
        // Publish activity before checking close intent. A close sees either
        // this guard or the worker observes stopping before touching storage.
        let activity = self.bulk_control.active.enter();
        if self.bulk_control.stopping.get() {
            drop(activity);
            // Stop may have observed active=true and delegated its acknowledgment
            // to us. Even when no item started, the closing window must hear it.
            if let Some(event) = self.bulk_control.stopped_event() {
                let _ = output.send(event).await;
            }
            return false;
        }
        let before = self.store.bulk_job(id.clone()).await.ok().map(|j| {
            (
                j.remaining,
                j.completed,
                j.restored,
                j.failed,
                j.uncertain,
                j.cancelled,
            )
        });
        let result = self
            .perform_bulk_job(&id, output, position, limit)
            .await
            .map(Arc::new)
            .map_err(|e| format!("{e:#}"));
        let failed = result.is_err();
        let progressed = result.as_ref().is_ok_and(|j| {
            Some((
                j.remaining,
                j.completed,
                j.restored,
                j.failed,
                j.uncertain,
                j.cancelled,
            )) != before
        });
        if let Ok(job) = &result
            && job.remaining > 0
            && limit != usize::MAX
        {
            let _ = output.send(Event::BulkUpdate(job.clone())).await;
        } else {
            let _ = output.send(Event::BulkFinished(id, result)).await;
        }
        drop(activity);
        if failed {
            // A visible error cancels pending close and leaves recovery usable.
            self.bulk_control.stopping.set(false);
        } else if let Some(event) = self.bulk_control.stopped_event() {
            let _ = output.send(event).await;
        }
        progressed
    }
    async fn perform_bulk_job(
        &self,
        id: &str,
        output: &mut Output,
        position: Option<u64>,
        limit: usize,
    ) -> anyhow::Result<Job> {
        let lease = self.store.bulk_lease(id.to_owned()).await?;
        let mut job = self.store.resume_bulk(&lease).await?;
        output
            .send(Event::BulkUpdate(Arc::new(job.clone())))
            .await?;
        let mut last_progress = None::<std::time::Instant>;
        let mut completed = 0;
        loop {
            if self.bulk_control.stopping.get() || job.remaining == 0 || completed >= limit {
                break;
            }
            if let Some(item) = self.store.pending_bulk_flag_repair(&lease).await? {
                job = self
                    .store
                    .finish_bulk_item(item, Ok(Receipt::Unchanged))
                    .await?;
                completed += 1;
                output
                    .send(Event::BulkUpdate(Arc::new(job.clone())))
                    .await?;
                continue;
            }
            // Waiting for provider capacity has not changed any server state.
            // A close may abandon this wait without claiming a journal step.
            let _slot = tokio::select! {
                biased;
                _ = self.bulk_control.stopping.requested() => break,
                slot = self.provider_slots.acquire() => slot,
            };
            if self.bulk_control.stopping.get() {
                break;
            }
            let Some(item) = self
                .store
                .claim_bulk_item_at(id.to_owned(), position)
                .await?
            else {
                break;
            };
            job = self.store.bulk_job(id.to_owned()).await?;
            if last_progress.is_none_or(|t| t.elapsed() >= Duration::from_millis(100)) {
                output
                    .send(Event::BulkUpdate(Arc::new(job.clone())))
                    .await?;
                last_progress = Some(std::time::Instant::now());
            }
            // Errors before touching the provider are definite rejections.
            let preflight=async {
                let original=item.original.as_ref().context("This message is no longer available")?;
                if !item.undo {
                    let current=self.store.mail_metadata(original.id.clone()).await?;
                    anyhow::ensure!(current.account_id==original.account_id && current.folder==original.folder,
                        "This message changed folders before the operation began. Refresh and select it again.");
                }
                Ok::<_,anyhow::Error>(())
            }.await;
            let result = match preflight {
                Err(error) => Err((format!("{error:#}"), false)),
                Ok(()) => self
                    .perform_bulk_item(&job.action, &item, output.clone())
                    .await
                    .or_else(|error| {
                        if error.is::<crate::bulk::Superseded>() {
                            Ok(Receipt::Superseded)
                        } else {
                            Err(error)
                        }
                    })
                    .map_err(|error| item_failure(error, self.demo)),
            };
            // Observe receipt persistence to completion. Never discard an
            // accepted server result under a generic provider-job timeout.
            let identity = match &result {
                Ok(Receipt::Move(receipt)) => Some((
                    item.job.clone(),
                    item.id.clone(),
                    receipt.current.as_ref().map(|m| m.id.clone()),
                )),
                _ => None,
            };
            let failed = result.is_err();
            job = self.store.finish_bulk_item(item, result).await?;
            completed += 1;
            if let Some((job, source, current)) = identity {
                output
                    .send(Event::BulkIdentity(job, source, current))
                    .await?;
            }
            if failed
                || job.remaining == 0
                || last_progress.is_none_or(|t| t.elapsed() >= Duration::from_millis(100))
            {
                output
                    .send(Event::BulkUpdate(Arc::new(job.clone())))
                    .await?;
                last_progress = Some(std::time::Instant::now());
            }
        }
        self.store.bulk_job(id.to_owned()).await
    }
    async fn perform_bulk_item(
        &self,
        action: &Action,
        item: &Item,
        output: Output,
    ) -> anyhow::Result<Receipt> {
        let original = item
            .original
            .as_ref()
            .context("The original message is unavailable")?;
        if item.undo {
            return match item
                .receipt
                .as_ref()
                .context("The completed action has no Undo receipt")?
            {
                Receipt::Move(receipt) => self
                    .undo_move(original.clone(), receipt, output, Some(item))
                    .await
                    .map(|(_, receipt)| Receipt::Move(Box::new(receipt))),
                Receipt::Flags { before, after } => {
                    self.change_bulk_flags(original, *before, Some(*after), item)
                        .await
                }
                Receipt::Unchanged | Receipt::Superseded => Ok(Receipt::Unchanged),
            };
        }
        match action {
            Action::Move { account, folder } => {
                if let Some(account) = account.as_ref().filter(|a| *a != &original.account_id) {
                    self.transfer_message(
                        original,
                        account.clone(),
                        folder.clone(),
                        output,
                        Some(item),
                    )
                    .await
                    .map(|(_, r)| Receipt::Move(Box::new(r)))
                } else if original.folder
                    == self
                        .resolve_destination(&original.account_id, folder)
                        .await?
                {
                    Ok(Receipt::Unchanged)
                } else {
                    self.change_folder(original, folder, output, Some(item))
                        .await
                        .map(|(_, r)| Receipt::Move(Box::new(r)))
                }
            }
            Action::Flags(changes) => self.change_bulk_flags(original, *changes, None, item).await,
        }
    }
}

fn item_failure(error: anyhow::Error, demo: bool) -> (String, bool) {
    let uncertain = !demo && !error.is::<crate::mail_actions::FlagsRejected>();
    (format!("{error:#}"), uncertain)
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_flag_refusal_is_rejected_but_unknown_and_cache_errors_are_uncertain() {
        let rejection =
            anyhow::Error::new(crate::mail_actions::FlagsRejected("STORE refused".into()))
                .context("Saving selected flags");
        assert!(!super::item_failure(rejection, false).1);
        for message in [
            "STORE connection closed",
            "Receipt cache write failed",
            "NO refused",
        ] {
            assert!(super::item_failure(anyhow::anyhow!(message), false).1);
        }
        assert!(
            super::item_failure(
                crate::mail_actions::MoveRefused("MOVE refused".into()).into(),
                false
            )
            .1,
            "Move fallback remains owned by the move runner"
        );
    }

    #[test]
    fn stopped_acknowledgement_rechecks_activity_after_a_new_stop_generation() {
        use std::sync::atomic::Ordering;
        let control = super::Control::default();
        control.stop_generation.store(1, Ordering::Release);
        control.stopping.set(true);
        assert!(
            !control.active.get(),
            "The first stop observed an idle owner"
        );
        let first = control.stopped_event().expect("first drained stop");
        control.stopping.set(false);
        assert!(control.stopped_event().is_none());
        let running = control.active.enter();
        control.stop_generation.store(2, Ordering::Release);
        control.stopping.set(true);
        assert!(
            control.stopped_event().is_none(),
            "The earlier idle observation cannot acknowledge new active work"
        );
        assert!(
            matches!(first, super::Event::BulkStopped(1)),
            "A captured acknowledgment keeps its original generation"
        );
        drop(running);
        assert!(matches!(
            control.stopped_event(),
            Some(super::Event::BulkStopped(2))
        ));
    }
    use super::*;
    use crate::store::MailSelectionId;

    async fn fixture(count: usize) -> Engine {
        let engine = super::super::calendar_tests::engine();
        let mail = (0..count).map(|i| parse_mail("fixture", &i.to_string(), "INBOX",
            format!("From: fixture@example.test\r\nSubject: Group {i:03}\r\n\r\nOriginal body {i}").into_bytes(),true,false).unwrap()).collect();
        engine.store.upsert(mail).await.unwrap();
        engine
    }
    async fn start(engine: &Engine, id: &str, query: MailQuery, action: Action) {
        let source = MailSelectionId::default();
        engine
            .store
            .capture_selection(source, 0, query, true, vec![])
            .await
            .unwrap();
        let review = engine.store.freeze_selection(source, 0).await.unwrap();
        engine.store.release_selection(source).await.unwrap();
        engine
            .store
            .start_bulk(id.into(), review.id, action)
            .await
            .unwrap();
    }
    async fn execute(engine: &Engine, id: &str) -> Job {
        let (output, mut input) = futures::channel::mpsc::channel(2);
        let collect = async {
            let mut previous = 0;
            while let Some(event) = input.next().await {
                match event {
                    Event::BulkUpdate(job) => {
                        assert!(job.revision >= previous);
                        previous = job.revision;
                    }
                    Event::BulkFinished(finished, result) => {
                        assert_eq!(finished, id);
                        return (*result.unwrap()).clone();
                    }
                    Event::Error(error) => panic!("{error}"),
                    _ => {}
                }
            }
            panic!("Missing durable completion")
        };
        let (_, job) = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::join!(engine.execute_bulk_job(id.into(), output), collect)
        })
        .await
        .unwrap();
        job
    }
    fn movement(folder: &str) -> Action {
        Action::Move {
            account: None,
            folder: folder.into(),
        }
    }

    #[tokio::test]
    async fn unchanged_calendar_failure_backs_off_without_blocking_other_work_or_spinning_on_scratch_failure()
     {
        for scratch_failure in [false, true] {
            let engine = fixture(1).await;
            engine
                .store
                .put(
                    "calendars",
                    vec![CalendarSource {
                        id: "calendar".into(),
                        name: "Calendar".into(),
                        kind: CalendarKind::CalDav,
                        url: "https://calendar.example/".into(),
                        username: "fixture".into(),
                        access: Default::default(),
                    }],
                )
                .await
                .unwrap();
            let event = super::super::calendar_tests::event("calendar");
            engine
                .store
                .admit_calendar_action("blocked".into(), event.clone(), false)
                .await
                .unwrap();
            let claimed = engine
                .store
                .claim_calendar_action("blocked".into())
                .await
                .unwrap();
            engine
                .store
                .wait_calendar_action(
                    "blocked".into(),
                    claimed.revision,
                    crate::providers::calendar::WaitReason::Offline,
                    "Offline".into(),
                )
                .await
                .unwrap();
            let mut other = event;
            other.id = "different-event".into();
            engine
                .store
                .admit_calendar_action("healthy-calendar".into(), other, false)
                .await
                .unwrap();
            engine.store.run(move |c| {
                c.execute("UPDATE calendar_actions SET data=json_set(data,'$.retry_at',0) WHERE id='blocked'",[])?;
                c.execute_batch("CREATE TEMP TRIGGER reject_calendar_write BEFORE UPDATE ON calendar_actions WHEN old.id='blocked' BEGIN SELECT RAISE(ABORT,'Fixture calendar storage failure'); END;")?;
                if scratch_failure {
                    c.execute_batch("CREATE TEMP TRIGGER reject_action_backoff BEFORE INSERT ON scratch.action_backoff BEGIN SELECT RAISE(ABORT,'Fixture scratch storage failure'); END;")?;
                }
                Ok(())
            }).await.unwrap();
            start(
                &engine,
                "healthy-mail",
                MailQuery::default(),
                Action::Flags(crate::mail_actions::Flags {
                    unread: Some(false),
                    starred: None,
                }),
            )
            .await;
            let (sender, input) = CommandSender::channel();
            let (output, mut events) = futures::channel::mpsc::channel(64);
            let owner = tokio::spawn(engine.clone().run_bulk_queue(input.bulk, output.clone()));
            let (mut failures, mut mail_done, mut calendar_done) = (0, false, scratch_failure);
            tokio::time::timeout(Duration::from_secs(5), async {
                while failures == 0 || !mail_done || !calendar_done {
                    match events.next().await.expect("owner event") {
                        Event::CalendarJob(id, Err(_)) if id == "blocked" => failures += 1,
                        Event::CalendarJob(id, Ok(job))
                            if id == "healthy-calendar" && job.status == "succeeded" =>
                        {
                            calendar_done = true
                        }
                        Event::BulkFinished(id, Ok(job)) if id == "healthy-mail" => {
                            assert_eq!(job.completed, 1);
                            mail_done = true;
                        }
                        _ => {}
                    }
                }
            })
            .await
            .expect("unrelated work completes beside failed calendar storage");
            sender.try_send(Command::BulkRun(String::new())).unwrap();
            let quiet = tokio::time::sleep(Duration::from_millis(150));
            tokio::pin!(quiet);
            loop {
                tokio::select! {
                    _=&mut quiet=>break,
                    event=events.next()=>{
                        if matches!(event,Some(Event::CalendarJob(id,Err(_))) if id=="blocked") { failures+=1; }
                    }
                }
            }
            assert_eq!(
                failures, 1,
                "unchanged work must not spin, including failed cooldown writes"
            );
            assert_eq!(
                engine
                    .store
                    .calendar_job("blocked".into())
                    .await
                    .unwrap()
                    .status,
                "waiting"
            );
            engine.execute(Command::BulkStop(1), output).await.unwrap();
            drop(sender);
            tokio::time::timeout(Duration::from_secs(2), owner)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn calendar_retry_crossing_candidate_read_keeps_its_wake_and_excludes_busy_sources() {
        let engine = fixture(0).await;
        engine
            .store
            .put(
                "calendars",
                vec![CalendarSource {
                    id: "calendar".into(),
                    name: "Calendar".into(),
                    kind: CalendarKind::CalDav,
                    url: "https://calendar.example/".into(),
                    username: "fixture".into(),
                    access: Default::default(),
                }],
            )
            .await
            .unwrap();
        let event = super::super::calendar_tests::event("calendar");
        engine
            .store
            .admit_calendar_action("waiting".into(), event, false)
            .await
            .unwrap();
        let claimed = engine
            .store
            .claim_calendar_action("waiting".into())
            .await
            .unwrap();
        engine
            .store
            .wait_calendar_action(
                "waiting".into(),
                claimed.revision,
                crate::providers::calendar::WaitReason::Offline,
                "Offline".into(),
            )
            .await
            .unwrap();
        assert!(
            engine
                .store
                .next_action_work(2, String::new(), Vec::new(), Vec::new(), false)
                .await
                .unwrap()
                .is_none()
        );
        engine.store.run(|c| {
            c.execute("UPDATE calendar_actions SET data=json_set(data,'$.retry_at',unixepoch()-1) WHERE id='waiting'",[])?;
            Ok(())
        }).await.unwrap();
        assert!(
            engine
                .store
                .next_unreserved_calendar_retry(Vec::new())
                .await
                .unwrap()
                .is_some_and(|at| at <= chrono::Utc::now().timestamp())
        );
        assert!(
            engine
                .store
                .next_unreserved_calendar_retry(vec!["calendar:calendar".into()])
                .await
                .unwrap()
                .is_none()
        );
        let ready = engine
            .store
            .next_action_work(2, String::new(), Vec::new(), Vec::new(), false)
            .await
            .unwrap()
            .expect("deadline remains runnable");
        assert_eq!(ready.work.id(), "waiting");
        engine.bulk_control.stopping.set(true);
        let (sender, input) = CommandSender::channel();
        drop(sender);
        let (output, mut events) = futures::channel::mpsc::channel(4);
        tokio::time::timeout(
            Duration::from_secs(2),
            engine.clone().run_bulk_queue(input.bulk, output),
        )
        .await
        .expect("stopped owner ignores due deadlines");
        assert!(matches!(events.next().await, Some(Event::BulkStopped(0))));
        assert!(events.next().await.is_none());
        let retained = engine.store.calendar_job("waiting".into()).await.unwrap();
        assert_eq!(retained.status, "waiting");
        assert_eq!(retained.attempts, 1);
    }

    #[tokio::test]
    async fn cache_repair_has_a_slot_when_every_provider_and_account_worker_is_waiting() {
        let engine = fixture(1).await;
        let flags = crate::mail_actions::Flags {
            unread: Some(false),
            starred: None,
        };
        start(
            &engine,
            "zz-repair",
            MailQuery::default(),
            Action::Flags(flags),
        )
        .await;
        let item = engine
            .store
            .claim_bulk_item("zz-repair".into())
            .await
            .unwrap()
            .unwrap();
        engine
            .store
            .acknowledge_bulk_flags(
                item,
                Receipt::Flags {
                    before: crate::mail_actions::Flags {
                        unread: Some(true),
                        starred: None,
                    },
                    after: flags,
                },
            )
            .await
            .unwrap();
        let mut occupied = Vec::new();
        for index in 0..dispatch::NETWORK_CONCURRENCY {
            occupied.push(engine.provider_slots.acquire().await);
            let account = if index == 0 {
                "fixture".into()
            } else {
                format!("waiting-{index}")
            };
            let mail = parse_mail(
                &account,
                "1",
                "INBOX",
                b"Subject: Wait\r\n\r\nBody".to_vec(),
                true,
                false,
            )
            .unwrap();
            let original = mail.summary.clone();
            engine.store.upsert(vec![mail]).await.unwrap();
            engine
                .store
                .start_individual_mail_action(format!("a{index}"), original, Action::Flags(flags))
                .await
                .unwrap();
        }
        let first = engine
            .store
            .next_action_work(0, String::new(), Vec::new(), Vec::new(), false)
            .await
            .unwrap()
            .expect("repair priority");
        assert_eq!(first.work.id(), "zz-repair");
        let following = engine
            .store
            .next_action_work(0, first.cursor, Vec::new(), vec!["zz-repair".into()], false)
            .await
            .unwrap()
            .expect("queued work follows repair cursor");
        assert_eq!(following.work.id(), "a0");
        let (sender, input) = CommandSender::channel();
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let owner = tokio::spawn(engine.clone().run_bulk_queue(input.bulk, output.clone()));
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = events.next().await {
                if let Event::BulkFinished(id, result) = event
                    && id == "zz-repair"
                {
                    assert_eq!(result.unwrap().completed, 1);
                    break;
                }
            }
        })
        .await
        .expect("cache receipt has independent local capacity");
        for index in 0..dispatch::NETWORK_CONCURRENCY {
            let job = engine.store.bulk_job(format!("a{index}")).await.unwrap();
            assert_eq!((job.remaining, job.running, job.uncertain), (1, 0, 0));
        }
        engine.execute(Command::BulkStop(1), output).await.unwrap();
        drop(sender);
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = events.next().await {
                if matches!(event, Event::BulkStopped(1)) {
                    break;
                }
            }
            owner.await.unwrap();
        })
        .await
        .expect("unclaimed waits close without provider permits");
        drop(occupied);
    }

    #[tokio::test]
    async fn acknowledged_flags_repair_without_waiting_for_provider_capacity() {
        let engine = fixture(1).await;
        let flags = crate::mail_actions::Flags {
            unread: Some(false),
            starred: None,
        };
        start(
            &engine,
            "cache-repair",
            MailQuery::default(),
            Action::Flags(flags),
        )
        .await;
        let item = engine
            .store
            .claim_bulk_item("cache-repair".into())
            .await
            .expect("claim")
            .expect("item");
        engine
            .store
            .acknowledge_bulk_flags(
                item,
                Receipt::Flags {
                    before: crate::mail_actions::Flags {
                        unread: Some(true),
                        starred: None,
                    },
                    after: flags,
                },
            )
            .await
            .expect("acknowledgement");
        let mut occupied = Vec::new();
        for _ in 0..8 {
            occupied.push(engine.provider_slots.acquire().await);
        }
        let job = execute(&engine, "cache-repair").await;
        assert_eq!((job.completed, job.remaining, job.uncertain), (1, 0, 0));
        assert_eq!(
            engine
                .store
                .query(MailQuery::default())
                .await
                .expect("page")
                .unread,
            0
        );
        drop(occupied);
    }

    #[tokio::test]
    async fn close_abandons_provider_capacity_wait_without_claiming_or_touching_mail() {
        let engine = fixture(2).await;
        start(
            &engine,
            "capacity-close",
            MailQuery::default(),
            movement("Archive"),
        )
        .await;
        let mut slots = Vec::new();
        for _ in 0..8 {
            slots.push(engine.provider_slots.acquire().await);
        }
        let (output, mut events) = futures::channel::mpsc::channel(8);
        let worker = tokio::spawn({
            let engine = engine.clone();
            let output = output.clone();
            async move {
                engine
                    .execute_bulk_job("capacity-close".into(), output)
                    .await
            }
        });
        let initial = tokio::time::timeout(Duration::from_secs(5), events.next())
            .await
            .unwrap();
        assert!(
            matches!(initial, Some(Event::BulkUpdate(ref job)) if job.remaining == 2 && job.running == 0)
        );
        engine.execute(Command::BulkStop(1), output).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), worker)
            .await
            .unwrap()
            .unwrap();
        let mut acknowledged = false;
        while let Some(event) = events.next().await {
            if matches!(event, Event::BulkStopped(1)) {
                acknowledged = true;
                break;
            }
        }
        assert!(acknowledged);
        let job = engine
            .store
            .bulk_job("capacity-close".into())
            .await
            .unwrap();
        assert_eq!(
            (job.remaining, job.running, job.completed, job.uncertain),
            (2, 0, 0, 0)
        );
        let items = engine
            .store
            .bulk_items("capacity-close".into(), None)
            .await
            .unwrap();
        assert!(
            items
                .iter()
                .all(|item| item.status == "queued" && item.receipt.is_none())
        );
        // Capacity remains held until after close was acknowledged.
        assert_eq!(slots.len(), 8);
        drop(slots);
        engine.bulk_control.stopping.set(false);
        let completed = execute(&engine, "capacity-close").await;
        assert_eq!((completed.completed, completed.uncertain), (2, 0));
    }

    #[tokio::test]
    async fn stop_at_worker_entry_acknowledges_close_without_claiming_mail() {
        let engine = fixture(2).await;
        start(
            &engine,
            "closing",
            MailQuery::default(),
            movement("Archive"),
        )
        .await;
        engine.bulk_control.stopping.set(true);
        let (output, mut input) = futures::channel::mpsc::channel(2);
        engine.execute_bulk_job("closing".into(), output).await;
        assert!(matches!(input.next().await, Some(Event::BulkStopped(0))));
        assert!(!engine.bulk_control.active.get());
        let job = engine.store.bulk_job("closing".into()).await.unwrap();
        assert_eq!((job.remaining, job.running, job.completed), (2, 0, 0));
        assert_eq!(
            engine
                .store
                .query(MailQuery {
                    folder: "Archive".into(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .total,
            2,
            "Pending projection survives while queued provider work is unclaimed"
        );
        assert!(
            engine
                .store
                .bulk_items("closing".into(), None)
                .await
                .unwrap()
                .iter()
                .all(|item| item.status == "queued")
        );
    }

    #[tokio::test]
    async fn bounded_group_execution_and_undo_use_exact_membership_and_new_identities() {
        let engine = fixture(125).await;
        let before = engine.store.query(MailQuery::default()).await.unwrap().rows;
        let raw = engine
            .store
            .raw_message(before[0].id.clone())
            .await
            .unwrap();
        start(
            &engine,
            "archive",
            MailQuery::default(),
            movement("Archive"),
        )
        .await;
        engine
            .store
            .upsert(vec![
                parse_mail(
                    "fixture",
                    "arrival",
                    "INBOX",
                    b"Subject: Arrival\r\n\r\nNot selected".to_vec(),
                    true,
                    false,
                )
                .unwrap(),
            ])
            .await
            .unwrap();
        let job = execute(&engine, "archive").await;
        assert_eq!(
            (job.completed, job.remaining, job.failed, job.uncertain),
            (125, 0, 0, 0)
        );
        let page = engine
            .store
            .query(MailQuery {
                folder: "Archive".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(
            (page.total, page.rows.len(), page.bulk_pending.len()),
            (125, 50, 0)
        );
        let items = engine
            .store
            .bulk_items("archive".into(), None)
            .await
            .unwrap();
        let Receipt::Move(receipt) = items[0].receipt.as_ref().unwrap() else {
            panic!("Expected MOVE receipt")
        };
        let current = receipt.current.as_ref().unwrap();
        assert_ne!(current.id, before[0].id);
        assert_eq!(
            engine.store.raw_message(current.id.clone()).await.unwrap(),
            raw
        );
        engine
            .store
            .request_bulk_undo("archive".into())
            .await
            .unwrap();
        assert_eq!(
            engine
                .store
                .query(MailQuery {
                    folder: "INBOX".into(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .total,
            126
        );
        let job = execute(&engine, "archive").await;
        assert_eq!(
            (job.restored, job.remaining, job.failed, job.uncertain),
            (125, 0, 0, 0)
        );
        assert_eq!(
            engine
                .store
                .query(MailQuery {
                    folder: "Archive".into(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .total,
            0
        );
    }

    #[tokio::test]
    async fn individual_mutations_and_undo_cannot_bypass_group_ownership() {
        let engine = fixture(1).await;
        let original = engine.store.query(MailQuery::default()).await.unwrap().rows[0].clone();
        let (output, _input) = futures::channel::mpsc::channel(8);
        let (_, receipt) = engine
            .change_folder(&original, "Archive", output.clone(), None)
            .await
            .unwrap();
        let current = receipt.current.clone().unwrap();
        let mut preferences: Preferences = engine.store.get("preferences").await.unwrap();
        preferences.cross_account_moves = true;
        engine.store.put("preferences", preferences).await.unwrap();
        start(
            &engine,
            "reading",
            MailQuery::default(),
            Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await;
        for error in [
            engine
                .change_folder(&current, "Trash", output.clone(), None)
                .await
                .unwrap_err(),
            engine
                .transfer_message(
                    &current,
                    "other".into(),
                    "INBOX".into(),
                    output.clone(),
                    None,
                )
                .await
                .unwrap_err(),
            engine
                .undo_move(original.clone(), &receipt, output.clone(), None)
                .await
                .unwrap_err(),
            engine
                .change_flags(
                    &current,
                    crate::mail_actions::Flags {
                        unread: None,
                        starred: Some(true),
                    },
                )
                .await
                .unwrap_err(),
        ] {
            assert!(
                error.to_string().contains("group action is pending"),
                "{error:#}"
            );
        }
        let unchanged = engine
            .store
            .mail_metadata(current.id.clone())
            .await
            .unwrap();
        assert!(unchanged.unread && !unchanged.starred);
        assert_eq!(unchanged.folder, "Archive");
        assert_eq!(execute(&engine, "reading").await.completed, 1);
        let (_, restored) = engine
            .undo_move(original, &receipt, output, None)
            .await
            .unwrap();
        let restored = engine
            .store
            .mail_metadata(restored.current.unwrap().id)
            .await
            .unwrap();
        assert_eq!(restored.folder, "INBOX");
        assert!(
            !restored.unread,
            "Undo retains the group's acknowledged read change"
        );
    }

    #[tokio::test]
    async fn continue_makes_a_paused_group_eligible_without_replaying_completed_receipts() {
        let engine = fixture(2).await;
        start(&engine, "paused", MailQuery::default(), movement("Archive")).await;
        let item = engine
            .store
            .claim_bulk_item("paused".into())
            .await
            .unwrap()
            .unwrap();
        engine
            .store
            .finish_bulk_item(item, Ok(Receipt::Unchanged))
            .await
            .unwrap();
        engine
            .store
            .run(|c| {
                c.execute("UPDATE bulk_jobs SET paused=1 WHERE id='paused'", [])?;
                Ok(())
            })
            .await
            .unwrap();
        assert!(
            engine
                .store
                .next_pending_bulk(String::new())
                .await
                .unwrap()
                .is_none()
        );
        engine.bulk_control.stopping.set(true);
        let (output, mut input) = futures::channel::mpsc::channel(2);
        engine
            .execute(Command::BulkResume("paused".into()), output)
            .await
            .unwrap();
        assert!(matches!(input.next().await,Some(Event::BulkResumed(id)) if id=="paused"));
        assert_eq!(
            engine
                .store
                .next_pending_bulk(String::new())
                .await
                .unwrap()
                .as_deref(),
            Some("paused")
        );
        assert!(!engine.bulk_control.stopping.get());
        let job = execute(&engine, "paused").await;
        assert_eq!(job.completed, 2);
        assert_eq!(
            engine
                .store
                .query(MailQuery {
                    folder: "Archive".into(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .total,
            1,
            "The already completed receipt must not be replayed"
        );
    }
    #[tokio::test]
    async fn flag_undo_follows_an_acknowledged_move_without_reverting_its_location() {
        let engine = fixture(1).await;
        start(
            &engine,
            "read",
            MailQuery::default(),
            Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await;
        assert_eq!(execute(&engine, "read").await.completed, 1);
        start(&engine, "move", MailQuery::default(), movement("Archive")).await;
        assert_eq!(execute(&engine, "move").await.completed, 1);
        engine.store.request_bulk_undo("read".into()).await.unwrap();
        assert_eq!(execute(&engine, "read").await.restored, 1);
        let page = engine
            .store
            .query(MailQuery {
                folder: "Archive".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!((page.total, page.unread), (1, 1));
    }

    #[tokio::test]
    async fn read_flag_and_undo_keep_newer_unrelated_intent_and_report_partial_failures() {
        let engine = fixture(4).await;
        let rows = engine.store.query(MailQuery::default()).await.unwrap().rows;
        start(
            &engine,
            "read",
            MailQuery::default(),
            Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await;
        engine.store.remove(rows[0].id.clone()).await.unwrap();
        let job = execute(&engine, "read").await;
        assert_eq!((job.completed, job.failed, job.uncertain), (3, 1, 0));
        engine
            .store
            .patch_flags(
                rows[1].clone(),
                crate::mail_actions::Flags {
                    unread: None,
                    starred: Some(true),
                },
            )
            .await
            .unwrap();
        engine.store.request_bulk_undo("read".into()).await.unwrap();
        let job = execute(&engine, "read").await;
        assert_eq!((job.restored, job.failed), (3, 1));
        let current = engine
            .store
            .mail_metadata(rows[1].id.clone())
            .await
            .unwrap();
        assert!(current.unread && current.starred);
        start(
            &engine,
            "flag",
            MailQuery::default(),
            Action::Flags(crate::mail_actions::Flags {
                unread: None,
                starred: Some(true),
            }),
        )
        .await;
        execute(&engine, "flag").await;
        engine.store.request_bulk_undo("flag".into()).await.unwrap();
        execute(&engine, "flag").await;
        let page = engine.store.query(MailQuery::default()).await.unwrap();
        assert_eq!(page.rows.iter().filter(|m| m.starred).count(), 1);
    }
    #[tokio::test]
    async fn reverse_job_order_revisits_successors_after_their_receipt_without_another_wake() {
        let engine = fixture(1).await;
        start(
            &engine,
            "z-first",
            MailQuery::default(),
            Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await;
        start(
            &engine,
            "a-second",
            MailQuery::default(),
            movement("Archive"),
        )
        .await;
        let (sender, input) = CommandSender::channel();
        drop(sender);
        let (output, mut events) = futures::channel::mpsc::channel(2);
        let collect = async { while events.next().await.is_some() {} };
        tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(engine.clone().run_bulk_queue(input.bulk, output), collect);
        })
        .await
        .expect("both ordered jobs complete");
        assert_eq!(
            engine
                .store
                .bulk_job("a-second".into())
                .await
                .unwrap()
                .completed,
            1
        );
        let page = engine
            .store
            .query(MailQuery {
                folder: "Archive".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!((page.total, page.unread), (1, 0));
    }

    #[tokio::test]
    async fn held_account_does_not_block_new_account_or_calendar_and_close_waits_for_all_steps() {
        let engine = fixture(1).await;
        start(
            &engine,
            "held",
            MailQuery::default(),
            Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await;
        let held = engine.account_access("fixture").await;
        let (sender, input) = CommandSender::channel();
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let owner = tokio::spawn(engine.clone().run_bulk_queue(input.bulk, output.clone()));
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = events.next().await {
                if matches!(event, Event::BulkUpdate(job) if job.id == "held" && job.running == 1) {
                    break;
                }
            }
        })
        .await
        .expect("held step started");
        engine
            .store
            .upsert(vec![
                parse_mail(
                    "independent",
                    "1",
                    "INBOX",
                    b"Subject: Free\r\n\r\nBody".to_vec(),
                    true,
                    false,
                )
                .unwrap(),
            ])
            .await
            .unwrap();
        start(
            &engine,
            "independent",
            MailQuery {
                account: Some("independent".into()),
                ..Default::default()
            },
            Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await;
        engine
            .store
            .put(
                "calendars",
                vec![CalendarSource {
                    id: "calendar".into(),
                    name: "Calendar".into(),
                    kind: CalendarKind::CalDav,
                    url: "https://calendar.example/".into(),
                    username: "fixture".into(),
                    access: Default::default(),
                }],
            )
            .await
            .unwrap();
        let mut event = super::super::calendar_tests::event("calendar");
        event.etag = Some("revision".into());
        engine
            .store
            .admit_calendar_action("calendar-job".into(), event, false)
            .await
            .unwrap();
        sender.try_send(Command::BulkRun(String::new())).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut mail_done, mut calendar_done) = (false, false);
            while let Some(event) = events.next().await {
                match event {
                    Event::BulkFinished(id, Ok(job)) if id == "independent" => {
                        mail_done = job.completed == 1
                    }
                    Event::CalendarJob(id, Ok(job)) if id == "calendar-job" => {
                        calendar_done = job.cache_applied
                    }
                    Event::Error(error) => panic!("{error}"),
                    _ => {}
                }
                if mail_done && calendar_done {
                    break;
                }
            }
        })
        .await
        .expect("independent work progresses while one account is held");
        assert_eq!(
            engine.store.bulk_job("held".into()).await.unwrap().running,
            1
        );
        engine
            .execute(Command::BulkStop(1), output.clone())
            .await
            .unwrap();
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, Event::BulkStopped(_)));
        }
        assert!(engine.bulk_control.active.get());
        drop(held);
        drop(sender);
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = events.next().await {
                if matches!(event, Event::BulkStopped(1)) {
                    break;
                }
            }
            owner.await.unwrap();
        })
        .await
        .expect("last receipt closes the owner");
        assert!(!engine.bulk_control.active.get());
    }

    #[tokio::test]
    async fn a_coalesced_wake_executes_every_persisted_group() {
        let engine = fixture(8).await;
        let (sender, input) = CommandSender::channel();
        for i in 0..8 {
            let id = format!("job-{i}");
            start(
                &engine,
                &id,
                MailQuery {
                    search: format!("Group {i:03}"),
                    ..Default::default()
                },
                movement("Archive"),
            )
            .await;
            sender.try_send(Command::BulkRun(id)).unwrap();
        }
        drop(sender);
        let (output, mut events) = futures::channel::mpsc::channel(2);
        let collect = async {
            let mut count = 0;
            while let Some(event) = events.next().await {
                if let Event::BulkFinished(_, result) = event {
                    assert_eq!(result.unwrap().completed, 1);
                    count += 1;
                }
            }
            count
        };
        let (_, count) = tokio::time::timeout(Duration::from_secs(10), async {
            tokio::join!(engine.clone().run_bulk_queue(input.bulk, output), collect)
        })
        .await
        .unwrap();
        assert_eq!(count, 8);
        assert_eq!(
            engine
                .store
                .query(MailQuery {
                    folder: "Archive".into(),
                    ..Default::default()
                })
                .await
                .unwrap()
                .total,
            8
        );
    }
    #[tokio::test]
    async fn closing_finishes_the_current_receipt_and_leaves_unsent_work_for_next_launch() {
        let engine = fixture(8).await;
        start(&engine, "close", MailQuery::default(), movement("Archive")).await;
        // Hold account ownership after a provider slot and journal claim.
        // Once admitted, the operation must retain its eventual receipt.
        let account = engine.account_access("fixture").await;
        let (output, mut events) = futures::channel::mpsc::channel(8);
        let running = tokio::spawn({
            let engine = engine.clone();
            let output = output.clone();
            async move { engine.execute_bulk_job("close".into(), output).await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(event) = events.next().await {
                if let Event::BulkUpdate(job) = event
                    && job.running == 1
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        engine
            .execute(Command::BulkStop(1), output.clone())
            .await
            .unwrap();
        assert!(engine.bulk_control.active.get());
        drop(account);
        let job = tokio::time::timeout(Duration::from_secs(5), async {
            let mut saved = None;
            while let Some(event) = events.next().await {
                match event {
                    Event::BulkFinished(_, result) => saved = Some(result.unwrap()),
                    Event::BulkStopped(1) => {
                        return saved.expect("Close followed the durable receipt");
                    }
                    _ => {}
                }
            }
            panic!("Missing close acknowledgment")
        })
        .await
        .unwrap();
        running.await.unwrap();
        assert_eq!(
            (job.completed, job.remaining, job.running, job.uncertain),
            (1, 7, 0, 0)
        );
        let mut reopened = engine.clone();
        reopened.bulk_control = Default::default();
        let done = execute(&reopened, "close").await;
        assert_eq!((done.completed, done.remaining, done.uncertain), (8, 0, 0));
    }
    #[tokio::test]
    async fn cross_account_groups_preserve_originals_and_can_undo_after_the_preference_changes() {
        let engine = fixture(3).await;
        for id in ["fixture", "destination"] {
            let account:Account=serde_json::from_value(serde_json::json!({"id":id,"name":id,"email":format!("{id}@example.test"),"protocol":"Imap","host":"imap.example.test","port":993,"username":id,"smtp_host":"smtp.example.test","smtp_port":465})).unwrap();
            engine.store.save_account(account).await.unwrap();
        }
        let mut preferences = Preferences {
            cross_account_moves: true,
            ..Default::default()
        };
        engine
            .store
            .put("preferences", preferences.clone())
            .await
            .unwrap();
        let original = engine.store.query(MailQuery::default()).await.unwrap().rows;
        let raw = engine
            .store
            .raw_message(original[0].id.clone())
            .await
            .unwrap();
        start(
            &engine,
            "transfer",
            MailQuery::default(),
            Action::Move {
                account: Some("destination".into()),
                folder: "INBOX".into(),
            },
        )
        .await;
        let done = execute(&engine, "transfer").await;
        assert_eq!((done.completed, done.failed, done.uncertain), (3, 0, 0));
        let items = engine
            .store
            .bulk_items("transfer".into(), None)
            .await
            .unwrap();
        let Receipt::Move(receipt) = items[0].receipt.as_ref().unwrap() else {
            panic!("Missing transfer receipt")
        };
        assert_eq!(
            engine
                .store
                .raw_message(receipt.current.as_ref().unwrap().id.clone())
                .await
                .unwrap(),
            raw
        );
        preferences.cross_account_moves = false;
        engine.store.put("preferences", preferences).await.unwrap();
        engine
            .store
            .request_bulk_undo("transfer".into())
            .await
            .unwrap();
        let restored = execute(&engine, "transfer").await;
        assert_eq!(
            (restored.restored, restored.failed, restored.uncertain),
            (3, 0, 0)
        );
        let page = engine.store.query(MailQuery::default()).await.unwrap();
        assert_eq!(page.total, 3);
        assert!(
            page.rows
                .iter()
                .all(|m| m.account_id == "fixture" && m.folder == "INBOX")
        );
    }
}
