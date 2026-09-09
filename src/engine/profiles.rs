use super::*;
use crate::profiles::discovery::{Action, Observation, Session};

impl Engine {
    pub(super) async fn run_profiles(self, mut input: mpsc::Receiver<Command>, mut output: Output) {
        let mut session: Option<Session> = None;
        #[cfg(feature = "test-support")]
        let fixture = if self.demo && std::env::args().any(|arg| arg == "--profile-discovery") {
            crate::profiles::fixture::Fixture::start(2, true, Duration::from_millis(600))
                .await
                .ok()
        } else {
            None
        };
        while let Some(command) = input.recv().await {
            let Command::Profiles(request) = command else {
                continue;
            };
            let result = async {
                if matches!(request.action, Action::Close) {
                    if session.as_ref().is_some_and(|s| s.panel == request.panel) {
                        session.take().unwrap().close().await?;
                    }
                    return Ok(Observation::default());
                }
                if matches!(request.action, Action::Load) {
                    return Ok(Observation {
                        namespace: self.store.get("profile_namespace").await?,
                        ..Default::default()
                    });
                }
                // One accepted provider step retains the active grant; disconnect
                // takes the write side between steps. Never access the keychain
                // or Google before the cached workspace is ready.
                let network = matches!(request.action, Action::Open { .. } | Action::Advance);
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
                    let opened = session.as_ref().unwrap().observe(None).await?;
                    if let Some(state) = &opened.state
                        && state.phase == shep_profile_core::drive::catalog::Phase::Complete
                        && state.error.is_none()
                    {
                        session
                            .as_mut()
                            .unwrap()
                            .run(
                                Action::Refresh {
                                    revision: state.revision,
                                    full: false,
                                },
                                None,
                            )
                            .await
                    } else {
                        Ok(opened)
                    }
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
        if let Some(active) = session {
            let _ = active.close().await;
        }
    }
}
