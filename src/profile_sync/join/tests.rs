use super::*;
use crate::{
    model::*,
    profile_sync::drive::tests::{binding, file, fixture, session},
    providers::test_http::{Reply, Server},
};
use serde_json::json;

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
                    ..Default::default()
                },
            };
            p.appearance = Appearance::Light;
            p.backup_folder = "/device-only".into();
        })
        .await
        .unwrap();
    store
}
fn record() -> ReservedUpload {
    let mut op = fixture().operation().clone();
    op.parents.clear();
    let record = Record::decode(binding().namespace(), op.encode().unwrap()).unwrap();
    ReservedUpload {
        binding: binding(),
        remote: RemoteRecord {
            id: "original-profile".into(),
            key: record.key(),
            size: record.bytes().len() as u64,
            sha256: record.sha256.clone(),
        },
        record,
    }
}
fn download(record: &ReservedUpload) -> Vec<Reply> {
    vec![
        Reply::new(200, file(record).to_string()),
        Reply::binary(200, record.record.bytes().to_vec()),
    ]
}
fn scan(record: &ReservedUpload) -> Vec<Reply> {
    let mut replies = vec![
        Reply::new(
            200,
            json!({"user":{"permissionId":"fixture-user"}}).to_string(),
        ),
        Reply::new(200, json!({"startPageToken":"before"}).to_string()),
        Reply::new(
            200,
            json!({"files":[file(record)],"incompleteSearch":false}).to_string(),
        ),
    ];
    replies.extend(download(record));
    replies.push(Reply::new(
        200,
        json!({"changes":[],"newStartPageToken":"after"}).to_string(),
    ));
    replies
}
async fn reviewed(store: &Store, paths: &Paths, record: &ReservedUpload) -> Review {
    let mut replies = scan(record);
    replies.push(Reply::new(200, json!({"files":[file(record)]}).to_string()));
    replies.extend(download(record));
    let mut server = Server::start(replies).await;
    let session = session(&server);
    let discovery = setup::Discovery::from_catalog(
        catalog::discover(store, &session, paths, &Control::default())
            .await
            .unwrap(),
    );
    assert_eq!(
        discovery.profiles()[0].name.as_deref(),
        Some("Personal devices")
    );
    let review = prepare(
        store,
        &session,
        paths,
        paths.journal().await.unwrap(),
        &discovery,
        &discovery.profiles()[0].cursor(),
        &Control::default(),
    )
    .await
    .unwrap();
    server.finish().await;
    assert!(server.requests().iter().all(|r| r.method == "GET"));
    review
}

#[tokio::test]
async fn profile_join_applies_reviewed_values_once_preserves_local_accounts_and_requires_reconnection_after_restart()
 {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let paths = Paths::for_cache(&dir.path().join("cache.sqlite")).unwrap();
    let record = record();
    let Action::AccountConnection {
        account: connection,
    } = &record.record.operation().changes[0].action
    else {
        panic!("account")
    };
    let local_account = metadata::review_account(connection, "Existing local account").unwrap();
    store.save_account(local_account.clone()).await.unwrap();
    let review = reviewed(&store, &paths, &record).await;
    assert_eq!((review.accounts, review.settings), (1, 1));
    assert_eq!(
        store.workspace().await.unwrap().accounts.len(),
        1,
        "Review does not apply"
    );
    let snapshot = accept(&store, &paths, review.clone(), &Control::default())
        .await
        .unwrap();
    let baseline = store
        .profile_replication(review.selection.binding.clone())
        .await
        .unwrap();
    assert_eq!(baseline.revision, review.revision);
    assert_eq!(
        baseline.local_only,
        BTreeSet::from([local_account.id.clone()])
    );
    assert_eq!(
        baseline.fields["setting:appearance"]
            .remote
            .as_ref()
            .unwrap(),
        &record.record.operation().changes[2]
    );
    assert!(!baseline.fields.contains_key("setting:preview_lines"));
    assert!(store.capture_profile_change().await.unwrap().is_none());
    assert_eq!(snapshot.enrollment.selection.unwrap().origin, Origin::Join);
    let workspace = store.workspace().await.unwrap();
    assert_eq!(workspace.accounts.len(), 2);
    assert_eq!(workspace.preferences.appearance, Appearance::Dark);
    assert_eq!(workspace.preferences.backup_folder, "/device-only");
    assert_eq!(workspace.accounts[0].name, local_account.name);
    let added = &workspace.accounts[1];
    assert_eq!(baseline.accounts.get(&added.id), Some(&connection.id));
    assert_ne!(added.id, connection.id.to_string());
    assert_eq!(added.smtp_username, connection.smtp_username);
    assert!(workspace.account_reconnect.contains(&added.id));
    assert!(
        store
            .require_account_reconnected(added.id.clone())
            .await
            .is_err()
    );
    assert_eq!(store.accounts_ready_to_sync().await.unwrap().len(), 1);
    store
        .update_preferences(|p| p.appearance = Appearance::System)
        .await
        .unwrap();
    drop(store);
    let store = Store::open(dir.path().join("cache.sqlite")).unwrap();
    accept(&store, &paths, review.clone(), &Control::default())
        .await
        .unwrap();
    let preserved = store
        .profile_replication(review.selection.binding)
        .await
        .unwrap();
    assert_eq!(preserved.fields, baseline.fields);
    let next = store.capture_profile_change().await.unwrap().unwrap();
    assert_eq!(next.expected_revision, baseline.revision);
    assert_eq!(
        next.change.extra,
        record.record.operation().changes[2].extra
    );
    assert!(matches!(next.change.action,Action::Setting {value,..} if value == "System"));
    let reopened = store.workspace().await.unwrap();
    assert_eq!(
        reopened.accounts.len(),
        2,
        "Lost acknowledgment must not add duplicates"
    );
    assert_eq!(
        reopened.preferences.appearance,
        Appearance::System,
        "Retry preserves later edits"
    );
    assert!(
        store
            .require_account_reconnected(added.id.clone())
            .await
            .is_err()
    );
    // This storage acknowledgment is called by production SaveAccount only
    // after the keychain write succeeds. The test never accesses a keychain.
    store.save_account(added.clone()).await.unwrap();
    store
        .require_account_reconnected(added.id.clone())
        .await
        .unwrap();
    assert_eq!(store.accounts_ready_to_sync().await.unwrap().len(), 2);
}

#[tokio::test]
async fn profile_join_categories_stale_local_or_google_intent_and_stop_prevent_partial_application()
{
    for failure in ["preferences", "categories", "google", "stop"] {
        let dir = tempfile::tempdir().unwrap();
        let store = local(dir.path()).await;
        let paths = Paths::for_cache(&dir.path().join("cache.sqlite")).unwrap();
        let review = reviewed(&store, &paths, &record()).await;
        let (stop, control) = Control::channel();
        match failure {
            "preferences" => {
                store
                    .update_preferences(|p| p.unified_inbox = false)
                    .await
                    .unwrap();
            }
            "categories" => {
                store
                    .change_profile_sync_options(Changes {
                        accounts: Some(false),
                        ..Default::default()
                    })
                    .await
                    .unwrap();
            }
            "google" => {
                store
                    .update_preferences(|p| p.google_lifecycle.disconnected = true)
                    .await
                    .unwrap();
            }
            _ => {
                stop.send_replace(true);
            }
        }
        assert!(
            accept(&store, &paths, review, &control).await.is_err(),
            "{failure}"
        );
        let workspace = store.workspace().await.unwrap();
        assert!(workspace.accounts.is_empty());
        assert_eq!(workspace.preferences.appearance, Appearance::Light);
        assert!(
            store
                .profile_enrollment()
                .await
                .unwrap()
                .enrollment
                .selection
                .is_none()
        );
    }
    for accounts in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = local(dir.path()).await;
        let paths = Paths::for_cache(&dir.path().join("cache.sqlite")).unwrap();
        store
            .change_profile_sync_options(Changes {
                accounts: Some(accounts),
                settings: Some(!accounts),
                enabled: None,
            })
            .await
            .unwrap();
        let review = reviewed(&store, &paths, &record()).await;
        assert_eq!(
            (review.accounts, review.settings),
            (usize::from(accounts), usize::from(!accounts))
        );
        accept(&store, &paths, review, &Control::default())
            .await
            .unwrap();
        let workspace = store.workspace().await.unwrap();
        assert_eq!(workspace.accounts.len(), usize::from(accounts));
        assert_eq!(
            workspace.preferences.appearance,
            if accounts {
                Appearance::Light
            } else {
                Appearance::Dark
            }
        );
    }
}

#[tokio::test]
async fn profile_catalog_refresh_uses_saved_changes_and_old_reviews_cannot_create_or_page() {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let paths = Paths::for_cache(&dir.path().join("cache.sqlite")).unwrap();
    let mut replies = scan(&record());
    replies.extend([
        Reply::new(
            200,
            json!({"user":{"permissionId":"fixture-user"}}).to_string(),
        ),
        Reply::new(
            200,
            json!({"changes":[],"newStartPageToken":"latest"}).to_string(),
        ),
    ]);
    let mut server = Server::start(replies).await;
    let session = session(&server);
    let first = setup::Discovery::from_catalog(
        catalog::discover(&store, &session, &paths, &Control::default())
            .await
            .unwrap(),
    );
    let next = first
        .page(&store, Some(first.profiles()[0].cursor()))
        .await
        .unwrap();
    assert!(next.profiles().is_empty());
    let fresh = next.page(&store, None).await.unwrap();
    assert_eq!(fresh.profile_count(), 1);
    let latest = catalog::discover(&store, &session, &paths, &Control::default())
        .await
        .unwrap();
    assert!(first.page(&store, None).await.is_err());
    assert!(
        setup::create(
            &store,
            &session,
            &paths.journal().await.unwrap(),
            first,
            "New".into(),
            Options {
                enabled: true,
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(latest.profiles[0].name.as_deref(), Some("Personal devices"));
    server.finish().await;
    assert_eq!(
        server
            .requests()
            .iter()
            .filter(|r| r.target.contains("startPageToken"))
            .count(),
        1
    );
    assert!(
        server
            .requests()
            .last()
            .unwrap()
            .target
            .contains("pageToken=after")
    );
}

#[tokio::test]
async fn profile_catalog_held_read_cancels_and_reopens_its_owned_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let paths = Paths::for_cache(&dir.path().join("cache.sqlite")).unwrap();
    let (held, entered, release) = Reply::new(200, "{}").held();
    let mut server = Server::start(vec![
        Reply::new(
            200,
            json!({"user":{"permissionId":"fixture-user"}}).to_string(),
        ),
        Reply::new(200, json!({"startPageToken":"saved"}).to_string()),
        held,
    ])
    .await;
    let session = session(&server);
    let (stop, control) = Control::channel();
    let task = {
        let store = store.clone();
        let paths = paths.clone();
        tokio::spawn(async move { catalog::discover(&store, &session, &paths, &control).await })
    };
    entered.await.unwrap();
    stop.send_replace(true);
    let result = tokio::time::timeout(std::time::Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap();
    assert!(result.unwrap_err().is::<control::Stopped>());
    release.send(()).unwrap();
    server.finish().await;
    let scope = shep_profile_core::drive::catalog::Scope {
        namespace: binding().namespace().into(),
        principal: binding().identity().into(),
    };
    let catalog =
        shep_profile_core::drive::catalog::Discovery::open(paths.catalog(&scope).unwrap(), scope)
            .await
            .unwrap();
    assert_eq!(
        catalog.state().await.unwrap().phase,
        shep_profile_core::drive::catalog::Phase::Files
    );
    catalog.close().await.unwrap();
}

#[tokio::test]
async fn profile_join_rolls_back_accounts_if_settings_fail_and_rejects_changed_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = local(dir.path()).await;
    let paths = Paths::for_cache(&dir.path().join("cache.sqlite")).unwrap();
    let record = record();
    let review = reviewed(&store, &paths, &record).await;
    let Action::AccountConnection { account } = &record.record.operation().changes[0].action else {
        panic!("account")
    };
    let values = Values {
        accounts: vec![metadata::review_account(account, "Cloud").unwrap()],
        account_changes: vec![record.record.operation().changes[0].clone()],
        settings: vec![Change {
            action: Action::Setting {
                key: shep_profile_core::SettingKey::Appearance,
                value: json!("Invalid"),
            },
            extra: Default::default(),
        }],
    };
    assert!(
        store
            .accept_profile_join(review.clone(), values)
            .await
            .is_err()
    );
    let workspace = store.workspace().await.unwrap();
    assert!(workspace.accounts.is_empty());
    assert!(workspace.account_reconnect.is_empty());
    assert!(
        store
            .profile_enrollment()
            .await
            .unwrap()
            .enrollment
            .selection
            .is_none()
    );
    let mut replica = Replica::open(
        paths.history(&review.selection.binding).unwrap(),
        review.selection.binding.clone(),
        paths.journal().await.unwrap(),
    )
    .await
    .unwrap();
    replica
        .edit(history::LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: review.revision,
            changes: vec![Change {
                action: Action::ProfileName {
                    name: "Changed elsewhere".into(),
                },
                extra: Default::default(),
            }],
            resolutions: vec![],
        })
        .await
        .unwrap();
    replica.close().await.unwrap();
    assert!(
        accept(&store, &paths, review, &Control::default())
            .await
            .is_err()
    );
    assert!(store.workspace().await.unwrap().accounts.is_empty());
}

#[tokio::test]
async fn profile_join_does_not_apply_unknown_connection_fields_or_resurrect_removed_accounts() {
    for removed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = local(dir.path()).await;
        let paths = Paths::for_cache(&dir.path().join("cache.sqlite")).unwrap();
        let mut op = record().record.operation().clone();
        let Action::AccountConnection { account } = &mut op.changes[0].action else {
            panic!("account")
        };
        if !removed {
            account
                .extra
                .insert("future_tls_requirement".into(), json!(true));
        }
        let account = account.id;
        let previous = if removed {
            let previous = record();
            op.parents = vec![op.operation];
            op.operation = Uuid::new_v4();
            op.changes = vec![Change {
                action: Action::AccountRemoved { id: account },
                extra: Default::default(),
            }];
            Some(previous)
        } else {
            None
        };
        let raw = Record::decode(binding().namespace(), op.encode().unwrap()).unwrap();
        let record = ReservedUpload {
            binding: binding(),
            remote: RemoteRecord {
                id: if removed {
                    "removed-profile"
                } else {
                    "original-profile"
                }
                .into(),
                key: raw.key(),
                size: raw.bytes().len() as u64,
                sha256: raw.sha256.clone(),
            },
            record: raw,
        };
        let records = previous
            .iter()
            .chain(std::iter::once(&record))
            .collect::<Vec<_>>();
        let metadata = records.iter().map(|r| file(r)).collect::<Vec<_>>();
        let listing = json!({"files":metadata,"incompleteSearch":false});
        let mut replies = vec![
            Reply::new(
                200,
                json!({"user":{"permissionId":"fixture-user"}}).to_string(),
            ),
            Reply::new(200, json!({"startPageToken":"before"}).to_string()),
            Reply::new(200, listing.to_string()),
        ];
        for r in &records {
            replies.extend(download(r));
        }
        replies.push(Reply::new(
            200,
            json!({"changes":[],"newStartPageToken":"after"}).to_string(),
        ));
        replies.push(Reply::new(200, listing.to_string()));
        for r in &records {
            replies.extend(download(r));
        }
        let mut server = Server::start(replies).await;
        let session = session(&server);
        let discovery = setup::Discovery::from_catalog(
            catalog::discover(&store, &session, &paths, &Control::default())
                .await
                .unwrap(),
        );
        let result = prepare(
            &store,
            &session,
            &paths,
            paths.journal().await.unwrap(),
            &discovery,
            &discovery.profiles()[0].cursor(),
            &Control::default(),
        )
        .await;
        if removed {
            let review = result.unwrap();
            assert_eq!(review.accounts, 0);
            accept(&store, &paths, review, &Control::default())
                .await
                .unwrap();
        } else {
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("additional connection fields")
            );
        }
        assert!(store.workspace().await.unwrap().accounts.is_empty());
        server.finish().await;
    }
}
