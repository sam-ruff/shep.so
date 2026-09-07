use super::*;
use tokio::time::Instant as Deadline;

#[derive(Clone)]
pub(super) struct Settings(tokio::sync::watch::Sender<u64>);
impl Default for Settings {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(15).0)
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

impl Engine {
    pub(super) async fn run_mail_sync(
        self,
        requests: mpsc::Receiver<Command>,
        output: Output,
        background: bool,
    ) {
        drive(
            self.mail_sync_settings.clone(),
            requests,
            output,
            background,
            move |events| {
                let engine = self.clone();
                async move {
                    let _slot = engine.provider_slots.acquire().await;
                    engine.execute(Command::Sync, events).await
                }
            },
        )
        .await;
    }
}

/// The production scheduling loop accepts an object-scoped cycle implementation
/// so timer/overlap/error tests can use virtual time without SQLite or sockets.
async fn drive<F, Fut>(
    settings_source: Settings,
    mut requests: mpsc::Receiver<Command>,
    mut output: Output,
    background: bool,
    run_cycle: F,
) where
    F: Fn(Output) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let mut settings = settings_source.0.subscribe();
    let mut interval = Duration::from_secs((*settings.borrow_and_update()).clamp(5, 3600));
    let mut jobs = tokio::task::JoinSet::new();
    let mut manual_pending = false;
    let mut manual_busy = false;
    let mut closed = false;
    let mut last_started: Option<Deadline> = None;
    let mut due = Deadline::now();
    loop {
        if jobs.is_empty() && requests.try_recv().is_ok() {
            manual_pending = true;
        }
        if jobs.is_empty() && !closed && (manual_pending || background && Deadline::now() >= due) {
            let manual = std::mem::take(&mut manual_pending);
            if manual && !manual_busy {
                manual_busy = true;
                let _ = output.send(Event::Busy("sync".into(), true)).await;
            } else if !manual {
                let _ = output
                    .send(Event::Busy("background-sync".into(), true))
                    .await;
            }
            last_started = Some(Deadline::now());
            let run_cycle = run_cycle.clone();
            let events = output.clone();
            jobs.spawn(async move {
                let result = tokio::time::timeout(Duration::from_secs(600), run_cycle(events))
                    .await
                    .context("Mail refresh timed out. Try Refresh again.")
                    .and_then(|result| result);
                (manual, result)
            });
        }
        if closed && jobs.is_empty() {
            break;
        }
        tokio::select! {
            biased;
            result = jobs.join_next(), if !jobs.is_empty() => {
                match result {
                    Some(Ok((manual, result))) => {
                        if !manual { let _ = output.send(Event::Busy("background-sync".into(), false)).await; }
                        let _ = output.send(Event::MailSyncFinished(result.map_err(|error|format!("{error:#}")))).await;
                    }
                    Some(Err(_)) => {
                        let _ = output.send(Event::Busy("background-sync".into(), false)).await;
                        let _ = output.send(Event::MailSyncFinished(Err("Mail refresh stopped unexpectedly. Try Refresh again.".into()))).await;
                    }
                    _ => {}
                }
                if requests.try_recv().is_ok() { manual_pending = true; }
                if manual_busy && !manual_pending {
                    manual_busy = false;
                    let _ = output.send(Event::Busy("sync".into(), false)).await;
                }
                due = last_started.unwrap_or_else(Deadline::now) + interval;
            }
            request = requests.recv(), if !closed => {
                if request.is_some() {
                    manual_pending = true;
                    if !manual_busy {
                        manual_busy = true;
                        let _ = output.send(Event::Busy("sync".into(), true)).await;
                    }
                } else {
                    closed = true;
                    manual_pending = false;
                }
            }
            changed = settings.changed(), if !closed => {
                if changed.is_ok() {
                    interval = Duration::from_secs((*settings.borrow_and_update()).clamp(5, 3600));
                    due = last_started.unwrap_or_else(Deadline::now) + interval;
                }
            }
            _ = tokio::time::sleep_until(due), if background && jobs.is_empty() && !closed => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Harness {
        requests: mpsc::Sender<Command>,
        events: futures::channel::mpsc::Receiver<Event>,
        task: tokio::task::JoinHandle<()>,
        trace: Vec<String>,
    }
    impl Drop for Harness {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Harness {
        fn new(settings: Settings, background: bool, fail_first: bool) -> Self {
            let (requests, input) = mpsc::channel(1);
            let (output, events) = futures::channel::mpsc::channel(32);
            let attempts = Arc::new(AtomicUsize::new(0));
            let task = tokio::spawn(drive(
                settings,
                input,
                output,
                background,
                move |mut output| {
                    let attempts = attempts.clone();
                    async move {
                        let round = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                        output
                            .send(Event::Notice(format!("started {round}")))
                            .await?;
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        anyhow::ensure!(!fail_first || round != 1, "Fixture sync failed");
                        output
                            .send(Event::Notice(format!("finished {round}")))
                            .await?;
                        Ok(())
                    }
                },
            ));
            Self {
                requests,
                events,
                task,
                trace: vec![],
            }
        }
        async fn event(&mut self, wanted: &str) {
            tokio::time::timeout(Duration::from_secs(120), async {
                loop {
                    let label = match self.events.next().await.expect("Sync worker stopped") {
                        Event::Busy(key, value) => format!("{key}:{value}"),
                        Event::Notice(text) => text,
                        Event::Error(_) | Event::MailSyncFinished(Err(_)) => "error".into(),
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
    }

    #[tokio::test(start_paused = true)]
    async fn background_checks_start_immediately_and_repeat_every_fifteen_seconds() {
        let start = Deadline::now();
        let mut harness = Harness::new(Settings::default(), true, false);
        harness.event("started 1").await;
        assert_eq!(Deadline::now(), start);
        harness.event("background-sync:false").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(2));
        harness.event("started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(15));
        assert!(
            !harness.trace.iter().any(|event| event.starts_with("sync:")),
            "Automatic checks must not change the manual refresh state"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn refresh_during_background_coalesces_and_refresh_during_manual_runs_again() {
        let start = Deadline::now();
        let mut harness = Harness::new(Settings::default(), true, false);
        harness.event("started 1").await;
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("sync:true").await;
        harness.requests.send(Command::Sync).await.unwrap();
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(2));
        // A new click during the follow-up is retained as one more cycle.
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("started 3").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(4));
        harness.event("sync:false").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(6));
        assert_eq!(
            harness
                .trace
                .iter()
                .filter(|e| e.as_str() == "sync:true")
                .count(),
            1
        );
        harness.event("started 4").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(19));
    }

    #[tokio::test(start_paused = true)]
    async fn interval_changes_take_effect_without_restarting_and_manual_refresh_bypasses_the_wait()
    {
        let start = Deadline::now();
        let settings = Settings::default();
        let mut harness = Harness::new(settings.clone(), true, false);
        harness.event("background-sync:false").await;
        settings.set(5);
        harness.event("started 2").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(5));
        harness.event("background-sync:false").await;
        settings.set(60);
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("started 3").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(7));
        harness.event("sync:false").await;
        harness.event("started 4").await;
        assert_eq!(Deadline::now(), start + Duration::from_secs(67));
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_background_check_does_not_poison_manual_retry() {
        let mut harness = Harness::new(Settings::default(), true, true);
        harness.event("error").await;
        assert!(harness.trace.contains(&"background-sync:false".to_owned()));
        harness.requests.send(Command::Sync).await.unwrap();
        harness.event("finished 2").await;
        harness.event("sync:false").await;
    }

    #[tokio::test(start_paused = true)]
    async fn a_timed_out_cycle_releases_the_worker_and_runs_the_queued_refresh() {
        let settings = Settings::default();
        let (requests, input) = mpsc::channel(1);
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let attempts = Arc::new(AtomicUsize::new(0));
        let observed = attempts.clone();
        let task = tokio::spawn(drive(settings, input, output, true, move |_| {
            let attempts = attempts.clone();
            async move {
                if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                    std::future::pending::<()>().await;
                }
                Ok(())
            }
        }));
        assert!(
            matches!(events.next().await, Some(Event::Busy(key, true)) if key == "background-sync")
        );
        requests.send(Command::Sync).await.unwrap();
        let mut timed_out = false;
        tokio::time::timeout(Duration::from_secs(605), async {
            while let Some(event) = events.next().await {
                match event {
                    Event::MailSyncFinished(Err(error)) => {
                        assert!(error.contains("timed out"));
                        timed_out = true;
                    }
                    Event::MailSyncFinished(Ok(())) => break,
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        assert!(timed_out);
        assert_eq!(observed.load(Ordering::SeqCst), 2);
        drop(requests);
        task.await.unwrap();
    }

    #[test]
    fn unrelated_preference_saves_do_not_reschedule_checks() {
        let settings = Settings::default();
        let receiver = settings.0.subscribe();
        settings.set(15);
        assert!(!receiver.has_changed().unwrap());
        settings.set(5);
        assert!(receiver.has_changed().unwrap());
    }
}
