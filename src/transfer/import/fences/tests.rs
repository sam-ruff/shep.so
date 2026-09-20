use super::*;
use crate::{
    bulk, folder_actions,
    folders::Mailbox,
    model::{Appearance, WindowSize},
    outgoing::{DeliveryState, SentState, Submission},
    store::MailSelectionId,
};

#[tokio::test]
async fn imported_flag_acknowledgements_keep_cache_only_repair_and_never_become_dispatchable() {
    use crate::{mail_actions::Flags, model::MailQuery};
    let original = tempfile::tempdir().expect("source directory");
    let local = tempfile::tempdir().expect("destination directory");
    let path = original.path().join("source.sqlite");
    let source = super::super::tests::workspace(&path).await;
    let mail = source.query(MailQuery::default()).await.expect("page").rows[0].clone();
    let changes = Flags {
        unread: None,
        starred: Some(!mail.starred),
    };
    source
        .start_individual_mail_action(
            "acknowledged".into(),
            mail.clone(),
            bulk::Action::Flags(changes),
        )
        .await
        .expect("admission");
    let item = source
        .claim_bulk_item("acknowledged".into())
        .await
        .expect("claim")
        .expect("item");
    source
        .acknowledge_bulk_flags(
            item,
            bulk::Receipt::Flags {
                before: Flags {
                    unread: None,
                    starred: Some(mail.starred),
                },
                after: changes,
            },
        )
        .await
        .expect("receipt");
    let destination = Store::open(local.path().join("shep.sqlite")).expect("destination");
    let catalog = crate::profiles::Catalog::open(local.path(), "shep.sqlite").expect("catalogue");
    let prepared = stage(destination, path)
        .await
        .expect("stage")
        .finish()
        .await
        .expect("join")
        .expect("prepared");
    let saved = prepared
        .install(catalog, "Another device".into(), Preferences::default())
        .expect("install")
        .finish()
        .await
        .expect("join")
        .expect("saved");
    let imported = Store::open(saved.path).expect("imported");
    let job = imported.bulk_job("acknowledged".into()).await.expect("job");
    assert!(job.paused);
    assert_eq!(job.uncertain, 0);
    assert!(
        imported
            .claim_bulk_item(job.id.clone())
            .await
            .expect("no provider replay")
            .is_none()
    );
    let lease = imported.bulk_lease(job.id).await.expect("lease");
    imported
        .resume_bulk(&lease)
        .await
        .expect("explicit continue");
    let item = imported
        .pending_bulk_flag_repair(&lease)
        .await
        .expect("repair")
        .expect("receipt");
    imported
        .finish_bulk_item(item, Ok(bulk::Receipt::Unchanged))
        .await
        .expect("cache repair");
    assert_eq!(
        imported
            .mail_metadata(mail.id.clone())
            .await
            .expect("repaired")
            .starred,
        !mail.starred
    );
    assert_eq!(
        source
            .mail_metadata(mail.id)
            .await
            .expect("source unchanged")
            .starred,
        mail.starred
    );
}

#[tokio::test]
async fn calendar_import_fences_queued_work_and_preserves_cache_only_receipts() {
    let original = tempfile::tempdir().expect("original");
    let local = tempfile::tempdir().expect("local");
    let path = original.path().join("source.sqlite");
    let source = super::super::tests::workspace(&path).await;
    source
        .put(
            "calendars",
            vec![crate::model::CalendarSource {
                id: "home".into(),
                name: "Home".into(),
                kind: crate::model::CalendarKind::CalDav,
                url: "https://example.test/home/".into(),
                username: "fixture".into(),
                access: Default::default(),
            }],
        )
        .await
        .expect("source");
    let start = chrono::Utc::now();
    let mut event = crate::model::CalendarEvent {
        id: "one".into(),
        source_id: "home".into(),
        title: "Retained".into(),
        start,
        end: start + chrono::Duration::hours(1),
        all_day: false,
        etag: Some("v1".into()),
        remote_url: None,
        location: String::new(),
        description: String::new(),
    };
    source
        .admit_calendar_action("ack".into(), event.clone(), false)
        .await
        .expect("admit");
    let job = source
        .claim_calendar_action("ack".into())
        .await
        .expect("claim");
    source
        .record_calendar_receipt("ack".into(), job.revision, event.clone())
        .await
        .expect("receipt");
    event.id = "two".into();
    source
        .admit_calendar_action("queued".into(), event, false)
        .await
        .expect("queued");
    let destination = Store::open(local.path().join("shep.sqlite")).expect("destination");
    let catalog = crate::profiles::Catalog::open(local.path(), "shep.sqlite").expect("catalog");
    let prepared = stage(destination, path)
        .await
        .expect("stage")
        .finish()
        .await
        .expect("finish")
        .expect("prepared");
    let saved = prepared
        .install(catalog, "Other device".into(), Preferences::default())
        .expect("install")
        .finish()
        .await
        .expect("finish")
        .expect("saved");
    let imported = Store::open(saved.path).expect("imported");
    assert_eq!(
        imported
            .calendar_job("queued".into())
            .await
            .expect("fenced")
            .status,
        "uncertain"
    );
    assert!(
        imported
            .claim_calendar_action("queued".into())
            .await
            .is_err()
    );
    assert_eq!(
        imported
            .calendar_job("ack".into())
            .await
            .expect("receipt")
            .status,
        "repair"
    );
    imported
        .apply_calendar_receipt("ack".into())
        .await
        .expect("cache only");
    assert!(
        imported
            .next_calendar_action()
            .await
            .expect("next")
            .is_none()
    );
    imported
        .run(|c| {
            assert_eq!(
                count(
                    c,
                    "SELECT count(*) FROM imported_operations WHERE kind='calendar-action'"
                )?,
                2
            );
            Ok(())
        })
        .await
        .expect("archive");
    assert_eq!(
        source
            .calendar_job("queued".into())
            .await
            .expect("original")
            .status,
        "queued"
    );
}

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
    source
        .put(crate::profile_sync::state::NATIVE_EDITS_KEY, value.clone())
        .await
        .unwrap();
    source
        .put(crate::profile_sync::state::STORAGE_KEY, value.clone())
        .await
        .unwrap();
    source
        .put(crate::profile_sync::join::STORAGE_KEY, value.clone())
        .await
        .unwrap();
    source
        .put(crate::profile_sync::vault::STORAGE_KEY, value.clone())
        .await
        .unwrap();
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
    for key in [
        STORAGE_KEY,
        SEED_KEY,
        crate::profile_sync::join::STORAGE_KEY,
        crate::profile_sync::state::STORAGE_KEY,
        crate::profile_sync::state::NATIVE_EDITS_KEY,
        crate::profile_sync::vault::STORAGE_KEY,
    ] {
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
        DeliveryState::Queued,
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
        let mut wire = Submission::new(
            account.clone(),
            &draft,
            crate::compose::build(&account, &draft, vec![]).unwrap(),
        )
        .unwrap();
        if delivery == DeliveryState::Queued {
            wire.info.delivery = DeliveryState::Queued;
        }
        let raw = wire.raw.clone();
        let info = source.begin_outgoing(wire, draft).await.unwrap();
        if !matches!(delivery, DeliveryState::Queued | DeliveryState::Submitting) {
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
        (1, 1, 4, 1)
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
                4
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
            c.execute_batch(
                "DROP TABLE backup_history; DROP TABLE imported_operations; DROP TABLE calendar_actions; DROP TABLE IF EXISTS bulk_flag_receipts; DROP TABLE bulk_field_owners; DROP TABLE bulk_admissions; DROP INDEX bulk_item_unconfirmed_identity; DROP TRIGGER mail_lineage_insert; DROP TRIGGER mail_lineage_replace; DROP TRIGGER mail_lineage_delete; DROP TABLE mail_lineage; DROP TABLE mail_lineage_alias; DROP TABLE mail_identity_history; PRAGMA user_version=2;",
            )?;
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
    {
        let copy = Connection::open(&saved.path).expect("prepared copy");
        assert_eq!(count(&copy, "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name IN ('bulk_flag_receipts','calendar_actions')").expect("action schemas"), 2);
    }
    let imported = Store::open(saved.path).unwrap();
    imported
        .run(|c| {
            assert_eq!(
                count(c, "PRAGMA user_version")?,
                crate::store::DATABASE_VERSION as u64
            );
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
            None,
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

#[tokio::test]
async fn backup_history_version_three_exports_migrate_without_changing_the_source() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("v3.sqlite");
    let source = super::super::tests::workspace(&path).await;
    source
        .run(|c| {
            c.execute_batch("DROP TABLE backup_history; DROP TABLE calendar_actions; DROP TABLE IF EXISTS bulk_flag_receipts; DROP TABLE bulk_field_owners; DROP TABLE bulk_admissions; DROP INDEX bulk_item_unconfirmed_identity; DROP TRIGGER mail_lineage_insert; DROP TRIGGER mail_lineage_replace; DROP TRIGGER mail_lineage_delete; DROP TABLE mail_lineage; DROP TABLE mail_lineage_alias; DROP TABLE mail_identity_history; PRAGMA user_version=3;")?;
            Ok(())
        })
        .await
        .unwrap();
    let destination = Store::open(directory.path().join("shep.sqlite")).unwrap();
    let catalog = crate::profiles::Catalog::open(directory.path(), "shep.sqlite").unwrap();
    let prepared = stage(destination, path)
        .await
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let saved = prepared
        .install(
            catalog,
            "Version three export".into(),
            Preferences::default(),
        )
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let connection = Connection::open(saved.path).unwrap();
    assert_eq!(
        count(&connection, "PRAGMA user_version").unwrap(),
        crate::store::DATABASE_VERSION as u64
    );
    assert_eq!(
        count(&connection, "SELECT count(*) FROM backup_history").unwrap(),
        0
    );
    source
        .run(|c| {
            assert_eq!(count(c, "PRAGMA user_version")?, 3);
            assert_eq!(
                count(
                    c,
                    "SELECT count(*) FROM sqlite_schema WHERE name='backup_history'"
                )?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn backup_history_old_import_marker_recovery_migrates_without_repeating_preparation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("source.sqlite");
    let _source = super::super::tests::workspace(&path).await;
    let destination = Store::open(directory.path().join("shep.sqlite")).unwrap();
    let prepared = stage(destination, path)
        .await
        .unwrap()
        .finish()
        .await
        .unwrap()
        .unwrap();
    let (_alive, cancel) = watch::channel(false);
    apply(
        prepared.path(),
        None,
        prepared.id,
        "Imported once",
        &Preferences::default(),
        &cancel,
    )
    .unwrap();
    let c = Connection::open(prepared.path()).unwrap();
    let archived = count(&c, "SELECT count(*) FROM imported_operations").unwrap();
    let preferences: String = c
        .query_row("SELECT value FROM kv WHERE key='preferences'", [], |r| {
            r.get(0)
        })
        .unwrap();
    c.execute_batch("DROP TABLE backup_history; DROP TABLE calendar_actions; DROP TABLE IF EXISTS bulk_flag_receipts; DROP TABLE bulk_field_owners; DROP TABLE bulk_admissions; DROP INDEX bulk_item_unconfirmed_identity; DROP TRIGGER mail_lineage_insert; DROP TRIGGER mail_lineage_replace; DROP TRIGGER mail_lineage_delete; DROP TABLE mail_lineage; DROP TABLE mail_lineage_alias; DROP TABLE mail_identity_history; PRAGMA user_version=3;")
        .unwrap();
    drop(c);
    // An older app already prepared the same profile before an interrupted
    // publication. Recovery must migrate it without archiving/fencing it again.
    apply(
        prepared.path(),
        None,
        prepared.id,
        "Do not replace prior preparation",
        &Preferences::default(),
        &cancel,
    )
    .unwrap();
    let c = Connection::open(prepared.path()).unwrap();
    assert_eq!(
        count(&c, "PRAGMA user_version").unwrap(),
        crate::store::DATABASE_VERSION as u64
    );
    assert_eq!(count(&c, "SELECT count(*) FROM backup_history").unwrap(), 0);
    assert_eq!(
        count(&c, "SELECT count(*) FROM imported_operations").unwrap(),
        archived
    );
    assert_eq!(
        c.query_row::<String, _, _>("SELECT value FROM kv WHERE key='preferences'", [], |r| r
            .get(0))
            .unwrap(),
        preferences
    );
}
