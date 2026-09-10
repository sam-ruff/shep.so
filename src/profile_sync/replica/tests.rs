use super::*;
use crate::profile_sync::drive::tests::{binding, file, fixture, session};
use crate::providers::test_http::{Reply as HttpReply, Server};
use serde_json::json;
use shep_profile_core::{Action, Change, SettingKey};

fn history_binding() -> history::Binding {
    let key = fixture().key();
    history::Binding {
        namespace: binding().namespace().into(),
        principal: binding().identity().into(),
        profile: key.profile,
        generation: key.generation,
    }
}

#[tokio::test]
async fn profile_replica_reopened_poll_downloads_only_new_records_and_still_checks_drive() {
    let dir = tempfile::tempdir().unwrap();
    let mut replica = open(dir.path()).await;
    let mut original = crate::profile_sync::drive::tests::reserved();
    // The codec fixture deliberately references an absent parent. This wire
    // scenario needs a complete initial history, independent of that fixture.
    let mut initial = original.record.operation().clone();
    initial.parents.clear();
    original.record = Record::decode(binding().namespace(), initial.encode().unwrap()).unwrap();
    original.remote.size = original.record.bytes().len() as u64;
    original.remote.sha256 = original.record.sha256.clone();
    let mut first = Server::start(pull_replies(std::slice::from_ref(&original))).await;
    assert_eq!(
        replica
            .pull(&session(&first))
            .await
            .unwrap()
            .state()
            .operations,
        1
    );
    first.finish().await;
    assert_eq!(first.requests().len(), 3);
    replica.close().await.unwrap();

    let mut replica = open(dir.path()).await;
    // This transcript intentionally provides no metadata/content download reply.
    let mut unchanged = Server::start(list_replies(std::slice::from_ref(&original))).await;
    assert_eq!(
        replica
            .pull(&session(&unchanged))
            .await
            .unwrap()
            .state()
            .operations,
        1
    );
    unchanged.finish().await;
    assert_eq!(unchanged.requests().len(), 1);

    let mut operation = original.record.operation().clone();
    operation.parents = vec![operation.operation];
    operation.operation = Uuid::new_v4();
    operation.changes = vec![change(Action::ProfileName {
        name: "Changed on another device".into(),
    })];
    let record = Record::decode(binding().namespace(), operation.encode().unwrap()).unwrap();
    let next = ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: "next-operation".into(),
            key: record.key(),
            size: record.bytes().len() as u64,
            sha256: record.sha256.clone(),
        },
        record,
    };
    let mut replies = list_replies(&[original, next.clone()]);
    replies.push(HttpReply::new(200, file(&next).to_string()));
    replies.push(HttpReply::binary(200, next.record.bytes().to_vec()));
    let mut changed = Server::start(replies).await;
    assert_eq!(
        replica
            .pull(&session(&changed))
            .await
            .unwrap()
            .state()
            .operations,
        2
    );
    changed.finish().await;
    assert_eq!(changed.requests().len(), 3);
    assert!(
        changed.requests()[1..]
            .iter()
            .all(|r| r.target.contains("next-operation"))
    );

    let mut unavailable = Server::start(vec![HttpReply::new(503, "offline")]).await;
    assert!(replica.pull(&session(&unavailable)).await.is_err());
    unavailable.finish().await;
    assert_eq!(replica.state().await.unwrap().operations, 2);
    let mut removed = Server::start(list_replies(&[])).await;
    assert_eq!(
        replica
            .pull(&session(&removed))
            .await
            .unwrap()
            .remote_records(),
        0
    );
    removed.finish().await;
    assert_eq!(replica.state().await.unwrap().operations, 2);
    replica.close().await.unwrap();
}
async fn open(path: &std::path::Path) -> Replica {
    Replica::open(
        path.join("history.sqlite"),
        history_binding(),
        journal::Journal::open(Some(&path.join("drive.sqlite"))).unwrap(),
    )
    .await
    .unwrap()
}
fn change(action: Action) -> Change {
    Change {
        action,
        extra: Default::default(),
    }
}
fn appearance(value: &str) -> Change {
    change(Action::Setting {
        key: SettingKey::Appearance,
        value: json!(value),
    })
}
async fn edit(replica: &mut Replica, changes: Vec<Change>) -> State {
    replica
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: replica.state().await.unwrap().revision,
            changes,
            resolutions: vec![],
        })
        .await
        .unwrap()
}
async fn pending(replica: &Replica, id: &str) -> ReservedUpload {
    let Reply::Upload(Some(pending)) = replica.history.request(Command::NextUpload).await.unwrap()
    else {
        panic!("no pending edit")
    };
    let record = Record::decode(binding().namespace(), pending.record.into_bytes()).unwrap();
    ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: id.into(),
            key: record.key(),
            size: record.bytes().len() as u64,
            sha256: record.sha256.clone(),
        },
        record,
    }
}
fn list_replies(records: &[ReservedUpload]) -> Vec<HttpReply> {
    let mut replies = vec![];
    if records.is_empty() {
        replies.push(HttpReply::new(200, r#"{"files":[]}"#));
    }
    for (index, page) in records.chunks(100).enumerate() {
        let mut result = json!({"files":page.iter().map(file).collect::<Vec<_>>()});
        if (index + 1) * 100 < records.len() {
            result["nextPageToken"] = json!(format!("page-{}", index + 1));
        }
        replies.push(HttpReply::new(200, result.to_string()));
    }
    replies
}
fn pull_replies(records: &[ReservedUpload]) -> Vec<HttpReply> {
    let mut replies = list_replies(records);
    for record in records {
        replies.push(HttpReply::new(200, file(record).to_string()));
        replies.push(HttpReply::binary(200, record.record.bytes().to_vec()));
    }
    replies
}
async fn pull(replica: &mut Replica, records: &[ReservedUpload]) -> Pulled {
    let mut replies = list_replies(records);
    for record in records {
        if !replica
            .journal
            .fixture_cached_download(&binding(), &record.remote)
            .await
        {
            replies.push(HttpReply::new(200, file(record).to_string()));
            replies.push(HttpReply::binary(200, record.record.bytes().to_vec()));
        }
    }
    let mut server = Server::start(replies).await;
    let pulled = replica.pull(&session(&server)).await.unwrap();
    server.finish().await;
    assert!(server.requests().iter().all(|r| r.method == "GET"));
    pulled
}
async fn publish_new(
    replica: &mut Replica,
    pulled: &mut Pulled,
    id: &str,
    intent: PublishIntent,
) -> ReservedUpload {
    let upload = pending(replica, id).await;
    let mut server = Server::start(vec![
        HttpReply::new(200, json!({"ids":[id],"space":"appDataFolder"}).to_string()),
        HttpReply::new(404, ""),
        HttpReply::new(201, file(&upload).to_string()),
    ])
    .await;
    assert!(
        replica
            .publish_next(&session(&server), pulled, intent)
            .await
            .unwrap()
    );
    server.finish().await;
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].target.contains("generateIds"));
    assert_eq!(requests[2].method, "POST");
    assert!(
        requests[2]
            .bytes
            .windows(upload.record.bytes().len())
            .any(|v| v == upload.record.bytes())
    );
    upload
}
async fn setting_values(replica: &Replica) -> Vec<Change> {
    let target = "setting:appearance".to_string();
    let versions = replica.versions(target.clone(), None).await.unwrap();
    let mut values = vec![];
    for version in versions {
        values.push(
            replica
                .value(target.clone(), version.operation)
                .await
                .unwrap(),
        );
    }
    values
}

#[tokio::test]
async fn profile_replica_two_devices_publish_offline_conflicts_and_reviewed_resolution() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut desktop = open(a.path()).await;
    let mut other = open(b.path()).await;
    edit(
        &mut desktop,
        vec![
            change(Action::ProfileName {
                name: "Personal".into(),
            }),
            appearance("Dark"),
        ],
    )
    .await;
    let mut initial = pull(&mut desktop, &[]).await;
    let root = publish_new(
        &mut desktop,
        &mut initial,
        "root",
        PublishIntent::ReviewedNewProfile,
    )
    .await;
    assert_eq!(desktop.state().await.unwrap().queued, 0);
    let received = pull(&mut other, std::slice::from_ref(&root)).await;
    assert_ne!(received.state().device, initial.state().device);
    assert_eq!(setting_values(&other).await, vec![appearance("Dark")]);

    // Both devices change the same field without seeing the other's edit.
    edit(&mut desktop, vec![appearance("Light")]).await;
    edit(&mut other, vec![appearance("System")]).await;
    let mut first = pull(&mut desktop, std::slice::from_ref(&root)).await;
    let left = publish_new(
        &mut desktop,
        &mut first,
        "left",
        PublishIntent::ExistingProfile,
    )
    .await;
    let mut second = pull(&mut other, &[root.clone(), left.clone()]).await;
    assert_eq!(second.state().conflicts, 1);
    let right = publish_new(
        &mut other,
        &mut second,
        "right",
        PublishIntent::ExistingProfile,
    )
    .await;
    let all = vec![root.clone(), left.clone(), right.clone()];
    let merged = pull(&mut desktop, &all).await;
    assert_eq!(merged.state().conflicts, 1);
    assert_eq!(setting_values(&desktop).await.len(), 2);
    let target = "setting:appearance".to_string();
    let versions = desktop.versions(target.clone(), None).await.unwrap();
    desktop
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: merged.state().revision,
            changes: vec![appearance("Dark")],
            resolutions: vec![history::Resolution {
                target,
                versions: versions.into_iter().map(|v| v.operation).collect(),
            }],
        })
        .await
        .unwrap();
    let mut reviewed = pull(&mut desktop, &all).await;
    let resolved = publish_new(
        &mut desktop,
        &mut reviewed,
        "resolved",
        PublishIntent::ExistingProfile,
    )
    .await;
    let mut final_history = all;
    final_history.push(resolved);
    let done = pull(&mut other, &final_history).await;
    assert_eq!(done.state().conflicts, 0);
    assert_eq!(setting_values(&other).await, vec![appearance("Dark")]);
    desktop.close().await.unwrap();
    other.close().await.unwrap();
    let reopened = open(b.path()).await;
    assert_eq!(reopened.state().await.unwrap().operations, 4);
    assert_eq!(setting_values(&reopened).await, vec![appearance("Dark")]);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn profile_replica_lost_commit_reply_reopens_same_reservation_and_adopts_remote_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let mut replica = open(dir.path()).await;
    edit(&mut replica, vec![appearance("Dark")]).await;
    let mut pulled = pull(&mut replica, &[]).await;
    let upload = pending(&replica, "original-id").await;
    let mut server = Server::start(vec![
        HttpReply::new(200, r#"{"ids":["original-id"],"space":"appDataFolder"}"#),
        HttpReply::new(404, ""),
        HttpReply::disconnect(),
        HttpReply::new(503, "offline"),
    ])
    .await;
    assert!(
        replica
            .publish_next(
                &session(&server),
                &mut pulled,
                PublishIntent::ReviewedNewProfile
            )
            .await
            .is_err()
    );
    server.finish().await;
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.method == "POST")
            .count(),
        1
    );
    assert_eq!(replica.state().await.unwrap().queued, 1);
    replica.close().await.unwrap();
    let mut replica = open(dir.path()).await;
    let mut recovered = pull(&mut replica, std::slice::from_ref(&upload)).await;
    let mut server = Server::start(vec![HttpReply::new(200, file(&upload).to_string())]).await;
    assert!(
        replica
            .publish_next(
                &session(&server),
                &mut recovered,
                PublishIntent::ExistingProfile
            )
            .await
            .unwrap()
    );
    server.finish().await;
    assert_eq!(server.requests().len(), 1);
    assert!(server.requests()[0].target.contains("/original-id?"));
    assert_eq!(recovered.state().queued, 0);
    assert!(
        replica
            .journal
            .load(&binding(), upload.remote.key)
            .await
            .unwrap()
            .unwrap()
            .acknowledged()
    );
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_replica_requires_explicit_creation_and_rejects_missing_existing_and_stale_pull() {
    let dir = tempfile::tempdir().unwrap();
    let mut replica = open(dir.path()).await;
    edit(&mut replica, vec![appearance("Dark")]).await;
    let mut empty = pull(&mut replica, &[]).await;
    let mut no_requests = Server::start(vec![]).await;
    assert!(
        replica
            .publish_next(
                &session(&no_requests),
                &mut empty,
                PublishIntent::ExistingProfile
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("existing cloud profile is missing")
    );
    edit(
        &mut replica,
        vec![change(Action::ProfileName {
            name: "Two queued changes".into(),
        })],
    )
    .await;
    assert!(
        replica
            .publish_next(
                &session(&no_requests),
                &mut empty,
                PublishIntent::ReviewedNewProfile
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("changed after discovery")
    );
    no_requests.finish().await;
    assert!(no_requests.requests().is_empty());
    let mut fresh = pull(&mut replica, &[]).await;
    publish_new(
        &mut replica,
        &mut fresh,
        "one",
        PublishIntent::ReviewedNewProfile,
    )
    .await;
    publish_new(
        &mut replica,
        &mut fresh,
        "two",
        PublishIntent::ReviewedNewProfile,
    )
    .await;
    assert_eq!(fresh.state().queued, 0);
    // Even explicitly selecting New cannot recreate a previously observed or
    // uploaded generation from a now-empty cloud listing.
    edit(&mut replica, vec![appearance("Light")]).await;
    let mut missing = pull(&mut replica, &[]).await;
    let mut no_requests = Server::start(vec![]).await;
    assert!(
        replica
            .publish_next(
                &session(&no_requests),
                &mut missing,
                PublishIntent::ReviewedNewProfile
            )
            .await
            .is_err()
    );
    no_requests.finish().await;
    assert!(no_requests.requests().is_empty());
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_replica_refuses_incomplete_ancestry_and_preserves_exact_extension_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let mut replica = open(dir.path()).await;
    let record = fixture();
    let child = ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: "child".into(),
            key: record.key(),
            size: record.bytes().len() as u64,
            sha256: record.sha256.clone(),
        },
        record,
    };
    let mut server = Server::start(pull_replies(std::slice::from_ref(&child))).await;
    assert!(
        replica
            .pull(&session(&server))
            .await
            .unwrap_err()
            .downcast_ref::<history::Error>()
            .is_some_and(|e| matches!(e, history::Error::Incomplete))
    );
    server.finish().await;
    assert_eq!(replica.state().await.unwrap().waiting, 1);
    let mut parent = child.record.operation().clone();
    parent.operation = parent.parents[0];
    parent.parents.clear();
    parent.changes = vec![change(Action::ProfileName {
        name: "Earlier name".into(),
    })];
    let record = Record::decode(binding().namespace(), parent.encode().unwrap()).unwrap();
    let parent = ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: "parent".into(),
            key: record.key(),
            size: record.bytes().len() as u64,
            sha256: record.sha256.clone(),
        },
        record,
    };
    let done = pull(&mut replica, &[child.clone(), parent]).await;
    assert_eq!(done.state().waiting, 0);
    let changes = setting_values(&replica).await;
    assert_eq!(changes[0], child.record.operation().changes[2]);
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_replica_rejects_changed_reserved_identity_and_foreign_or_replaced_pull() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut first = open(a.path()).await;
    let mut other = open(b.path()).await;
    edit(&mut first, vec![appearance("Dark")]).await;
    edit(&mut other, vec![appearance("Light")]).await;
    let mut first_pull = pull(&mut first, &[]).await;
    let mut server = Server::start(vec![]).await;
    assert!(
        other
            .publish_next(
                &session(&server),
                &mut first_pull,
                PublishIntent::ReviewedNewProfile
            )
            .await
            .is_err()
    );
    // Replacement discovery invalidates the proof even without a history edit.
    pull(&mut first, &[]).await;
    assert!(
        first
            .publish_next(
                &session(&server),
                &mut first_pull,
                PublishIntent::ReviewedNewProfile
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("discovery changed")
    );
    server.finish().await;
    assert!(server.requests().is_empty());
    let remote = pending(&first, "another-file-id").await;
    first
        .history
        .request(Command::Reserve {
            operation: remote.remote.key.operation,
            file_id: "original-id".into(),
        })
        .await
        .unwrap();
    let mut contradictory = pull(&mut first, std::slice::from_ref(&remote)).await;
    let mut server = Server::start(vec![]).await;
    assert!(
        first
            .publish_next(
                &session(&server),
                &mut contradictory,
                PublishIntent::ExistingProfile
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("different Drive identities")
    );
    server.finish().await;
    assert!(server.requests().is_empty());
    assert_eq!(first.state().await.unwrap().queued, 1);
    first.close().await.unwrap();
    other.close().await.unwrap();
}

#[tokio::test]
async fn profile_replica_recovers_core_reservation_before_transport_prepare_and_partial_acknowledgment()
 {
    let dir = tempfile::tempdir().unwrap();
    let mut replica = open(dir.path()).await;
    edit(&mut replica, vec![appearance("Dark")]).await;
    let upload = pending(&replica, "persisted-first").await;
    replica
        .history
        .request(Command::Reserve {
            operation: upload.remote.key.operation,
            file_id: upload.remote.id.clone(),
        })
        .await
        .unwrap();
    replica.close().await.unwrap();
    let mut replica = open(dir.path()).await;
    assert!(
        replica
            .journal
            .load(&binding(), upload.remote.key)
            .await
            .unwrap()
            .is_none()
    );
    let mut pulled = pull(&mut replica, &[]).await;
    let mut server = Server::start(vec![
        HttpReply::new(404, ""),
        HttpReply::new(201, file(&upload).to_string()),
    ])
    .await;
    assert!(
        replica
            .publish_next(
                &session(&server),
                &mut pulled,
                PublishIntent::ReviewedNewProfile
            )
            .await
            .unwrap()
    );
    server.finish().await;
    assert_eq!(server.requests().len(), 2);
    assert!(
        !server
            .requests()
            .iter()
            .any(|r| r.target.contains("generateIds"))
    );
    replica.close().await.unwrap();

    // Simulate provider acknowledgment committed but core acknowledgment lost.
    // A missing file then must not be recreated, even with a New intent.
    let dir = tempfile::tempdir().unwrap();
    let mut replica = open(dir.path()).await;
    edit(&mut replica, vec![appearance("Dark")]).await;
    let upload = pending(&replica, "confirmed-id").await;
    replica
        .history
        .request(Command::Reserve {
            operation: upload.remote.key.operation,
            file_id: upload.remote.id.clone(),
        })
        .await
        .unwrap();
    let durable = replica.journal.prepare(upload.clone()).await.unwrap();
    replica
        .journal
        .acknowledge(&durable, &upload.remote)
        .await
        .unwrap();
    replica.close().await.unwrap();
    let mut replica = open(dir.path()).await;
    let mut pulled = pull(&mut replica, &[]).await;
    let mut server = Server::start(vec![HttpReply::new(404, "")]).await;
    assert!(
        replica
            .publish_next(
                &session(&server),
                &mut pulled,
                PublishIntent::ReviewedNewProfile
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("previously confirmed")
    );
    server.finish().await;
    assert_eq!(server.requests().len(), 1);
    assert_eq!(server.requests()[0].method, "GET");
    assert_eq!(replica.state().await.unwrap().queued, 1);
    replica.close().await.unwrap();
}

#[tokio::test]
async fn profile_replica_remote_account_and_profile_tombstones_survive_stale_device_edits() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let mut first = open(a.path()).await;
    let mut second = open(b.path()).await;
    let connection = fixture().operation().changes[0].clone();
    let name = fixture().operation().changes[1].clone();
    let Action::AccountName { id, .. } = name.action else {
        panic!()
    };
    edit(
        &mut first,
        vec![
            connection,
            name,
            change(Action::ProfileName {
                name: "Personal".into(),
            }),
        ],
    )
    .await;
    let mut initial = pull(&mut first, &[]).await;
    let root = publish_new(
        &mut first,
        &mut initial,
        "root",
        PublishIntent::ReviewedNewProfile,
    )
    .await;
    pull(&mut second, std::slice::from_ref(&root)).await;
    edit(&mut first, vec![change(Action::AccountRemoved { id })]).await;
    edit(
        &mut second,
        vec![change(Action::AccountName {
            id,
            name: "Old offline rename".into(),
        })],
    )
    .await;
    let mut pulled = pull(&mut first, std::slice::from_ref(&root)).await;
    let removed = publish_new(
        &mut first,
        &mut pulled,
        "account-removed",
        PublishIntent::ExistingProfile,
    )
    .await;
    let mut pulled = pull(&mut second, &[root.clone(), removed.clone()]).await;
    let fields = second.fields(None).await.unwrap();
    assert!(
        !fields
            .iter()
            .any(|f| f.target == format!("account:{id}:name")
                || f.target == format!("account:{id}:connection"))
    );
    let stale = publish_new(
        &mut second,
        &mut pulled,
        "stale-rename",
        PublishIntent::ExistingProfile,
    )
    .await;
    let all = vec![root, removed, stale];
    pull(&mut first, &all).await;
    assert!(
        !first
            .fields(None)
            .await
            .unwrap()
            .iter()
            .any(|f| f.target == format!("account:{id}:name"))
    );
    edit(&mut first, vec![change(Action::ProfileRemoved)]).await;
    let mut pulled = pull(&mut first, &all).await;
    let deleted = publish_new(
        &mut first,
        &mut pulled,
        "profile-removed",
        PublishIntent::ExistingProfile,
    )
    .await;
    let mut all = all;
    all.push(deleted);
    let pulled = pull(&mut second, &all).await;
    assert!(pulled.state().removed);
    assert!(
        second
            .edit(history::LocalEdit {
                operation: Uuid::new_v4(),
                expected_revision: pulled.state().revision,
                changes: vec![appearance("Light")],
                resolutions: vec![]
            })
            .await
            .is_err()
    );
    first.close().await.unwrap();
    second.close().await.unwrap();
    let reopened = open(b.path()).await;
    assert!(reopened.state().await.unwrap().removed);
    reopened.close().await.unwrap();
}

#[tokio::test]
async fn profile_replica_pages_and_drains_long_reverse_order_ancestry_without_history_limit() {
    let dir = tempfile::tempdir().unwrap();
    let mut replica = open(dir.path()).await;
    let mut template = fixture().operation().clone();
    template.parents.clear();
    template.extra.clear();
    let mut records = vec![];
    for index in 0..105 {
        let operation = Uuid::new_v4();
        if index > 0 {
            template.parents = vec![template.operation];
        }
        template.operation = operation;
        template.changes = vec![change(Action::ProfileName {
            name: format!("Version {index}"),
        })];
        let record = Record::decode(binding().namespace(), template.encode().unwrap()).unwrap();
        records.push(ReservedUpload {
            binding: binding(),
            remote: RemoteRecord {
                id: format!("record-{index}"),
                key: record.key(),
                size: record.bytes().len() as u64,
                sha256: record.sha256.clone(),
            },
            record,
        });
    }
    records.reverse();
    let done = pull(&mut replica, &records).await;
    assert_eq!(done.remote_records(), 105);
    assert_eq!(done.state().operations, 105);
    assert_eq!((done.state().waiting, done.state().ready), (0, 0));
    let versions = replica.versions("profile:name".into(), None).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(
        replica
            .value("profile:name".into(), versions[0].operation)
            .await
            .unwrap(),
        change(Action::ProfileName {
            name: "Version 104".into()
        })
    );
    replica.close().await.unwrap();
}
