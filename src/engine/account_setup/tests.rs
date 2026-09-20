use super::*;
use crate::credentials::{Backend, Credentials, Scope};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc::{self, Receiver, SyncSender},
};

struct Secrets {
    values: BTreeMap<String, SecretString>,
    events: SyncSender<(char, String)>,
    fail_write: bool,
}
impl Backend for Secrets {
    fn read(&mut self, key: &str) -> anyhow::Result<Option<SecretString>> {
        self.events.send(('r', key.into())).unwrap();
        Ok(self.values.get(key).cloned())
    }
    fn write(&mut self, key: &str, value: SecretString) -> anyhow::Result<()> {
        self.events.send(('w', key.into())).unwrap();
        anyhow::ensure!(!self.fail_write, "Fixture keychain write rejected");
        self.values.insert(key.into(), value);
        Ok(())
    }
    fn delete(&mut self, _: &str) -> anyhow::Result<()> {
        panic!("Setup cannot delete credentials")
    }
}
fn account() -> Account {
    Account {
        id: "shared-account".into(),
        name: "Shared account".into(),
        email: "alex@example.test".into(),
        protocol: Protocol::Imap,
        host: "incoming.example.test".into(),
        port: 993,
        username: "alex".into(),
        smtp_host: "outgoing.example.test".into(),
        smtp_port: 465,
        incoming_security: ConnectionSecurity::Tls,
        incoming_auth: Default::default(),
        smtp_security: Some(ConnectionSecurity::Tls),
        smtp_auth: Default::default(),
        smtp_username: "alex".into(),
        smtp_separate_password: false,
        sent_copy: Default::default(),
        sent_folder: "Sent".into(),
    }
}
fn engine(store: Store, fail_write: bool) -> (Engine, Receiver<(char, String)>) {
    let mut engine = super::super::calendar_tests::engine();
    engine.demo = false;
    engine.store = store;
    let mut tester = crate::profile_sync::vault::MockTester::new();
    tester.expect_test().returning(|_, _, _| Ok(()));
    engine.account_tester = Arc::new(tester);
    let (events, observed) = mpsc::sync_channel(32);
    engine.credentials = Credentials::with_backend(
        Scope::Legacy,
        Secrets {
            values: [
                ("shared-account".into(), secret("old-incoming")),
                ("shared-account:smtp".into(), secret("old-smtp")),
            ]
            .into(),
            events,
            fail_write,
        },
    )
    .with_account_store(engine.store.clone());
    (engine, observed)
}
fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

#[tokio::test]
async fn removal_during_incoming_probe_retires_late_result_without_smtp_or_ui_error() {
    struct Held {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl crate::profile_sync::vault::Tester for Held {
        async fn test(
            &self,
            _: &Account,
            target: ConnectionTarget,
            _: &SecretString,
        ) -> anyhow::Result<()> {
            assert_eq!(target, ConnectionTarget::Incoming);
            self.entered.notify_one();
            self.release.notified().await;
            Ok(())
        }
    }
    let store = Store::memory().expect("store");
    let original = account();
    store.save_account(original.clone()).await.expect("account");
    let attempt = store
        .admit_account_setup(
            uuid::Uuid::new_v4().to_string(),
            original.clone(),
            Some(original.clone()),
        )
        .await
        .expect("admission");
    let (mut engine, _observed) = engine(store.clone(), false);
    let held = Arc::new(Held {
        entered: Default::default(),
        release: Default::default(),
    });
    engine.account_tester = held.clone();
    let request = attempt.id.clone();
    let pending = tokio::spawn(async move {
        run(
            &engine,
            Command::ConnectAccount(request, secret("fresh"), secret("")),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), held.entered.notified())
        .await
        .expect("incoming started");
    let review = store
        .removal_preview(crate::store::ConnectionRef {
            kind: crate::store::ConnectionKind::Account,
            id: original.id.clone(),
        })
        .await
        .expect("review");
    store.remove_connection(review, true).await.expect("remove");
    held.release.notify_one();
    let events = tokio::time::timeout(Duration::from_secs(1), pending)
        .await
        .expect("late result settles")
        .expect("task")
        .expect("removed result is retired");
    assert!(store.account_setup(attempt.id).await.is_err());
    assert!(
        store
            .workspace()
            .await
            .expect("workspace")
            .accounts
            .iter()
            .all(|saved| saved.id != original.id)
    );
    assert!(!events.iter().any(|event| matches!(event,
        Event::AccountSetupChanged(saved) if saved.stage != crate::store::account_setup::Stage::Staged
    )));
}

#[tokio::test]
async fn account_setup_stop_drains_active_writes_but_not_the_connection_lifecycle() {
    let store = Store::memory().expect("store");
    let (engine, _) = engine(store.clone(), false);
    let attempt = store
        .admit_account_setup(uuid::Uuid::new_v4().to_string(), account(), None)
        .await
        .expect("admit");
    let unrelated = engine.connection_lifecycle.read().await;
    let active_write = engine.account_setup_writes.read().await;
    let mut stop = Box::pin(run(&engine, Command::InterruptAccountSetups(7)));
    assert!(futures::poll!(&mut stop).is_pending());
    drop(active_write);
    let events = tokio::time::timeout(Duration::from_secs(1), stop)
        .await
        .expect("unrelated provider read cannot delay setup drain")
        .expect("stopped");
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::AccountSetupsStopped(7, Ok(()))))
    );
    assert_eq!(
        store.account_setup(attempt.id).await.expect("saved").stage,
        crate::store::account_setup::Stage::Interrupted
    );
    drop(unrelated);
}

#[tokio::test]
async fn a_held_probe_allows_another_account_to_activate_and_close_interrupts_without_activating() {
    struct Held {
        entered: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl crate::profile_sync::vault::Tester for Held {
        async fn test(
            &self,
            account: &Account,
            _: ConnectionTarget,
            _: &SecretString,
        ) -> anyhow::Result<()> {
            if account.id == "shared-account" {
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            Ok(())
        }
    }
    let dir = tempfile::tempdir().expect("directory");
    let path = dir.path().join("accounts.sqlite");
    let store = Store::open(&path).expect("store");
    let original = account();
    store
        .save_account(original.clone())
        .await
        .expect("old account");
    let (mut engine, _observed) = engine(store, false);
    let held = Arc::new(Held {
        entered: Default::default(),
    });
    engine.account_tester = held.clone();
    let mut desired = original.clone();
    desired.host = "new.example.test".into();
    let first_engine = engine.clone();
    let first = tokio::spawn(async move {
        run(
            &first_engine,
            Command::SaveAccount(desired, secret("fresh"), secret("")),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(1), held.entered.notified())
        .await
        .expect("probe started");
    let mut other = original.clone();
    other.id = "other-account".into();
    let saved = tokio::time::timeout(
        Duration::from_secs(1),
        run(
            &engine,
            Command::SaveAccount(other.clone(), secret("other"), secret("")),
        ),
    )
    .await
    .expect("other account must not wait for the held probe")
    .expect("other activated");
    assert!(
        saved
            .iter()
            .any(|event| matches!(event, Event::AccountSaved(id) if id == &other.id))
    );
    assert_eq!(
        engine
            .store
            .account_credential_key(original.clone(), false)
            .await
            .expect("old active"),
        original.id
    );
    engine.bulk_control.stopping.set(true);
    let error = tokio::time::timeout(Duration::from_secs(1), first)
        .await
        .expect("close stops a read-only probe")
        .expect("task")
        .expect_err("interrupted");
    assert!(error.to_string().contains("interrupted"));
    let attempts = engine
        .store
        .account_setup_page(None)
        .await
        .expect("saved attempts");
    assert!(
        attempts
            .iter()
            .any(|attempt| attempt.account.id == original.id
                && attempt.stage == crate::store::account_setup::Stage::Interrupted)
    );
    drop(engine);
    let reopened = Store::open(path).expect("reopen");
    assert_eq!(
        reopened
            .account_credential_key(original.clone(), false)
            .await
            .expect("old connection retained"),
        original.id
    );
    assert_ne!(
        reopened
            .account_credential_key(other, false)
            .await
            .expect("independent account retained"),
        "other-account"
    );
}
async fn pending(store: &Store, account: &Account) {
    store.save_account(account.clone()).await.unwrap();
    store
        .put(
            crate::profile_sync::join::RECONNECT_KEY,
            BTreeSet::from([account.id.clone()]),
        )
        .await
        .unwrap();
}
async fn run(engine: &Engine, command: Command) -> anyhow::Result<Vec<Event>> {
    let (output, events) = futures::channel::mpsc::channel(32);
    engine.execute(command, output).await?;
    Ok(events.collect().await)
}

#[tokio::test]
async fn shared_account_setup_tests_refuse_old_keychain_credentials_before_network_access() {
    for (target, separate) in [
        (ConnectionTarget::Incoming, false),
        (ConnectionTarget::Smtp, false),
        (ConnectionTarget::Smtp, true),
    ] {
        let mut account = account();
        account.smtp_separate_password = separate;
        let store = Store::memory().unwrap();
        pending(&store, &account).await;
        let (engine, observed) = engine(store, false);
        let events = run(
            &engine,
            Command::TestConnection(account.clone(), secret(""), secret(""), target),
        )
        .await
        .unwrap();
        assert!(
            matches!(&events[..], [Event::ConnectionTest(t, Err(error))] if *t == target && error.contains("Reconnect"))
        );
        assert!(
            observed.try_recv().is_err(),
            "Old credentials must never be read"
        );
        assert!(
            engine
                .store
                .require_account_reconnected(account.id)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn shared_account_setup_blank_save_and_failed_write_retain_reconnection_across_restart() {
    for (incoming, smtp, fail_write) in [
        ("", "", false),
        ("fresh-incoming", "", false),
        ("fresh-incoming", "fresh-smtp", true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.sqlite");
        let mut account = account();
        account.smtp_separate_password = true;
        let store = Store::open(&path).unwrap();
        pending(&store, &account).await;
        let (engine, observed) = engine(store, fail_write);
        let error = run(
            &engine,
            Command::SaveAccount(account.clone(), secret(incoming), secret(smtp)),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains(if fail_write {
            "Fixture keychain"
        } else {
            "Reconnect"
        }));
        let calls: Vec<_> = observed.try_iter().collect();
        if fail_write {
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, 'w');
            assert!(calls[0].1.starts_with("shared-account:setup:"));
            assert!(calls[0].1.ends_with(":incoming"));
        } else {
            assert!(
                calls.is_empty(),
                "Resolve every secret before changing the keychain"
            );
        }
        drop(engine);
        let reopened = Store::open(&path).unwrap();
        assert!(
            reopened
                .require_account_reconnected(account.id)
                .await
                .is_err()
        );
        assert!(reopened.accounts_ready_to_sync().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn cached_mail_rejects_each_incoming_identity_edit_before_secret_writes() {
    let original = account();
    let store = Store::memory().expect("store");
    store.save_account(original.clone()).await.expect("account");
    let mail = crate::model::parse_mail(
        &original.id,
        "cached",
        "INBOX",
        b"Subject: Retained\r\n\r\nSaved body".to_vec(),
        true,
        false,
    )
    .expect("mail");
    store.upsert(vec![mail]).await.expect("cache");
    let edits: Vec<fn(&mut Account)> = vec![
        |a| a.protocol = Protocol::Pop3,
        |a| a.host = "other.example.test".into(),
        |a| a.host = a.host.to_uppercase(),
        |a| a.port = 1993,
        |a| a.username = "other".into(),
        |a| a.username = a.username.to_uppercase(),
        |a| a.incoming_security = ConnectionSecurity::StartTls,
        |a| a.incoming_auth = crate::model::IncomingAuth::Plain,
    ];
    let (engine, observed) = engine(store.clone(), false);
    for edit in edits {
        let mut changed = original.clone();
        edit(&mut changed);
        let (output, _) = futures::channel::mpsc::channel(16);
        let error = engine
            .execute(
                Command::SaveAccount(changed.clone(), secret("fresh"), secret("")),
                output,
            )
            .await
            .expect_err("migration required");
        assert!(error.to_string().contains("cached mail"), "{error:#}");
        assert!(
            store
                .admit_account_setup(
                    uuid::Uuid::new_v4().to_string(),
                    changed,
                    Some(original.clone())
                )
                .await
                .is_err()
        );
    }
    assert!(observed.try_recv().is_err(), "no credential read or write");
    assert!(
        store
            .account_setup_page(None)
            .await
            .expect("attempts")
            .is_empty()
    );
    let mut cosmetic = original.clone();
    cosmetic.name = "Renamed".into();
    cosmetic.email = "alias@example.test".into();
    cosmetic.sent_folder = "Sent Mail".into();
    cosmetic.smtp_host = "new-smtp.example.test".into();
    store
        .check_account_mailbox_identity(cosmetic)
        .await
        .expect("incoming identity retained");
}

#[tokio::test]
async fn shared_account_setup_explicit_save_reconnects_without_losing_cached_mail() {
    for (auth, separate) in [
        (SmtpAuth::None, true),
        (SmtpAuth::default(), false),
        (SmtpAuth::default(), true),
    ] {
        let mut account = account();
        account.smtp_auth = auth;
        account.smtp_separate_password = separate;
        let store = Store::memory().unwrap();
        pending(&store, &account).await;
        let mail = crate::model::parse_mail(
            &account.id,
            "cached",
            "INBOX",
            b"Subject: Retained\r\n\r\nSaved body".to_vec(),
            true,
            false,
        )
        .unwrap();
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await.unwrap();
        let (engine, observed) = engine(store, false);
        let smtp = if auth == SmtpAuth::None {
            ""
        } else {
            "fresh-smtp"
        };
        let events = run(
            &engine,
            Command::SaveAccount(account.clone(), secret("fresh-incoming"), secret(smtp)),
        )
        .await
        .unwrap();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::AccountSaved(id) if *id == account.id))
        );
        engine
            .store
            .require_account_reconnected(account.id.clone())
            .await
            .unwrap();
        assert_eq!(
            engine.store.accounts_ready_to_sync().await.unwrap(),
            vec![account]
        );
        assert_eq!(
            engine.store.detail(id).await.unwrap().summary.subject,
            "Retained"
        );
        let calls: Vec<_> = observed.try_iter().collect();
        let writes: Vec<_> = calls
            .iter()
            .filter(|(operation, _)| *operation == 'w')
            .collect();
        assert_eq!(
            writes.len(),
            if separate && auth != SmtpAuth::None {
                2
            } else {
                1
            }
        );
        assert!(
            writes
                .iter()
                .all(|(_, key)| key.starts_with("shared-account:setup:"))
        );
        assert!(
            calls
                .iter()
                .filter(|(operation, _)| *operation == 'r')
                .count()
                >= writes.len()
        );
    }
}

#[tokio::test]
async fn shared_account_setup_explicit_probe_preserves_guard_and_ready_edits_reuse_correct_slots() {
    let mut account = account();
    account.smtp_separate_password = true;
    let store = Store::memory().unwrap();
    pending(&store, &account).await;
    let (engine, observed) = engine(store, false);
    for (target, expected) in [
        (ConnectionTarget::Incoming, "fresh-incoming"),
        (ConnectionTarget::Smtp, "fresh-smtp"),
    ] {
        let resolved = engine
            .setup_password(
                &account,
                &secret("fresh-incoming"),
                &secret("fresh-smtp"),
                target,
            )
            .await
            .unwrap();
        assert_eq!(resolved.expose_secret(), expected);
    }
    assert!(observed.try_recv().is_err());
    assert!(
        engine
            .store
            .require_account_reconnected(account.id.clone())
            .await
            .is_err()
    );
    engine.store.save_account(account.clone()).await.unwrap();
    account.name = "Renamed locally".into();
    run(
        &engine,
        Command::SaveAccount(account, secret(""), secret("")),
    )
    .await
    .unwrap();
    let calls: Vec<_> = observed.try_iter().collect();
    assert_eq!(
        &calls[..2],
        &[
            ('r', "shared-account".into()),
            ('r', "shared-account:smtp".into())
        ]
    );
    let writes: Vec<_> = calls
        .iter()
        .filter(|(operation, _)| *operation == 'w')
        .collect();
    assert_eq!(writes.len(), 2);
    assert!(
        writes
            .iter()
            .all(|(_, key)| key.starts_with("shared-account:setup:"))
    );
}
