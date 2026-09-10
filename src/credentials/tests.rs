use super::*;
use std::collections::HashMap;

struct Fake {
    entries: HashMap<String, SecretString>,
    gate: Option<(oneshot::Sender<()>, oneshot::Receiver<()>)>,
    finished: Option<oneshot::Sender<HashMap<String, SecretString>>>,
}
impl Backend for Fake {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        anyhow::ensure!(!key.ends_with("locked"), "The fixture keychain is locked");
        Ok(self.entries.get(key).cloned())
    }
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        if let Some((started, held)) = self.gate.take() {
            started.send(()).unwrap();
            let _ = held.blocking_recv();
        }
        self.entries.insert(key.into(), value);
        Ok(())
    }
    fn delete(&mut self, key: &str) -> anyhow::Result<()> {
        self.entries.remove(key);
        Ok(())
    }
}
impl Drop for Fake {
    fn drop(&mut self) {
        if let Some(finished) = self.finished.take() {
            let _ = finished.send(std::mem::take(&mut self.entries));
        }
    }
}

fn fake() -> Fake {
    Fake {
        entries: Default::default(),
        gate: None,
        finished: None,
    }
}

#[tokio::test]
async fn cache_key_creation_is_ordered_verified_and_missing_keys_never_regenerate() {
    let credentials = Credentials::with_backend(Scope::CacheRoot(uuid::Uuid::new_v4()), fake());
    let keys = crate::cache_cipher::key_store::Keys::with_credentials(credentials.clone());
    assert!(keys.load().await.is_err());
    let (first, second) = tokio::join!(keys.create(), keys.create());
    let first = first.unwrap().encode();
    assert_eq!(first.as_str(), second.unwrap().encode().as_str());
    assert_eq!(first.as_str(), keys.load().await.unwrap().encode().as_str());
    // A portable profile's similarly named item cannot read or remove this key.
    let profile = Credentials {
        scope: Scope::Profile(uuid::Uuid::new_v4()),
        ..credentials.clone()
    };
    assert!(
        profile
            .read_optional("encryption-key-v1")
            .await
            .unwrap()
            .is_none()
    );
    profile.delete("encryption-key-v1").await.unwrap();
    assert_eq!(first.as_str(), keys.load().await.unwrap().encode().as_str());
    credentials.delete("encryption-key-v1").await.unwrap();
    assert!(keys.load().await.is_err());
    assert!(
        credentials
            .read_optional("encryption-key-v1")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cache_key_creation_does_not_bypass_locked_or_broken_credentials() {
    let credentials = Credentials::with_backend(Scope::CacheRoot(uuid::Uuid::new_v4()), fake());
    assert!(
        credentials
            .read_or_create("locked", "fixture".into())
            .await
            .is_err()
    );
    struct Broken;
    impl Backend for Broken {
        fn read(&mut self, _: &str) -> anyhow::Result<Option<SecretString>> {
            Ok(None)
        }
        fn write(&mut self, _: &str, _: SecretString) -> anyhow::Result<()> {
            Ok(())
        }
        fn delete(&mut self, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }
    let broken = Credentials::with_backend(Scope::CacheRoot(uuid::Uuid::new_v4()), Broken);
    assert!(
        broken
            .read_or_create("fixture", "value".into())
            .await
            .is_err()
    );
}

#[test]
fn portable_connections_cannot_alias_tokens_passphrases_smtp_or_each_other() {
    use crate::model::*;
    let account: Account = serde_json::from_value(serde_json::json!({"id":"work","name":"Work","email":"work@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"work","smtp_host":"smtp.example.test","smtp_port":465})).unwrap();
    for id in [
        "google-oauth",
        "GOOGLE-OAUTH",
        "backup-passphrase",
        "BACKUP-PASSPHRASE",
        "backup-passphrase:target",
        "work:smtp",
        "work\0",
        "caldav:work",
    ] {
        let mut changed = account.clone();
        changed.id = id.into();
        assert!(
            validate_connections(&[changed], &[]).is_err(),
            "accepted {id:?}"
        );
    }
    let mut collision = account.clone();
    collision.id = "WORK".into();
    assert!(validate_connections(&[account.clone(), collision], &[]).is_err());
    let mut calendar = CalendarSource {
        id: "WORK".into(),
        name: "Calendar".into(),
        kind: CalendarKind::CalDav,
        url: "https://calendar.example.test/".into(),
        username: "user".into(),
        access: Default::default(),
    };
    assert!(
        validate_connections(
            std::slice::from_ref(&account),
            std::slice::from_ref(&calendar)
        )
        .is_err()
    );
    calendar.id = format!("caldav:{}", "a".repeat(64));
    assert!(validate_connections(&[account], &[calendar]).is_ok());
}

#[tokio::test]
async fn imported_profiles_cannot_fall_back_to_or_delete_other_credentials() {
    let mut backend = fake();
    backend
        .entries
        .insert("account".into(), "legacy fixture".into());
    backend
        .entries
        .insert("google-oauth".into(), "legacy google fixture".into());
    let legacy = Credentials::with_backend(Scope::Legacy, backend);
    let first = Credentials {
        scope: Scope::Profile(uuid::Uuid::new_v4()),
        ..legacy.clone()
    };
    let second = Credentials {
        scope: Scope::Profile(uuid::Uuid::new_v4()),
        ..legacy.clone()
    };
    assert!(first.read_optional("account").await.unwrap().is_none());
    assert!(first.read_optional("google-oauth").await.unwrap().is_none());
    first
        .write("account", "first fixture".into())
        .await
        .unwrap();
    second
        .write("account", "second fixture".into())
        .await
        .unwrap();
    first.delete("account").await.unwrap();
    first.delete("google-oauth").await.unwrap();
    assert_eq!(
        legacy.read("account").await.unwrap().expose_secret(),
        "legacy fixture"
    );
    assert_eq!(
        legacy.read("google-oauth").await.unwrap().expose_secret(),
        "legacy google fixture"
    );
    assert_eq!(
        second.read("account").await.unwrap().expose_secret(),
        "second fixture"
    );
    let forged = second.scope.key("account");
    assert!(first.read_optional(&forged).await.unwrap().is_none());
}

#[tokio::test]
async fn restore_preserves_existing_passwords_and_does_not_treat_a_locked_store_as_missing() {
    let credentials = Credentials::with_backend(Scope::Legacy, fake());
    credentials
        .write("account", "existing fixture".into())
        .await
        .unwrap();
    credentials
        .restore_missing("account", "older fixture".into())
        .await
        .unwrap();
    assert_eq!(
        credentials.read("account").await.unwrap().expose_secret(),
        "existing fixture"
    );
    credentials
        .restore_missing("new-account", "restored fixture".into())
        .await
        .unwrap();
    assert_eq!(
        credentials
            .read("new-account")
            .await
            .unwrap()
            .expose_secret(),
        "restored fixture"
    );
    assert!(
        credentials
            .restore_missing("locked", "must not write".into())
            .await
            .is_err()
    );
    // A failed operation does not close or poison the owning worker.
    credentials.delete("new-account").await.unwrap();
    assert!(
        credentials
            .read_optional("new-account")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn cancelled_observers_and_last_handle_drop_still_drain_accepted_writes_in_order() {
    let (started, waiting) = oneshot::channel();
    let (release, held) = oneshot::channel();
    let (finished, result) = oneshot::channel();
    let credentials = Credentials::with_backend(
        Scope::Legacy,
        Fake {
            entries: Default::default(),
            gate: Some((started, held)),
            finished: Some(finished),
        },
    );
    let first = tokio::spawn({
        let credentials = credentials.clone();
        async move { credentials.write("account", "first fixture".into()).await }
    });
    waiting.await.unwrap();
    for index in 0..CAPACITY {
        let (reply, observer) = oneshot::channel();
        credentials
            .commands
            .try_send(Request {
                key: "account".into(),
                operation: Operation::Write(format!("queued fixture {index}").into()),
                reply,
            })
            .unwrap_or_else(|_| panic!("capacity must be available"));
        drop(observer);
    }
    assert_eq!(credentials.commands.capacity(), 0);
    // This read's future cannot enter the full queue; cancelling it must not
    // alter accepted work or create an extra unbounded queue behind the actor.
    assert!(futures::FutureExt::now_or_never(credentials.read("account")).is_none());
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    drop(credentials);
    release.send(()).unwrap();
    let entries = tokio::time::timeout(std::time::Duration::from_secs(5), result)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        entries["account"].expose_secret(),
        format!("queued fixture {}", CAPACITY - 1)
    );
}
