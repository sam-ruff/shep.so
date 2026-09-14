use super::*;
use std::collections::HashMap;
use tokio::time::Instant as Deadline;

/// Result key for failures that belong to the account listing rather than to
/// one account's check.
const LISTING: &str = "accounts";

#[derive(Clone)]
pub(super) struct Settings(tokio::sync::watch::Sender<u64>);
impl Default for Settings {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(5).0)
    }
}
impl Settings {
    pub(super) fn set(&self, seconds: u64) {
        self.0.send_if_modified(|current| {
            if *current == seconds {
                false
            } else {
                *current = seconds;
                true
            }
        });
    }
}

/// One independently scheduled unit of mail checking.
pub(super) trait SyncTarget: Clone + Send + Sync + 'static {
    fn key(&self) -> &str;
    fn name(&self) -> &str;
}

#[derive(Clone)]
pub(super) enum Target {
    Preview,
    Account(Box<Account>),
}
impl SyncTarget for Target {
    fn key(&self) -> &str {
        match self {
            Self::Preview => "preview",
            Self::Account(account) => &account.id,
        }
    }
    fn name(&self) -> &str {
        match self {
            Self::Preview => "Preview",
            Self::Account(account) => &account.name,
        }
    }
}

impl Engine {
    pub(super) async fn run_mail_sync(
        self,
        requests: mpsc::Receiver<Command>,
        output: Output,
        background: bool,
    ) {
        let lister = self.clone();
        let checker = self.clone();
        drive(
            self.mail_sync_settings.clone(),
            requests,
            output,
            background,
            move || {
                let engine = lister.clone();
                async move { engine.sync_targets().await }
            },
            move |target, events| {
                let engine = checker.clone();
                async move {
                    let _slot = engine.provider_slots.acquire().await;
                    engine.check_target(target, events).await
                }
            },
        )
        .await;
    }

    async fn sync_targets(&self) -> anyhow::Result<Vec<Target>> {
        if self.demo {
            return Ok(vec![Target::Preview]);
        }
        let accounts = self.store.accounts_ready_to_sync().await?;
        Ok(accounts
            .into_iter()
            .map(|account| Target::Account(Box::new(account)))
            .collect())
    }

    async fn check_target(&self, target: Target, output: Output) -> anyhow::Result<()> {
        match target {
            Target::Preview => self.check_preview(output).await,
            Target::Account(account) => self.check_account(*account, output).await,
        }
    }

    async fn check_preview(&self, mut output: Output) -> anyhow::Result<()> {
        #[cfg(feature = "test-support")]
        {
            if std::env::args().any(|arg| arg == "--held-account-sync") {
                return self.preview_held_account_sync(output).await;
            }
            let (round, arrival) = crate::test_support::sync_mail(&self.store).await?;
            if let Some(arrival) = arrival {
                output.send(Event::MailArrived(Arc::new(arrival))).await?;
            }
            output.send(Event::PreviewSync(round)).await?;
        }
        #[cfg(not(feature = "test-support"))]
        tokio::time::sleep(Duration::from_millis(1500)).await;
        output.send(Event::Changed).await?;
        Ok(())
    }

    /// Checks one account and publishes its cache changes, move recovery
    /// request and workspace without waiting for any other account.
    pub(super) async fn check_account(
        &self,
        account: Account,
        mut output: Output,
    ) -> anyhow::Result<()> {
        let result = self.sync_account(account, output.clone()).await;
        output.send(Event::Changed).await?;
        output.send(Event::PendingMovesReady).await?;
        self.workspace(&mut output).await?;
        output.send(Event::Changed).await?;
        result
    }
}

#[derive(Default)]
struct Slot {
    running: bool,
    started: Option<Deadline>,
    /// The manual request count this target's latest check has covered.
    served: u64,
}

/// Per-target scheduling state. Each target has its own cadence and a running
/// check never delays another target.
#[derive(Default)]
struct Schedule {
    slots: HashMap<String, Slot>,
    requests: u64,
    /// A manual request has arrived since the last account listing.
    pending: bool,
}

impl Schedule {
    fn request(&mut self) {
        self.requests += 1;
        self.pending = true;
    }

    /// Chooses the listed targets to start now, each with whether a manual
    /// request asked for it. Running targets are skipped until they finish.
    fn start<T: SyncTarget>(
        &mut self,
        targets: Vec<T>,
        now: Deadline,
        interval: Duration,
        background: bool,
    ) -> Vec<(T, bool)> {
        self.pending = false;
        self.slots
            .retain(|key, slot| slot.running || targets.iter().any(|target| target.key() == key));
        let mut started = Vec::new();
        for target in targets {
            let slot = self.slots.entry(target.key().to_owned()).or_default();
            if slot.running {
                continue;
            }
            let manual = slot.served < self.requests;
            let due = background && slot.started.is_none_or(|started| started + interval <= now);
            if !manual && !due {
                continue;
            }
            slot.running = true;
            slot.started = Some(now);
            slot.served = self.requests;
            started.push((target, manual));
        }
        started
    }

    fn finish(&mut self, key: &str) {
        if let Some(slot) = self.slots.get_mut(key) {
            slot.running = false;
        }
    }

    /// Every known target has started a check since the latest manual request.
    fn served(&self) -> bool {
        !self.pending && self.slots.values().all(|slot| slot.served == self.requests)
    }

    /// Treats waiting manual requests as answered when no check can start.
    fn settle(&mut self) {
        self.pending = false;
        for slot in self.slots.values_mut().filter(|slot| !slot.running) {
            slot.served = self.requests;
        }
    }

    fn next_due(&self, now: Deadline, interval: Duration) -> Deadline {
        self.slots
            .values()
            .filter(|slot| !slot.running)
            .filter_map(|slot| slot.started)
            .map(|started| started + interval)
            .min()
            .unwrap_or(now + interval)
    }
}

/// The production scheduling loop accepts object-scoped listing and checking
/// implementations so timer/overlap/error tests can use virtual time without
/// SQLite or sockets.
async fn drive<T, L, LF, R, RF>(
    settings_source: Settings,
    mut requests: mpsc::Receiver<Command>,
    mut output: Output,
    background: bool,
    list: L,
    run: R,
) where
    T: SyncTarget,
    L: Fn() -> LF,
    LF: std::future::Future<Output = anyhow::Result<Vec<T>>>,
    R: Fn(T, Output) -> RF,
    RF: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let mut settings = settings_source.0.subscribe();
    let mut interval = Duration::from_secs((*settings.borrow_and_update()).clamp(5, 3600));
    let mut schedule = Schedule::default();
    let mut jobs = tokio::task::JoinSet::new();
    let mut owners: HashMap<tokio::task::Id, (String, bool)> = HashMap::new();
    let mut manual_busy = false;
    let mut manual_running = 0usize;
    let mut background_running = 0usize;
    let mut listing_failed = false;
    let mut closed = false;
    let mut pass = true;
    let mut due = Deadline::now();
    loop {
        if pass && !closed {
            pass = false;
            while requests.try_recv().is_ok() {
                schedule.request();
            }
            if !manual_busy && schedule.pending {
                manual_busy = true;
                let _ = output.send(Event::Busy("sync".into(), true)).await;
            }
            let now = Deadline::now();
            if background || schedule.pending {
                match list().await {
                    Ok(targets) => {
                        if std::mem::take(&mut listing_failed) {
                            let _ = output
                                .send(Event::MailSyncFinished(LISTING.into(), Ok(())))
                                .await;
                        }
                        let was_idle = background_running == 0;
                        for (target, manual) in schedule.start(targets, now, interval, background) {
                            if manual {
                                manual_running += 1;
                            } else {
                                background_running += 1;
                            }
                            let name = target.name().to_owned();
                            let key = target.key().to_owned();
                            let work = run(target, output.clone());
                            let handle = jobs.spawn(async move {
                                tokio::time::timeout(Duration::from_secs(600), work)
                                    .await
                                    .context("Mail refresh timed out. Try Refresh again.")
                                    .and_then(|result| result)
                                    .with_context(|| format!("{name} sync failed"))
                            });
                            owners.insert(handle.id(), (key, manual));
                        }
                        if was_idle && background_running > 0 {
                            let _ = output
                                .send(Event::Busy("background-sync".into(), true))
                                .await;
                        }
                    }
                    Err(error) => {
                        listing_failed = true;
                        schedule.settle();
                        let _ = output
                            .send(Event::MailSyncFinished(
                                LISTING.into(),
                                Err(format!("{error:#}")),
                            ))
                            .await;
                    }
                }
            }
            if manual_busy && manual_running == 0 && schedule.served() {
                manual_busy = false;
                let _ = output.send(Event::Busy("sync".into(), false)).await;
            }
            due = schedule.next_due(now, interval);
        }
        if closed && jobs.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            finished = jobs.join_next_with_id(), if !jobs.is_empty() => {
                let Some(finished) = finished else { continue };
                let (id, result) = match finished {
                    Ok((id, result)) => (id, result),
                    Err(error) => (error.id(), Err(anyhow::anyhow!("Mail refresh stopped unexpectedly. Try Refresh again."))),
                };
                let Some((key, manual)) = owners.remove(&id) else { continue };
                schedule.finish(&key);
                if manual {
                    manual_running = manual_running.saturating_sub(1);
                } else {
                    background_running = background_running.saturating_sub(1);
                    if background_running == 0 {
                        let _ = output.send(Event::Busy("background-sync".into(), false)).await;
                    }
                }
                let _ = output.send(Event::MailSyncFinished(key, result.map_err(|error| format!("{error:#}")))).await;
                pass = true;
            }
            request = requests.recv(), if !closed => {
                if request.is_some() {
                    schedule.request();
                    if !manual_busy {
                        manual_busy = true;
                        let _ = output.send(Event::Busy("sync".into(), true)).await;
                    }
                    pass = true;
                } else {
                    closed = true;
                }
            }
            changed = settings.changed(), if !closed => {
                if changed.is_ok() {
                    interval = Duration::from_secs((*settings.borrow_and_update()).clamp(5, 3600));
                    due = schedule.next_due(Deadline::now(), interval);
                }
            }
            _ = tokio::time::sleep_until(due), if background && !closed => {
                pass = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn an_account_check_requests_pending_move_recovery_without_running_it() {
        use crate::mail_actions::{Fingerprint, MoveReceipt, journal::MoveRecord};
        let mut engine = super::super::calendar_tests::engine();
        engine.demo = false;
        let original = parse_mail(
            "fixture",
            "42.7",
            "INBOX",
            b"Subject: retained\r\n\r\nOriginal".to_vec(),
            true,
            false,
        )
        .expect("mail");
        engine
            .store
            .upsert(vec![original.clone()])
            .await
            .expect("cache");
        let mut receipt = MoveReceipt::server(
            &original.summary,
            "fixture",
            "Archive",
            None,
            Fingerprint::of(&original.raw),
        );
        receipt.connections = vec![("fixture".into(), "fixture-connection".into())];
        let record = MoveRecord::new(original.summary, receipt);
        engine
            .store
            .prepare_mail_move(record.clone())
            .await
            .expect("pending journal");
        // The account has no saved details, so the check fails before any
        // credential or network access and still requests recovery.
        let account: Account = serde_json::from_value(serde_json::json!({
            "id": "fixture", "name": "Fixture", "email": "fixture@example.test",
            "protocol": "Imap", "host": "localhost", "port": 993, "username": "fixture",
            "smtp_host": "localhost", "smtp_port": 465
        }))
        .expect("account");
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let result = engine.check_account(account, output).await;
        assert!(result.is_err(), "the unsaved fixture account cannot sync");
        let mut ready = 0;
        while let Some(event) = events.next().await {
            if matches!(event, Event::PendingMovesReady) {
                ready += 1;
            }
        }
        assert_eq!(ready, 1);
        let saved = engine
            .store
            .mail_move(record.token)
            .await
            .expect("retained journal");
        assert_eq!(
            saved.attempted, 0,
            "sync must not claim a mutable recovery attempt"
        );
        assert_eq!(
            saved.stage,
            crate::mail_actions::journal::MoveStage::Started
        );
    }

    #[derive(Clone)]
    struct Fixture {
        key: &'static str,
        seconds: u64,
        fail_first: bool,
        hang_first: bool,
    }
    impl Fixture {
        fn new(key: &'static str, seconds: u64) -> Self {
            Self {
                key,
                seconds,
                fail_first: false,
                hang_first: false,
            }
        }
        fn failing_first(mut self) -> Self {
            self.fail_first = true;
            self
        }
        fn hanging_first(mut self) -> Self {
            self.hang_first = true;
            self
        }
    }
    impl SyncTarget for Fixture {
        fn key(&self) -> &str {
            self.key
        }
        fn name(&self) -> &str {
            self.key
        }
    }

    struct Harness {
        requests: mpsc::Sender<Command>,
        events: futures::channel::mpsc::Receiver<Event>,
        accounts: Arc<Mutex<Vec<Fixture>>>,
        task: tokio::task::JoinHandle<()>,
        trace: Vec<String>,
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Harness {
        fn new(settings: Settings, background: bool, fixtures: Vec<Fixture>) -> Self {
            let (requests, input) = mpsc::channel(1);
            let (output, events) = futures::channel::mpsc::channel(64);
            let accounts = Arc::new(Mutex::new(fixtures));
            let listed = accounts.clone();
            let rounds: Arc<Mutex<HashMap<&'static str, usize>>> = Default::default();
            let task = tokio::spawn(drive(
                settings,
                input,
                output,
                background,
                move || {
                    let accounts = listed.lock().expect("fixtures").clone();
                    async move { Ok(accounts) }
                },
                move |fixture: Fixture, mut output| {
                    let round = {
                        let mut rounds = rounds.lock().expect("rounds");
                        let round = rounds.entry(fixture.key).or_default();
                        *round += 1;
                        *round
                    };
                    async move {
                        output
                            .send(Event::Notice(format!("{} started {round}", fixture.key)))
                            .await?;
                        if fixture.hang_first && round == 1 {
                            std::future::pending::<()>().await;
                        }
                        tokio::time::sleep(Duration::from_secs(fixture.seconds)).await;
                        anyhow::ensure!(!fixture.fail_first || round != 1, "Fixture sync failed");
                        output
                            .send(Event::Notice(format!("{} finished {round}", fixture.key)))
                            .await?;
                        Ok(())
                    }
                },
            ));
            Self {
                requests,
                events,
                accounts,
                task,
                trace: vec![],
            }
        }
        fn single(settings: Settings, background: bool, fail_first: bool) -> Self {
            let mut fixture = Fixture::new("a", 2);
            fixture.fail_first = fail_first;
            Self::new(settings, background, vec![fixture])
        }
        async fn event(&mut self, wanted: &str) {
            tokio::time::timeout(Duration::from_secs(700), async {
                loop {
                    let label = match self.events.next().await.expect("Sync worker stopped") {
                        Event::Busy(key, value) => format!("{key}:{value}"),
                        Event::Notice(text) => text,
                        Event::MailSyncFinished(key, Err(error)) => {
                            assert!(error.starts_with(&key), "{error} names its account");
                            format!("{key} error")
                        }
                        Event::MailSyncFinished(key, Ok(())) => format!("{key} ok"),
                        _ => continue,
                    };
                    self.trace.push(label.clone());
                    if label == wanted {
                        break;
                    }
                }
            })
            .await
            .expect("Expected sync transition did not happen");
        }
        fn count(&self, label: &str) -> usize {
            self.trace.iter().filter(|e| e.as_str() == label).count()
        }
        /// Closes the request channel and waits for the worker to stop.
        async fn close(mut self) {
            let (unused, _) = mpsc::channel(1);
            drop(std::mem::replace(&mut self.requests, unused));
            let task = std::mem::replace(&mut self.task, tokio::spawn(async {}));
            tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .expect("worker stops after its requests close")
                .unwrap();
        }
    }

    #[tokio::test(start_paused = true)]
    async fn background_checks_start_immediately_and_repeat_every_five_seconds() {
        let start = Deadline::now();
        let mut harness = Harness::single(Settings::default(), true, false);
        harness.event("a started 1").await;
        assert_eq!(Deadline::now(), start);
        harness.event("background-sync:false").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(2));
        harness.event("a started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(5));
        assert!(
            !harness.trace.iter().any(|event| event.starts_with("sync:")),
            "Automatic checks must not change the manual refresh state"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_during_background_coalesces_and_refresh_during_manual_runs_again() {
        let start = Deadline::now();
        let mut harness = Harness::single(Settings::default(), true, false);
        harness.event("a started 1").await;
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("sync:true").await;
        harness.requests.send(Command::Sync).await.unwrap();
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("a started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(2));
        // A new click during the follow-up is retained as one more check.
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("a started 3").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(4));
        harness.event("sync:false").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(6));
        assert_eq!(harness.count("sync:true"), 1);
        harness.event("a started 4").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(9));
    }

    #[tokio::test(start_paused = true)]
    async fn interval_changes_take_effect_without_restarting_and_manual_refresh_bypasses_the_wait()
    {
        let start = Deadline::now();
        let settings = Settings::default();
        settings.set(15);
        let mut harness = Harness::single(settings.clone(), true, false);
        harness.event("background-sync:false").await;
        settings.set(5);
        harness.event("a started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(5));
        harness.event("background-sync:false").await;
        settings.set(60);
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("a started 3").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(7));
        harness.event("sync:false").await;
        harness.event("a started 4").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(67));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_background_check_does_not_poison_manual_retry() {
        let mut harness = Harness::single(Settings::default(), true, true);
        harness.event("a error").await;
        assert!(harness.trace.contains(&"background-sync:false".to_owned()));
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("a finished 2").await;
        harness.event("sync:false").await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_slow_account_never_delays_a_fast_one() {
        let start = Deadline::now();
        let mut harness = Harness::new(
            Settings::default(),
            true,
            vec![Fixture::new("slow", 60), Fixture::new("fast", 1)],
        );
        harness.event("slow started 1").await;
        harness.event("fast started 1").await;
        assert_eq!(Deadline::now(), start);
        harness.event("fast started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(5));
        harness.event("fast started 12").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(55));
        assert_eq!(harness.count("slow started 1"), 1);
        assert!(
            !harness.trace.contains(&"background-sync:false".to_owned()),
            "background stays busy while any account is still checking"
        );
        harness.event("slow finished 1").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(60));
        harness.event("fast started 13").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(60));
        assert_eq!(harness.count("slow started 2"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_failing_account_does_not_stop_the_other() {
        let mut harness = Harness::new(
            Settings::default(),
            true,
            vec![
                Fixture::new("broken", 1).failing_first(),
                Fixture::new("fine", 1),
            ],
        );
        harness.event("fine started 3").await;
        assert_eq!(harness.count("broken error"), 1);
        assert_eq!(harness.count("broken finished 2"), 1);
        assert_eq!(harness.count("fine ok"), 2);
        assert_eq!(harness.count("fine error"), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn manual_refresh_waits_for_every_account_and_coalesces_clicks() {
        let start = Deadline::now();
        let mut harness = Harness::new(
            Settings::default(),
            true,
            vec![Fixture::new("slow", 8), Fixture::new("fast", 1)],
        );
        harness.event("fast finished 1").await;
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("sync:true").await;
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("fast started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(1));
        harness.event("fast finished 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(2));
        assert_eq!(harness.count("sync:false"), 0);
        // The slow account's manual check begins when its background check ends.
        harness.event("slow started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(8));
        harness.event("sync:false").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(16));
        assert_eq!(harness.count("sync:true"), 1);
        assert_eq!(harness.count("fast started 2"), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn a_timed_out_account_releases_only_itself() {
        let start = Deadline::now();
        let mut harness = Harness::new(
            Settings::default(),
            true,
            vec![
                Fixture::new("stuck", 1).hanging_first(),
                Fixture::new("fast", 1),
            ],
        );
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("stuck error").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(600));
        assert_eq!(harness.count("fast started 100"), 1);
        assert_eq!(harness.count("sync:true"), 1);
        // The manual refresh completes once the stuck account's check ends.
        harness.event("sync:false").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(600));
        harness.event("stuck finished 2").await;
        harness.close().await;
    }

    #[tokio::test(start_paused = true)]
    async fn accounts_added_or_removed_are_picked_up_on_the_next_check() {
        let mut harness = Harness::new(Settings::default(), true, vec![Fixture::new("a", 1)]);
        harness.event("a finished 1").await;
        harness.accounts.lock().unwrap().push(Fixture::new("b", 1));
        harness.event("b started 1").await;
        harness.accounts.lock().unwrap().remove(0);
        // A check already started in the same pass may finish; no later one begins.
        harness.event("b started 4").await;
        assert_eq!(harness.count("a started 1"), 1);
        assert_eq!(harness.count("a started 3"), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn manual_refresh_without_accounts_finishes_immediately() {
        let mut harness = Harness::new(Settings::default(), false, vec![]);
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("sync:true").await;
        harness.event("sync:false").await;
        assert!(harness.trace.iter().all(|event| !event.contains("started")));
    }

    #[test]
    fn unrelated_preference_saves_do_not_reschedule_checks() {
        let settings = Settings::default();
        let receiver = settings.0.subscribe();
        settings.set(5);
        assert!(!receiver.has_changed().unwrap());
        settings.set(15);
        assert!(receiver.has_changed().unwrap());
    }
}
