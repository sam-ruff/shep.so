use super::*;
use crate::profiles::{
    discovery::{Grant, Request},
    sync::{control, runner::Runner},
};
use shep_profile_core::drive::Drive;

pub(super) struct Background {
    runner: Option<Runner>,
    grant: Option<Grant>,
    pub deadline: tokio::time::Instant,
}
impl Default for Background {
    fn default() -> Self {
        Self {
            runner: None,
            grant: None,
            deadline: tokio::time::Instant::now() + Duration::from_secs(1),
        }
    }
}
impl Background {
    pub async fn close(&mut self) -> anyhow::Result<()> {
        self.grant = None;
        if let Some(runner) = self.runner.take() {
            runner.close().await?;
        }
        Ok(())
    }
    pub async fn command(
        &mut self,
        engine: &Engine,
        request: &Request,
        session: Option<&Session>,
        command: &control::Command,
    ) -> anyhow::Result<Observation> {
        let profile = match command {
            control::Command::Current => None,
            control::Command::Prepare(source) => {
                let session = session
                    .context("Open the completed profile review before choosing ongoing sync.")?;
                anyhow::ensure!(
                    session.panel == request.panel && session.grant == request.grant,
                    "Google setup changed. Reopen the completed profile review."
                );
                let _google = engine.google_connection_lock.try_read().context(
                    "Google connection is changing. Retry after sign-in or disconnect finishes.",
                )?;
                let prefs = engine.store.get("preferences").await?;
                request.grant.check(&prefs)?;
                self.close().await?;
                let subscription = control::prepare(
                    &engine.store,
                    session.root(),
                    session.scope(),
                    source.clone(),
                )
                .await?;
                Some(subscription.binding.storage_key()?)
            }
            control::Command::Enable {
                profile,
                revision,
                enabled,
            } => {
                let _google = if *enabled {
                    Some(engine.google_connection_lock.try_read().context("Google connection is changing. Retry after sign-in or disconnect finishes.")?)
                } else {
                    None
                };
                if *enabled {
                    let subscription = engine
                        .store
                        .profile_sync_subscription(profile.clone())
                        .await?;
                    control::check_grant(
                        &request.grant,
                        &engine.store.get("preferences").await?,
                        &subscription,
                    )?;
                    let namespace: String = engine.store.get("profile_namespace").await?;
                    anyhow::ensure!(
                        namespace == subscription.binding.namespace,
                        "Open the profile's application namespace before resuming sync."
                    );
                }
                engine
                    .store
                    .profile_sync_enable(profile.clone(), *revision, *enabled)
                    .await?;
                if !enabled {
                    self.close().await?;
                }
                Some(profile.clone())
            }
            control::Command::Field {
                profile,
                revision,
                key,
                enabled,
            } => {
                engine
                    .store
                    .profile_sync_field_enable(profile.clone(), *revision, *key, *enabled)
                    .await?;
                Some(profile.clone())
            }
            control::Command::Check { profile } => {
                let _google = engine.google_connection_lock.try_read().context(
                    "Google connection is changing. Retry after sign-in or disconnect finishes.",
                )?;
                let subscription = engine
                    .store
                    .profile_sync_subscription(profile.clone())
                    .await?;
                control::check_grant(
                    &request.grant,
                    &engine.store.get("preferences").await?,
                    &subscription,
                )?;
                anyhow::ensure!(
                    subscription.enabled,
                    "Enable this profile before checking for changes."
                );
                if let Some(runner) = &mut self.runner {
                    runner.rescan().await?;
                }
                engine
                    .store
                    .profile_sync_error(profile.clone(), None)
                    .await?;
                Some(profile.clone())
            }
        };
        if let Some(runner) = &mut self.runner {
            runner.wake();
        }
        self.deadline = tokio::time::Instant::now();
        let mut observation = if let Some(session) = session {
            session.observe(None).await?
        } else {
            Observation::default()
        };
        let mut sync = engine.store.profile_sync_observe(profile).await?;
        self.progress(&mut sync);
        observation.sync = Some(sync);
        Ok(observation)
    }
    fn progress(&self, observation: &mut control::Observation) {
        if let Some(runner) = &self.runner
            && observation
                .subscription
                .as_ref()
                .is_some_and(|s| s.binding == runner.binding)
        {
            observation.phase = Some(
                if observation.subscription.as_ref().unwrap().error.is_some() {
                    "Sync needs attention"
                } else {
                    runner.status()
                }
                .into(),
            );
            observation.queued = Some(runner.queued);
        }
    }
    pub async fn tick(
        &mut self,
        engine: &Engine,
        output: &mut Output,
        #[cfg(feature = "test-support")] fixture: Option<&crate::profiles::fixture::Fixture>,
    ) {
        self.deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let mut profile = None;
        let mut applied = None;
        let mut observed_grant = None;
        let result: anyhow::Result<Option<control::Observation>>=async {
            let Some(subscription)=engine.store.profile_sync_active().await? else {self.close().await?; return Ok(None)};
            let key=subscription.binding.storage_key()?;
            profile=Some(key.clone());
            // Frozen publication/enrollment reviews and accepted history steps
            // have one owner. Leaving Preferences does not stop ongoing sync.
            if engine.store.profile_sync_review_pending().await? {
                let mut observation=engine.store.profile_sync_observe(Some(key)).await?;
                observation.phase=Some("Waiting for the open profile review".into());
                return Ok(Some(observation));
            }
            let network=self.runner.as_ref().is_some_and(Runner::needs_network);
            let _slot=if network {
                match engine.provider_slots.try_acquire() {
                    Ok(slot)=>Some(slot),
                    Err(_)=>{self.deadline=tokio::time::Instant::now()+Duration::from_millis(250);return Ok(None)}
                }
            } else {None};
            // Never hold the lifecycle lock while waiting for provider capacity.
            let _google=match engine.google_connection_lock.try_read() {
                Ok(lock)=>lock,
                Err(_)=>{self.deadline=tokio::time::Instant::now()+Duration::from_millis(250);return Ok(None)}
            };
            let prefs:Preferences=engine.store.get("preferences").await?;
            let grant=Grant::from_preferences(&prefs);
            observed_grant=Some(grant.clone());
            control::check_grant(&grant,&prefs,&subscription)?;
            let namespace:String=engine.store.get("profile_namespace").await?;
            anyhow::ensure!(namespace==subscription.binding.namespace,"The application namespace changed. Reopen this profile's namespace before continuing sync.");
            if self.grant.as_ref()!=Some(&grant) || self.runner.as_ref().is_some_and(|r|r.binding!=subscription.binding) {
                self.close().await?;
            }
            if self.runner.is_none() {
                let root=if engine.demo {
                    #[cfg(feature="test-support")]
                    { fixture.context("Ongoing profile sync is unavailable in this preview.")?.root.path().join("discovery") }
                    #[cfg(not(feature="test-support"))]
                    { anyhow::bail!("Ongoing profile sync is unavailable in this preview.") }
                } else {
                    directories::ProjectDirs::from("so","shep","Shep").context("Could not locate the app data directory")?.data_local_dir().join("profiles/discovery")
                };
                let mut runner=Runner::open(root,&subscription).await?;
                // A reconnect may use a different Google project's app-data
                // space even though principal and namespace are unchanged.
                // Never continue its predecessor's change token as proof.
                if let Err(error)=runner.rescan().await {let _=runner.close().await;return Err(error);}
                self.runner=Some(runner);
                self.grant=Some(grant);
                // Opening/replacing an owner can change the next phase. Acquire
                // its provider slot on the next tick, never reuse the old phase.
                self.deadline=tokio::time::Instant::now()+Duration::from_millis(20);
                return Ok(Some(engine.store.profile_sync_observe(Some(key)).await?));
            }
            let runner=self.runner.as_mut().unwrap();
            runner.wake();
            let drive=if runner.needs_network() {
                if engine.demo {
                    #[cfg(feature="test-support")]
                    {Some(fixture.context("Profile sync is unavailable in this preview.")?.connect(namespace,subscription.binding.principal.as_str()).await?)}
                    #[cfg(not(feature="test-support"))]
                    {anyhow::bail!("Profile sync is unavailable in this preview.")}
                } else {
                    let token=engine.google.token_for(&prefs,providers::google::Service::Drive).await?;
                    Some(Drive::connect(token,namespace,Some(subscription.binding.principal.as_str())).await?)
                }
            } else {None};
            let step=runner.step(&engine.store,drive.as_ref()).await?;
            applied=step.applied.map(|(_,snapshot)|Arc::new(snapshot));
            if !step.idle {self.deadline=tokio::time::Instant::now()+Duration::from_millis(20);}
            let mut observation=engine.store.profile_sync_observe(Some(key)).await?;
            observation.phase=Some(runner.status().into());
            observation.queued=Some(runner.queued);
            Ok(Some(observation))
        }.await;
        let observation = match result {
            Ok(observation) => observation,
            Err(error) => {
                if let Some(profile) = profile {
                    let _ = engine
                        .store
                        .profile_sync_error(profile.clone(), Some(error.to_string()))
                        .await;
                    engine.store.profile_sync_observe(Some(profile)).await.ok()
                } else {
                    None
                }
            }
        };
        if let Some(mut observation) = observation {
            if observation.phase.is_none() {
                self.progress(&mut observation);
            }
            let _ = output
                .send(Event::ProfileSync(
                    observed_grant,
                    Arc::new(observation),
                    applied,
                ))
                .await;
        }
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests;
