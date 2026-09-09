use super::*;
use crate::{
    bulk, folder_actions,
    folders::Mailbox,
    model::{Appearance, WindowSize},
    outgoing::{DeliveryState, SentState, Submission},
    store::MailSelectionId,
};

#[tokio::test]
async fn profile_enrollment_is_archived_on_database_import_without_replaying_device_sync() {
    use crate::profile_sync::enrollment::{Enrollment, SEED_KEY, STORAGE_KEY};
    let original = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let path = original.path().join("source.sqlite");
    let source = super::super::tests::workspace(&path).await;
    // Keep future/opaque device state too; it cannot be interpreted as a new
    // target-device enrollment or require its external history to exist here.
    let value = serde_json::json!({"revision":99,"future_saved_choice":true});
    source.put(STORAGE_KEY, value.clone()).await.unwrap();
    source.put(SEED_KEY, value.clone()).await.unwrap();
    let destination = Store::open(local.path().join("shep.sqlite")).unwrap();
    let catalog = crate::profiles::Catalog::open(local.path(), "shep.sqlite").unwrap();
    let prepared = stage(destination, path)
        .await
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let saved = prepared
        .install(catalog, "Another device".into(), Preferences::default())
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let imported = Store::open(saved.path).unwrap();
    assert_eq!(
        imported.profile_enrollment().await.unwrap().enrollment,
        Enrollment::default()
    );
    assert!(
        imported
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .google_lifecycle
            .disconnected
    );
    for key in [STORAGE_KEY, SEED_KEY] {
        let archived: String = imported
            .run(move |c| {
                Ok(c.query_row(
            "SELECT data FROM imported_operations WHERE kind='profile-enrollment' AND identity=?",
            [key], |r| r.get(0))?)
            })
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&archived).unwrap(),
            value
        );
        assert_eq!(source.get::<serde_json::Value>(key).await.unwrap(), value);
        assert!(
            imported
                .get::<Option<serde_json::Value>>(key)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(imported.query(Default::default()).await.unwrap().total, 1);
}

#[tokio::test]
async fn import_preserves_mail_and_receipts_but_requires_review_of_other_device_pending_work() {
    let original = tempfile::tempdir().unwrap();
    let local = tempfile::tempdir().unwrap();
    let path = original.path().join("source.sqlite");
    let source = super::super::tests::workspace(&path).await;
    let selection = MailSelectionId::default();
    source
        .capture_selection(selection, 0, Default::default(), true, vec![])
        .await
        .unwrap();
    let frozen = source.freeze_selection(selection, 0).await.unwrap();
    source
        .start_bulk(
            "pending-bulk".into(),
            frozen.id,
            bulk::Action::Move {
                account: None,
                folder: "Archive".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(
        source
            .query(crate::model::MailQuery {
                folder: "INBOX".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        0
    );

    let account = source.get::<Vec<Account>>("accounts").await.unwrap()[0].clone();
    let mut attempts = Vec::new();
    for (index, delivery) in [
        DeliveryState::Submitting,
        DeliveryState::Rejected,
        DeliveryState::Accepted,
    ]
    .into_iter()
    .enumerate()
    {
        let draft = Draft {
            id: format!("outgoing-{index}"),
            account_id: account.id.clone(),
            to: "friend@example.test".into(),
            subject: "Keep this delivery identity".into(),
            body: "Original unsent text".into(),
            ..Default::default()
        };
        source.save_draft(draft.clone()).await.unwrap();
        let wire = Submission::new(
            account.clone(),
            &draft,
            crate::compose::build(&account, &draft, vec![]).unwrap(),
        )
        .unwrap();
        let raw = wire.raw.clone();
        let info = source.begin_outgoing(wire, draft).await.unwrap();
        if delivery != DeliveryState::Submitting {
            source
                .record_delivery(info.attempt.clone(), delivery, None)
                .await
                .unwrap();
        }
        attempts.push((info.attempt, delivery, raw));
    }
    // A distinct account lets this real folder journal coexist with the group.
    let mut folder_account = account;
    folder_account.id = "folders".into();
    source.save_account(folder_account).await.unwrap();
    source
        .save_folder_catalog(
            "folders".into(),
            ["INBOX", "Projects", "Projects/Child"]
                .into_iter()
                .map(|name| Mailbox {
                    delimiter: Some('/'),
                    ..Mailbox::flat(name.into())
                })
                .collect(),
        )
        .await
        .unwrap();
    let review = source
        .folder_review(
            "folders".into(),
            "Projects".into(),
            folder_actions::Action::Delete,
        )
        .await
        .unwrap();
    source
        .start_folder_change("folder-delete".into(), review)
        .await
        .unwrap();
    let lease = source.folder_lease("folder-delete".into()).await.unwrap();
    let acknowledged = source.claim_folder_step(&lease).await.unwrap().unwrap();
    source
        .record_folder_outcome(
            &lease,
            acknowledged.position,
            folder_actions::Outcome::Applied,
        )
        .await
        .unwrap();
    source
        .run(|c| {
            c.execute(
                "INSERT INTO credential_cleanup VALUES('account','removed','removed:smtp')",
                [],
            )?;
            c.execute(
                "INSERT INTO notification_mailboxes VALUES('work','7',1)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let original_preferences = Preferences {
        appearance: Appearance::Dark,
        backup_folder: "/old/device/backups".into(),
        auto_backup: true,
        backup_ready: true,
        google_connection_id: "drive:fixture".into(),
        ..Default::default()
    };
    source
        .save_preferences(original_preferences.clone())
        .await
        .unwrap();
    let local_preferences = Preferences {
        window_size: Some(WindowSize {
            width: 1100.,
            height: 720.,
        }),
        backup_folder: local.path().join("backups").display().to_string(),
        ..Default::default()
    };
    let destination = Store::open(local.path().join("shep.sqlite")).unwrap();
    let catalog = crate::profiles::Catalog::open(local.path(), "shep.sqlite").unwrap();
    let prepared = stage(destination, path)
        .await
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            prepared.review.pending_bulk,
            prepared.review.pending_folders,
            prepared.review.pending_outgoing,
            prepared.review.pending_credentials
        ),
        (1, 1, 3, 1)
    );
    let saved = prepared
        .install(catalog, "Reviewed import".into(), local_preferences.clone())
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let imported = Store::open(saved.path).unwrap();
    assert_eq!(
        imported
            .query(crate::model::MailQuery {
                folder: "INBOX".into(),
                ..Default::default()
            })
            .await
            .unwrap()
            .total,
        1,
        "Imported speculative folder projections must not hide original mail"
    );
    let job = imported.bulk_job("pending-bulk".into()).await.unwrap();
    assert!(job.paused);
    assert_eq!(job.uncertain, 1);
    assert!(imported.claim_bulk_item(job.id).await.unwrap().is_none());
    assert!(
        imported
            .next_pending_bulk(String::new())
            .await
            .unwrap()
            .is_none()
    );
    let folder = imported.folder_job("folder-delete".into()).await.unwrap();
    assert_eq!(
        folder.steps[acknowledged.position].status,
        folder_actions::Status::Acknowledged
    );
    assert!(
        folder
            .steps
            .iter()
            .filter(|s| s.position != acknowledged.position)
            .all(|s| s.status == folder_actions::Status::Uncertain)
    );
    for (attempt, delivery, raw) in attempts {
        let wire = imported.outgoing_submission(attempt.clone()).await.unwrap();
        assert_eq!(wire.raw, raw);
        assert_eq!(
            wire.info.delivery,
            if delivery == DeliveryState::Accepted {
                DeliveryState::Accepted
            } else {
                DeliveryState::Uncertain
            }
        );
        assert_eq!(
            wire.info.sent,
            if delivery == DeliveryState::Accepted {
                SentState::Uncertain
            } else {
                SentState::Pending
            }
        );
        assert!(wire.info.error.as_ref().unwrap().contains("another device"));
        assert_eq!(
            source.outgoing_info(attempt).await.unwrap().delivery,
            delivery
        );
    }
    let prefs = imported.get::<Preferences>("preferences").await.unwrap();
    assert_eq!(prefs.appearance, Appearance::Dark);
    assert_eq!(prefs.window_size, local_preferences.window_size);
    assert_eq!(prefs.backup_folder, local_preferences.backup_folder);
    assert!(!prefs.auto_backup && !prefs.backup_ready && prefs.last_backup.is_none());
    assert!(prefs.google_connection_id.is_empty() && prefs.google_lifecycle.disconnected);
    imported
        .run(|c| {
            assert_eq!(count(c, "SELECT count(*) FROM credential_cleanup")?, 0);
            assert_eq!(count(c, "SELECT count(*) FROM notification_seen")?, 0);
            assert_eq!(count(c, "SELECT count(*) FROM notification_mailboxes")?, 0);
            assert_eq!(
                count(
                    c,
                    "SELECT count(*) FROM imported_operations WHERE kind='outgoing'"
                )?,
                3
            );
            assert_eq!(
                count(
                    c,
                    "SELECT count(*) FROM imported_operations WHERE kind='bulk-effect'"
                )?,
                1
            );
            assert_eq!(
                count(
                    c,
                    "SELECT count(*) FROM imported_operations WHERE kind='credential-cleanup'"
                )?,
                1
            );
            assert_eq!(
                count(
                    c,
                    "SELECT count(*) FROM imported_operations WHERE kind='folder-step'"
                )?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        source
            .get::<Preferences>("preferences")
            .await
            .unwrap()
            .auto_backup
    );
    assert_eq!(
        source
            .bulk_job("pending-bulk".into())
            .await
            .unwrap()
            .remaining,
        1
    );
}

#[tokio::test]
async fn version_two_exports_migrate_privately_and_future_stores_are_not_modified() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v2.sqlite");
    let source = super::super::tests::workspace(&path).await;
    source
        .run(|c| {
            c.execute_batch("DROP TABLE imported_operations; PRAGMA user_version=2;")?;
            Ok(())
        })
        .await
        .unwrap();
    let destination = Store::open(directory.path().join("shep.sqlite")).unwrap();
    let catalog = crate::profiles::Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let prepared = stage(destination, path.clone())
        .await
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let saved = prepared
        .install(catalog, "Old export".into(), Preferences::default())
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let imported = Store::open(saved.path).unwrap();
    imported
        .run(|c| {
            assert_eq!(count(c, "PRAGMA user_version")?, 3);
            assert_eq!(
                count(
                    c,
                    "SELECT count(*) FROM imported_operations WHERE kind='preferences'"
                )?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
    source
        .run(|c| {
            assert_eq!(count(c, "PRAGMA user_version")?, 2);
            c.execute_batch("PRAGMA user_version=999;")?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(Store::open(path).is_err());
    source
        .run(|c| {
            assert_eq!(count(c, "PRAGMA user_version")?, 999);
            assert_eq!(
                count(
                    c,
                    "SELECT count(*) FROM sqlite_schema WHERE name='imported_operations'"
                )?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn failure_halfway_through_preparation_rolls_back_operation_changes_and_no_profile_is_published()
 {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.sqlite");
    let source = super::super::tests::workspace(&path).await;
    source.run(|c| {
        c.execute("INSERT INTO bulk_jobs(id,action,source,created) VALUES('job','{}','fixture',0)",[])?;
        c.execute("INSERT INTO bulk_items(job,position,id,status) VALUES('job',0,'mail','queued')",[])?;
        c.execute("INSERT INTO outgoing(draft,attempt,account,stage,created,data,logical_id) VALUES('draft','attempt','work','Submitting',0,'broken JSON','message')",[])?;
        Ok(())
    }).await.unwrap();
    let destination = Store::open(directory.path().join("shep.sqlite")).unwrap();
    let prepared = stage(destination, path)
        .await
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let (_alive, cancel) = watch::channel(false);
    assert!(
        apply(
            prepared.path(),
            prepared.id,
            "Broken outgoing record",
            &Preferences::default(),
            &cancel
        )
        .is_err()
    );
    let c = Connection::open_with_flags(prepared.path(), OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(
        count(&c, "SELECT count(*) FROM imported_operations").unwrap(),
        0
    );
    assert_eq!(
        count(&c, "SELECT count(*) FROM bulk_items WHERE status='queued'").unwrap(),
        1
    );
    assert_eq!(count(&c, "SELECT paused FROM bulk_jobs").unwrap(), 0);
    assert_eq!(
        count(
            &c,
            "SELECT count(*) FROM kv WHERE key='profile_import_ready_v1'"
        )
        .unwrap(),
        0
    );
}
