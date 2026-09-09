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
    );
    (engine, observed)
}
fn secret(value: &str) -> SecretString {
    SecretString::from(value)
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
            assert_eq!(calls, vec![('w', "shared-account:smtp".into())]);
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
        let expected = if separate && auth != SmtpAuth::None {
            vec![
                ('w', "shared-account:smtp".into()),
                ('w', "shared-account".into()),
            ]
        } else {
            vec![('w', "shared-account".into())]
        };
        assert_eq!(calls, expected);
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
    assert_eq!(
        observed.try_iter().collect::<Vec<_>>(),
        vec![
            ('r', "shared-account".into()),
            ('r', "shared-account:smtp".into()),
            ('w', "shared-account:smtp".into()),
            ('w', "shared-account".into())
        ]
    );
}
