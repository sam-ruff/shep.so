use super::*;
use crate::profiles::{
    fixture::{Fixture, NAMESPACE},
    sync::runner::tests::enrolled,
};
use uuid::Uuid;
fn engine(store: Store) -> Engine {
    Engine {
        store,
        google: Default::default(),
        demo: true,
        account_locks: Default::default(),
        calendar_locks: Default::default(),
        calendar_setup_lock: Default::default(),
        connection_lifecycle_lock: Default::default(),
        secret_remover: Arc::new(crate::engine::removals::OsSecretRemover),
        outbound: Arc::new(providers::outgoing::Servers),
        google_connection_lock: Default::default(),
        passphrases: Arc::new(backup::OsPassphraseStore),
        restore_credentials: Arc::new(backup::restore::OsCredentialRestorer),
        backup_uploads: Default::default(),
        mail_sync_settings: Default::default(),
        provider_slots: Default::default(),
        printing: Default::default(),
        bulk_control: Default::default(),
    }
}
#[tokio::test]
async fn background_runs_without_preferences_and_pause_survives_provider_saturation_and_reconnect()
{
    let fixture = Fixture::start(1, false, Duration::ZERO).await.unwrap();
    let drive = fixture
        .connect(NAMESPACE.into(), "drive:fixture")
        .await
        .unwrap();
    let (store, subscription) = enrolled(fixture.root.path(), &drive).await;
    let key = subscription.binding.storage_key().unwrap();
    let engine = engine(store);
    let mut background = Background::default();
    let (mut output, mut events) = futures::channel::mpsc::channel(32);
    for _ in 0..20 {
        background.tick(&engine, &mut output, Some(&fixture)).await;
        while events.try_recv().is_ok() {}
        if background.runner.as_ref().unwrap().needs_network() {
            break;
        }
    }
    assert!(background.runner.as_ref().unwrap().needs_network());
    let slots = futures::future::join_all((0..8).map(|_| engine.provider_slots.acquire())).await;
    tokio::time::timeout(
        Duration::from_secs(1),
        background.tick(&engine, &mut output, Some(&fixture)),
    )
    .await
    .unwrap();
    assert!(events.try_recv().is_err());
    let prefs: Preferences = engine.store.get("preferences").await.unwrap();
    let grant = Grant::from_preferences(&prefs);
    let request = Request {
        panel: Uuid::new_v4(),
        serial: 1,
        grant: grant.clone(),
        action: Action::Load,
    };
    let current = background
        .command(&engine, &request, None, &control::Command::Current)
        .await
        .unwrap();
    assert!(current.sync.unwrap().subscription.unwrap().enabled);
    let paused = background
        .command(
            &engine,
            &request,
            None,
            &control::Command::Enable {
                profile: key.clone(),
                revision: subscription.revision,
                enabled: false,
            },
        )
        .await
        .unwrap()
        .sync
        .unwrap()
        .subscription
        .unwrap();
    assert!(!paused.enabled);
    assert!(background.runner.is_none());
    let mut disconnected = prefs.clone();
    disconnected.google_lifecycle.disconnected = true;
    engine.store.put("preferences", disconnected).await.unwrap();
    assert!(
        background
            .command(
                &engine,
                &request,
                None,
                &control::Command::Enable {
                    profile: key.clone(),
                    revision: paused.revision,
                    enabled: true
                }
            )
            .await
            .is_err()
    );
    assert!(
        !engine
            .store
            .profile_sync_subscription(key.clone())
            .await
            .unwrap()
            .enabled
    );
    drop(slots);
    engine
        .store
        .put("preferences", prefs.clone())
        .await
        .unwrap();
    background
        .command(
            &engine,
            &request,
            None,
            &control::Command::Enable {
                profile: key.clone(),
                revision: paused.revision,
                enabled: true,
            },
        )
        .await
        .unwrap();
    // Holding the Google lifecycle writer also defers background work rather
    // than obstructing cached local commands.
    let lock = engine.google_connection_lock.write().await;
    tokio::time::timeout(
        Duration::from_secs(1),
        background.tick(&engine, &mut output, Some(&fixture)),
    )
    .await
    .unwrap();
    let active = engine.store.profile_sync_active().await.unwrap().unwrap();
    let paused = tokio::time::timeout(
        Duration::from_secs(1),
        background.command(
            &engine,
            &request,
            None,
            &control::Command::Enable {
                profile: key.clone(),
                revision: active.revision,
                enabled: false,
            },
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .sync
    .unwrap()
    .subscription
    .unwrap();
    assert!(!paused.enabled);
    let resume = control::Command::Enable {
        profile: key.clone(),
        revision: paused.revision,
        enabled: true,
    };
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            background.command(&engine, &request, None, &resume)
        )
        .await
        .unwrap()
        .is_err()
    );
    drop(lock);
    background
        .command(&engine, &request, None, &resume)
        .await
        .unwrap();
    let mut completed = false;
    for _ in 0..120 {
        background.tick(&engine, &mut output, Some(&fixture)).await;
        while let Ok(event) = events.try_recv() {
            if let Event::ProfileSync(_, observation, _) = event {
                assert!(
                    observation.subscription.as_ref().unwrap().error.is_none(),
                    "{:?}",
                    observation
                );
                completed |= observation
                    .subscription
                    .as_ref()
                    .unwrap()
                    .last_synced
                    .is_some();
            }
        }
        if completed {
            break;
        }
    }
    assert!(completed);
    assert!(fixture.attempts.lock().unwrap().is_empty());
    // A different Google application project can return an empty inventory
    // with otherwise plausible change tokens. Cached proof must not authorize
    // uploading local intent into that space.
    let mut changed: Preferences = engine.store.get("preferences").await.unwrap();
    changed.google_grant.client_id = "other-project-client".into();
    changed.appearance = crate::model::Appearance::Light;
    engine.store.put("preferences", changed).await.unwrap();
    fixture
        .hidden_files
        .lock()
        .unwrap()
        .extend((10000..=10002).map(|id| format!("fixture-{id}")));
    let mut missing = false;
    for _ in 0..150 {
        background.tick(&engine, &mut output, Some(&fixture)).await;
        while let Ok(event) = events.try_recv() {
            if let Event::ProfileSync(_, observation, _) = event {
                missing |= observation.subscription.as_ref().unwrap().error.is_some();
            }
        }
        if missing {
            break;
        }
    }
    assert!(
        missing,
        "a changed project's missing originals must stop automatic sync"
    );
    assert!(
        fixture.attempts.lock().unwrap().is_empty(),
        "old cached proof cannot authorize new uploads"
    );
    // A frozen review suspends the background owner, including after the
    // Preferences session itself has gone away.
    engine
        .store
        .publication_prepare(
            shep_profile_core::drive::catalog::Scope {
                namespace: NAMESPACE.into(),
                principal: "drive:fixture".into(),
            },
            Uuid::new_v4(),
            crate::profiles::publication::Specification {
                name: "Another profile".into(),
                include_accounts: false,
                settings: std::collections::BTreeMap::from([(
                    shep_profile_core::SettingKey::Appearance,
                    serde_json::json!("Light"),
                )]),
            },
        )
        .await
        .unwrap();
    background.tick(&engine, &mut output, Some(&fixture)).await;
    let Event::ProfileSync(_, observation, _) = events.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(
        observation.phase.as_deref(),
        Some("Waiting for the open profile review")
    );
    background.close().await.unwrap();
}
