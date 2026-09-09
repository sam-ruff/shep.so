mod background;
use super::*;
use crate::profiles::discovery::{Action, Observation, Session};

impl Engine {
    pub(super) async fn run_profiles(self, mut input: mpsc::Receiver<Command>, mut output: Output) {
        let mut session: Option<Session> = None;
        let mut background = background::Background::default();
        #[cfg(feature = "test-support")]
        let fixture = if self.demo && std::env::args().any(|arg| arg == "--profile-discovery") {
            if std::env::args().any(|arg| arg == "--profile-sync") {
                crate::profiles::fixture::Fixture::start_sync(Duration::from_millis(600))
                    .await
                    .ok()
            } else if std::env::args().any(|arg| arg == "--profile-pages") {
                crate::profiles::fixture::Fixture::start_paged(true, Duration::from_millis(600))
                    .await
                    .ok()
            } else {
                crate::profiles::fixture::Fixture::start(2, true, Duration::from_millis(600))
                    .await
                    .ok()
            }
        } else {
            None
        };
        loop {
            let command = tokio::select! {
                biased;
                command=input.recv()=>match command {Some(command)=>command,None=>break},
                _=tokio::time::sleep_until(background.deadline)=>{
                    background.tick(&self,&mut output,#[cfg(feature="test-support")] fixture.as_ref()).await;
                    continue;
                }
            };
            let Command::Profiles(request) = command else {
                continue;
            };
            let result = async {
                if let Action::Sync(command) = &request.action {
                    return background
                        .command(&self, &request, session.as_ref(), command)
                        .await;
                }
                if matches!(request.action, Action::Close) {
                    if session.as_ref().is_some_and(|s| s.panel == request.panel) {
                        session.take().unwrap().close().await?;
                    }
                    return Ok(Observation::default());
                }
                if matches!(request.action, Action::Load) {
                    return Ok(Observation {
                        namespace: self.store.get("profile_namespace").await?,
                        sync: Some(self.store.profile_sync_observe(None).await?),
                        ..Default::default()
                    });
                }
                // One accepted provider step retains the active grant; disconnect
                // takes the write side between steps. Never access the keychain
                // or Google before the cached workspace is ready.
                let network = matches!(request.action, Action::Open { .. } | Action::Advance)
                    || if let Action::Publication(command) = &request.action {
                        session
                            .as_ref()
                            .context("Reopen Profiles and sync.")?
                            .publication_network(&self.store, command)
                            .await?
                    } else {
                        false
                    };
                let _slot = if network {
                    Some(self.provider_slots.acquire().await)
                } else {
                    None
                };
                let _google = self.google_connection_lock.read().await;
                let prefs: Preferences = self.store.get("preferences").await?;
                request.grant.check(&prefs)?;
                let namespace = if let Action::Open { namespace } = &request.action {
                    namespace.clone()
                } else {
                    let active = session
                        .as_ref()
                        .context("Open profile discovery before continuing.")?;
                    anyhow::ensure!(
                        active.panel == request.panel && active.grant == request.grant,
                        "Google setup changed. Reopen profile discovery."
                    );
                    active.namespace.clone()
                };
                // Validate the application namespace before touching credentials.
                shep_profile_core::drive::catalog::Scope {
                    namespace: namespace.clone(),
                    principal: request.grant.principal().into(),
                }
                .storage_key()?;
                let drive = if network {
                    if self.demo {
                        #[cfg(feature = "test-support")]
                        {
                            Some(
                                fixture
                                    .as_ref()
                                    .context("Profile discovery is unavailable in this preview.")?
                                    .connect(namespace.clone(), request.grant.principal())
                                    .await?,
                            )
                        }
                        #[cfg(not(feature = "test-support"))]
                        {
                            anyhow::bail!("Profile discovery is unavailable in this preview.");
                        }
                    } else {
                        let token = self
                            .google
                            .token_for(&prefs, providers::google::Service::Drive)
                            .await?;
                        Some(
                            shep_profile_core::drive::Drive::connect(
                                token,
                                namespace.clone(),
                                Some(request.grant.principal()),
                            )
                            .await?,
                        )
                    }
                } else {
                    None
                };
                if matches!(request.action, Action::Open { .. }) {
                    if let Some(old) = session.take() {
                        old.close().await?;
                    }
                    let root = if self.demo {
                        #[cfg(feature = "test-support")]
                        {
                            fixture
                                .as_ref()
                                .context("Profile preview is unavailable.")?
                                .root
                                .path()
                                .join("discovery")
                        }
                        #[cfg(not(feature = "test-support"))]
                        {
                            anyhow::bail!("Profile preview is unavailable.");
                        }
                    } else {
                        directories::ProjectDirs::from("so", "shep", "Shep")
                            .context("Could not locate the app data directory")?
                            .data_local_dir()
                            .join("profiles/discovery")
                    };
                    session = Some(
                        Session::open(
                            root,
                            request.panel,
                            request.grant.clone(),
                            drive
                                .as_ref()
                                .context("Reconnect Google to discover profiles.")?,
                        )
                        .await?,
                    );
                    self.store.put("profile_namespace", namespace).await?;
                    let active = session.as_mut().unwrap();
                    let scope_key = active.scope().storage_key()?;
                    let binding_key = format!("profile_discovery_client:{scope_key}");
                    let previous: String = self.store.get(&binding_key).await?;
                    let client_changed = previous != request.grant.client_id();
                    active
                        .run_enrollment(&self.store, crate::profiles::enrollment::Command::Current)
                        .await?;
                    let opened = active
                        .run_publication(
                            &self.store,
                            crate::profiles::publication::Command::Current,
                            None,
                        )
                        .await?;
                    let refreshed = if let Some(state) = &opened.state
                        && ((state.phase == shep_profile_core::drive::catalog::Phase::Complete
                            && state.error.is_none())
                            || client_changed)
                    {
                        session
                            .as_mut()
                            .unwrap()
                            .run(
                                Action::Refresh {
                                    revision: state.revision,
                                    full: client_changed,
                                },
                                None,
                            )
                            .await
                    } else {
                        Ok(opened)
                    }?;
                    // Namespace is configured by the app project, not inferred
                    // from an OAuth client ID. A changed client forces a full
                    // scan; missing known records prevent publication.
                    if refreshed.error.is_none() {
                        self.store
                            .put(&binding_key, request.grant.client_id().to_owned())
                            .await?;
                    }
                    Ok(refreshed)
                } else if let Action::Enrollment(command) = &request.action {
                    session
                        .as_mut()
                        .context("Reopen Profiles and sync.")?
                        .run_enrollment(&self.store, command.clone())
                        .await
                } else if let Action::Publication(command) = &request.action {
                    session
                        .as_mut()
                        .context("Reopen Profiles and sync.")?
                        .run_publication(&self.store, command.clone(), drive.as_ref())
                        .await
                } else {
                    session
                        .as_mut()
                        .context("Reopen profile discovery.")?
                        .run(request.action.clone(), drive.as_ref())
                        .await
                }
            }
            .await;
            let _ = output
                .send(Event::Profiles(
                    request.panel,
                    request.serial,
                    result
                        .map(Arc::new)
                        .map_err(|e: anyhow::Error| e.to_string()),
                ))
                .await;
        }
        let _ = background.close().await;
        if let Some(active) = session {
            let _ = active.close().await;
        }
    }
}
