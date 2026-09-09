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
