use super::*;
use crate::profile_sync::{
    self as sync,
    commands::{Request, Update},
    control::Control,
};

const NAMESPACE: &str = "so.shep";
struct Active {
    id: u64,
    stop: tokio::sync::watch::Sender<bool>,
    job: tokio::task::JoinHandle<anyhow::Result<Update>>,
}
impl Engine {
    /// A single owning coordinator admits network work while continuing to
    /// service local options/status. It drains admitted writes on channel close.
    pub(super) async fn run_profile_sync(
        self,
        mut input: mpsc::Receiver<Command>,
        mut output: Output,
    ) {
        let mut active: Option<Active> = None;
        let mut closing = false;
        let mut stops = Vec::new();
        loop {
            tokio::select! {biased;
                completed=async {(&mut active.as_mut().expect("guarded active profile job").job).await},if active.is_some()=> {
                    let current=active.take().expect("completed active job");
                    let update=match completed {
                        Ok(Ok(update))=>update,
                        Ok(Err(error)) if error.is::<sync::control::Stopped>()=>Update::Stopped,
                        Ok(Err(error))=>Update::Failed(format!("{error:#}")),
                        Err(_)=>Update::Failed("Profile sync stopped unexpectedly. Reopen its review to recover saved work.".into()),
                    };
                    let failed=matches!(update,Update::Failed(_));
                    let _=output.send(Event::ProfileSync(current.id,update.clone())).await;
                    for request in stops.drain(..) {
                        let _=output.send(Event::ProfileSync(request,if failed {update.clone()} else {Update::Stopped})).await;
                    }
                }
                command=input.recv(),if !closing=> {
                    let Some(Command::ProfileSync(request))=command else {
                        closing=true;
                        if let Some(active)=&active {active.stop.send_replace(true);}
                        continue;
                    };
                    let id=request.id();
                    match request {
                        Request::Change {changes,..}=> {
                            if let Some(active)=&active {active.stop.send_replace(true);}
                            let update=self.store.change_profile_sync_options(changes).await.map(|s|Update::Status(Arc::new(s))).unwrap_or_else(|e|Update::Failed(format!("{e:#}")));
                            let _=output.send(Event::ProfileSync(id,update)).await;
                        }
                        Request::Page {review,after,..}=> {
                            let update=review.page(&self.store,after).await.map(|r|Update::Review(Arc::new(r))).unwrap_or_else(|e|Update::Failed(format!("{e:#}")));
                            let _=output.send(Event::ProfileSync(id,update)).await;
                        }
                        Request::Status(_)=> {
                            let update=self.store.profile_enrollment().await.map(|s|Update::Status(Arc::new(s))).unwrap_or_else(|e|Update::Failed(format!("{e:#}")));
                            if !matches!(&update,Update::Status(s) if s.available) && let Some(active)=&active {active.stop.send_replace(true);}
                            let _=output.send(Event::ProfileSync(id,update)).await;
                        }
                        Request::Options {revision,options,..}=> {
                            let update=self.store.set_profile_sync_options(revision,options).await.map(|s|Update::Status(Arc::new(s))).unwrap_or_else(|e|Update::Failed(format!("{e:#}")));
                            // A stop never depends on a held provider response.
                            if matches!(update,Update::Status(_)) && let Some(active)=&active {active.stop.send_replace(true);}
                            let _=output.send(Event::ProfileSync(id,update)).await;
                        }
                        Request::Stop(_)=> {
                            if let Some(active)=&active {
                                active.stop.send_replace(true);
                                if stops.len()<CHANNEL_CAPACITY {stops.push(id);} else {
                                    let _=output.send(Event::ProfileSync(id,Update::Failed("Too many pending stop requests. Wait for the current save.".into()))).await;
                                }
                            } else {let _=output.send(Event::ProfileSync(id,Update::Stopped)).await;}
                        }
                        action=> {
                            if active.is_some() {
                                let _=output.send(Event::ProfileSync(id,Update::Failed("A profile check is already running. Stop it or wait for its result.".into()))).await;
                            } else {
                                let engine=self.clone();
                                let (stop,control)=Control::channel();
                                let events=output.clone();
                                active=Some(Active {id,stop,job:tokio::spawn(async move {engine.profile_job(action,control,events).await})});
                            }
                        }
                    }
                }
                else=>break,
            }
            if closing && active.is_none() {
                break;
            }
        }
    }

    async fn profile_job(
        &self,
        action: Request,
        control: Control,
        mut output: Output,
    ) -> anyhow::Result<Update> {
        let request = action.id();
        if matches!(action, Request::AfterLogin(_)) {
            let snapshot = self.store.profile_enrollment().await?;
            if !sync::onboarding::eligible(&snapshot) {
                return Ok(Update::Status(Arc::new(snapshot)));
            }
        }
        #[cfg(feature = "test-support")]
        let fixture = if self.demo {
            std::env::args().find_map(|a| a.strip_prefix("--profile-drive-url=").map(str::to_owned))
        } else {
            None
        };
        #[cfg(not(feature = "test-support"))]
        let fixture: Option<String> = None;
        anyhow::ensure!(
            !self.demo || fixture.is_some(),
            "Cloud profile sync is disabled in preview."
        );
        let local = self
            .profiles
            .as_ref()
            .context("Profile sync needs a saved local workspace.")?;
        let paths = sync::paths::Paths::for_cache(&local.catalog.path(local.current))?;
        if let Request::JoinAccept { review, .. } = &action {
            let saved =
                sync::join::accept(&self.store, &paths, (**review).clone(), &control).await?;
            self.workspace(&mut output).await.context(
                "The profile was joined, but the view could not reload. Reopen Preferences.",
            )?;
            return Ok(Update::Joined(Arc::new(saved)));
        }
        let journal = paths.journal().await?;
        let _slot = control
            .read(async { Ok(self.provider_slots.acquire().await) })
            .await?;
        let preferences: Preferences = self.store.get("preferences").await?;
        let namespace = self
            .store
            .profile_enrollment()
            .await?
            .enrollment
            .selection
            .map_or_else(|| NAMESPACE.into(), |s| s.binding.namespace);
        // Serialize grant acquisition with the existing Google lifecycle. The
        // profile pass retains its verified token, never this guard, during HTTP.
        let session = control
            .read(async {
                #[cfg(feature = "test-support")]
                if let Some(url) = fixture {
                    return sync::drive::Session::fixture(&url, &preferences, namespace).await;
                }
                let _guard = self.google_connection_lock.read().await;
                sync::drive::Session::connect(&self.google, &preferences, namespace).await
            })
            .await?;
        match action {
            Request::Sync(_) => {
                let snapshot = self.store.profile_enrollment().await?;
                let selection = snapshot
                    .enrollment
                    .selection
                    .as_ref()
                    .context("Choose a shared profile first.")?;
                anyhow::ensure!(
                    selection.ready && snapshot.enrollment.options.enabled,
                    "Enable a completed shared profile before syncing."
                );
                let mut replica = sync::replica::Replica::open(
                    paths.history(&selection.binding)?,
                    selection.binding.clone(),
                    journal,
                )
                .await?;
                let result =
                    sync::continuous::run(&self.store, &mut replica, &session, &control).await;
                let closed = replica.close().await;
                // A later upload error must not hide already committed remote
                // changes from the native view. Do not reload an unchanged cache.
                let current = self.store.profile_enrollment().await?;
                if current.preferences_revision != snapshot.preferences_revision
                    || current.connections_revision != snapshot.connections_revision
                {
                    self.workspace(&mut output).await?;
                }
                closed.context("Could not finish saving profile history. Keep the original workspace for recovery.")?;
                let report = result?;
                Ok(Update::Synced {
                    snapshot: Arc::new(current),
                    report,
                })
            }
            Request::Discover(_) | Request::AfterLogin(_) => {
                let review = Arc::new(sync::setup::Discovery::from_catalog(
                    sync::catalog::discover(&self.store, &session, &paths, &control).await?,
                ));
                Ok(if matches!(action, Request::AfterLogin(_)) {
                    if !sync::onboarding::eligible(review.local()) {
                        return Ok(Update::Status(Arc::new(review.local().clone())));
                    }
                    Update::LoginReview(review)
                } else {
                    Update::Review(review)
                })
            }
            Request::AutoJoin { review, .. } => {
                let saved =
                    sync::onboarding::join(&self.store, &session, &paths, &review, &control)
                        .await?;
                self.workspace(&mut output).await.context(
                    "The shared profile was imported. Reopen Preferences to refresh its view.",
                )?;
                Ok(saved)
            }
            Request::JoinReview { review, cursor, .. } => Ok(Update::JoinReview(Arc::new(
                sync::join::prepare(
                    &self.store,
                    &session,
                    &paths,
                    journal,
                    &review,
                    &cursor,
                    &control,
                )
                .await?,
            ))),
            Request::Create {
                review,
                name,
                options,
                ..
            } => {
                control.check()?;
                let pending = sync::setup::create(
                    &self.store,
                    &session,
                    &journal,
                    (*review).clone(),
                    name,
                    options,
                )
                .await?;
                output
                    .send(Event::ProfileSync(
                        request,
                        Update::Pending(Arc::new(pending.clone())),
                    ))
                    .await?;
                self.publish_profile_seed(paths, journal, session, pending, control)
                    .await
            }
            Request::Resume(_) => {
                let pending = self.store.profile_enrollment().await?;
                output
                    .send(Event::ProfileSync(
                        request,
                        Update::Pending(Arc::new(pending.clone())),
                    ))
                    .await?;
                self.publish_profile_seed(paths, journal, session, pending, control)
                    .await
            }
            _ => anyhow::bail!("Local profile controls reached the provider worker."),
        }
    }

    async fn publish_profile_seed(
        &self,
        paths: sync::paths::Paths,
        journal: sync::journal::Journal,
        session: sync::drive::Session,
        pending: sync::enrollment::Snapshot,
        control: Control,
    ) -> anyhow::Result<Update> {
        let binding = pending
            .enrollment
            .selection
            .as_ref()
            .context("Choose a shared profile first.")?
            .binding
            .clone();
        let mut replica =
            sync::replica::Replica::open(paths.history(&binding)?, binding, journal).await?;
        let result = sync::setup::publish_controlled(
            &self.store,
            &mut replica,
            &session,
            pending,
            chrono::Utc::now().timestamp(),
            &control,
        )
        .await;
        // Releasing a UI observer cannot release the history file lease early.
        let closed = replica.close().await;
        closed.context("Could not finish saving profile history. Keep this workspace open and review its saved upload receipts.")?;
        let saved = result?;
        Ok(Update::Published(Arc::new(saved)))
    }
}
