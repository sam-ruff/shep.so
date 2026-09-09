use super::*;
use crate::profile_sync::drive::tests::{binding, file, fixture, session};
use crate::{
    model::*,
    providers::test_http::{Reply, Server},
};
use serde_json::json;
use shep_profile_core::history::{Command, Reply as HistoryReply, Worker};

async fn local(path: &std::path::Path) -> Store {
    let store = Store::open(path.join("cache.sqlite")).unwrap();
    store
        .update_preferences(|p| {
            p.google_client_id = "fixture-client".into();
            p.google_connection_id = binding().identity().into();
            p.google_grant = GoogleGrant {
                id: "fixture-grant".into(),
                client_id: "fixture-client".into(),
                access: GoogleAccess {
                    known: true,
                    drive: true,
                    calendar_read: false,
                    calendar_write: false,
                },
            };
            p.appearance = Appearance::Dark;
            p.backup_folder = "/device/path-does-not-travel".into();
        })
        .await
        .unwrap();
    store
}
fn options() -> Options {
    Options {
        enabled: true,
        ..Default::default()
    }
}
async fn initial(store: &Store, journal: &journal::Journal) -> Snapshot {
    let mut server = Server::start(vec![Reply::new(200, r#"{"files":[]}"#)]).await;
    let session = session(&server);
    let review = discover(store, &session, journal).await.unwrap();
    assert_eq!(review.records(), 0);
    let result = create(
        store,
        &session,
        journal,
        review,
        "Personal".into(),
        options(),
    )
    .await
    .unwrap();
    server.finish().await;
    assert_eq!(
        server.requests().len(),
        1,
        "Creating local intent must not upload before journaling."
    );
    result
}
async fn open(path: &std::path::Path, pending: &Snapshot, journal: &journal::Journal) -> Replica {
    Replica::open(
        path.join("history.sqlite"),
        pending
            .enrollment
            .selection
            .as_ref()
            .unwrap()
            .binding
            .clone(),
        journal.clone(),
    )
    .await
    .unwrap()
}
async fn upload(path: &std::path::Path, pending: &Snapshot) -> ReservedUpload {
    let worker = Worker::open(
        path.join("history.sqlite"),
        pending
            .enrollment
            .selection
            .as_ref()
            .unwrap()
            .binding
            .clone(),
    )
    .await
    .unwrap();
    let HistoryReply::Upload(Some(pending)) = worker.request(Command::NextUpload).await.unwrap()
    else {
        panic!("No seed");
    };
    worker.close().await.unwrap();
    let record = Record::decode(binding().namespace(), pending.record.into_bytes()).unwrap();
    ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: "seed-file".into(),
            key: record.key(),
            size: record.bytes().len() as u64,
            sha256: record.sha256.clone(),
        },
        record,
    }
}

// Observe the exact queued wire records without mutating upload acknowledgments.
// Every caller has already closed its history worker (including SQLite WAL).
fn uploads(path: &std::path::Path) -> Vec<ReservedUpload> {
    let c = rusqlite::Connection::open_with_flags(
        path.join("history.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    c.prepare("SELECT raw FROM operations WHERE local=1 ORDER BY seq")
        .unwrap()
        .query_map([], |r| r.get::<_, Vec<u8>>(0))
        .unwrap()
        .enumerate()
        .map(|(i, raw)| {
            let record = Record::decode(binding().namespace(), raw.unwrap()).unwrap();
            ReservedUpload {
                binding: binding(),
                remote: RemoteRecord {
                    id: if i == 0 {
                        "seed-file".into()
                    } else {
                        format!("seed-file-{i}")
                    },
                    key: record.key(),
                    size: record.bytes().len() as u64,
                    sha256: record.sha256.clone(),
                },
                record,
            }
        })
        .collect()
}
fn fresh_uploads(records: &[ReservedUpload]) -> Vec<Reply> {
    records
        .iter()
        .flat_map(|r| {
            [
                Reply::new(
                    200,
                    json!({"ids":[r.remote.id],"space":"appDataFolder"}).to_string(),
                ),
                Reply::new(404, ""),
                Reply::new(201, file(r).to_string()),
            ]
        })
        .collect()
}
fn download_all(records: &[ReservedUpload]) -> Vec<Reply> {
    let mut replies = vec![Reply::new(
        200,
        json!({"files":records.iter().map(file).collect::<Vec<_>>()}).to_string(),
    )];
    for r in records {
        replies.extend([
            Reply::new(200, file(r).to_string()),
            Reply::binary(200, r.record.bytes().to_vec()),
        ]);
    }
    replies
}

#[tokio::test]
async fn profile_setup_upgrades_only_unstarted_legacy_seeds_without_changing_frozen_metadata() {
    for admitted in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = local(dir.path()).await;
        let journal = journal::Journal::open(None).unwrap();
        let pending = initial(&store, &journal).await;
        let mut replica = open(dir.path(), &pending, &journal).await;
        if admitted {
            let seed = store.profile_seed(pending.clone()).await.unwrap();
            let chunk = store
                .checkpoint_profile_seed(pending.clone(), seed.chunks[0].operation, 0)
                .await
                .unwrap();
            replica
                .edit(history::LocalEdit {
                    operation: chunk.operation,
                    expected_revision: 0,
                    changes: chunk.changes,
                    resolutions: vec![],
                })
                .await
                .unwrap();
        }
        let mut seed = store.profile_seed(pending.clone()).await.unwrap();
        seed.initialization = None;
        store.put(SEED_KEY, seed.clone()).await.unwrap();
        let before = replica.state().await.unwrap();
        let result = prepare(&store, &mut replica, &pending).await;
        let after: Seed = store.get::<Option<Seed>>(SEED_KEY).await.unwrap().unwrap();
        assert_eq!(after.account_ids, seed.account_ids);
        if admitted {
            assert!(result.is_err());
            assert!(after.initialization.is_none());
            assert_eq!(
                serde_json::to_value(&after).unwrap(),
                serde_json::to_value(&seed).unwrap()
            );
            assert_eq!(replica.state().await.unwrap().revision, before.revision);
        } else {
            result.unwrap();
            assert!(after.initialization.is_some());
            for (old, new) in seed.chunks.iter().zip(&after.chunks) {
                assert_eq!(old.operation, new.operation);
                assert_eq!(old.changes, new.changes);
                assert!(new.expected_revision.is_some());
            }
            assert!(replica.state().await.unwrap().initialized);
        }
        assert!(
            store
                .profile_enrollment()
                .await
                .unwrap()
                .enrollment
                .last_success
                .is_none()
        );
        replica.close().await.unwrap();
    }
}

#[tokio::test]
async fn profile_setup_publishes_reviewed_seed_after_restart_and_other_device_reads_exact_settings()
{
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let journal = journal::Journal::open(Some(&dir.path().join("drive.sqlite"))).unwrap();
    let pending = initial(&store, &journal).await;
    let mut replica = open(dir.path(), &pending, &journal).await;
    prepare(&store, &mut replica, &pending).await.unwrap();
    let before = replica.state().await.unwrap();
    prepare(&store, &mut replica, &pending).await.unwrap();
    assert_eq!(replica.state().await.unwrap().revision, before.revision);
    replica.close().await.unwrap();
    let saved = uploads(dir.path());
    assert!(
        saved
            .iter()
            .all(|r| !String::from_utf8_lossy(r.record.bytes()).contains("path-does-not-travel"))
    );
    drop(store);
    let store = Store::open(dir.path().join("cache.sqlite")).unwrap();
    let mut replica = open(dir.path(), &pending, &journal).await;
    let mut replies = vec![Reply::new(200, r#"{"files":[]}"#)];
    replies.extend(fresh_uploads(&saved));
    let mut server = Server::start(replies).await;
    let done = publish(
        &store,
        &mut replica,
        &session(&server),
        pending.clone(),
        123,
    )
    .await
    .unwrap();
    server.finish().await;
    assert!(done.enrollment.selection.as_ref().unwrap().ready);
    assert_eq!(done.enrollment.last_success, Some(123));
    assert_eq!(replica.state().await.unwrap().queued, 0);
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        saved.len()
    );
    replica.close().await.unwrap();

    let other = tempfile::tempdir().unwrap();
    let other_store = local(other.path()).await;
    other_store
        .update_preferences(|p| {
            p.appearance = Appearance::Light;
            p.backup_folder = "/other-device/keep-this".into();
        })
        .await
        .unwrap();
    let mut selection = pending.enrollment.selection.unwrap();
    selection.origin = Origin::Join;
    let joined = other_store
        .begin_profile_enrollment(
            other_store.profile_enrollment().await.unwrap(),
            selection,
            options(),
        )
        .await
        .unwrap();
    let other_journal = journal::Journal::open(None).unwrap();
    let mut other_replica = open(other.path(), &joined, &other_journal).await;
    let mut server = Server::start(download_all(&saved)).await;
    assert!(
        other_replica
            .pull(&session(&server))
            .await
            .unwrap()
            .state()
            .initialized
    );
    let versions = other_replica
        .versions("setting:appearance".into(), None)
        .await
        .unwrap();
    let change = other_replica
        .value("setting:appearance".into(), versions[0].operation)
        .await
        .unwrap();
    let (_, applied) = other_store
        .apply_profile_settings(joined, vec![change])
        .await
        .unwrap();
    assert_eq!(applied.value.appearance, Appearance::Dark);
    assert_eq!(applied.value.backup_folder, "/other-device/keep-this");
    server.finish().await;
    assert!(server.requests().iter().all(|r| r.method == "GET"));
    other_replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_setup_lost_upload_response_reopens_original_operation_and_finishes_without_posting_the_root_again()
 {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let path = dir.path().join("drive.sqlite");
    let journal = journal::Journal::open(Some(&path)).unwrap();
    let pending = initial(&store, &journal).await;
    let mut replica = open(dir.path(), &pending, &journal).await;
    prepare(&store, &mut replica, &pending).await.unwrap();
    replica.close().await.unwrap();
    let all = uploads(dir.path());
    let saved = upload(dir.path(), &pending).await;
    let mut replica = open(dir.path(), &pending, &journal).await;
    let mut first = Server::start(vec![
        Reply::new(200, r#"{"files":[]}"#),
        Reply::new(200, r#"{"ids":["seed-file"],"space":"appDataFolder"}"#),
        Reply::new(404, ""),
        Reply::disconnect(),
        Reply::new(503, "retry later"),
    ])
    .await;
    assert!(
        publish(&store, &mut replica, &session(&first), pending.clone(), 123)
            .await
            .is_err()
    );
    first.finish().await;
    assert!(
        store
            .profile_enrollment()
            .await
            .unwrap()
            .enrollment
            .last_success
            .is_none()
    );
    replica.close().await.unwrap();
    drop(journal);
    let journal = journal::Journal::open(Some(&path)).unwrap();
    let mut replica = open(dir.path(), &pending, &journal).await;
    let mut replies = vec![
        Reply::new(200, json!({"files":[file(&saved)]}).to_string()),
        Reply::new(200, file(&saved).to_string()),
        Reply::binary(200, saved.record.bytes().to_vec()),
        Reply::new(200, file(&saved).to_string()),
    ];
    replies.extend(fresh_uploads(&all[1..]));
    let mut second = Server::start(replies).await;
    publish(&store, &mut replica, &session(&second), pending, 124)
        .await
        .unwrap();
    second.finish().await;
    assert_eq!(
        second
            .requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        all.len() - 1
    );
    assert_eq!(
        journal
            .load(&binding(), saved.remote.key)
            .await
            .unwrap()
            .unwrap()
            .upload()
            .record
            .bytes(),
        saved.record.bytes()
    );
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_setup_rejects_stale_discovery_and_never_converts_visible_existing_records_to_empty()
 {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let journal = journal::Journal::open(None).unwrap();
    let remote = ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: "existing".into(),
            key: fixture().key(),
            size: fixture().bytes().len() as u64,
            sha256: fixture().sha256,
        },
        record: fixture(),
    };
    let mut server = Server::start(vec![
        Reply::new(200, json!({"files":[],"nextPageToken":"more"}).to_string()),
        Reply::new(200, json!({"files":[file(&remote)]}).to_string()),
        Reply::new(200, json!({"files":[]}).to_string()),
    ])
    .await;
    let session = session(&server);
    let review = discover(&store, &session, &journal).await.unwrap();
    assert_eq!(review.records(), 1);
    let current = discover(&store, &session, &journal).await.unwrap();
    assert!(
        create(&store, &session, &journal, review, "Old".into(), options())
            .await
            .is_err()
    );
    store
        .update_preferences(|p| p.appearance = Appearance::Light)
        .await
        .unwrap();
    assert!(
        create(
            &store,
            &session,
            &journal,
            current,
            "Stale".into(),
            options()
        )
        .await
        .is_err()
    );
    assert!(
        store
            .profile_enrollment()
            .await
            .unwrap()
            .enrollment
            .selection
            .is_none()
    );
    server.finish().await;
    assert!(server.requests().iter().all(|r| r.method == "GET"));
}

#[tokio::test]
async fn profile_setup_disabled_categories_and_newer_local_edits_stop_before_network_without_losing_seed()
 {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let journal = journal::Journal::open(None).unwrap();
    let pending = initial(&store, &journal).await;
    let seed = store.profile_seed(pending.clone()).await.unwrap();
    let mut replica = open(dir.path(), &pending, &journal).await;
    let mut server = Server::start(vec![]).await;
    let disabled = store
        .set_profile_sync_options(
            pending.enrollment.revision,
            Options {
                enabled: false,
                ..options()
            },
        )
        .await
        .unwrap();
    assert!(
        publish(&store, &mut replica, &session(&server), pending, 123)
            .await
            .is_err()
    );
    assert_eq!(replica.state().await.unwrap().operations, 0);
    let missing = store
        .set_profile_sync_options(
            disabled.enrollment.revision,
            Options {
                settings: false,
                ..options()
            },
        )
        .await
        .unwrap();
    assert!(
        publish(
            &store,
            &mut replica,
            &session(&server),
            missing.clone(),
            123
        )
        .await
        .is_err()
    );
    let enabled = store
        .set_profile_sync_options(missing.enrollment.revision, options())
        .await
        .unwrap();
    store
        .update_preferences(|p| p.appearance = Appearance::Light)
        .await
        .unwrap();
    assert!(
        publish(&store, &mut replica, &session(&server), enabled, 123)
            .await
            .is_err()
    );
    let now = store.profile_enrollment().await.unwrap();
    assert!(now.enrollment.last_success.is_none());
    assert_eq!(
        store.profile_seed(now).await.unwrap().chunks[0].operation,
        seed.chunks[0].operation
    );
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Light
    );
    server.finish().await;
    assert!(server.requests().is_empty());
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_setup_checkpoints_original_seed_before_upload_and_preserves_later_native_edits() {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let journal = journal::Journal::open(None).unwrap();
    let pending = initial(&store, &journal).await;
    let binding = pending
        .enrollment
        .selection
        .as_ref()
        .unwrap()
        .binding
        .clone();
    store
        .update_preferences(|p| p.appearance = Appearance::Light)
        .await
        .unwrap();
    let current = store.profile_enrollment().await.unwrap();
    let mut replica = open(dir.path(), &current, &journal).await;
    prepare(&store, &mut replica, &current).await.unwrap();
    let baseline = store.profile_replication(binding.clone()).await.unwrap();
    assert!(
        matches!(&baseline.fields["setting:appearance"].local.as_ref().unwrap().action,
        shep_profile_core::Action::Setting {value,..} if value == "Dark")
    );
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Light
    );
    // A retry after another local edit retains the exact first common values.
    store
        .update_preferences(|p| p.appearance = Appearance::System)
        .await
        .unwrap();
    let current = store.profile_enrollment().await.unwrap();
    prepare(&store, &mut replica, &current).await.unwrap();
    assert_eq!(
        store
            .profile_replication(binding.clone())
            .await
            .unwrap()
            .fields,
        baseline.fields
    );
    replica.close().await.unwrap();
    let saved = uploads(dir.path());
    let mut replica = open(dir.path(), &current, &journal).await;
    let mut replies = vec![Reply::new(200, r#"{"files":[]}"#)];
    replies.extend(fresh_uploads(&saved));
    let mut server = Server::start(replies).await;
    publish(&store, &mut replica, &session(&server), current, 123)
        .await
        .unwrap();
    server.finish().await;
    let pending = store.capture_profile_change().await.unwrap().unwrap();
    assert!(
        matches!(pending.change.action,shep_profile_core::Action::Setting {value,..} if value == "System")
    );
    assert_eq!(pending.expected_revision, baseline.revision);
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_setup_without_checkpoint_refuses_to_infer_common_values_from_changed_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let journal = journal::Journal::open(None).unwrap();
    let pending = initial(&store, &journal).await;
    let mut replica = open(dir.path(), &pending, &journal).await;
    prepare(&store, &mut replica, &pending).await.unwrap();
    // An older installation has the seed/history but no reconciliation record.
    store
        .run(|c| {
            c.execute(
                "DELETE FROM kv WHERE key=?",
                [super::super::state::STORAGE_KEY],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let revision = replica.state().await.unwrap().revision;
    replica
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: revision,
            changes: vec![shep_profile_core::Change {
                action: shep_profile_core::Action::Setting {
                    key: shep_profile_core::SettingKey::Appearance,
                    value: json!("Light"),
                },
                extra: Default::default(),
            }],
            resolutions: vec![],
        })
        .await
        .unwrap();
    assert!(prepare(&store, &mut replica, &pending).await.is_err());
    let binding = pending.enrollment.selection.unwrap().binding;
    assert!(
        store
            .profile_replication_optional(binding)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        store
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Dark
    );
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_setup_keeps_inflight_receipt_but_never_reenables_a_disabled_or_disconnected_profile()
 {
    for disconnect in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = local(dir.path()).await;
        let journal = journal::Journal::open(Some(&dir.path().join("drive.sqlite"))).unwrap();
        let pending = initial(&store, &journal).await;
        let mut replica = open(dir.path(), &pending, &journal).await;
        prepare(&store, &mut replica, &pending).await.unwrap();
        replica.close().await.unwrap();
        let saved = upload(dir.path(), &pending).await;
        let mut replica = open(dir.path(), &pending, &journal).await;
        let (reply, observed, release) = Reply::new(201, file(&saved).to_string()).held();
        let mut server = Server::start(vec![
            Reply::new(200, r#"{"files":[]}"#),
            Reply::new(200, r#"{"ids":["seed-file"],"space":"appDataFolder"}"#),
            Reply::new(404, ""),
            reply,
        ])
        .await;
        let session = session(&server);
        let writer = store.clone();
        let reviewed = pending.clone();
        let task = tokio::spawn(async move {
            let result = publish(&writer, &mut replica, &session, reviewed, 123).await;
            (replica, result)
        });
        tokio::time::timeout(std::time::Duration::from_secs(5), observed)
            .await
            .unwrap()
            .unwrap();
        // Actual cache writes/reads complete while the provider is indefinitely
        // held. These must not require the network task to release a state lock.
        if disconnect {
            store
                .disconnect_google(pending.google_revision)
                .await
                .unwrap();
        } else {
            store
                .set_profile_sync_options(
                    pending.enrollment.revision,
                    Options {
                        enabled: false,
                        ..options()
                    },
                )
                .await
                .unwrap();
        }
        store
            .update_preferences(|p| p.appearance = Appearance::Light)
            .await
            .unwrap();
        let paused = store.profile_enrollment().await.unwrap();
        assert!(!paused.enrollment.options.enabled);
        assert!(!task.is_finished());
        release.send(()).unwrap();
        let (replica, result) = task.await.unwrap();
        assert!(format!("{:#}", result.unwrap_err()).contains("upload was saved to Drive"));
        server.finish().await;
        assert_eq!(
            replica.state().await.unwrap().queued,
            2,
            "The committed root receipt must survive; metadata and completion stay queued."
        );
        assert!(
            journal
                .load(&binding(), saved.remote.key)
                .await
                .unwrap()
                .unwrap()
                .acknowledged()
        );
        let after = store.profile_enrollment().await.unwrap();
        assert_eq!(after.enrollment, paused.enrollment);
        assert!(after.enrollment.last_success.is_none());
        assert_eq!(
            store
                .get::<Preferences>("preferences")
                .await
                .unwrap()
                .appearance,
            Appearance::Light
        );
        replica.close().await.unwrap();
    }
}

#[tokio::test]
async fn profile_control_interrupts_a_held_read_but_waits_for_an_admitted_upload_receipt() {
    use crate::profile_sync::control::{Control, Stopped};
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let journal = journal::Journal::open(None).unwrap();
    let (reply, received, release) = Reply::new(200, r#"{"files":[]}"#).held();
    let mut server = Server::start(vec![reply]).await;
    let (stop, control) = Control::channel();
    let read_store = store.clone();
    let read_journal = journal.clone();
    let read_session = session(&server);
    let read = tokio::spawn(async move {
        discover_controlled(&read_store, &read_session, &read_journal, &control).await
    });
    received.await.unwrap();
    stop.send_replace(true);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(2), read)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err()
            .is::<Stopped>()
    );
    assert!(
        store
            .profile_enrollment()
            .await
            .unwrap()
            .enrollment
            .selection
            .is_none()
    );
    release.send(()).unwrap();
    server.finish().await;

    let pending = initial(&store, &journal).await;
    let mut replica = open(dir.path(), &pending, &journal).await;
    prepare(&store, &mut replica, &pending).await.unwrap();
    replica.close().await.unwrap();
    let saved = upload(dir.path(), &pending).await;
    let mut replica = open(dir.path(), &pending, &journal).await;
    let (reply, received, release) = Reply::new(201, file(&saved).to_string()).held();
    let mut server = Server::start(vec![
        Reply::new(200, r#"{"files":[]}"#),
        Reply::new(200, r#"{"ids":["seed-file"],"space":"appDataFolder"}"#),
        Reply::new(404, ""),
        reply,
    ])
    .await;
    let (stop, control) = Control::channel();
    let write_store = store.clone();
    let write_session = session(&server);
    let write = tokio::spawn(async move {
        let result = publish_controlled(
            &write_store,
            &mut replica,
            &write_session,
            pending,
            123,
            &control,
        )
        .await;
        (replica, result)
    });
    received.await.unwrap();
    stop.send_replace(true);
    assert!(!write.is_finished());
    store.put("other-local-work", true).await.unwrap();
    assert!(store.get::<bool>("other-local-work").await.unwrap());
    release.send(()).unwrap();
    let (replica, result) = write.await.unwrap();
    assert!(result.unwrap_err().is::<Stopped>());
    assert_eq!(replica.state().await.unwrap().queued, 2);
    assert!(
        journal
            .load(&binding(), saved.remote.key)
            .await
            .unwrap()
            .unwrap()
            .acknowledged()
    );
    assert!(
        store
            .profile_enrollment()
            .await
            .unwrap()
            .enrollment
            .last_success
            .is_none()
    );
    replica.close().await.unwrap();
    server.finish().await;
}
