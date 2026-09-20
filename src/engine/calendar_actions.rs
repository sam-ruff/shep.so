use super::*;
use crate::store::CalendarJob;

impl Engine {
    pub(super) async fn drain_calendar_jobs(&self, mut output: Output) {
        while !self.bulk_control.stopping.get() {
            let id = match self.store.next_calendar_action().await {
                Ok(Some(id)) => id,
                Ok(None) => break,
                Err(error) => {
                    let _ = output
                        .send(Event::Error(format!(
                            "Could not read pending calendar changes. {error:#}"
                        )))
                        .await;
                    break;
                }
            };
            self.bulk_control.active.set(true);
            let result = self.perform_calendar_job(&id, &mut output).await;
            self.bulk_control.active.set(false);
            match result {
                Ok(Some(job)) => {
                    let _ = output.send(Event::CalendarJob(id, Ok(Arc::new(job)))).await;
                }
                Ok(None) => {}
                Err(error) => {
                    if let Ok(mut job) = self.store.calendar_job(id.clone()).await {
                        if job.status == "running"
                            && let Ok(uncertain) = self.store.fail_calendar_action(id.clone(),job.revision,true,format!("The server result could not be recorded. Check this event. {error:#}")).await
                        {
                            job = uncertain;
                        }
                        let _ = output
                            .send(Event::CalendarJob(id.clone(), Ok(Arc::new(job))))
                            .await;
                    }
                    let _ = output
                        .send(Event::CalendarJob(id, Err(format!("{error:#}"))))
                        .await;
                    if self.bulk_control.stopping.get() {
                        let _ = output.send(Event::BulkStopped).await;
                    }
                    break;
                }
            }
            if self.bulk_control.stopping.get() {
                let _ = output.send(Event::BulkStopped).await;
                break;
            }
        }
    }

    async fn perform_calendar_job(
        &self,
        id: &str,
        output: &mut Output,
    ) -> anyhow::Result<Option<CalendarJob>> {
        if self.bulk_control.stopping.get() {
            return Ok(None);
        }
        let job = self.store.calendar_job(id.into()).await?;
        if job.status == "repair" {
            let _guard = self.calendar_access(&job.source.id).await;
            let saved = self.store.apply_calendar_receipt(id.into()).await?;
            self.send_calendar(output).await?;
            return Ok(Some(saved));
        }
        let _slot = tokio::select! {
            biased;
            _=self.bulk_control.stopping.requested()=>return Ok(None),
            slot=self.provider_slots.acquire()=>slot,
        };
        let _google = tokio::select! {
            biased;
            _=self.bulk_control.stopping.requested()=>return Ok(None),
            guard=self.google_connection_lock.read()=>guard,
        };
        let _guard = tokio::select! {
            biased;
            _=self.bulk_control.stopping.requested()=>return Ok(None),
            guard=self.calendar_access(&job.source.id)=>guard,
        };
        if self.bulk_control.stopping.get() {
            return Ok(None);
        }
        let claimed = match self.store.claim_calendar_action(id.into()).await {
            Ok(claimed) => claimed,
            Err(error) => {
                return Ok(Some(
                    self.store
                        .fail_calendar_action(id.into(), job.revision, false, format!("{error:#}"))
                        .await?,
                ));
            }
        };
        output
            .send(Event::CalendarJob(id.into(), Ok(Arc::new(claimed.clone()))))
            .await?;
        if self.demo {
            self.store
                .record_calendar_receipt(id.into(), claimed.revision, claimed.event.clone())
                .await?;
            let saved = self.store.apply_calendar_receipt(id.into()).await?;
            self.send_calendar(output).await?;
            return Ok(Some(saved));
        }
        let provider = match self.calendar_provider(&claimed.source).await {
            Ok(provider) => provider,
            Err(error) => {
                return Ok(Some(
                    self.store
                        .fail_calendar_action(
                            id.into(),
                            claimed.revision,
                            false,
                            format!("{error:#}"),
                        )
                        .await?,
                ));
            }
        };
        let saved = self
            .execute_calendar_step(provider.as_ref(), claimed)
            .await?;
        self.send_calendar(output).await?;
        Ok(Some(saved))
    }

    pub(super) async fn execute_calendar_step(
        &self,
        provider: &dyn CalendarProvider,
        job: CalendarJob,
    ) -> anyhow::Result<CalendarJob> {
        let result = if job.deleting {
            provider
                .delete_event(&job.source, &job.event)
                .await
                .map(|()| job.event.clone())
        } else {
            provider.save_event(&job.source, &job.event).await
        };
        match result {
            Ok(receipt) => {
                self.store
                    .record_calendar_receipt(job.id.clone(), job.revision, receipt)
                    .await?;
                self.store.apply_calendar_receipt(job.id).await
            }
            Err(error) => {
                if let Some(reason) = providers::calendar::mutation_wait_reason(&error) {
                    return self
                        .store
                        .wait_calendar_action(job.id, job.revision, reason, format!("{error:#}"))
                        .await;
                }
                self.store
                    .fail_calendar_action(
                        job.id,
                        job.revision,
                        providers::calendar::mutation_is_uncertain(&error),
                        format!("{error:#}"),
                    )
                    .await
            }
        }
    }

    pub(super) async fn check_calendar_job(
        &self,
        id: String,
        revision: u64,
        mut output: Output,
    ) -> anyhow::Result<()> {
        let result = async {
            let job = self.store.calendar_job(id.clone()).await?;
            anyhow::ensure!(job.revision == revision, "This calendar review changed. Open its current status.");
            let _google = self.google_connection_lock.read().await;
            let _guard = self.calendar_access(&job.source.id).await;
            let source = self.store.get::<Vec<CalendarSource>>("calendars").await?
                .into_iter().find(|source| source.id == job.source.id).context("The calendar was removed")?;
            anyhow::ensure!(source == job.source, "The calendar connection changed. Reconnect the original calendar to check this event.");
            let provider = self.calendar_provider(&source).await?;
            let checked = self.observe_calendar_job(provider.as_ref(), job).await?;
            self.send_calendar(&mut output).await?;
            Ok::<_,anyhow::Error>(checked)
        }.await;
        output
            .send(Event::CalendarJob(
                id,
                result.map(Arc::new).map_err(|error| format!("{error:#}")),
            ))
            .await?;
        Ok(())
    }

    async fn observe_calendar_job(
        &self,
        provider: &dyn CalendarProvider,
        job: CalendarJob,
    ) -> anyhow::Result<CalendarJob> {
        anyhow::ensure!(
            matches!(job.status.as_str(), "uncertain" | "repair"),
            "This calendar change does not need inspection."
        );
        let identity = job.receipt.as_ref().unwrap_or(&job.event);
        let current = provider.read_event(&job.source, identity).await?;
        self.store
            .check_calendar_action(job.id, job.revision, current)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn calendar_recovery_checks_only_eligible_identity_and_retains_failed_observation() {
        for mode in ["present", "absent", "fetch-error", "not-eligible"] {
            let engine = super::super::calendar_tests::engine();
            let source = CalendarSource {
                id: "home".into(),
                name: "Home".into(),
                kind: CalendarKind::CalDav,
                url: "https://calendar.example/home/".into(),
                username: "fixture".into(),
                access: Default::default(),
            };
            engine
                .store
                .put("calendars", vec![source])
                .await
                .expect("source");
            let mut event = super::super::calendar_tests::event("home");
            event.start -= chrono::Duration::days(500);
            event.end -= chrono::Duration::days(500);
            let queued = engine
                .store
                .admit_calendar_action("one".into(), event.clone(), false)
                .await
                .expect("admit");
            let job = if mode == "not-eligible" {
                queued
            } else {
                let claimed = engine
                    .store
                    .claim_calendar_action("one".into())
                    .await
                    .expect("claim");
                engine
                    .store
                    .fail_calendar_action(
                        "one".into(),
                        claimed.revision,
                        true,
                        "Lost response".into(),
                    )
                    .await
                    .expect("uncertain")
            };
            let mut provider = providers::MockCalendarProvider::new();
            if mode != "not-eligible" {
                provider
                    .expect_read_event()
                    .times(1)
                    .return_once(move |_, identity| {
                        assert_eq!(identity.id, event.id);
                        assert_eq!(identity.start, event.start);
                        match mode {
                            "fetch-error" => Err(anyhow::anyhow!("Offline")),
                            "absent" => Ok(None),
                            _ => Ok(Some(event)),
                        }
                    });
            }
            let result = engine.observe_calendar_job(&provider, job).await;
            let saved = engine
                .store
                .calendar_job("one".into())
                .await
                .expect("retained");
            assert_eq!(result.is_ok(), matches!(mode, "present" | "absent"));
            assert_eq!(saved.checked, matches!(mode, "present" | "absent"));
            assert_eq!(
                saved.status,
                if mode == "not-eligible" {
                    "queued"
                } else {
                    "uncertain"
                }
            );
        }
    }

    #[tokio::test]
    async fn calendar_provider_result_is_durable_before_cache_work() {
        for mode in [
            "save",
            "delete",
            "rejected",
            "cache-failure",
            "offline",
            "authentication",
        ] {
            let engine = super::super::calendar_tests::engine();
            let source = CalendarSource {
                id: "home".into(),
                name: "Home".into(),
                kind: CalendarKind::CalDav,
                url: "https://calendar.example/home/".into(),
                username: "fixture".into(),
                access: Default::default(),
            };
            engine
                .store
                .put("calendars", vec![source])
                .await
                .expect("source");
            let event = super::super::calendar_tests::event("home");
            engine
                .store
                .admit_calendar_action("one".into(), event.clone(), mode == "delete")
                .await
                .expect("admit");
            let job = engine
                .store
                .claim_calendar_action("one".into())
                .await
                .expect("claim");
            let mut provider = providers::MockCalendarProvider::new();
            if mode == "delete" {
                provider
                    .expect_delete_event()
                    .times(1)
                    .return_once(|_, _| Ok(()));
            } else {
                provider
                    .expect_save_event()
                    .times(1)
                    .return_once(move |_, _| {
                        if mode == "rejected" {
                            anyhow::bail!("Permission denied")
                        }
                        if mode == "offline" {
                            return Err(providers::calendar::WaitReason::Offline.into());
                        }
                        if mode == "authentication" {
                            return Err(providers::calendar::WaitReason::Authentication.into());
                        }
                        Ok(event)
                    });
            }
            if mode == "cache-failure" {
                engine
                    .store
                    .run(|c| {
                        c.execute("DROP TABLE events", [])?;
                        Ok(())
                    })
                    .await
                    .expect("break cache");
            }
            let result = engine.execute_calendar_step(&provider, job).await;
            let durable = engine
                .store
                .calendar_job("one".into())
                .await
                .expect("durable result");
            match mode {
                "offline" | "authentication" => {
                    assert_eq!(result.expect("waiting").status, "waiting");
                    assert_eq!(durable.retry_at.is_some(), mode == "offline");
                    assert!(durable.receipt.is_none());
                }
                "cache-failure" => {
                    assert!(result.is_err());
                    assert_eq!(durable.status, "repair");
                    assert!(durable.receipt.is_some());
                }
                "rejected" => {
                    assert_eq!(result.expect("rejection").status, "rejected");
                    assert!(durable.receipt.is_none());
                }
                _ => assert_eq!(result.expect("acknowledged").status, "succeeded"),
            }
        }
    }
}
