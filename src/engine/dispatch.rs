use super::*;

const NETWORK_CONCURRENCY: usize = 8;

/// Independent bounded queues keep local interaction independent of slow providers.
#[derive(Clone, Debug)]
pub struct CommandSender {
    reads: mpsc::Sender<Command>,
    prefetch: mpsc::Sender<Command>,
    persistence: mpsc::Sender<Command>,
    network: mpsc::Sender<Command>,
}

pub(super) struct Inputs {
    reads: mpsc::Receiver<Command>,
    prefetch: mpsc::Receiver<Command>,
    persistence: mpsc::Receiver<Command>,
    network: mpsc::Receiver<Command>,
}

impl CommandSender {
    pub(super) fn channel() -> (Self, Inputs) {
        let (reads, read_input) = mpsc::channel(CHANNEL_CAPACITY);
        let (prefetch, prefetch_input) = mpsc::channel(8);
        let (persistence, persistence_input) = mpsc::channel(CHANNEL_CAPACITY);
        let (network, network_input) = mpsc::channel(CHANNEL_CAPACITY);
        (
            Self {
                reads,
                prefetch,
                persistence,
                network,
            },
            Inputs {
                reads: read_input,
                prefetch: prefetch_input,
                persistence: persistence_input,
                network: network_input,
            },
        )
    }

    pub fn try_send(
        &self,
        command: Command,
    ) -> Result<(), Box<mpsc::error::TrySendError<Command>>> {
        let channel = match &command {
            Command::Query(_, _, true) | Command::Detail { prefetch: true, .. } => &self.prefetch,
            Command::Query(..)
            | Command::Detail { .. }
            | Command::Conversation(..)
            | Command::RemovalPreview(..)
            | Command::OutgoingPage(..) => &self.reads,
            Command::SavePreferences(..)
            | Command::SaveDraft(_)
            | Command::AutoSaveDraft(_)
            | Command::RemoveDraftFile(..) => &self.persistence,
            _ => &self.network,
        };
        channel.try_send(command).map_err(Box::new)
    }
}

impl Engine {
    pub(super) async fn run(self, input: Inputs, output: Output) {
        tokio::join!(
            self.clone().run_reads(input.reads, output.clone(), 2),
            self.clone().run_reads(input.prefetch, output.clone(), 1),
            self.clone()
                .run_persistence(input.persistence, output.clone()),
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
        let mut last_sync = Instant::now();
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
                        // Uploads have bounded HTTP requests and progress checks,
                        // plus a durable journal. Do not cancel a healthy transfer
                        // merely because the whole archive takes over ten minutes.
                        // Restore also must observe its blocking SQLite commit;
                        // dropping its future cannot cancel that transaction.
                        let result = if matches!(&command, Command::Backup(..) | Command::AutomaticBackup(_) | Command::Restore(..) | Command::Send(_) | Command::ResolveOutgoing(..) | Command::RepairOutgoing | Command::IndexConversations | Command::ConnectCalendars(..) | Command::SaveAccount(..) | Command::RemoveConnection(..) | Command::CleanupCredentials | Command::RestoreGoogleCalendars) {
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
                        if last_sync.elapsed()>=Duration::from_secs(prefs.sync_minutes*60)&&!busy.contains("sync")&&jobs.len()<NETWORK_CONCURRENCY{
                            last_sync=Instant::now();busy.insert("sync".into());let _=output.send(Event::Busy("sync".into(),true)).await;
                            let worker=engine.clone();let events=output.clone();jobs.spawn(async move{(Some("sync".into()),worker.execute(Command::Sync,events).await)});
                            if !busy.contains("calendar") && jobs.len()<NETWORK_CONCURRENCY {
                                busy.insert("calendar".into());
                                let worker=engine.clone();let events=output.clone();jobs.spawn(async move{(Some("calendar".into()),worker.execute(Command::SyncCalendar,events).await)});
                            }
                        }
                        let target = BackupTarget::from_preferences(&prefs);
                        let interval = Duration::from_secs(prefs.backup_hours * 3600);
                        let retry_due = last_backup_attempt.as_ref().is_none_or(|(previous, at)| *previous != target || at.elapsed() >= interval);
                        if prefs.auto_backup && prefs.backup_ready && retry_due && chrono::Utc::now().timestamp()-prefs.last_backup.unwrap_or(0)>=interval.as_secs()as i64&&!busy.contains("backup")&&jobs.len()<NETWORK_CONCURRENCY {
                                last_backup_attempt = Some((target.clone(), Instant::now()));
                                busy.insert("backup".into());let _=output.send(Event::Busy("backup".into(),true)).await;
                                let engine=engine.clone();let output=output.clone();jobs.spawn(async move{(Some("backup".into()),engine.execute(Command::AutomaticBackup(target),output).await)});
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

    struct Running(tokio::task::JoinHandle<()>);
    impl Drop for Running {
        fn drop(&mut self) {
            self.0.abort();
        }
    }

    #[tokio::test]
    async fn cached_reads_and_ordered_saves_complete_with_all_network_jobs_and_queue_occupied() {
        let store = Store::memory().unwrap();
        let mail = parse_mail("test-account", "1", "INBOX",
            b"From: Test <test@example.com>\r\nTo: reader@example.com\r\nSubject: Still readable\r\n\r\nCached content while the server is unavailable.".to_vec(), true, false).unwrap();
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await.unwrap();
        let engine = Engine {
            store: store.clone(),
            google: Default::default(),
            demo: true,
            account_locks: Default::default(),
            calendar_locks: Default::default(),
            calendar_setup_lock: Default::default(),
            connection_lifecycle_lock: Default::default(),
            secret_remover: Arc::new(removals::OsSecretRemover),
            outbound: Arc::new(providers::outgoing::Servers),
            google_connection_lock: Default::default(),
            passphrases: Arc::new(backup::OsPassphraseStore),
            restore_credentials: Arc::new(backup::restore::OsCredentialRestorer),
            backup_uploads: Default::default(),
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
            sender.try_send(Command::Sync).unwrap();
        }
        assert!(matches!(
            sender.try_send(Command::Sync),
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
                id,
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
        tokio::time::timeout(Duration::from_secs(5), async {
            let (mut page, mut detail, mut saved, mut conversation) = (false, false, false, false);
            while !(page && detail && saved && conversation) {
                match events.next().await.expect("Dispatcher stopped") {
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
        release.notify_waiters();
    }
}
