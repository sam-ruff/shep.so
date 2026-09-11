use super::*;
use crate::profile_sync::{
    RemoteRecord, ReservedUpload,
    drive::tests::{binding, file, fixture, session},
    journal::Journal,
    replica::{PublishIntent, Pulled, Replica},
};
use crate::providers::test_http::{Reply, Server};
use serde_json::{Value, json};
use shep_profile_core::{Action, Change, SettingKey};
use uuid::Uuid;

fn history_binding() -> history::Binding {
    let key = fixture().key();
    history::Binding {
        namespace: binding().namespace().into(),
        principal: binding().identity().into(),
        profile: key.profile,
        generation: key.generation,
    }
}
fn reserved(operation: shep_profile_core::Operation, id: &str) -> ReservedUpload {
    let record = Record::decode(binding().namespace(), operation.encode().unwrap()).unwrap();
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
/// A complete fictional profile: setup start, the codec fixture's account and
/// settings, then the completion marker, in causal order.
fn initialized() -> Vec<ReservedUpload> {
    let mut op = fixture().operation().clone();
    op.parents.clear();
    op.requires.push("initialization-v1".into());
    let root = Uuid::from_u128(op.operation.as_u128() ^ (1_u128 << 120));
    let end = Uuid::from_u128(op.operation.as_u128() ^ (2_u128 << 120));
    [
        (root, None, Some(false), "start"),
        (op.operation, Some(root), None, "data"),
        (end, Some(op.operation), Some(true), "complete"),
    ]
    .into_iter()
    .map(|(id, parent, setup, name)| {
        let mut op = op.clone();
        op.operation = id;
        op.parents = parent.into_iter().collect();
        if let Some(complete) = setup {
            op.changes = vec![Change {
                action: Action::ProfileSetup { complete },
                extra: Default::default(),
            }];
        }
        reserved(op, &format!("file-{name}"))
    })
    .collect()
}
fn appearance(value: &str) -> Change {
    Change {
        action: Action::Setting {
            key: SettingKey::Appearance,
            value: json!(value),
        },
        extra: Default::default(),
    }
}
fn follow(parent: &ReservedUpload, id: &str, changes: Vec<Change>) -> ReservedUpload {
    let mut op = parent.record.operation().clone();
    op.parents = vec![op.operation];
    op.operation = Uuid::new_v4();
    op.device = Uuid::new_v4();
    op.changes = changes;
    reserved(op, id)
}

fn identity() -> Reply {
    Reply::new(
        200,
        json!({"user":{"permissionId":"fixture-user"}}).to_string(),
    )
}
fn start(token: &str) -> Reply {
    Reply::new(200, json!({"startPageToken":token}).to_string())
}
fn files(records: &[&ReservedUpload], next: Option<&str>) -> Reply {
    let mut body = json!({"files":records.iter().map(|r|file(r)).collect::<Vec<_>>(),"incompleteSearch":false});
    if let Some(next) = next {
        body["nextPageToken"] = json!(next);
    }
    Reply::new(200, body.to_string())
}
fn download(record: &ReservedUpload) -> Vec<Reply> {
    vec![
        Reply::new(200, file(record).to_string()),
        Reply::binary(200, record.record.bytes().to_vec()),
    ]
}
fn changed(record: &ReservedUpload) -> Value {
    json!({"fileId":record.remote.id,"removed":false,"changeType":"file","file":file(record)})
}
fn changes(entries: Vec<Value>, next: Option<&str>, caught_up: &str) -> Reply {
    let mut body = json!({"changes":entries});
    match next {
        Some(next) => body["nextPageToken"] = json!(next),
        None => body["newStartPageToken"] = json!(caught_up),
    }
    Reply::new(200, body.to_string())
}
fn full_scan(records: &[ReservedUpload], token: &str) -> Vec<Reply> {
    let mut replies = vec![
        identity(),
        start("stream-start"),
        files(&records.iter().collect::<Vec<_>>(), None),
    ];
    for record in records {
        replies.extend(download(record));
    }
    replies.push(changes(vec![], None, token));
    replies
}

struct Device {
    paths: Paths,
    location: Location,
    replica: Replica,
}
impl Device {
    async fn open(root: &std::path::Path) -> Self {
        Self::open_profile(root, history_binding()).await
    }
    async fn open_profile(root: &std::path::Path, profile: history::Binding) -> Self {
        let paths = Paths::for_cache(&root.join("cache.sqlite")).unwrap();
        let journal = paths.journal().await.unwrap();
        let replica = Replica::open(paths.history(&profile).unwrap(), profile, journal)
            .await
            .unwrap();
        Self {
            location: paths.catalog_location(&binding()).unwrap(),
            paths,
            replica,
        }
    }
    async fn pull(&mut self, replies: Vec<Reply>) -> anyhow::Result<(Pulled, Vec<String>)> {
        let mut server = Server::start(replies).await;
        let result = self
            .replica
            .pull_catalog(&session(&server), &self.location, &Control::default())
            .await;
        server.finish().await;
        let targets = server
            .requests()
            .iter()
            .map(|r| format!("{} {}", r.method, r.target))
            .collect();
        result.map(|pulled| (pulled, targets))
    }
    async fn pulled(&mut self, replies: Vec<Reply>) -> (history::State, u64, Vec<String>) {
        let (pulled, targets) = self.pull(replies).await.unwrap();
        (pulled.state().clone(), pulled.imported(), targets)
    }
    async fn close(self) {
        self.replica.close().await.unwrap();
    }
}
fn count(targets: &[String], path: &str) -> usize {
    targets.iter().filter(|t| t.contains(path)).count()
}

#[tokio::test]
async fn profile_incremental_pull_polls_the_saved_token_and_imports_only_new_records() {
    let dir = tempfile::tempdir().unwrap();
    let records = initialized();
    let mut device = Device::open(dir.path()).await;
    let (state, imported, targets) = device.pulled(full_scan(&records, "token-1")).await;
    assert_eq!((state.operations, imported), (3, 3));
    assert!(state.initialized);
    assert_eq!(count(&targets, "/drive/v3/files?"), 1);
    assert_eq!(count(&targets, "startPageToken"), 1);
    assert_eq!(count(&targets, "alt=media"), 3);

    let (state, imported, targets) = device
        .pulled(vec![identity(), changes(vec![], None, "token-1")])
        .await;
    assert_eq!((state.operations, imported), (3, 0));
    assert_eq!(targets.len(), 2);
    assert!(targets[1].contains("/drive/v3/changes?"));
    assert!(targets[1].contains("pageToken=token-1"));

    let arrival = follow(&records[2], "file-arrival", vec![appearance("Dark")]);
    let mut replies = vec![
        identity(),
        changes(vec![changed(&arrival)], None, "token-2"),
    ];
    replies.extend(download(&arrival));
    let (state, imported, targets) = device.pulled(replies).await;
    assert_eq!((state.operations, imported), (4, 1));
    assert_eq!(targets.len(), 4);
    assert_eq!(count(&targets, "/drive/v3/files?"), 0);
    assert!(targets[2].contains("file-arrival") && targets[3].contains("file-arrival"));
    let versions = device
        .replica
        .versions("setting:appearance".into(), None)
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].operation, arrival.record.key().operation);

    // The next poll continues from the newest persisted token.
    let (state, imported, targets) = device
        .pulled(vec![identity(), changes(vec![], None, "token-2")])
        .await;
    assert_eq!((state.operations, imported), (4, 0));
    assert!(targets[1].contains("pageToken=token-2"));
    device.close().await;
}

#[tokio::test]
async fn profile_incremental_pull_publishes_local_edits_and_adopts_their_later_arrival() {
    let dir = tempfile::tempdir().unwrap();
    let records = initialized();
    let mut device = Device::open(dir.path()).await;
    device.pulled(full_scan(&records, "token-1")).await;
    let revision = device.replica.state().await.unwrap().revision;
    device
        .replica
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: revision,
            changes: vec![Change {
                action: Action::Setting {
                    key: SettingKey::Tooltips,
                    value: json!(true),
                },
                extra: Default::default(),
            }],
            resolutions: vec![],
        })
        .await
        .unwrap();
    let queued = device.replica.queued_record().await.unwrap().unwrap();
    let uploaded = ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: "file-local".into(),
            key: queued.key(),
            size: queued.bytes().len() as u64,
            sha256: queued.sha256.clone(),
        },
        record: queued,
    };
    let mut server = Server::start(vec![
        identity(),
        changes(vec![], None, "token-1"),
        Reply::new(
            200,
            json!({"ids":["file-local"],"space":"appDataFolder"}).to_string(),
        ),
        Reply::new(404, "{}"),
        Reply::new(201, file(&uploaded).to_string()),
    ])
    .await;
    let session = session(&server);
    let mut pulled = device
        .replica
        .pull_catalog(&session, &device.location, &Control::default())
        .await
        .unwrap();
    assert!(
        device
            .replica
            .publish_next(&session, &mut pulled, PublishIntent::ExistingProfile)
            .await
            .unwrap()
    );
    assert!(
        !device
            .replica
            .publish_next(&session, &mut pulled, PublishIntent::ExistingProfile)
            .await
            .unwrap()
    );
    server.finish().await;
    assert_eq!(server.requests().len(), 5);
    assert_eq!(server.requests()[4].method, "POST");
    assert_eq!(device.replica.state().await.unwrap().queued, 0);

    // The change stream later reports this device's own file; the enrolled
    // history already holds that exact operation and nothing is duplicated.
    let mut replies = vec![
        identity(),
        changes(vec![changed(&uploaded)], None, "token-2"),
    ];
    replies.extend(download(&uploaded));
    let (state, imported, _) = device.pulled(replies).await;
    assert_eq!((state.operations, state.queued, imported), (4, 0, 1));
    device.close().await;
}

#[tokio::test]
async fn profile_incremental_rejected_token_falls_back_to_a_full_listing_without_skipping_records()
{
    let dir = tempfile::tempdir().unwrap();
    let records = initialized();
    let mut device = Device::open(dir.path()).await;
    device.pulled(full_scan(&records, "token-1")).await;
    let arrival = follow(&records[2], "file-arrival", vec![appearance("Dark")]);
    let mut all = records.clone();
    all.push(arrival.clone());
    let mut replies = vec![identity(), Reply::new(400, r#"{"error":"expired"}"#)];
    replies.extend(full_scan(&all, "token-9").into_iter().skip(1));
    let (state, imported, targets) = device.pulled(replies).await;
    assert_eq!((state.operations, imported), (4, 1));
    assert!(targets[1].contains("pageToken=token-1"));
    assert!(targets[2].contains("startPageToken"));
    assert_eq!(count(&targets, "/drive/v3/files?"), 1);
    assert_eq!(count(&targets, "alt=media"), 4);

    let (state, imported, targets) = device
        .pulled(vec![identity(), changes(vec![], None, "token-9")])
        .await;
    assert_eq!((state.operations, imported), (4, 0));
    assert_eq!(targets.len(), 2);
    assert!(targets[1].contains("pageToken=token-9"));

    // A second rejection in one pass is reported, never retried indefinitely.
    let (error, targets) = {
        let mut server = Server::start(vec![
            identity(),
            Reply::new(410, "{}"),
            start("again"),
            Reply::new(410, "{}"),
        ])
        .await;
        let result = device
            .replica
            .pull_catalog(&session(&server), &device.location, &Control::default())
            .await;
        server.finish().await;
        let targets = server
            .requests()
            .iter()
            .map(|r| r.target.clone())
            .collect::<Vec<_>>();
        (result.err().unwrap(), targets)
    };
    assert!(error.to_string().contains("410"), "{error:#}");
    assert_eq!(targets.len(), 4);
    assert_eq!(device.replica.state().await.unwrap().operations, 4);
    device.close().await;
}

#[tokio::test]
async fn profile_incremental_interrupted_page_resumes_after_restart_and_replays_lost_checkpoints() {
    let dir = tempfile::tempdir().unwrap();
    let records = initialized();
    let mut device = Device::open(dir.path()).await;
    device.pulled(full_scan(&records, "token-1")).await;
    let first = follow(&records[2], "file-first", vec![appearance("Dark")]);
    let second = follow(&first, "file-second", vec![appearance("System")]);
    let error = device
        .pull(vec![
            identity(),
            changes(vec![changed(&first), changed(&second)], None, "token-2"),
            Reply::new(200, file(&first).to_string()),
            Reply::new(503, "offline"),
        ])
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("503"), "{error:#}");
    assert_eq!(device.replica.state().await.unwrap().operations, 3);
    device.close().await;

    let mut device = Device::open(dir.path()).await;
    let mut replies = vec![identity()];
    replies.extend(download(&first));
    replies.extend(download(&second));
    let (state, imported, targets) = device.pulled(replies).await;
    assert_eq!((state.operations, imported), (5, 2));
    assert_eq!(count(&targets, "/drive/v3/changes"), 0);
    assert_eq!(count(&targets, "/drive/v3/files?"), 0);
    assert_eq!(targets.len(), 5);

    // A lost copy checkpoint re-exports the same immutable records; the
    // history import is idempotent and no record is duplicated or skipped.
    let journal = rusqlite::Connection::open(dir.path().join("profile-sync/drive.sqlite")).unwrap();
    assert_eq!(
        journal
            .query_row("SELECT position FROM catalog_copies", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        5
    );
    journal
        .execute("UPDATE catalog_copies SET position=1", [])
        .unwrap();
    drop(journal);
    let (state, imported, targets) = device
        .pulled(vec![identity(), changes(vec![], None, "token-2")])
        .await;
    assert_eq!((state.operations, imported), (5, 4));
    assert_eq!(targets.len(), 2);
    assert!(targets[1].contains("pageToken=token-2"));
    device.close().await;
}

#[tokio::test]
async fn profile_incremental_out_of_order_changes_apply_once_their_parents_arrive() {
    let dir = tempfile::tempdir().unwrap();
    let records = initialized();
    let mut device = Device::open(dir.path()).await;
    device.pulled(full_scan(&records, "token-1")).await;
    let child = follow(&records[2], "file-child", vec![appearance("Dark")]);
    let grandchild = follow(&child, "file-grandchild", vec![appearance("System")]);
    let mut replies = vec![
        identity(),
        changes(
            vec![changed(&grandchild), changed(&child)],
            Some("more"),
            "",
        ),
    ];
    replies.extend(download(&grandchild));
    replies.extend(download(&child));
    replies.push(changes(vec![], None, "token-2"));
    let (state, imported, targets) = device.pulled(replies).await;
    assert_eq!((state.operations, imported), (5, 2));
    assert_eq!((state.waiting, state.ready, state.conflicts), (0, 0, 0));
    assert!(targets.last().unwrap().contains("pageToken=more"));
    let versions = device
        .replica
        .versions("setting:appearance".into(), None)
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].operation, grandchild.record.key().operation);
    device.close().await;
}

#[tokio::test]
async fn profile_incremental_copy_cursor_is_bound_to_the_history_and_observation_owners() {
    let dir = tempfile::tempdir().unwrap();
    let records = initialized();
    let mut device = Device::open(dir.path()).await;
    device.pulled(full_scan(&records, "token-1")).await;
    let history = device.paths.history(&history_binding()).unwrap();
    device.close().await;

    // A rebuilt enrolled history has a new device: the saved position cannot
    // skip records that the new history has never seen.
    for entry in std::fs::read_dir(dir.path().join("profile-sync")).unwrap() {
        let path = entry.unwrap().path();
        if path
            .to_string_lossy()
            .starts_with(&*history.to_string_lossy())
        {
            std::fs::remove_file(path).unwrap();
        }
    }
    let mut device = Device::open(dir.path()).await;
    let (state, imported, targets) = device
        .pulled(vec![identity(), changes(vec![], None, "token-1")])
        .await;
    assert_eq!((state.operations, imported), (3, 3));
    assert!(state.initialized);
    assert_eq!(targets.len(), 2);
    let scope = Scope {
        namespace: binding().namespace().into(),
        principal: binding().identity().into(),
    };
    let mut observations = device.paths.catalog(&scope).unwrap().into_os_string();
    observations.push(".observations");
    device.close().await;

    // A rebuilt observation owner disagrees with the catalog summary: one full
    // listing restores it, and the new source replays every record.
    std::fs::remove_dir_all(&observations).unwrap();
    let mut device = Device::open(dir.path()).await;
    let mut replies = vec![identity(), changes(vec![], None, "token-1")];
    replies.extend(full_scan(&records, "token-3").into_iter().skip(1));
    let (state, imported, targets) = device.pulled(replies).await;
    assert_eq!((state.operations, imported), (3, 3));
    assert_eq!(count(&targets, "startPageToken"), 1);
    assert_eq!(count(&targets, "alt=media"), 3);
    let (state, imported, targets) = device
        .pulled(vec![identity(), changes(vec![], None, "token-3")])
        .await;
    assert_eq!((state.operations, imported), (3, 0));
    assert_eq!(targets.len(), 2);
    device.close().await;
}

#[tokio::test]
async fn profile_incremental_missing_known_record_fails_verification_and_keeps_history() {
    let dir = tempfile::tempdir().unwrap();
    let records = initialized();
    let mut device = Device::open(dir.path()).await;
    device.pulled(full_scan(&records, "token-1")).await;
    let removed = json!({"fileId":records[1].remote.id,"removed":true,"changeType":"file"});
    let mut replies = vec![
        identity(),
        changes(vec![removed], None, "token-2"),
        start("stream-again"),
        files(&[&records[0], &records[2]], None),
    ];
    replies.extend(download(&records[0]));
    replies.extend(download(&records[2]));
    replies.push(changes(vec![], None, "token-3"));
    let error = device.pull(replies).await.err().unwrap();
    assert!(error.to_string().contains("missing"), "{error:#}");
    let state = device.replica.state().await.unwrap();
    assert_eq!((state.operations, state.waiting), (3, 0));
    let versions = device
        .replica
        .versions("setting:appearance".into(), None)
        .await
        .unwrap();
    assert_eq!(versions.len(), 1);
    device.close().await;
}

#[tokio::test]
async fn profile_incremental_unlisted_profile_is_reported_without_a_full_listing() {
    let dir = tempfile::tempdir().unwrap();
    let records = initialized();
    let mut device = Device::open(dir.path()).await;
    device.pulled(full_scan(&records, "token-1")).await;
    device.close().await;
    let mut other = Device::open_profile(
        dir.path(),
        history::Binding {
            profile: Uuid::new_v4(),
            generation: Uuid::new_v4(),
            ..history_binding()
        },
    )
    .await;
    let (error, targets) = {
        let mut server = Server::start(vec![identity(), changes(vec![], None, "token-1")]).await;
        let result = other
            .replica
            .pull_catalog(&session(&server), &other.location, &Control::default())
            .await;
        server.finish().await;
        (result.err().unwrap(), server.requests().len())
    };
    assert!(
        error.to_string().contains("missing from Drive"),
        "{error:#}"
    );
    assert_eq!(targets, 2);
    other.close().await;
}

#[tokio::test]
async fn profile_copy_cursor_ignores_positions_saved_for_other_owners() {
    let journal = Journal::open(None).unwrap();
    let identity = CopyIdentity {
        profile: Uuid::new_v4(),
        generation: Uuid::new_v4(),
        source: Uuid::new_v4(),
        device: Uuid::new_v4(),
    };
    assert_eq!(
        journal.copy_position(&binding(), identity).await.unwrap(),
        0
    );
    journal
        .checkpoint_copy(&binding(), identity, 7)
        .await
        .unwrap();
    assert_eq!(
        journal.copy_position(&binding(), identity).await.unwrap(),
        7
    );
    let other_source = CopyIdentity {
        source: Uuid::new_v4(),
        ..identity
    };
    let other_device = CopyIdentity {
        device: Uuid::new_v4(),
        ..identity
    };
    assert_eq!(
        journal
            .copy_position(&binding(), other_source)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        journal
            .copy_position(&binding(), other_device)
            .await
            .unwrap(),
        0
    );
    journal
        .checkpoint_copy(&binding(), other_device, 2)
        .await
        .unwrap();
    assert_eq!(
        journal.copy_position(&binding(), identity).await.unwrap(),
        0
    );
    assert_eq!(
        journal
            .copy_position(&binding(), other_device)
            .await
            .unwrap(),
        2
    );
    assert!(
        journal
            .checkpoint_copy(
                &binding(),
                CopyIdentity {
                    source: Uuid::nil(),
                    ..identity
                },
                1
            )
            .await
            .is_err()
    );
}
