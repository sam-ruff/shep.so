//! Serial durable group execution uses one shared provider slot at a time. UI
//! commands, reads, sync, and Undo intent continue on their independent queues.
use super::*;
use crate::bulk::{Action, Item, Job, Receipt};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

#[derive(Default)]
pub(super) struct Control {
    pub stopping: AtomicBool,
    pub active: AtomicBool,
}

impl Engine {
    pub(super) async fn run_bulk_queue(
        self,
        mut input: mpsc::Receiver<Command>,
        mut output: Output,
    ) {
        self.drain_bulk_jobs(output.clone()).await;
        while let Some(command) = input.recv().await {
            if matches!(command, Command::BulkRun(_)) {
                // This capacity-one channel is a wake signal. Exact jobs stay
                // durable in SQLite, including requests coalesced while busy.
                self.drain_bulk_jobs(output.clone()).await;
            } else {
                let _ = output
                    .send(Event::Error("Unexpected mail-operation command".into()))
                    .await;
            }
        }
    }
    async fn drain_bulk_jobs(&self, output: Output) {
        let mut after = String::new();
        loop {
            if self.bulk_control.stopping.load(SeqCst) {
                break;
            }
            match self.store.next_pending_bulk(after.clone()).await {
                Ok(Some(id)) => {
                    after = id.clone();
                    self.execute_bulk_job(id, output.clone()).await;
                }
                Ok(None) => break,
                Err(error) => {
                    let mut output = output.clone();
                    let _ = output
                        .send(Event::Error(format!(
                            "Could not resume mail changes. Open History to retry. {error:#}"
                        )))
                        .await;
                    break;
                }
            }
        }
    }

    async fn execute_bulk_job(&self, id: String, mut output: Output) {
        // Publish activity before checking close intent. A close sees either
        // this guard or the worker observes stopping before touching storage.
        self.bulk_control.active.store(true, SeqCst);
        if self.bulk_control.stopping.load(SeqCst) {
            self.bulk_control.active.store(false, SeqCst);
            // Stop may have observed active=true and delegated its acknowledgment
            // to us. Even when no item started, the closing window must hear it.
            let _ = output.send(Event::BulkStopped).await;
            return;
        }
        let result = self
            .perform_bulk_job(&id, &mut output)
            .await
            .map(Arc::new)
            .map_err(|e| format!("{e:#}"));
        let failed = result.is_err();
        let _ = output.send(Event::BulkFinished(id, result)).await;
        self.bulk_control.active.store(false, SeqCst);
        if failed {
            // A visible error cancels pending close and leaves recovery usable.
            self.bulk_control.stopping.store(false, SeqCst);
        } else if self.bulk_control.stopping.load(SeqCst) {
            let _ = output.send(Event::BulkStopped).await;
        }
    }
    async fn perform_bulk_job(&self, id: &str, output: &mut Output) -> anyhow::Result<Job> {
        let lease = self.store.bulk_lease(id.to_owned()).await?;
        let mut job = self.store.resume_bulk(&lease).await?;
        output
            .send(Event::BulkUpdate(Arc::new(job.clone())))
            .await?;
        let mut last_progress = None::<std::time::Instant>;
        loop {
            if self.bulk_control.stopping.load(SeqCst) {
                break;
            }
            let Some(item) = self.store.claim_bulk_item(id.to_owned()).await? else {
                break;
            };
            job = self.store.bulk_job(id.to_owned()).await?;
            if last_progress.is_none_or(|t| t.elapsed() >= Duration::from_millis(100)) {
                output
                    .send(Event::BulkUpdate(Arc::new(job.clone())))
                    .await?;
                last_progress = Some(std::time::Instant::now());
            }
            let _slot = self.provider_slots.acquire().await;
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
                    .map_err(|error| (format!("{error:#}"), !self.demo)),
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
                        .await?;
                    Ok(Receipt::Unchanged)
                }
                Receipt::Unchanged => Ok(Receipt::Unchanged),
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
                } else if original.folder == *folder {
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

#[cfg(test)]
mod tests {
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
    async fn stop_at_worker_entry_acknowledges_close_without_claiming_mail() {
        let engine = fixture(2).await;
        start(
            &engine,
            "closing",
            MailQuery::default(),
            movement("Archive"),
        )
        .await;
        engine.bulk_control.stopping.store(true, SeqCst);
        let (output, mut input) = futures::channel::mpsc::channel(2);
        engine.execute_bulk_job("closing".into(), output).await;
        assert!(matches!(input.next().await, Some(Event::BulkStopped)));
        assert!(!engine.bulk_control.active.load(SeqCst));
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
        engine.bulk_control.stopping.store(true, SeqCst);
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
        assert!(!engine.bulk_control.stopping.load(SeqCst));
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
        let mut permits = Vec::new();
        for _ in 0..8 {
            permits.push(engine.provider_slots.acquire().await);
        }
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
            .execute(Command::BulkStop, output.clone())
            .await
            .unwrap();
        assert!(engine.bulk_control.active.load(SeqCst));
        permits.clear();
        let job = tokio::time::timeout(Duration::from_secs(5), async {
            let mut saved = None;
            while let Some(event) = events.next().await {
                match event {
                    Event::BulkFinished(_, result) => saved = Some(result.unwrap()),
                    Event::BulkStopped => {
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
