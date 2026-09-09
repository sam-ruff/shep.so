use super::*;
use crate::transfer::{
    Export, Outcome, Request, Update,
    import::{Import, Installation, Prepared},
};
use futures::{FutureExt, future::BoxFuture};

#[derive(Clone, Copy)]
enum Kind {
    Export,
    Import,
}
enum Job {
    Export(Export),
    Import(Import),
    Install(Installation),
}
enum Completed {
    Export(Outcome),
    Review(Prepared),
    Installed(Option<crate::transfer::import::Installed>),
}
#[derive(Clone, Copy, PartialEq)]
enum Progress {
    Export(crate::transfer::Progress),
    Import(crate::transfer::import::Progress),
    Install(crate::transfer::import::InstallPhase),
}
impl Progress {
    fn update(self) -> Update {
        match self {
            Self::Export(p) => Update::Progress(p),
            Self::Import(p) => Update::ImportProgress(p),
            Self::Install(p) => Update::Installing(p),
        }
    }
}
impl Job {
    fn kind(&self) -> Kind {
        match self {
            Self::Export(_) => Kind::Export,
            _ => Kind::Import,
        }
    }
    fn cancel(&mut self) {
        match self {
            Self::Export(j) => j.cancel(),
            Self::Import(j) => j.cancel(),
            Self::Install(j) => j.cancel(),
        }
    }
    fn progress(&mut self) -> Progress {
        match self {
            Self::Export(j) => Progress::Export(*j.progress.borrow_and_update()),
            Self::Import(j) => Progress::Import(*j.progress.borrow_and_update()),
            Self::Install(j) => Progress::Install(*j.progress.borrow_and_update()),
        }
    }
    async fn finish(&mut self) -> anyhow::Result<Completed> {
        match self {
            Self::Export(j) => j.finish().await.map(Completed::Export),
            Self::Import(j) => j
                .finish()
                .await
                .map(|p| p.map_or(Completed::Installed(None), Completed::Review)),
            Self::Install(j) => j.finish().await.map(Completed::Installed),
        }
    }
}
enum Active {
    Starting(u64, Kind, BoxFuture<'static, anyhow::Result<Job>>),
    Working(u64, Job),
    Reviewing(u64, Prepared),
    Discarding(u64, BoxFuture<'static, ()>),
}
impl Active {
    fn id(&self) -> u64 {
        match self {
            Self::Starting(id, ..)
            | Self::Working(id, ..)
            | Self::Reviewing(id, ..)
            | Self::Discarding(id, ..) => *id,
        }
    }
}

fn failed(id: u64, kind: Kind, error: String) -> Event {
    Event::Database(
        id,
        match kind {
            Kind::Export => Update::Finished(Err(error)),
            Kind::Import => Update::ImportFinished(Err(error)),
        },
    )
}
fn cancel(active: &mut Option<Active>, request: u64) -> Option<Event> {
    if active.as_ref().is_none_or(|a| a.id() != request) {
        return None;
    }
    match active {
        Some(Active::Starting(_, kind, _)) => {
            let update = match kind {
                Kind::Export => Update::Finished(Ok(Outcome::Cancelled)),
                Kind::Import => Update::ImportFinished(Ok(None)),
            };
            *active = None;
            Some(Event::Database(request, update))
        }
        Some(Active::Working(_, job)) => {
            job.cancel();
            None
        }
        Some(Active::Reviewing(..)) => {
            if let Some(Active::Reviewing(id, prepared)) = active.take() {
                *active = Some(Active::Discarding(
                    id,
                    async move {
                        let _ = tokio::task::spawn_blocking(move || drop(prepared)).await;
                    }
                    .boxed(),
                ));
            }
            None
        }
        _ => None,
    }
}

fn request(active: &mut Option<Active>, command: Request, engine: &Engine) -> Option<Event> {
    const BUSY: &str = "Another database transfer is running. Finish or cancel it first.";
    match command {
        Request::Export {
            request,
            destination,
            replace,
        } => {
            if active.is_some() {
                return Some(failed(request, Kind::Export, BUSY.into()));
            }
            let store = engine.store.clone();
            let profile = engine.profiles.clone();
            let _demo = engine.demo;
            *active = Some(Active::Starting(
                request,
                Kind::Export,
                async move {
                    if let Some(profile) = profile {
                        profile
                            .catalog
                            .check_export_destination(destination.clone())
                            .await?;
                    }
                    #[cfg(feature = "test-support")]
                    if _demo && std::env::args().any(|arg| arg == "--hold-database-export") {
                        return crate::transfer::export_fixture_database(
                            store,
                            destination,
                            replace,
                        )
                        .await
                        .map(Job::Export);
                    }
                    crate::transfer::export_database(store, destination, replace)
                        .await
                        .map(Job::Export)
                }
                .boxed(),
            ));
        }
        Request::Import { request, source } => {
            if active.is_some() {
                return Some(failed(request, Kind::Import, BUSY.into()));
            }
            if engine.profiles.is_none() {
                return Some(failed(
                    request,
                    Kind::Import,
                    "Import requires a saved workspace".into(),
                ));
            }
            let store = engine.store.clone();
            let _demo = engine.demo;
            *active = Some(Active::Starting(
                request,
                Kind::Import,
                async move {
                    #[cfg(feature = "test-support")]
                    if _demo {
                        let hold = std::env::args().any(|arg| arg == "--hold-database-import");
                        return crate::transfer::import::stage_fixture(store, source, hold)
                            .await
                            .map(Job::Import);
                    }
                    crate::transfer::import::stage(store, source)
                        .await
                        .map(Job::Import)
                }
                .boxed(),
            ));
        }
        Request::Install { request, name } => {
            if !matches!(active, Some(Active::Reviewing(id, _)) if *id == request) {
                return None;
            }
            if let Err(error) = crate::profiles::name_checked(name.clone()) {
                return Some(Event::Database(
                    request,
                    Update::ReviewError(error.to_string()),
                ));
            }
            let session = engine.profiles.clone()?;
            let store = engine.store.clone();
            if let Some(Active::Reviewing(id, prepared)) = active.take() {
                *active = Some(Active::Starting(
                    id,
                    Kind::Import,
                    async move {
                        let local = store.get::<Preferences>("preferences").await?;
                        prepared
                            .install(session.catalog, name, local)
                            .map(Job::Install)
                    }
                    .boxed(),
                ));
            }
        }
        Request::Cancel(request) => return cancel(active, request),
    }
    None
}

impl Engine {
    pub(super) async fn run_database_transfers(
        self,
        mut input: mpsc::Receiver<Command>,
        mut output: Output,
    ) {
        let mut active = None;
        let mut closed = false;
        let mut last_progress = None;
        let mut ticks = tokio::time::interval(Duration::from_millis(100));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let mut event = None;
            let mut command = None;
            match &mut active {
                None if closed => break,
                None | Some(Active::Reviewing(..)) => command = Some(input.recv().await),
                Some(Active::Starting(id, kind, starting)) => {
                    tokio::select! { biased;
                        next = input.recv(), if !closed => command = Some(next),
                        result = starting => {
                            let (id, kind) = (*id, *kind);
                            match result {
                                Ok(job) => { active = Some(Active::Working(id, job)); last_progress = None; }
                                Err(error) => { active = None; event = Some(failed(id, kind, format!("{error:#}"))); }
                            }
                        }
                    }
                }
                Some(Active::Working(id, job)) => {
                    tokio::select! { biased;
                        next = input.recv(), if !closed => command = Some(next),
                        result = job.finish() => {
                            let (id, kind) = (*id, job.kind());
                            active = None;
                            event = Some(match result {
                                Ok(Completed::Review(prepared)) => {
                                    let event = Event::Database(id, Update::Review(Arc::new(prepared.review.clone())));
                                    active = Some(Active::Reviewing(id, prepared));
                                    event
                                }
                                Ok(Completed::Export(result)) => Event::Database(id, Update::Finished(Ok(result))),
                                Ok(Completed::Installed(result)) => Event::Database(id, Update::ImportFinished(Ok(result))),
                                Err(error) => failed(id, kind, format!("{error:#}")),
                            });
                        }
                        _ = ticks.tick() => {
                            let progress = job.progress();
                            if Some(progress) != last_progress { last_progress = Some(progress); event = Some(Event::Database(*id, progress.update())); }
                        }
                    }
                }
                Some(Active::Discarding(id, discarded)) => {
                    tokio::select! { biased;
                        next = input.recv(), if !closed => command = Some(next),
                        _ = discarded => { event = Some(Event::Database(*id, Update::ImportFinished(Ok(None)))); active = None; }
                    }
                }
            }
            if let Some(command) = command {
                match command {
                    Some(Command::Database(value)) => event = request(&mut active, value, &self),
                    Some(_) => {}
                    None => {
                        closed = true;
                        if let Some(id) = active.as_ref().map(Active::id) {
                            event = cancel(&mut active, id);
                        }
                    }
                }
            }
            if let Some(event) = event
                && output.send(event).await.is_err()
            {
                break;
            }
        }
    }
}
