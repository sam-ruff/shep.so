use super::*;
use tokio::sync::{Notify, oneshot};
struct Held {
    scope: Scope,
    entered: Notify,
    release: Mutex<Option<oneshot::Receiver<()>>>,
    fail: bool,
}
#[async_trait]
impl Remote for Held {
    async fn publish(
        &self,
        _worker: &shep_profile_core::history::Worker,
        _catalog: &Discovery,
    ) -> Result<Option<Uuid>> {
        bail!("Publication is not configured in this fixture");
    }

    fn scope(&self) -> Scope {
        self.scope.clone()
    }
    async fn advance(&self, catalog: &Discovery) -> Result<State> {
        let release = self.release.lock().await.take().unwrap();
        self.entered.notify_one();
        release.await.unwrap();
        if self.fail {
            bail!("Old provider error");
        }
        Ok(catalog.state().await?)
    }
}
fn scope() -> Scope {
    Scope {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture-owner".into(),
    }
}
#[tokio::test]
async fn close_fences_held_data_and_errors_but_drains_the_accepted_owner() {
    for fail in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mail.sqlite");
        let runtime = Arc::new(Runtime::default());
        let (send, receive) = oneshot::channel();
        let remote = Arc::new(Held {
            scope: scope(),
            entered: Notify::new(),
            release: Mutex::new(Some(receive)),
            fail,
        });
        let id = Uuid::new_v4();
        runtime.begin(id).await.unwrap();
        runtime.activate(&path, id, remote.clone()).await.unwrap();
        let held = remote.entered.notified();
        let cloned = runtime.clone();
        let pending = tokio::spawn(async move { cloned.run(id, Command::Advance).await });
        held.await;
        assert_eq!(
            runtime.run(id, Command::State).await.unwrap()["phase"],
            "initial"
        );
        assert!(
            runtime
                .run(id, Command::Advance)
                .await
                .unwrap_err()
                .to_string()
                .contains("busy")
        );
        let cloned = runtime.clone();
        let closing = tokio::spawn(async move { cloned.close(id).await });
        // Observe the actual slot transition; close still owns its drain waiter.
        while runtime.slots.lock().await.active.is_some() {
            tokio::task::yield_now().await;
        }
        assert!(!closing.is_finished());
        assert!(runtime.run(id, Command::State).await.is_err());
        send.send(()).unwrap();
        assert!(
            pending
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("changed")
        );
        closing.await.unwrap().unwrap();
        let next = Uuid::new_v4();
        runtime.begin(next).await.unwrap();
        runtime.activate(&path, next, remote.clone()).await.unwrap();
        runtime.close(id).await.unwrap();
        assert_eq!(
            runtime.run(next, Command::State).await.unwrap()["phase"],
            "initial"
        );
        runtime.close(next).await.unwrap();
    }
}
#[tokio::test]
async fn a_closed_pending_connection_cannot_activate_or_close_its_replacement() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let runtime = Runtime::default();
    let old = Uuid::new_v4();
    runtime.begin(old).await.unwrap();
    runtime.close(old).await.unwrap();
    let new = Uuid::new_v4();
    runtime.begin(new).await.unwrap();
    let remote = Arc::new(Held {
        scope: scope(),
        entered: Notify::new(),
        release: Mutex::new(None),
        fail: false,
    });
    assert!(runtime.activate(&path, old, remote.clone()).await.is_err());
    runtime.abandon(old).await;
    runtime.activate(&path, new, remote).await.unwrap();
    runtime.close(old).await.unwrap();
    assert!(
        runtime
            .run(old, Command::Profiles { after: None })
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .run(new, Command::Profiles { after: None })
            .await
            .unwrap(),
        serde_json::json!([])
    );
    runtime.close(new).await.unwrap();
}

struct HeldPublication {
    entered: Notify,
    release: Mutex<Option<oneshot::Receiver<()>>>,
}
#[async_trait]
impl Remote for HeldPublication {
    fn scope(&self) -> Scope {
        scope()
    }
    async fn advance(&self, _catalog: &Discovery) -> Result<State> {
        bail!("No fixture discovery requested")
    }
    async fn publish(
        &self,
        worker: &shep_profile_core::history::Worker,
        _catalog: &Discovery,
    ) -> Result<Option<Uuid>> {
        use shep_profile_core::history::{Command, Reply};
        self.entered.notify_one();
        self.release.lock().await.take().unwrap().await.unwrap();
        let Reply::Upload(Some(upload)) = worker.request(Command::NextUpload).await? else {
            panic!()
        };
        let file = upload.operation.to_string();
        worker
            .request(Command::Reserve {
                operation: upload.operation,
                file_id: file.clone(),
            })
            .await?;
        worker
            .request(Command::Confirm {
                operation: upload.operation,
                file_id: file,
                sha256: upload.sha256,
            })
            .await?;
        Ok(Some(upload.operation))
    }
}
#[tokio::test]
async fn a_closed_google_session_finishes_accepted_publication_receipts_but_fences_late_ui_results()
{
    let (_directory, profile) = crate::tests::profile().await;
    let _held_provider_capacity = profile.operations.hold_network_capacity().await;
    let runtime = Arc::new(Runtime::default());
    let (send, receive) = oneshot::channel();
    let remote = Arc::new(HeldPublication {
        entered: Notify::new(),
        release: Mutex::new(Some(receive)),
    });
    let session = Uuid::new_v4();
    runtime.begin(session).await.unwrap();
    runtime
        .activate(&profile.database.path, session, remote.clone())
        .await
        .unwrap();
    let mut root = profile.database.path.as_os_str().to_owned();
    root.push(".profile-discovery");
    let catalog_path =
        PathBuf::from(root).join(format!("{}.sqlite", scope().storage_key().unwrap()));
    // Fixture observation starts with successful discovery, without Google traffic.
    let fixture = rusqlite::Connection::open(catalog_path).unwrap();
    fixture
        .execute("UPDATE state SET phase='complete'", [])
        .unwrap();
    drop(fixture);
    let id = Uuid::new_v4();
    let prepared = runtime
        .creation(
            &profile.database,
            session,
            creation::Command::Prepare {
                id,
                specification: creation::Specification {
                    name: "Held publication".into(),
                    include_accounts: false,
                    settings: Default::default(),
                },
            },
        )
        .await
        .unwrap();
    let binding: shep_profile_core::history::Binding =
        serde_json::from_value(prepared["binding"].clone()).unwrap();
    runtime
        .creation(
            &profile.database,
            session,
            creation::Command::Approve {
                id,
                settings: Default::default(),
            },
        )
        .await
        .unwrap();
    for _ in 0..3 {
        runtime
            .creation(&profile.database, session, creation::Command::Step { id })
            .await
            .unwrap();
    }
    let clone = runtime.clone();
    let db = profile.database.clone();
    let entered = remote.entered.notified();
    let pending = tokio::spawn(async move {
        clone
            .creation(&db, session, creation::Command::Step { id })
            .await
    });
    entered.await;
    assert_eq!(
        runtime
            .creation(&profile.database, session, creation::Command::Current)
            .await
            .unwrap()["phase"],
        "uploading"
    );
    assert!(
        runtime
            .run(session, Command::Advance)
            .await
            .unwrap_err()
            .to_string()
            .contains("busy")
    );
    let clone = runtime.clone();
    let closing = tokio::spawn(async move { clone.close(session).await });
    while runtime.slots.lock().await.active.is_some() {
        tokio::task::yield_now().await;
    }
    assert!(!closing.is_finished());
    send.send(()).unwrap();
    assert!(
        pending
            .await
            .unwrap()
            .unwrap_err()
            .to_string()
            .contains("changed")
    );
    closing.await.unwrap().unwrap();
    let next = Uuid::new_v4();
    runtime.begin(next).await.unwrap();
    runtime
        .activate(&profile.database.path, next, remote)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .creation(&profile.database, next, creation::Command::Current)
            .await
            .unwrap()["uploaded"],
        1
    );
    runtime.close(session).await.unwrap();
    assert!(runtime.run(next, Command::State).await.is_ok());
    runtime.close(next).await.unwrap();
    let mut root = profile.database.path.as_os_str().to_owned();
    root.push(".published-profiles");
    let worker = shep_profile_core::history::Worker::open(
        PathBuf::from(root).join(format!("{}.sqlite", binding.storage_key().unwrap())),
        binding,
    )
    .await
    .unwrap();
    let shep_profile_core::history::Reply::State(state) = worker
        .request(shep_profile_core::history::Command::State)
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(state.queued, 2);
    assert!(state.initialized);
    worker.close().await.unwrap();
}
