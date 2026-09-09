use super::*;
use crate::{
    model::{Appearance, Preferences},
    profiles::{
        discovery::{Action as DiscoveryAction, Grant, Session},
        enrollment,
        fixture::{Fixture, NAMESPACE},
    },
};
use shep_profile_core::{Action, drive::catalog::Phase as CatalogPhase, history::LocalEdit};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

async fn enrolled(root: &Path, drive: &Drive) -> (Store, Subscription) {
    let store = Store::open(root.join("mail.sqlite")).unwrap();
    let mut prefs = Preferences {
        google_connection_id: "drive:fixture".into(),
        ..Default::default()
    };
    prefs.google_grant.id = "fixture-grant".into();
    prefs.google_grant.access.known = true;
    prefs.google_grant.access.drive = true;
    store.put("preferences", prefs.clone()).await.unwrap();
    let mut session = Session::open(
        root.join("discovery"),
        Uuid::new_v4(),
        Grant::from_preferences(&prefs),
        drive,
    )
    .await
    .unwrap();
    for _ in 0..100 {
        let result = session
            .run(DiscoveryAction::Advance, Some(drive))
            .await
            .unwrap();
        assert!(result.error.is_none(), "{:?}", result.error);
        if result.state.unwrap().phase == CatalogPhase::Complete {
            break;
        }
    }
    let current = session.observe(None).await.unwrap();
    let profile = &current.rows[0];
    let id = current.enrollment.next_id.unwrap();
    let prepared = session
        .run_enrollment(
            &store,
            enrollment::Command::Prepare {
                id,
                profile: profile.profile,
                generation: profile.generation,
                revision: profile.revision,
            },
        )
        .await
        .unwrap();
    assert!(prepared.error.is_none(), "{:?}", prepared.error);
    for _ in 0..100 {
        let result = session
            .run_enrollment(&store, enrollment::Command::Step { id })
            .await
            .unwrap();
        assert!(result.error.is_none(), "{:?}", result.error);
        if result.enrollment.review.unwrap().phase == "review" {
            break;
        }
    }
    let result = session
        .run_enrollment(
            &store,
            enrollment::Command::Approve {
                id,
                accounts: false,
                settings: true,
            },
        )
        .await
        .unwrap();
    assert!(result.error.is_none(), "{:?}", result.error);
    let mut complete = None;
    for _ in 0..100 {
        let result = session
            .run_enrollment(&store, enrollment::Command::Step { id })
            .await
            .unwrap();
        assert!(result.error.is_none(), "{:?}", result.error);
        let review = result.enrollment.review.unwrap();
        if review.phase == "complete" {
            complete = Some(review);
            break;
        }
    }
    let review = complete.expect("initial enrollment completed");
    session.close().await.unwrap();
    let history = open_history(root, &review.binding).await;
    let Reply::State(state) = history.request(Command::State).await.unwrap() else {
        panic!()
    };
    history.close().await.unwrap();
    let subscription = store
        .profile_sync_seed(Seed {
            binding: review.binding,
            device: state.device,
            name: "Work".into(),
            history_revision: state.revision,
            fields: BTreeMap::from([(SettingKey::Appearance, Some(setting("Dark")))]),
        })
        .await
        .unwrap();
    let subscription = store
        .profile_sync_enable(
            subscription.binding.storage_key().unwrap(),
            subscription.revision,
            true,
        )
        .await
        .unwrap();
    (store, subscription)
}
fn setting(value: &str) -> Change {
    Change {
        action: Action::Setting {
            key: SettingKey::Appearance,
            value: value.into(),
        },
        extra: Default::default(),
    }
}
async fn open_history(root: &Path, binding: &Binding) -> Worker {
    Worker::open(
        root.join("discovery/histories")
            .join(format!("{}.sqlite", binding.storage_key().unwrap())),
        binding.clone(),
    )
    .await
    .unwrap()
}
async fn edit(store: &Store, value: Appearance) {
    let mut prefs: Preferences = store.get("preferences").await.unwrap();
    prefs.appearance = value;
    store
        .save_profile_preferences(prefs, BTreeSet::from([SettingKey::Appearance]))
        .await
        .unwrap();
}
async fn appearance(store: &Store) -> Appearance {
    store
        .get::<Preferences>("preferences")
        .await
        .unwrap()
        .appearance
}
async fn cycle(runner: &mut Runner, store: &Store, drive: &Drive) {
    runner.wake();
    for _ in 0..200 {
        if runner.step(store, Some(drive)).await.unwrap().idle {
            return;
        }
    }
    panic!("bounded sync cycle did not complete")
}
#[tokio::test]
async fn two_enrolled_devices_exchange_changes_and_preserve_concurrent_versions() {
    let fixture = Fixture::start(1, false, Duration::ZERO).await.unwrap();
    let drive = fixture
        .connect(NAMESPACE.into(), "drive:fixture")
        .await
        .unwrap();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let (sa, suba) = enrolled(a.path(), &drive).await;
    let (sb, subb) = enrolled(b.path(), &drive).await;
    assert_ne!(suba.device, subb.device);
    let key = suba.binding.storage_key().unwrap();
    let mut ra = Runner::open(a.path().join("discovery"), &suba)
        .await
        .unwrap();
    let mut rb = Runner::open(b.path().join("discovery"), &subb)
        .await
        .unwrap();
    edit(&sa, Appearance::Light).await;
    cycle(&mut ra, &sa, &drive).await;
    cycle(&mut rb, &sb, &drive).await;
    assert_eq!(appearance(&sb).await, Appearance::Light);
    assert_eq!(
        sb.profile_sync_subscription(key.clone())
            .await
            .unwrap()
            .pending,
        0
    );
    assert_eq!(fixture.attempts.lock().unwrap().len(), 1);
    // Both device histories record offline edits before either receives the other.
    edit(&sa, Appearance::Dark).await;
    edit(&sb, Appearance::System).await;
    ra.wake();
    rb.wake();
    ra.step(&sa, None).await.unwrap();
    rb.step(&sb, None).await.unwrap();
    cycle(&mut ra, &sa, &drive).await;
    cycle(&mut rb, &sb, &drive).await;
    cycle(&mut ra, &sa, &drive).await;
    assert_eq!(appearance(&sa).await, Appearance::Dark);
    assert_eq!(appearance(&sb).await, Appearance::System);
    assert_eq!(
        sa.profile_sync_subscription(key.clone())
            .await
            .unwrap()
            .conflicts,
        1
    );
    assert_eq!(
        sb.profile_sync_subscription(key).await.unwrap().conflicts,
        1
    );
    for (root, sub) in [(a.path(), &suba), (b.path(), &subb)] {
        let h = open_history(root, &sub.binding).await;
        let Reply::Versions(versions) = h
            .request(Command::Versions {
                target: "setting:appearance".into(),
                after: None,
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(versions.len(), 2);
        h.close().await.unwrap();
    }
    ra.close().await.unwrap();
    rb.close().await.unwrap();
}
#[tokio::test]
async fn lost_copy_receipt_replays_and_rebuilt_catalog_resets_its_cursor() {
    let fixture = Fixture::start(1, false, Duration::ZERO).await.unwrap();
    let drive = fixture
        .connect(NAMESPACE.into(), "drive:fixture")
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let (store, sub) = enrolled(root.path(), &drive).await;
    let key = sub.binding.storage_key().unwrap();
    let mut runner = Runner::open(root.path().join("discovery"), &sub)
        .await
        .unwrap();
    store.run(|db| {db.execute_batch("CREATE TEMP TRIGGER lost_sync_cursor BEFORE UPDATE OF remote_cursor ON profile_sync WHEN NEW.remote_cursor>OLD.remote_cursor BEGIN SELECT RAISE(FAIL,'synthetic lost copy receipt');END;")?;Ok(())}).await.unwrap();
    let mut failed = false;
    for _ in 0..100 {
        if let Err(error) = runner.step(&store, Some(&drive)).await {
            assert!(
                error.to_string().contains("synthetic lost copy receipt"),
                "{error}"
            );
            failed = true;
            break;
        }
    }
    assert!(failed);
    runner.close().await.unwrap();
    store
        .run(|db| {
            db.execute_batch("DROP TRIGGER lost_sync_cursor")?;
            Ok(())
        })
        .await
        .unwrap();
    let mut runner = Runner::open(root.path().join("discovery"), &sub)
        .await
        .unwrap();
    cycle(&mut runner, &store, &drive).await;
    let before = store.profile_sync_subscription(key.clone()).await.unwrap();
    assert!(before.remote_cursor > 0);
    runner.close().await.unwrap();
    // The observation DB is disposable; the enrolled history and receipt DB are not.
    tokio::fs::remove_dir_all(root.path().join("discovery/ongoing"))
        .await
        .unwrap();
    let mut runner = Runner::open(root.path().join("discovery"), &sub)
        .await
        .unwrap();
    for _ in 0..100 {
        runner.step(&store, Some(&drive)).await.unwrap();
        if matches!(runner.phase, Phase::Copy(_)) {
            break;
        }
    }
    assert!(matches!(runner.phase, Phase::Copy(_)));
    assert_eq!(
        store
            .profile_sync_subscription(key.clone())
            .await
            .unwrap()
            .remote_cursor,
        0
    );
    cycle(&mut runner, &store, &drive).await;
    let after = store.profile_sync_subscription(key).await.unwrap();
    assert_ne!(before.remote_device, after.remote_device);
    assert_eq!(after.remote_cursor, 3);
    let history = open_history(root.path(), &sub.binding).await;
    let Reply::State(state) = history.request(Command::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(state.operations, 3);
    assert_eq!(state.queued, 0);
    history.close().await.unwrap();
    runner.close().await.unwrap();
}
#[tokio::test]
async fn lost_edit_receipt_cannot_acknowledge_a_later_remote_value() {
    let fixture = Fixture::start(1, false, Duration::ZERO).await.unwrap();
    let drive = fixture
        .connect(NAMESPACE.into(), "drive:fixture")
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let (store, sub) = enrolled(root.path(), &drive).await;
    let key = sub.binding.storage_key().unwrap();
    edit(&store, Appearance::Light).await;
    let request = store
        .profile_sync_prepare_edit(key.clone(), SettingKey::Appearance)
        .await
        .unwrap()
        .unwrap();
    let history = open_history(root.path(), &sub.binding).await;
    let Reply::State(first) = history
        .request(Command::Edit {
            edit: request.request.clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    let mut remote = shep_profile_core::history::Journal::memory(sub.binding.clone()).unwrap();
    let mut after = 0;
    loop {
        let Reply::Record(record) = history
            .request(Command::ExportRecord {
                expected_revision: first.revision,
                after,
            })
            .await
            .unwrap()
        else {
            panic!()
        };
        let Some(record) = record else { break };
        after = record.position;
        remote.import(record.record.as_bytes()).unwrap();
    }
    assert_ne!(remote.state().unwrap().device, sub.device);
    remote
        .edit(LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: remote.state().unwrap().revision,
            changes: vec![setting("System")],
            resolutions: vec![],
        })
        .unwrap();
    let record = remote.next_upload().unwrap().unwrap().record;
    let Reply::State(later) = history.request(Command::Import { record }).await.unwrap() else {
        panic!()
    };
    // Reply replay returns current history, not the original field receipt.
    let Reply::State(replay) = history
        .request(Command::Edit {
            edit: request.request.clone(),
        })
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(replay.revision, later.revision);
    let revision = receipt_revision(&history, &request).await.unwrap();
    store
        .profile_sync_edit_saved(request, revision)
        .await
        .unwrap();
    history.close().await.unwrap();
    let mut runner = Runner::open(root.path().join("discovery"), &sub)
        .await
        .unwrap();
    cycle(&mut runner, &store, &drive).await;
    assert_eq!(appearance(&store).await, Appearance::System);
    assert_eq!(
        store
            .profile_sync_subscription(key)
            .await
            .unwrap()
            .conflicts,
        0
    );
    runner.close().await.unwrap();
}
#[tokio::test]
async fn missing_replaced_or_rolled_back_device_history_cannot_start_sync() {
    let fixture = Fixture::start(1, false, Duration::ZERO).await.unwrap();
    let drive = fixture
        .connect(NAMESPACE.into(), "drive:fixture")
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let (store, sub) = enrolled(root.path(), &drive).await;
    let path = root
        .path()
        .join("discovery/histories")
        .join(format!("{}.sqlite", sub.binding.storage_key().unwrap()));
    let old_copy = root.path().join("old-history.sqlite");
    tokio::fs::copy(&path, &old_copy).await.unwrap();
    edit(&store, Appearance::Light).await;
    let mut runner = Runner::open(root.path().join("discovery"), &sub)
        .await
        .unwrap();
    runner.step(&store, None).await.unwrap();
    runner.close().await.unwrap();
    let current = store
        .profile_sync_subscription(sub.binding.storage_key().unwrap())
        .await
        .unwrap();
    assert!(current.history_revision > sub.history_revision);
    tokio::fs::copy(&old_copy, &path).await.unwrap();
    assert!(
        Runner::open(root.path().join("discovery"), &current)
            .await
            .is_err()
    );
    // Even a stale caller snapshot cannot begin a step after opening the old file.
    let mut stale = Runner::open(root.path().join("discovery"), &sub)
        .await
        .unwrap();
    assert!(stale.step(&store, None).await.is_err());
    stale.close().await.unwrap();
    tokio::fs::remove_file(&path).await.unwrap();
    assert!(
        Runner::open(root.path().join("discovery"), &sub)
            .await
            .is_err()
    );
    assert!(!path.exists());
    let history = open_history(root.path(), &sub.binding).await;
    history.close().await.unwrap();
    assert!(
        Runner::open(root.path().join("discovery"), &sub)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn committed_upload_with_lost_reply_survives_pause_and_restart_without_reupload() {
    let fixture = Fixture::start(1, false, Duration::ZERO).await.unwrap();
    let drive = fixture
        .connect(NAMESPACE.into(), "drive:fixture")
        .await
        .unwrap();
    let root = tempfile::tempdir().unwrap();
    let (store, sub) = enrolled(root.path(), &drive).await;
    let key = sub.binding.storage_key().unwrap();
    edit(&store, Appearance::Light).await;
    fixture
        .upload_failure
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let mut runner = Runner::open(root.path().join("discovery"), &sub)
        .await
        .unwrap();
    let mut failed = false;
    for _ in 0..100 {
        if runner.step(&store, Some(&drive)).await.is_err() {
            failed = true;
            break;
        }
    }
    assert!(failed);
    assert_eq!(fixture.attempts.lock().unwrap().len(), 1);
    let current = store.profile_sync_subscription(key.clone()).await.unwrap();
    assert!(current.error.is_some());
    store
        .profile_sync_enable(key.clone(), current.revision, false)
        .await
        .unwrap();
    assert!(runner.step(&store, Some(&drive)).await.is_err());
    assert_eq!(fixture.attempts.lock().unwrap().len(), 1);
    runner.close().await.unwrap();
    drop(store);
    let store = Store::open(root.path().join("mail.sqlite")).unwrap();
    let paused = store.profile_sync_subscription(key.clone()).await.unwrap();
    assert!(!paused.enabled);
    let h = open_history(root.path(), &sub.binding).await;
    let Reply::Upload(Some(saved)) = h.request(Command::NextUpload).await.unwrap() else {
        panic!()
    };
    assert!(saved.file_id.is_some());
    h.close().await.unwrap();
    let enabled = store
        .profile_sync_enable(key.clone(), paused.revision, true)
        .await
        .unwrap();
    let mut runner = Runner::open(root.path().join("discovery"), &enabled)
        .await
        .unwrap();
    cycle(&mut runner, &store, &drive).await;
    assert_eq!(fixture.attempts.lock().unwrap().len(), 1);
    let h = open_history(root.path(), &sub.binding).await;
    let Reply::State(state) = h.request(Command::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(state.queued, 0);
    assert_eq!(state.operations, 4);
    assert!(
        store
            .profile_sync_subscription(key)
            .await
            .unwrap()
            .last_synced
            .is_some()
    );
    assert_eq!(appearance(&store).await, Appearance::Light);
    h.close().await.unwrap();
    runner.close().await.unwrap();
}
