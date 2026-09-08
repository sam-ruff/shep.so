use super::*;
use crate::transfer::{Export, Outcome, Progress, Request, Update};
use futures::{FutureExt, future::BoxFuture};

enum Active {
    Starting(u64, BoxFuture<'static, anyhow::Result<Export>>),
    Copying(u64, Export),
}

fn request(
    active: &mut Option<Active>,
    command: Request,
    store: Store,
    _demo: bool,
) -> Option<Event> {
    match command {
        Request::Export {
            request,
            destination,
            replace,
        } => {
            if active.is_some() {
                return Some(Event::Database(
                    request,
                    Update::Finished(Err(
                        "Another database transfer is running. Finish or cancel it first.".into(),
                    )),
                ));
            }
            #[cfg(feature = "test-support")]
            if _demo && std::env::args().any(|argument| argument == "--hold-database-export") {
                *active = Some(Active::Starting(
                    request,
                    crate::transfer::export_fixture_database(store, destination, replace).boxed(),
                ));
                return None;
            }
            *active = Some(Active::Starting(
                request,
                crate::transfer::export_database(store, destination, replace).boxed(),
            ));
        }
        Request::Cancel(request) => match active {
            Some(Active::Starting(id, _)) if *id == request => {
                *active = None;
                return Some(Event::Database(
                    request,
                    Update::Finished(Ok(Outcome::Cancelled)),
                ));
            }
            Some(Active::Copying(id, job)) if *id == request => job.cancel(),
            _ => {}
        },
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
        let mut last_progress = Progress::default();
        let mut ticks = tokio::time::interval(Duration::from_millis(100));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let mut event = None;
            let mut command = None;
            match &mut active {
                None if closed => break,
                None => command = Some(input.recv().await),
                Some(Active::Starting(id, starting)) => {
                    tokio::select! {
                        biased;
                        next = input.recv(), if !closed => command = Some(next),
                        result = starting => {
                            let id = *id;
                            match result {
                                Ok(job) => {
                                    active = Some(Active::Copying(id, job));
                                    last_progress = Progress::default();
                                }
                                Err(error) => {
                                    active = None;
                                    event = Some(Event::Database(id, Update::Finished(Err(format!("{error:#}")))));
                                }
                            }
                        }
                    }
                }
                Some(Active::Copying(id, job)) => {
                    tokio::select! {
                        biased;
                        next = input.recv(), if !closed => command = Some(next),
                        result = job.finish() => {
                            event = Some(Event::Database(*id, Update::Finished(result.map_err(|error| format!("{error:#}")))));
                            active = None;
                        }
                        _ = ticks.tick() => {
                            let progress = *job.progress.borrow_and_update();
                            if progress != last_progress {
                                last_progress = progress;
                                event = Some(Event::Database(*id, Update::Progress(progress)));
                            }
                        }
                    }
                }
            }
            if let Some(command) = command {
                match command {
                    Some(Command::Database(value)) => {
                        event = request(&mut active, value, self.store.clone(), self.demo)
                    }
                    Some(_) => {}
                    None => {
                        closed = true;
                        match &mut active {
                            Some(Active::Copying(_, job)) => job.cancel(),
                            _ => active = None,
                        }
                    }
                }
            }
            if let Some(event) = event
                && output.send(event).await.is_err()
            {
                break; // Dropping an active handle cancels its private copy.
            }
        }
    }
}
