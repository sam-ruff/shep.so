use super::*;
use futures::poll;

fn memory() -> Worker {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch("CREATE TABLE saved(value INTEGER NOT NULL)")
        .unwrap();
    Worker::new(connection).unwrap()
}

#[tokio::test]
async fn accepted_jobs_are_bounded_ordered_and_survive_cancelled_observers() {
    let worker = memory();
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut first = Box::pin(worker.run(move |_| {
        started.send(()).unwrap();
        held.blocking_recv().unwrap();
        Ok(())
    }));
    assert!(poll!(first.as_mut()).is_pending());
    waiting.await.unwrap();
    let mut accepted = Vec::new();
    for value in 0..CAPACITY {
        let mut job = Box::pin(worker.run(move |connection| {
            connection.execute("INSERT INTO saved VALUES(?)", [value as i64])?;
            Ok(())
        }));
        assert!(poll!(job.as_mut()).is_pending());
        accepted.push(job);
    }
    assert_eq!(worker.commands.capacity(), 0);
    let mut overflow = Box::pin(worker.run(|connection| {
        connection.execute("INSERT INTO saved VALUES(-1)", [])?;
        Ok(())
    }));
    assert!(poll!(overflow.as_mut()).is_pending());
    drop(overflow); // Never admitted; cannot become a write later.
    drop(accepted); // Admitted; acknowledgement receivers may disappear.
    release.send(()).unwrap();
    first.await.unwrap();
    let saved = worker
        .run(|connection| {
            Ok(connection
                .prepare("SELECT value FROM saved ORDER BY rowid")?
                .query_map([], |row| row.get::<_, i64>(0))?
                .collect::<Result<Vec<_>, _>>()?)
        })
        .await
        .unwrap();
    assert_eq!(saved, (0..CAPACITY as i64).collect::<Vec<_>>());
}

#[tokio::test]
async fn dropping_the_last_handle_still_drains_already_accepted_writes() {
    let worker = memory();
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut first = Box::pin(worker.run(move |_| {
        started.send(()).unwrap();
        held.blocking_recv().unwrap();
        Ok(())
    }));
    assert!(poll!(first.as_mut()).is_pending());
    waiting.await.unwrap();
    drop(first);
    let (committed, observed) = oneshot::channel();
    let mut save = Box::pin(worker.run(move |connection| {
        connection.execute("INSERT INTO saved VALUES(9)", [])?;
        let value: i64 = connection.query_row("SELECT value FROM saved", [], |r| r.get(0))?;
        committed.send(value).unwrap();
        Ok(())
    }));
    assert!(poll!(save.as_mut()).is_pending());
    drop(save);
    drop(worker);
    release.send(()).unwrap();
    assert_eq!(observed.await.unwrap(), 9);
}

#[tokio::test]
async fn one_connection_retains_temporary_tables_and_rolls_back_failed_transactions() {
    let worker = memory();
    worker
        .run(|connection| {
            connection.execute_batch("CREATE TEMP TABLE selection(id TEXT)")?;
            connection.execute("INSERT INTO selection VALUES('one')", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        worker
            .run::<()>(|connection| {
                let transaction = connection.transaction()?;
                transaction.execute("INSERT INTO saved VALUES(1)", [])?;
                anyhow::bail!("fixture transaction failure")
            })
            .await
            .is_err()
    );
    let counts = worker
        .run(|connection| {
            Ok((
                connection
                    .query_row("SELECT count(*) FROM selection", [], |r| r.get::<_, i64>(0))?,
                connection.query_row("SELECT count(*) FROM saved", [], |r| r.get::<_, i64>(0))?,
            ))
        })
        .await
        .unwrap();
    assert_eq!(counts, (1, 0));
}

#[tokio::test]
async fn local_leases_release_without_free_queue_space_and_cancelled_grants_do_not_leak() {
    let worker = memory();
    let lease = worker.lease("owned".into()).await.unwrap();
    assert!(worker.lease("owned".into()).await.is_err());
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut first = Box::pin(worker.run(move |_| {
        started.send(()).unwrap();
        held.blocking_recv().unwrap();
        Ok(())
    }));
    assert!(poll!(first.as_mut()).is_pending());
    waiting.await.unwrap();
    let mut abandoned = Box::pin(worker.lease("abandoned".into()));
    assert!(poll!(abandoned.as_mut()).is_pending());
    drop(abandoned);
    let mut queued = Vec::new();
    for _ in 1..CAPACITY {
        let mut job = Box::pin(worker.run(|_| Ok(())));
        assert!(poll!(job.as_mut()).is_pending());
        queued.push(job);
    }
    assert_eq!(worker.commands.capacity(), 0);
    drop(lease);
    release.send(()).unwrap();
    first.await.unwrap();
    for job in queued {
        job.await.unwrap();
    }
    let _again = worker.lease("owned".into()).await.unwrap();
    let _recovered = worker.lease("abandoned".into()).await.unwrap();
}

#[tokio::test]
async fn root_ownership_outlives_admitted_writes_and_the_last_handle() {
    use crate::cache_cipher::ownership::Guard;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owned.sqlite");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE saved(value INTEGER NOT NULL)")
        .unwrap();
    let guard = Arc::new(Guard::reader(directory.path()).unwrap());
    let worker = Worker::owned(connection, "shep-owned-fixture", Some(guard.clone())).unwrap();
    drop(guard);
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut first = Box::pin(worker.run(move |_| {
        started.send(()).unwrap();
        held.blocking_recv().unwrap();
        Ok(())
    }));
    assert!(poll!(first.as_mut()).is_pending());
    waiting.await.unwrap();
    drop(first);
    let (committed, observed) = oneshot::channel();
    let mut save = Box::pin(worker.run(move |connection| {
        connection.execute("INSERT INTO saved VALUES(4)", [])?;
        committed.send(()).unwrap();
        Ok(())
    }));
    assert!(poll!(save.as_mut()).is_pending());
    drop(save);
    drop(worker);
    // Every handle is gone, yet a migration cannot start beneath the
    // admitted write that is still running on the owning thread.
    assert!(Guard::migration(directory.path()).is_err());
    release.send(()).unwrap();
    observed.await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while Guard::migration(directory.path()).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "ownership never released"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let reopened = Connection::open(&path).unwrap();
    let saved: i64 = reopened
        .query_row("SELECT value FROM saved", [], |r| r.get(0))
        .unwrap();
    assert_eq!(saved, 4);
}

#[tokio::test]
async fn worker_failure_rejects_pending_and_new_requests() {
    let worker = memory();
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let mut first = Box::pin(worker.run::<()>(move |_| {
        started.send(()).unwrap();
        held.blocking_recv().unwrap();
        panic!("fixture worker failure")
    }));
    assert!(poll!(first.as_mut()).is_pending());
    waiting.await.unwrap();
    let mut pending = Box::pin(worker.run(|connection| {
        connection.execute("INSERT INTO saved VALUES(1)", [])?;
        Ok(())
    }));
    assert!(poll!(pending.as_mut()).is_pending());
    release.send(()).unwrap();
    assert!(first.await.is_err());
    assert!(pending.await.is_err());
    assert!(worker.run(|_| Ok(())).await.is_err());
}
