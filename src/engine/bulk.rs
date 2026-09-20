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
        self.execute_bulk_work(id, None, usize::MAX, None, &mut output)
            .await;
    }

    async fn execute_bulk_work(
        &self,
        id: String,
        position: Option<u64>,
        limit: usize,
        lease: Option<Arc<crate::store::BulkLease>>,
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
        let before_item = match position {
            Some(position) => self.store.bulk_item_state(id.clone(), position).await.ok(),
            None => None,
        };
        let result = self
            .perform_bulk_job(&id, output, position, limit, lease)
            .await
            .map(Arc::new)
            .map_err(|e| format!("{e:#}"));
        let failed = result.is_err();
        let progressed = if let Some(position) = position {
            self.store
                .bulk_item_state(id.clone(), position)
                .await
                .is_ok_and(|state| Some(state) != before_item)
        } else {
            result.as_ref().is_ok_and(|j| {
                Some((
                    j.remaining,
                    j.completed,
                    j.restored,
                    j.failed,
                    j.uncertain,
                    j.cancelled,
                )) != before
            })
        };
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
        progressed && !failed
    }
    async fn perform_bulk_job(
        &self,
        id: &str,
        output: &mut Output,
        position: Option<u64>,
        limit: usize,
        lease: Option<Arc<crate::store::BulkLease>>,
    ) -> anyhow::Result<Job> {
        let (lease, mut job) = match lease {
            Some(lease) => (lease, self.store.bulk_job(id.to_owned()).await?),
            None => {
                let lease = Arc::new(self.store.bulk_lease(id.to_owned()).await?);
                let job = self.store.resume_bulk(&lease).await?;
                (lease, job)
            }
        };
        output
            .send(Event::BulkUpdate(Arc::new(job.clone())))
            .await?;
        let mut last_progress = None::<std::time::Instant>;
        let mut completed = 0;
        loop {
            if self.bulk_control.stopping.get()
                || job.paused
                || job.remaining == 0
                || completed >= limit
            {
                break;
            }
            if let Some(item) = self
                .store
                .pending_bulk_flag_repair_at(&lease, position)
                .await?
            {
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
            let item = match position {
                Some(position) => self.store.claim_owned_bulk_item(&lease, position).await?,
                None => self.store.claim_bulk_item_at(id.to_owned(), None).await?,
            };
            let Some(item) = item else {
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
    async fn failed_receipt_completion_requests_backoff_even_after_claim_progress() {
        let engine = fixture(1).await;
        start(
            &engine,
            "receipt-failure",
            MailQuery::default(),
            Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await;
        engine
            .store
            .run(|c| {
                c.execute_batch(
                    "CREATE TEMP TRIGGER fail_receipt_completion BEFORE UPDATE ON bulk_items
                     WHEN old.job='receipt-failure' AND new.status='done'
                     BEGIN SELECT RAISE(ABORT,'Fixture receipt failure'); END;",
                )?;
                Ok(())
            })
            .await
            .expect("inject receipt failure");
        let (mut output, _events) = futures::channel::mpsc::channel(32);
        assert!(
            !engine
                .execute_bulk_work("receipt-failure".into(), Some(0), 1, None, &mut output)
                .await,
            "a failed step must request cooldown even when its claim changed state"
        );
        let job = engine
            .store
            .bulk_job("receipt-failure".into())
            .await
            .expect("retained journal");
        assert_eq!(job.completed, 0);
        assert_eq!(job.remaining, 1);
        engine
            .store
            .run(|c| {
                c.execute_batch("DROP TRIGGER fail_receipt_completion")?;
                Ok(())
            })
            .await
            .expect("restore storage");
        assert!(
            engine
                .execute_bulk_work("receipt-failure".into(), Some(0), 1, None, &mut output)
                .await,
            "the retained receipt can finish after storage recovers"
        );
        assert_eq!(
            engine
                .store
                .bulk_job("receipt-failure".into())
                .await
                .expect("completed receipt")
                .completed,
            1
        );
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
    async fn pause_after_selection_survives_recovery_and_continue_preserves_newer_undo() {
        let engine = fixture(2).await;
        let flags = crate::mail_actions::Flags {
            unread: Some(false),
            starred: None,
        };
        start(
            &engine,
            "pause-race",
            MailQuery::default(),
            Action::Flags(flags),
        )
        .await;
        let selected = engine
            .store
            .next_action_work(0, String::new(), vec![], vec![], false)
            .await
            .unwrap()
            .unwrap();
        let crate::store::Work::Mail { id, position } = selected.work else {
            panic!("mail work")
        };
        engine
            .store
            .run(|c| {
                c.execute("UPDATE bulk_jobs SET paused=1 WHERE id='pause-race'", [])?;
                Ok(())
            })
            .await
            .unwrap();
        let mut held = Vec::new();
        for _ in 0..dispatch::NETWORK_CONCURRENCY {
            held.push(engine.provider_slots.acquire().await);
        }
        let (mut output, _events) = futures::channel::mpsc::channel(16);
        let changed = tokio::time::timeout(
            Duration::from_secs(1),
            engine.execute_bulk_work(id.clone(), Some(position), 1, None, &mut output),
        )
        .await
        .expect("paused recovery never waits for provider capacity");
        assert!(!changed);
        let paused = engine.store.bulk_job(id.clone()).await.unwrap();
        assert!(paused.paused);
        assert_eq!(
            (
                paused.remaining,
                paused.running,
                paused.completed,
                paused.uncertain
            ),
            (2, 0, 0, 0)
        );
        for item in engine.store.bulk_items(id.clone(), None).await.unwrap() {
            assert!(engine.store.mail_metadata(item.id).await.unwrap().unread);
        }
        drop(held);
        engine.store.continue_bulk(id.clone()).await.unwrap();
        assert!(
            engine
                .execute_bulk_work(id.clone(), Some(position), 1, None, &mut output)
                .await
        );
        let selected = engine
            .store
            .next_action_work(0, String::new(), vec![], vec![], false)
            .await
            .unwrap()
            .unwrap();
        let crate::store::Work::Mail {
            position: stale_position,
            ..
        } = selected.work
        else {
            panic!("mail work")
        };
        assert_ne!(stale_position, position);
        let undo = engine.store.request_bulk_undo(id.clone()).await.unwrap();
        assert!(undo.undo_requested);
        let lease = engine.store.bulk_lease(id.clone()).await.unwrap();
        let recovered = engine.store.resume_bulk(&lease).await.unwrap();
        assert!(recovered.undo_requested);
        assert_eq!((recovered.remaining, recovered.cancelled), (1, 1));
        drop(lease);
        assert!(
            !engine
                .execute_bulk_work(id.clone(), Some(stale_position), 1, None, &mut output)
                .await
        );
        engine.store.continue_bulk(id.clone()).await.unwrap();
        let finished = execute(&engine, &id).await;
        assert_eq!(
            (finished.restored, finished.cancelled, finished.uncertain),
            (1, 1, 0)
        );
        assert!(finished.undo_requested);
        assert!(
            engine
                .store
                .query(MailQuery::default())
                .await
                .unwrap()
                .rows
                .iter()
                .all(|mail| mail.unread)
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
    async fn independent_items_share_one_group_lease_and_stop_drains_without_recovering_live_siblings()
     {
        let engine = fixture(2).await;
        engine
            .store
            .upsert(vec![
                parse_mail(
                    "free",
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
            "shared",
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
                if let Event::BulkUpdate(job) = event
                    && job.id == "shared"
                    && job.completed == 1
                    && job.running == 1
                {
                    assert_eq!((job.remaining, job.uncertain), (2, 0));
                    break;
                }
            }
        })
        .await
        .expect("free sibling finishes while the first account remains held");
        assert!(
            engine.store.bulk_lease("shared".into()).await.is_err(),
            "the shared lease stays owned through the held sibling"
        );
        let items = engine
            .store
            .bulk_items("shared".into(), None)
            .await
            .unwrap();
        assert_eq!(items.iter().filter(|i| i.status == "running").count(), 1);
        assert_eq!(
            items.iter().filter(|i| i.status == "queued").count(),
            1,
            "same account does not claim a second provider step"
        );
        engine
            .execute(Command::BulkStop(1), output.clone())
            .await
            .unwrap();
        while let Ok(event) = events.try_recv() {
            assert!(!matches!(event, Event::BulkStopped(1)));
        }
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
        .expect("stop observes every accepted receipt");
        let job = engine.store.bulk_job("shared".into()).await.unwrap();
        assert_eq!(
            (job.completed, job.remaining, job.uncertain, job.running),
            (2, 1, 0, 0)
        );
        assert!(!engine.bulk_control.active.get());
        let lease = engine.store.bulk_lease("shared".into()).await.unwrap();
        assert_eq!(engine.store.resume_bulk(&lease).await.unwrap().uncertain, 0);
        drop(lease);
        engine.bulk_control.stopping.set(false);
        assert_eq!(execute(&engine, "shared").await.completed, 3);
    }

    #[tokio::test]
    async fn blocked_candidate_pages_reach_later_accounts_and_stop_preserves_skipped_work() {
        let engine = fixture(120).await;
        let mut free = parse_mail(
            "free",
            "1",
            "INBOX",
            b"Subject: Free\r\n\r\nBody".to_vec(),
            true,
            false,
        )
        .unwrap();
        free.summary.timestamp = -1;
        engine.store.upsert(vec![free]).await.unwrap();
        start(
            &engine,
            "pages",
            MailQuery::default(),
            Action::Flags(crate::mail_actions::Flags {
                unread: Some(false),
                starred: None,
            }),
        )
        .await;
        engine.store.run(|c| {
            assert_eq!(c.query_row("SELECT position FROM bulk_items WHERE job='pages' AND json_extract(original,'$.account_id')='free'", [], |r|r.get::<_,i64>(0))?, 120);
            Ok(())
        }).await.unwrap();
        let held = engine.account_access("fixture").await;
        let (sender, input) = CommandSender::channel();
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let owner = tokio::spawn(engine.clone().run_bulk_queue(input.bulk, output.clone()));
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match events.next().await.unwrap() {
                    Event::BulkUpdate(job) if job.id == "pages" && job.completed == 1 => {
                        assert_eq!((job.running, job.remaining, job.uncertain), (1, 120, 0));
                        break;
                    }
                    Event::Error(error) => panic!("{error}"),
                    _ => {
                        sender.try_send(Command::BulkRun(String::new())).unwrap();
                    }
                }
            }
        })
        .await
        .expect("wakeups cannot keep the scan at the first blocked page");
        engine
            .execute(Command::BulkStop(1), output.clone())
            .await
            .unwrap();
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
        .expect("stop drains the accepted work without scanning or claiming skipped rows");
        let job = engine.store.bulk_job("pages".into()).await.unwrap();
        assert_eq!(
            (job.completed, job.remaining, job.running, job.uncertain),
            (2, 119, 0, 0)
        );
        engine.bulk_control.stopping.set(false);
        assert_eq!(execute(&engine, "pages").await.completed, 121);
    }

    #[tokio::test]
    async fn leased_item_claims_and_repairs_remain_position_specific() {
        let engine = fixture(2).await;
        engine
            .store
            .upsert(vec![
                parse_mail(
                    "free",
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
        let flags = crate::mail_actions::Flags {
            unread: Some(false),
            starred: None,
        };
        start(
            &engine,
            "shared",
            MailQuery::default(),
            Action::Flags(flags),
        )
        .await;
        let lease = engine.store.bulk_lease("shared".into()).await.unwrap();
        engine.store.resume_bulk(&lease).await.unwrap();
        let items = engine
            .store
            .bulk_items("shared".into(), None)
            .await
            .unwrap();
        let same: Vec<_> = items
            .iter()
            .filter(|i| i.original.as_ref().unwrap().account_id == "fixture")
            .collect();
        let other = items
            .iter()
            .find(|i| i.original.as_ref().unwrap().account_id == "free")
            .unwrap();
        let first = engine
            .store
            .claim_owned_bulk_item(&lease, same[0].position)
            .await
            .unwrap()
            .unwrap();
        assert!(
            engine
                .store
                .claim_owned_bulk_item(&lease, same[1].position)
                .await
                .unwrap()
                .is_none()
        );
        let second = engine
            .store
            .claim_owned_bulk_item(&lease, other.position)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            engine
                .store
                .bulk_job("shared".into())
                .await
                .unwrap()
                .running,
            2
        );
        for item in [first, second] {
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
        }
        assert!(
            engine
                .store
                .pending_bulk_flag_repair_at(&lease, Some(same[1].position))
                .await
                .unwrap()
                .is_none()
        );
        let selected = engine
            .store
            .pending_bulk_flag_repair_at(&lease, Some(other.position))
            .await
            .unwrap()
            .unwrap();
        engine
            .store
            .finish_bulk_item(selected, Ok(Receipt::Unchanged))
            .await
            .unwrap();
        assert!(
            engine
                .store
                .pending_bulk_flag_repair_at(&lease, Some(other.position))
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            engine
                .store
                .pending_bulk_flag_repair_at(&lease, Some(same[0].position))
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            engine
                .store
                .claim_owned_bulk_item(&lease, same[1].position)
                .await
                .unwrap()
                .is_none(),
            "repair retains account priority"
        );
        let repair = engine
            .store
            .pending_bulk_flag_repair_at(&lease, Some(same[0].position))
            .await
            .unwrap()
            .unwrap();
        engine
            .store
            .finish_bulk_item(repair, Ok(Receipt::Unchanged))
            .await
            .unwrap();
        assert!(
            engine
                .store
                .claim_owned_bulk_item(&lease, same[1].position)
                .await
                .unwrap()
                .is_some()
        );
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
        let ready = engine
            .store
            .next_action_work(0, String::new(), vec![], vec![], false)
            .await
            .unwrap()
            .unwrap();
        assert!(ready.accounts.contains(&"mail:fixture".into()));
        assert!(ready.accounts.contains(&"mail:destination".into()));
        for occupied in ["mail:fixture", "mail:destination"] {
            assert!(
                engine
                    .store
                    .next_action_work(0, String::new(), vec![occupied.into()], vec![], false)
                    .await
                    .unwrap()
                    .is_none(),
                "inverse reserves both the current and restored account"
            );
        }
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
