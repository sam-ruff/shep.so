use super::*;

const NETWORK_CONCURRENCY: usize = 8;

#[derive(Clone)]
pub(super) struct Slots(Arc<tokio::sync::Semaphore>);
impl Default for Slots {
    fn default() -> Self {
        Self(Arc::new(tokio::sync::Semaphore::new(NETWORK_CONCURRENCY)))
    }
}
impl Slots {
    pub(super) async fn acquire(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.0
            .clone()
            .acquire_owned()
            .await
            .expect("Provider slots stay open")
    }
}

/// Independent bounded queues keep local interaction independent of slow providers.
#[derive(Clone, Debug)]
pub struct CommandSender {
    reads: mpsc::Sender<Command>,
    prefetch: mpsc::Sender<Command>,
    persistence: mpsc::Sender<Command>,
    network: mpsc::Sender<Command>,
    sync: mpsc::Sender<Command>,
    printing: mpsc::Sender<Command>,
    selections: mpsc::Sender<Command>,
    bulk: mpsc::Sender<Command>,
    database: mpsc::Sender<Command>,
}

pub(super) struct Inputs {
    reads: mpsc::Receiver<Command>,
    prefetch: mpsc::Receiver<Command>,
    persistence: mpsc::Receiver<Command>,
    network: mpsc::Receiver<Command>,
    sync: mpsc::Receiver<Command>,
    printing: mpsc::Receiver<Command>,
    selections: mpsc::Receiver<Command>,
    pub(super) bulk: mpsc::Receiver<Command>,
    database: mpsc::Receiver<Command>,
}

impl CommandSender {
    #[cfg(test)]
    pub(crate) fn database_test_channels()
    -> (Self, mpsc::Receiver<Command>, mpsc::Receiver<Command>) {
        let (sender, inputs) = Self::channel();
        (sender, inputs.database, inputs.persistence)
    }
    #[cfg(test)]
    pub(crate) fn network_test_channel() -> (Self, mpsc::Receiver<Command>) {
        let (sender, inputs) = Self::channel();
        (sender, inputs.network)
    }

    #[cfg(test)]
    pub(crate) fn foreground_test_channel() -> (Self, mpsc::Receiver<Command>) {
        let (sender, inputs) = Self::channel();
        (sender, inputs.reads)
    }

    #[cfg(test)]
    pub(crate) fn persistence_test_channel() -> (Self, mpsc::Receiver<Command>) {
        let (sender, inputs) = Self::channel();
        (sender, inputs.persistence)
    }

    #[cfg(test)]
    pub(crate) fn draft_review_test_channels()
    -> (Self, mpsc::Receiver<Command>, mpsc::Receiver<Command>) {
        let (sender, inputs) = Self::channel();
        (sender, inputs.persistence, inputs.reads)
    }

    #[cfg(test)]
    pub(crate) fn selection_test_channel() -> (Self, mpsc::Receiver<Command>) {
        let (sender, inputs) = Self::channel();
        (sender, inputs.selections)
    }

    pub(super) fn channel() -> (Self, Inputs) {
        let (reads, read_input) = mpsc::channel(CHANNEL_CAPACITY);
        let (prefetch, prefetch_input) = mpsc::channel(8);
        let (persistence, persistence_input) = mpsc::channel(CHANNEL_CAPACITY);
        let (network, network_input) = mpsc::channel(CHANNEL_CAPACITY);
        let (sync, sync_input) = mpsc::channel(1);
        let (printing, print_input) = mpsc::channel(2);
        let (selections, selection_input) = mpsc::channel(CHANNEL_CAPACITY);
        let (bulk, bulk_input) = mpsc::channel(1);
        let (database, database_input) = mpsc::channel(1);
        (
            Self {
                reads,
                prefetch,
                persistence,
                network,
                sync,
                printing,
                selections,
                bulk,
                database,
            },
            Inputs {
                reads: read_input,
                prefetch: prefetch_input,
                persistence: persistence_input,
                network: network_input,
                sync: sync_input,
                printing: print_input,
                selections: selection_input,
                bulk: bulk_input,
                database: database_input,
            },
        )
    }

    pub fn try_send(
        &self,
        command: Command,
    ) -> Result<(), Box<mpsc::error::TrySendError<Command>>> {
        if matches!(command, Command::Sync | Command::BulkRun(_)) {
            let queue = if matches!(command, Command::Sync) {
                &self.sync
            } else {
                &self.bulk
            };
            return match queue.try_send(command) {
                // A pending full refresh already covers another click. Retain
                // one follow-up request while a cycle is active, never drop it
                // because the unrelated provider queue is occupied.
                Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
                Err(error) => Err(Box::new(error)),
            };
        }
        let channel = match &command {
            Command::Database(_) => &self.database,
            Command::Folder(request) => {
                if request.is_read() {
                    &self.reads
                } else {
                    &self.selections
                }
            }
            Command::Print(..) => &self.printing,
            Command::Selection(..)
            | Command::ReviewSelection(..)
            | Command::ReleaseSelection(_)
            | Command::BulkStart(..)
            | Command::BulkUndo(_)
            | Command::BulkResolve(_)
            | Command::BulkStop
            | Command::BulkResume(_) => &self.selections,
            Command::BulkRun(_) => &self.bulk,
            Command::Query(_, _, true) | Command::Detail { prefetch: true, .. } => &self.prefetch,
            Command::Query(..)
            | Command::MoveRecoveries(..)
            | Command::Detail { .. }
            | Command::Conversation(..)
            | Command::RemovalPreview(..)
            | Command::OutgoingPage(..)
            | Command::BulkJobs(..)
            | Command::BulkItems(..) => &self.reads,
            Command::SavePreferences(..)
            | Command::SaveDraft(_)
            | Command::AutoSaveDraft(_)
            | Command::DeleteDraft(_)
            | Command::ForwardDraft(..)
            | Command::RemoveDraftFile(..) => &self.persistence,
            _ => &self.network,
        };
        channel.try_send(command).map_err(Box::new)
    }
}

impl Engine {
    pub(super) async fn run(self, input: Inputs, output: Output) {
        let background = !self.demo || {
            #[cfg(feature = "test-support")]
            {
                std::env::args().any(|arg| arg == "--background-sync")
            }
            #[cfg(not(feature = "test-support"))]
            {
                false
            }
        };
        tokio::join!(
            self.clone()
                .run_mail_sync(input.sync, output.clone(), background),
            self.clone().run_reads(input.reads, output.clone(), 2),
            self.clone().run_reads(input.prefetch, output.clone(), 1),
            self.clone().run_reads(input.printing, output.clone(), 1),
            self.clone()
                .run_persistence(input.selections, output.clone()),
            self.clone()
                .run_persistence(input.persistence, output.clone()),
            self.clone().run_bulk_queue(input.bulk, output.clone()),
            self.clone()
                .run_database_transfers(input.database, output.clone()),
            self.run_network(input.network, output),
        );
    }

    async fn run_reads(
        self,
        mut input: mpsc::Receiver<Command>,
        mut output: Output,
        concurrency: usize,
    ) {
        let mut jobs = tokio::task::JoinSet::new();
        let mut closed = false;
        loop {
            tokio::select! {
                result = jobs.join_next(), if !jobs.is_empty() => {
                    match result {
                        Some(Ok(Err(e))) => { let _ = output.send(Event::Error(format!("{e:#}"))).await; }
                        Some(Err(_)) => { let _ = output.send(Event::Error("A local read failed. Try opening the message again.".into())).await; }
                        _ => {}
                    }
                }
                command = input.recv(), if !closed && jobs.len() < concurrency => {
                    if let Some(command) = command {
                        let engine = self.clone(); let events = output.clone();
                        jobs.spawn(async move { engine.execute(command, events).await });
                    } else { closed = true; }
                }
                else => break,
            }
        }
    }

    async fn run_persistence(self, mut input: mpsc::Receiver<Command>, mut output: Output) {
        // Settings/draft writes retain FIFO order while reads and providers continue.
        while let Some(command) = input.recv().await {
            if let Err(e) = self.execute(command, output.clone()).await {
                let _ = output.send(Event::Error(format!("{e:#}"))).await;
            }
        }
    }

    async fn run_network(self, mut input: mpsc::Receiver<Command>, mut output: Output) {
        let engine = self;
        let demo = engine.demo;
        let mut jobs = tokio::task::JoinSet::new();
        let mut busy = HashSet::new();
        let mut timer = tokio::time::interval(Duration::from_secs(60));
        timer.tick().await;
        let mut last_calendar_sync = Instant::now();
        let mut last_backup_attempt: Option<(BackupTarget, Instant)> = None;
        loop {
            tokio::select! {
                biased;
                result=jobs.join_next(),if !jobs.is_empty()=>{
                    if let Some(Ok((key,result)))=result{
                        if let Some(key)=key{busy.remove(&key);let _=output.send(Event::Busy(key,false)).await;}
                        if let Err(e)=result{let _=output.send(Event::Error(format!("{e:#}"))).await;}
                    }
                }
                command=input.recv(),if jobs.len()<NETWORK_CONCURRENCY=>{
                    let Some(command)=command else{break;};
                    let key=command.key();
                    if key.as_ref().is_some_and(|k|busy.contains(k)){continue;}
                    if let Some(key)=&key{busy.insert(key.clone());let _=output.send(Event::Busy(key.clone(),true)).await;}
                    let engine=engine.clone();let output=output.clone();
                    jobs.spawn(async move {
                        let _slot = engine.provider_slots.acquire().await;
                        // Uploads have bounded HTTP requests and progress checks,
                        // plus a durable journal. Do not cancel a healthy transfer
                        // merely because the whole archive takes over ten minutes.
                        // Restore also must observe its blocking SQLite commit;
                        // dropping its future cannot cancel that transaction.
                        let result = if matches!(&command, Command::Move(..) | Command::Transfer(..) | Command::UndoMove(..) | Command::RecoverMailMove(..) | Command::Flags(..) | Command::Backup(..) | Command::AutomaticBackup(_) | Command::Restore(..) | Command::Send(_) | Command::DisconnectGoogle(_) | Command::CleanupGoogle | Command::GoogleLogin(..) | Command::ResolveOutgoing(..) | Command::RepairOutgoing | Command::IndexConversations | Command::ConnectCalendars(..) | Command::SaveAccount(..) | Command::RemoveConnection(..) | Command::CleanupCredentials | Command::RestoreGoogleCalendars) {
                            engine.execute(command, output).await
                        } else {
                            tokio::time::timeout(Duration::from_secs(600), engine.execute(command, output)).await
                                .context("The operation timed out. Try again.").and_then(|result| result)
                        };
                        (key, result)
                    });
                }
                _=timer.tick(),if !demo=>{
                    if let Ok(prefs)=engine.store.get::<Preferences>("preferences").await{
                        if last_calendar_sync.elapsed() >= Duration::from_secs(prefs.sync_minutes * 60) && !busy.contains("calendar") && jobs.len() < NETWORK_CONCURRENCY {
                            last_calendar_sync = Instant::now();
                            busy.insert("calendar".into());
                            let worker = engine.clone(); let events = output.clone();
                            jobs.spawn(async move {
                                let _slot = worker.provider_slots.acquire().await;
                                (Some("calendar".into()), worker.execute(Command::SyncCalendar, events).await)
                            });
                        }
                        let target = BackupTarget::from_preferences(&prefs);
                        let interval = Duration::from_secs(prefs.backup_hours * 3600);
                        let retry_due = last_backup_attempt.as_ref().is_none_or(|(previous, at)| *previous != target || at.elapsed() >= interval);
                        if prefs.auto_backup && prefs.backup_ready && retry_due && chrono::Utc::now().timestamp()-prefs.last_backup.unwrap_or(0)>=interval.as_secs()as i64&&!busy.contains("backup")&&jobs.len()<NETWORK_CONCURRENCY {
                                last_backup_attempt = Some((target.clone(), Instant::now()));
                                busy.insert("backup".into());let _=output.send(Event::Busy("backup".into(),true)).await;
                                let engine=engine.clone();let output=output.clone();jobs.spawn(async move{let _slot = engine.provider_slots.acquire().await; (Some("backup".into()),engine.execute(Command::AutomaticBackup(target),output).await)});
                            }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn manual_refresh_has_a_coalescing_queue_independent_of_provider_backpressure() {
        let (sender, mut inputs) = CommandSender::channel();
        for _ in 0..CHANNEL_CAPACITY {
            sender.try_send(Command::LoadImages(vec![])).unwrap();
        }
        assert!(sender.try_send(Command::LoadImages(vec![])).is_err());
        for _ in 0..100 {
            sender.try_send(Command::Sync).unwrap();
        }
        assert_eq!(inputs.sync.len(), 1);
        assert!(matches!(inputs.sync.recv().await, Some(Command::Sync)));
        assert!(inputs.sync.try_recv().is_err());
        sender.try_send(Command::Sync).unwrap();
        assert!(matches!(inputs.sync.recv().await, Some(Command::Sync)));
        drop(inputs);
        assert!(sender.try_send(Command::Sync).is_err());
    }

    struct Running(tokio::task::JoinHandle<()>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    #[tokio::test]
    async fn cached_reads_and_ordered_saves_complete_with_all_network_jobs_and_queue_occupied() {
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(directory.path().join("cache.sqlite")).unwrap();
        let mail = parse_mail("test-account", "1", "INBOX",
            b"From: Test <test@example.com>\r\nTo: reader@example.com\r\nSubject: Still readable\r\n\r\nCached content while the server is unavailable.".to_vec(), true, false).unwrap();
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await.unwrap();
        let engine = Engine {
            store: store.clone(),
            google: Default::default(),
            demo: true,
            account_work: Default::default(),
            calendar_work: Default::default(),
            calendar_setup_lock: Default::default(),
            connection_lifecycle_lock: Default::default(),
            secret_remover: Arc::new(removals::OsSecretRemover),
            outbound: Arc::new(providers::outgoing::Servers),
            google_connection_lock: Default::default(),
            passphrases: Arc::new(backup::OsPassphraseStore),
            restore_credentials: Arc::new(backup::restore::OsCredentialRestorer),
            backup_uploads: Default::default(),
            mail_sync_settings: Default::default(),
            provider_slots: Default::default(),
            printing: Default::default(),
            bulk_control: Default::default(),
        };
        let (sender, input) = CommandSender::channel();
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let _running = Running(tokio::spawn(engine.run(input, output)));
        let started = Arc::new(tokio::sync::Barrier::new(NETWORK_CONCURRENCY + 1));
        let release = Arc::new(tokio::sync::Notify::new());
        for _ in 0..NETWORK_CONCURRENCY {
            sender
                .try_send(Command::HoldBackend {
                    started: started.clone(),
                    release: release.clone(),
                })
                .unwrap();
        }
        // This is an ordering/correctness test: provider jobs remain blocked until
        // after local work completes. The timeout only bounds a deadlocked test.
        tokio::time::timeout(Duration::from_secs(5), started.wait())
            .await
            .unwrap();
        for _ in 0..CHANNEL_CAPACITY {
            sender.try_send(Command::LoadImages(vec![])).unwrap();
        }
        assert!(matches!(
            sender.try_send(Command::LoadImages(vec![])),
            Err(error) if matches!(*error, mpsc::error::TrySendError::Full(_))
        ));
        sender
            .try_send(Command::Query(42, MailQuery::default(), false))
            .unwrap();
        sender
            .try_send(Command::Conversation(43, id.clone(), None, None))
            .unwrap();
        sender
            .try_send(Command::Detail {
                revision: 0,
                id: id.clone(),
                prefetch: false,
            })
            .unwrap();
        sender
            .try_send(Command::SavePreferences(
                1,
                Preferences {
                    reader_font_size: 12,
                    ..Default::default()
                },
            ))
            .unwrap();
        sender
            .try_send(Command::SavePreferences(
                2,
                Preferences {
                    reader_font_size: 18,
                    ..Default::default()
                },
            ))
            .unwrap();
        let draft = Draft {
            id: "saved-draft".into(),
            account_id: "test-account".into(),
            to: "friend@example.com".into(),
            subject: "First draft".into(),
            body: "First text".into(),
            ..Default::default()
        };
        sender
            .try_send(Command::AutoSaveDraft(draft.clone()))
            .unwrap();
        sender
            .try_send(Command::SaveDraft(Draft {
                body: "Latest text".into(),
                revision: 1,
                ..draft
            }))
            .unwrap();
        let selection = crate::store::MailSelectionId::default();
        sender
            .try_send(Command::Selection(
                44,
                selections::Request::Capture(selection, MailQuery::default()),
                vec![id.clone()],
            ))
            .unwrap();
        sender
            .try_send(Command::Selection(
                45,
                selections::Request::Change(selection, 0, crate::store::SelectionChange::All),
                vec![id.clone()],
            ))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut page, mut detail, mut saved, mut conversation, mut selected) =
                (false, false, false, false, false);
            while !(page && detail && saved && conversation && selected) {
                match events.next().await.expect("Dispatcher stopped") {
                    Event::Selection(45, result) => {
                        let snapshot = result.unwrap().unwrap();
                        assert_eq!(snapshot.selected, 1);
                        assert!(snapshot.visible.contains(&id));
                        selected = true;
                    }
                    Event::Page(42, result, false) => {
                        assert_eq!(result.total, 1);
                        page = true;
                    }
                    Event::Conversation(43, _, result) => {
                        assert_eq!(result.unwrap().total, 1);
                        conversation = true;
                    }
                    Event::Detail {
                        revision: 0,
                        result: Ok(result),
                        prefetch: false,
                        ..
                    } => {
                        assert_eq!(result.summary.subject, "Still readable");
                        detail = true;
                    }
                    Event::DraftSaved(_, 1, result) => {
                        result.unwrap();
                        saved = true;
                    }
                    Event::Error(error) => panic!("Local operation failed: {error}"),
                    _ => {}
                }
            }
        })
        .await
        .expect("Local work waited for blocked provider jobs");
        let workspace = store.workspace().await.unwrap();
        assert_eq!(workspace.preferences.reader_font_size, 18);
        assert_eq!(workspace.drafts.len(), 1);
        assert_eq!(workspace.drafts[0].body, "Latest text");
        sender
            .try_send(Command::DeleteDraft("saved-draft".into()))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Event::DraftDeleted(id, result) =
                    events.next().await.expect("Dispatcher stopped")
                {
                    assert_eq!(id, "saved-draft");
                    assert!(result.unwrap().drafts.is_empty());
                    break;
                }
            }
        })
        .await
        .expect("Discard waited for blocked provider jobs");
        assert!(store.workspace().await.unwrap().drafts.is_empty());
        sender
            .try_send(Command::ForwardDraft(
                id.clone(),
                "forward-without-network".into(),
            ))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Event::ForwardDraft(id, result) =
                    events.next().await.expect("Dispatcher stopped")
                {
                    assert_eq!(id, "forward-without-network");
                    assert_eq!(result.unwrap().drafts[0].subject, "Fwd: Still readable");
                    break;
                }
            }
        })
        .await
        .expect("Forward waited for blocked provider jobs");
        let destination = directory.path().join("export.sqlite");
        sender
            .try_send(Command::Database(crate::transfer::Request::Export {
                request: 78,
                destination: destination.clone(),
                replace: false,
            }))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Event::Database(78, crate::transfer::Update::Finished(result)) = events.next().await.unwrap() {
                    assert!(matches!(result.unwrap(), crate::transfer::Outcome::Saved { path, .. } if path == destination));
                    break;
                }
            }
        }).await.expect("Database export waited for blocked provider jobs");
        sender
            .try_send(Command::Print(77, id, Default::default()))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Event::Print(request, result) =
                    events.next().await.expect("Dispatcher stopped")
                {
                    assert_eq!(request, 77);
                    let preview = result.unwrap();
                    let response = reqwest::get(&preview.url)
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap();
                    assert!(response.contains("Still readable"));
                    break;
                }
            }
        })
        .await
        .expect("Print waited for blocked provider jobs");
        release.notify_waiters();
    }
}
