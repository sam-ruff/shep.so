use super::*;
use crate::drive::catalog::{Discovery, Error as DiscoveryError, Phase, Scope};

struct Record {
    metadata: Value,
    bytes: Vec<u8>,
}
fn record(n: u128, profile: u128, parents: &[u128], changes: Vec<Action>) -> Record {
    let operation = Operation {
        format: crate::FORMAT.into(),
        major: 1,
        minor: 0,
        requires: vec![
            "causal-v1".into(),
            "accounts-v1".into(),
            "settings-v1".into(),
        ],
        namespace: NAMESPACE.into(),
        profile: Uuid::from_u128(1000 + profile),
        generation: Uuid::from_u128(2000 + profile),
        device: Uuid::from_u128(3000 + n % 3),
        operation: Uuid::from_u128(4000 + n),
        parents: parents.iter().map(|n| Uuid::from_u128(4000 + n)).collect(),
        changes: changes
            .into_iter()
            .map(|action| Change {
                action,
                extra: Default::default(),
            })
            .collect(),
        extra: Default::default(),
    };
    let bytes = operation.encode().unwrap();
    let upload = history::Upload {
        operation: operation.operation,
        record: String::from_utf8(bytes.clone()).unwrap(),
        sha256: wire::sha256(&bytes),
        file_id: None,
    };
    let mut metadata = super::metadata(&upload);
    metadata["id"] = json!(format!("fixture-file-{n}"));
    metadata["appProperties"]["shepProfile"] = json!(operation.profile);
    metadata["appProperties"]["shepGeneration"] = json!(operation.generation);
    Record { metadata, bytes }
}
fn named(name: &str) -> Vec<Action> {
    vec![
        Action::ProfileName { name: name.into() },
        Action::Setting {
            key: SettingKey::Appearance,
            value: json!("Dark"),
        },
    ]
}
fn scope() -> Scope {
    Scope {
        namespace: NAMESPACE.into(),
        principal: PRINCIPAL.into(),
    }
}
fn start() -> Step {
    value(json!({"startPageToken":"before-scan"}))
}
fn files(records: &[&Record], next: Option<&str>) -> Step {
    let mut body = json!({"incompleteSearch":false,"files":records.iter().map(|r|r.metadata.clone()).collect::<Vec<_>>()});
    if let Some(next) = next {
        body["nextPageToken"] = json!(next);
    }
    value(body)
}
fn downloads(record: &Record) -> Vec<Step> {
    vec![
        value(record.metadata.clone()),
        reply(TestResponse::new(200, record.bytes.clone())),
    ]
}
fn changed(record: &Record) -> Value {
    json!({"fileId":record.metadata["id"],"removed":false,"changeType":"file","file":record.metadata})
}
fn caught_up(token: &str) -> Step {
    value(json!({"changes":[],"newStartPageToken":token}))
}
async fn finish(discovery: &Discovery, drive: &Drive) -> crate::drive::catalog::State {
    for _ in 0..1000 {
        let state = discovery.advance(drive).await.unwrap();
        if state.phase == Phase::Complete {
            return state;
        }
    }
    panic!("discovery did not finish its fixture");
}

#[tokio::test]
async fn staged_scan_reopens_replays_arrivals_and_updates_profiles_without_touching_local_edits() {
    let first = record(1, 1, &[], named("Work"));
    let second = record(2, 2, &[], named("Personal"));
    let arrival = record(
        3,
        1,
        &[1],
        vec![Action::Setting {
            key: SettingKey::UnifiedInbox,
            value: json!(true),
        }],
    );
    let tombstone = record(4, 2, &[2], vec![Action::ProfileRemoved]);
    let mut steps = vec![identity(), start(), files(&[&first], Some("second-page"))];
    steps.extend(downloads(&first));
    steps.push(files(&[&second], None));
    steps.extend(downloads(&second));
    steps.push(value(
        json!({"changes":[changed(&arrival),changed(&first)],"nextPageToken":"tail"}),
    ));
    steps.extend(downloads(&arrival));
    steps.extend(downloads(&first));
    steps.push(caught_up("after-scan"));
    steps.push(value(
        json!({"changes":[changed(&tombstone)],"newStartPageToken":"after-removal"}),
    ));
    steps.extend(downloads(&tombstone));
    let server = Server::start(steps).await;
    let drive = server.connect(Some(PRINCIPAL)).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog.sqlite");
    let local = Worker::open(
        directory.path().join("enrolled.sqlite"),
        history::Binding {
            namespace: NAMESPACE.into(),
            principal: PRINCIPAL.into(),
            profile: Uuid::from_u128(1001),
            generation: Uuid::from_u128(2001),
        },
    )
    .await
    .unwrap();
    let local_edit = history::LocalEdit {
        operation: Uuid::from_u128(8888),
        expected_revision: 0,
        changes: vec![Change {
            action: Action::Setting {
                key: SettingKey::Appearance,
                value: json!("System"),
            },
            extra: Default::default(),
        }],
        resolutions: vec![],
    };
    local
        .request(Command::Edit { edit: local_edit })
        .await
        .unwrap();
    let discovery = Discovery::open(path.clone(), scope()).await.unwrap();
    assert_eq!(discovery.advance(&drive).await.unwrap().phase, Phase::Files);
    assert_eq!(discovery.advance(&drive).await.unwrap().pending, 1);
    discovery.close().await.unwrap();
    let discovery = Discovery::open(path, scope()).await.unwrap();
    assert_eq!(discovery.state().await.unwrap().pending, 1);
    let complete = finish(&discovery, &drive).await;
    assert_eq!(
        (complete.files, complete.profiles, complete.pending),
        (3, 2, 0)
    );
    assert_eq!(complete.completed_revision, Some(complete.revision));
    let profiles = discovery.profiles(None).await.unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(profiles[0].name.as_deref(), Some("Work"));
    assert_eq!(profiles[0].settings, 2);
    assert_eq!(profiles[1].name.as_deref(), Some("Personal"));
    assert_eq!(state(&local).await.operations, 1);
    assert_eq!(state(&local).await.queued, 1);
    discovery.refresh(complete.revision, false).await.unwrap();
    let complete = finish(&discovery, &drive).await;
    assert_eq!(complete.files, 4);
    let profiles = discovery.profiles(None).await.unwrap();
    assert!(profiles[1].removed);
    assert_eq!(profiles[1].settings, 0);
    assert!(profiles[1].name.is_none());
    let requests = server.finish().await;
    assert!(requests.iter().all(|r| r.method == "GET"));
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("startPageToken"))
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path() == "/drive/v3/files")
            .count(),
        2
    );
    assert!(requests.iter().any(|r| {
        r.url.path() == "/drive/v3/changes"
            && r.url
                .query_pairs()
                .any(|(k, v)| k == "pageToken" && v == "after-scan")
    }));
    discovery.close().await.unwrap();
    local.close().await.unwrap();
}

#[tokio::test]
async fn failed_media_keeps_the_staged_page_and_error_through_restart_before_retry() {
    let first = record(1, 1, &[], named("Work"));
    let mut steps = vec![
        identity(),
        start(),
        files(&[&first], None),
        value(first.metadata.clone()),
        reply(TestResponse::new(503, vec![])),
    ];
    steps.extend(downloads(&first));
    steps.push(caught_up("after-scan"));
    let server = Server::start(steps).await;
    let drive = server.connect(None).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog.sqlite");
    let discovery = Discovery::open(path.clone(), scope()).await.unwrap();
    discovery.advance(&drive).await.unwrap();
    discovery.advance(&drive).await.unwrap();
    assert!(matches!(
        discovery.advance(&drive).await,
        Err(DiscoveryError::Provider(Error::Http(503)))
    ));
    let failed = discovery.state().await.unwrap();
    assert_eq!(failed.pending, 1);
    assert!(failed.error.is_some());
    assert!(failed.completed_revision.is_none());
    assert!(matches!(
        discovery.advance(&drive).await,
        Err(DiscoveryError::Failed)
    ));
    discovery.close().await.unwrap();
    let discovery = Discovery::open(path, scope()).await.unwrap();
    let reopened = discovery.state().await.unwrap();
    assert_eq!(reopened.revision, failed.revision);
    assert_eq!(reopened.error, failed.error);
    discovery.retry(reopened.revision).await.unwrap();
    let complete = finish(&discovery, &drive).await;
    assert_eq!(complete.files, 1);
    assert_eq!(complete.profiles, 1);
    assert!(complete.error.is_none());
    let requests = server.finish().await;
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path() == "/drive/v3/files")
            .count(),
        1
    );
    discovery.close().await.unwrap();
}

#[tokio::test]
async fn long_page_cycles_and_duplicate_file_identities_do_not_become_empty_discovery() {
    let first = record(1, 1, &[], named("Work"));
    for duplicate in [false, true] {
        let mut steps = vec![identity(), start()];
        if duplicate {
            steps.push(files(&[&first], Some("again")));
            steps.extend(downloads(&first));
            steps.push(files(&[&first], None));
        } else {
            steps.extend([
                files(&[], Some("a")),
                files(&[], Some("b")),
                files(&[], Some("a")),
            ]);
        }
        let server = Server::start(steps).await;
        let drive = server.connect(None).await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        let discovery = Discovery::open(directory.path().join("catalog.sqlite"), scope())
            .await
            .unwrap();
        loop {
            match discovery.advance(&drive).await {
                Ok(state) => assert_ne!(state.phase, Phase::Complete),
                Err(error) => {
                    assert!(matches!(error, DiscoveryError::Integrity));
                    break;
                }
            }
        }
        let state = discovery.state().await.unwrap();
        assert!(state.error.is_some());
        assert!(state.completed_revision.is_none());
        assert_eq!(state.files, u64::from(duplicate));
        server.finish().await;
        discovery.close().await.unwrap();
    }
    let mut copy = first.metadata.clone();
    copy["id"] = json!("copied-operation");
    let mut steps = vec![identity(), start(), files(&[&first], Some("copy"))];
    steps.extend(downloads(&first));
    steps.push(value(
        json!({"incompleteSearch":false,"files":[copy.clone()]}),
    ));
    steps.push(value(copy));
    steps.push(reply(TestResponse::new(200, first.bytes.clone())));
    let server = Server::start(steps).await;
    let drive = server.connect(None).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let discovery = Discovery::open(directory.path().join("catalog.sqlite"), scope())
        .await
        .unwrap();
    loop {
        match discovery.advance(&drive).await {
            Ok(state) => assert_ne!(state.phase, Phase::Complete),
            Err(error) => {
                assert!(matches!(error, DiscoveryError::Integrity));
                break;
            }
        }
    }
    assert_eq!(discovery.state().await.unwrap().files, 1);
    assert_eq!(discovery.profiles(None).await.unwrap()[0].operations, 1);
    server.finish().await;
    discovery.close().await.unwrap();
}

#[tokio::test]
async fn missing_or_reclassified_known_files_preserve_the_last_completed_profile() {
    for kind in ["removed", "other", "absent"] {
        let first = record(1, 1, &[], named("Work"));
        let mut steps = vec![identity(), start(), files(&[&first], None)];
        steps.extend(downloads(&first));
        steps.push(caught_up("baseline"));
        if kind == "absent" {
            steps.extend([start(), files(&[], None), caught_up("missing")]);
        } else {
            let event = if kind == "removed" {
                json!({"fileId":first.metadata["id"],"removed":true,"changeType":"file"})
            } else {
                json!({"fileId":first.metadata["id"],"removed":false,"changeType":"file","file":{"id":first.metadata["id"]}})
            };
            steps.push(value(
                json!({"changes":[event],"newStartPageToken":"missing"}),
            ));
        }
        let server = Server::start(steps).await;
        let drive = server.connect(None).await.unwrap();
        let directory = tempfile::tempdir().unwrap();
        let discovery = Discovery::open(directory.path().join("catalog.sqlite"), scope())
            .await
            .unwrap();
        let before = finish(&discovery, &drive).await;
        discovery
            .refresh(before.revision, kind == "absent")
            .await
            .unwrap();
        loop {
            match discovery.advance(&drive).await {
                Ok(state) => assert_ne!(state.phase, Phase::Complete),
                Err(error) => {
                    assert!(matches!(
                        error,
                        DiscoveryError::Missing | DiscoveryError::Integrity
                    ));
                    break;
                }
            }
        }
        let after = discovery.state().await.unwrap();
        assert_eq!(after.completed_revision, Some(before.revision));
        assert!(after.error.is_some());
        assert_ne!(after.phase, Phase::Complete);
        let profiles = discovery.profiles(None).await.unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name.as_deref(), Some("Work"));
        assert!(!profiles[0].removed);
        server.finish().await;
        discovery.close().await.unwrap();
    }
}

#[tokio::test]
async fn profiles_are_paged_and_conflicts_and_missing_ancestry_remain_explicit() {
    let mut records = (1..=51)
        .map(|n| record(n, n, &[], named(&format!("Profile {n}"))))
        .collect::<Vec<_>>();
    records.push(record(100, 1, &[], named("Concurrent name")));
    records.push(record(101, 52, &[999], named("Missing history")));
    let mut steps = vec![
        identity(),
        start(),
        files(&records[..50].iter().collect::<Vec<_>>(), Some("last")),
    ];
    for record in &records[..50] {
        steps.extend(downloads(record));
    }
    steps.push(files(&records[50..].iter().collect::<Vec<_>>(), None));
    for record in &records[50..] {
        steps.extend(downloads(record));
    }
    steps.push(caught_up("finished"));
    let server = Server::start(steps).await;
    let drive = server.connect(None).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let discovery = Discovery::open(directory.path().join("catalog.sqlite"), scope())
        .await
        .unwrap();
    let complete = finish(&discovery, &drive).await;
    assert_eq!(complete.files, 53);
    assert_eq!(complete.profiles, 52);
    assert_eq!(complete.incomplete_profiles, 1);
    let first = discovery.profiles(None).await.unwrap();
    assert_eq!(first.len(), 50);
    assert!(first[0].name_conflict);
    assert!(first[0].name.is_none());
    assert!(first[0].conflicts > 0);
    let last = discovery.profiles(Some(first[49].cursor())).await.unwrap();
    assert_eq!(last.len(), 2);
    assert_eq!(last[1].waiting, 1);
    assert!(last[1].name.is_none());
    assert_eq!(last[1].settings, 0);
    assert!(
        discovery
            .profiles(Some(last[1].cursor()))
            .await
            .unwrap()
            .is_empty()
    );
    server.finish().await;
    discovery.close().await.unwrap();
}

#[tokio::test]
async fn a_prepared_identity_survives_a_failed_receipt_after_history_commit_and_full_rescan() {
    let first = record(1, 1, &[], named("Work"));
    let mut steps = vec![identity(), start(), files(&[&first], None)];
    steps.extend(downloads(&first));
    steps.extend([
        start(),
        files(&[], None),
        caught_up("missing-prepared-file"),
    ]);
    steps.extend([start(), files(&[&first], None)]);
    steps.extend(downloads(&first));
    steps.push(caught_up("recovered"));
    let server = Server::start(steps).await;
    let drive = server.connect(None).await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog.sqlite");
    let discovery = Discovery::open(path.clone(), scope()).await.unwrap();
    discovery.advance(&drive).await.unwrap();
    discovery.advance(&drive).await.unwrap();
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TRIGGER fail_receipt BEFORE INSERT ON profiles BEGIN SELECT RAISE(ABORT,'fixture receipt failure'); END;").unwrap();
    assert!(matches!(
        discovery.advance(&drive).await,
        Err(DiscoveryError::Storage)
    ));
    let prepared = discovery.state().await.unwrap();
    assert_eq!(prepared.files, 1);
    assert_eq!(prepared.profiles, 0);
    assert_eq!(prepared.pending, 1);
    assert!(
        !db.query_row("SELECT verified FROM files", [], |r| r.get::<_, bool>(0))
            .unwrap()
    );
    let mut root = path.as_os_str().to_owned();
    root.push(".observations");
    let binding = history::Binding {
        namespace: NAMESPACE.into(),
        principal: PRINCIPAL.into(),
        profile: Uuid::from_u128(1001),
        generation: Uuid::from_u128(2001),
    };
    let source = rusqlite::Connection::open(
        std::path::PathBuf::from(root).join(format!("{}.sqlite", binding.storage_key().unwrap())),
    )
    .unwrap();
    assert_eq!(
        source
            .query_row("SELECT count(*) FROM operations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    db.execute_batch("DROP TRIGGER fail_receipt;").unwrap();
    drop(source);
    drop(db);
    discovery.close().await.unwrap();
    let discovery = Discovery::open(path, scope()).await.unwrap();
    discovery.refresh(prepared.revision, true).await.unwrap();
    loop {
        match discovery.advance(&drive).await {
            Ok(state) => assert_ne!(state.phase, Phase::Complete),
            Err(error) => {
                assert!(matches!(error, DiscoveryError::Missing));
                break;
            }
        }
    }
    let missing = discovery.state().await.unwrap();
    assert_eq!(missing.files, 1);
    assert!(missing.error.is_some());
    discovery.refresh(missing.revision, true).await.unwrap();
    let recovered = finish(&discovery, &drive).await;
    assert_eq!((recovered.files, recovered.profiles), (1, 1));
    let profile = discovery.profiles(None).await.unwrap().remove(0);
    assert_eq!(profile.operations, 1);
    assert_eq!(profile.settings, 1);
    server.finish().await;
    discovery.close().await.unwrap();
}

#[tokio::test]
async fn held_reads_do_not_block_observations_or_apply_to_a_restarted_scan() {
    for status in [200, 503] {
        let (entered, received) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel();
        let server = Server::start(vec![
            identity(),
            Box::new(move |_| {
                entered.send(()).unwrap();
                TestResponse::new(
                    status,
                    json!({"startPageToken":"old-scan"})
                        .to_string()
                        .into_bytes(),
                )
                .held(held)
            }),
        ])
        .await;
        let drive = std::sync::Arc::new(server.connect(None).await.unwrap());
        let directory = tempfile::tempdir().unwrap();
        let discovery = Discovery::open(directory.path().join("catalog.sqlite"), scope())
            .await
            .unwrap();
        let cloned = discovery.clone();
        let grant = drive.clone();
        let pending = tokio::spawn(async move { cloned.advance(&grant).await });
        tokio::time::timeout(Duration::from_secs(5), received)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            discovery.advance(&drive).await,
            Err(DiscoveryError::Busy)
        ));
        let before = discovery.state().await.unwrap();
        assert_eq!(before.phase, Phase::Initial);
        let restarted = discovery.refresh(before.revision, true).await.unwrap();
        release.send(()).unwrap();
        assert!(matches!(
            pending.await.unwrap(),
            Err(DiscoveryError::Changed)
        ));
        let after = discovery.state().await.unwrap();
        assert_eq!(after.revision, restarted.revision);
        assert_eq!(after.scan, 2);
        assert!(after.error.is_none());
        assert_eq!(after.phase, Phase::Initial);
        server.finish().await;
        discovery.close().await.unwrap();
    }
}

#[tokio::test]
async fn a_cancelled_read_can_retry_but_a_foreign_google_session_cannot_start_discovery() {
    let (entered, received) = tokio::sync::oneshot::channel();
    let (release, held) = tokio::sync::oneshot::channel();
    let server = Server::start(vec![
        identity(),
        Box::new(move |_| {
            entered.send(()).unwrap();
            TestResponse::json(json!({"startPageToken":"abandoned"})).held(held)
        }),
        start(),
        files(&[], None),
        caught_up("empty-but-verified"),
    ])
    .await;
    let drive = std::sync::Arc::new(server.connect(None).await.unwrap());
    let directory = tempfile::tempdir().unwrap();
    let discovery = Discovery::open(directory.path().join("catalog.sqlite"), scope())
        .await
        .unwrap();
    let cloned = discovery.clone();
    let grant = drive.clone();
    let pending = tokio::spawn(async move { cloned.advance(&grant).await });
    tokio::time::timeout(Duration::from_secs(5), received)
        .await
        .unwrap()
        .unwrap();
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    release.send(()).unwrap();
    assert_eq!(discovery.state().await.unwrap().phase, Phase::Initial);
    let complete = finish(&discovery, &drive).await;
    assert_eq!((complete.files, complete.profiles), (0, 0));
    let foreign = Server::start(vec![value(
        json!({"user":{"permissionId":"another-owner"}}),
    )])
    .await;
    let grant = foreign.connect(None).await.unwrap();
    assert!(matches!(
        discovery.advance(&grant).await,
        Err(DiscoveryError::Binding)
    ));
    assert_eq!(discovery.state().await.unwrap().revision, complete.revision);
    assert_eq!(foreign.finish().await.len(), 1);
    server.finish().await;
    discovery.close().await.unwrap();
}
