use super::*;
use crate::{
    model::{Appearance, Preferences},
    profiles::{
        discovery::{Action as DiscoveryAction, Grant, Session},
        fixture::{Fixture, NAMESPACE},
    },
};
use serde_json::json;
use shep_profile_core::{
    SettingKey,
    drive::{Drive, catalog::Phase},
};
use std::{collections::BTreeSet, time::Duration};

async fn setup(root: &Path) -> (Store, Fixture, Drive, Session) {
    let db = Store::open(root.join("mail.sqlite")).unwrap();
    let mut prefs = Preferences::default();
    prefs.google_grant.id = "fixture-grant".into();
    prefs.google_grant.access.known = true;
    prefs.google_grant.access.drive = true;
    prefs.google_connection_id = "drive:fixture".into();
    db.put("preferences", prefs.clone()).await.unwrap();
    let fixture = Fixture::start(1, false, Duration::ZERO).await.unwrap();
    let drive = fixture
        .connect(NAMESPACE.into(), "drive:fixture")
        .await
        .unwrap();
    let mut session = Session::open(
        root.join("discovery"),
        Uuid::new_v4(),
        Grant::from_preferences(&prefs),
        &drive,
    )
    .await
    .unwrap();
    for _ in 0..100 {
        let o = session
            .run(DiscoveryAction::Advance, Some(&drive))
            .await
            .unwrap();
        assert!(o.error.is_none(), "{:?}", o.error);
        if o.state.unwrap().phase == Phase::Complete {
            return (db, fixture, drive, session);
        }
    }
    panic!("discovery did not finish")
}
async fn prepare(db: &Store, session: &mut Session) -> Review {
    let state = session.observe(None).await.unwrap();
    let p = &state.rows[0];
    let id = state.enrollment.next_id.unwrap();
    let o = session
        .run_enrollment(
            db,
            Command::Prepare {
                id,
                profile: p.profile,
                generation: p.generation,
                revision: p.revision,
            },
        )
        .await
        .unwrap();
    assert!(o.error.is_none(), "{:?}", o.error);
    o.enrollment.review.unwrap()
}
async fn until(db: &Store, session: &mut Session, id: Uuid, phase: &str) -> Review {
    for _ in 0..1000 {
        let o = session
            .run_enrollment(db, Command::Step { id })
            .await
            .unwrap();
        assert!(o.error.is_none(), "{:?}", o.error);
        let review = o.enrollment.review.unwrap();
        if review.phase == phase {
            return review;
        }
    }
    panic!("enrollment did not reach {phase}")
}
#[tokio::test]
async fn original_records_and_atomic_application_survive_lost_receipts_and_reopen() {
    let root = tempfile::tempdir().unwrap();
    let (db, _fixture, drive, mut session) = setup(root.path()).await;
    let original = prepare(&db, &mut session).await;
    db.run(|db|{db.execute_batch("CREATE TRIGGER lose_cursor BEFORE UPDATE ON profile_enrollments WHEN json_extract(NEW.review,'$.cursor')>json_extract(OLD.review,'$.cursor') BEGIN SELECT RAISE(FAIL,'synthetic lost cursor');END;")?;Ok(())}).await.unwrap();
    let failed = session
        .run_enrollment(&db, Command::Step { id: original.id })
        .await
        .unwrap();
    assert!(failed.error.unwrap().contains("synthetic lost cursor"));
    db.run(|db| {
        db.execute_batch("DROP TRIGGER lose_cursor")?;
        Ok(())
    })
    .await
    .unwrap();
    let review = until(&db, &mut session, original.id, "review").await;
    assert_eq!(review.copied, 3);
    assert_eq!(review.rows, 2);
    let history = Worker::open(
        root.path()
            .join("discovery/histories")
            .join(format!("{}.sqlite", review.binding.storage_key().unwrap())),
        review.binding.clone(),
    )
    .await
    .unwrap();
    let Reply::State(state) = history.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(state.operations, 3);
    let Reply::Upload(upload) = history.request(HistoryCommand::NextUpload).await.unwrap() else {
        panic!()
    };
    assert!(upload.is_none());
    history.close().await.unwrap();
    let approved = session
        .run_enrollment(
            &db,
            Command::Approve {
                id: review.id,
                accounts: true,
                settings: true,
            },
        )
        .await
        .unwrap();
    assert!(approved.error.is_none(), "{:?}", approved.error);
    db.run(|db|{db.execute_batch("CREATE TRIGGER lose_apply BEFORE UPDATE OF receipt ON profile_enrollment_rows WHEN NEW.receipt IS NOT NULL BEGIN SELECT RAISE(FAIL,'synthetic lost application receipt');END;")?;Ok(())}).await.unwrap();
    let failed = session
        .run_enrollment(&db, Command::Step { id: review.id })
        .await
        .unwrap();
    assert!(
        failed
            .error
            .unwrap()
            .contains("synthetic lost application receipt")
    );
    assert!(db.get::<Vec<Account>>("accounts").await.unwrap().is_empty());
    db.run(|db| {
        db.execute_batch("DROP TRIGGER lose_apply")?;
        Ok(())
    })
    .await
    .unwrap();
    let complete = until(&db, &mut session, review.id, "complete").await;
    assert_eq!(complete.applied, 1);
    let accounts: Vec<Account> = db.get("accounts").await.unwrap();
    assert_eq!(accounts.len(), 1);
    let account = accounts[0].clone();
    assert!(
        db.require_profile_active(account.id.clone())
            .await
            .unwrap_err()
            .to_string()
            .contains("Reconnect")
    );
    assert_eq!(
        db.get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::Dark
    );
    session.close().await.unwrap();
    drop(db);
    let db = Store::open(root.path().join("mail.sqlite")).unwrap();
    let prefs: Preferences = db.get("preferences").await.unwrap();
    let mut session = Session::open(
        root.path().join("discovery"),
        Uuid::new_v4(),
        Grant::from_preferences(&prefs),
        &drive,
    )
    .await
    .unwrap();
    let o = session.run_enrollment(&db, Command::Current).await.unwrap();
    assert_eq!(o.enrollment.review.unwrap().id, review.id);
    let o = session
        .run_enrollment(&db, Command::Step { id: review.id })
        .await
        .unwrap();
    assert!(o.error.is_none());
    assert_eq!(db.get::<Vec<Account>>("accounts").await.unwrap(), accounts);
    assert!(db.profile_reconnect_required(account.id).await.unwrap());
    session.close().await.unwrap();
}
#[tokio::test]
async fn newer_reverted_preference_intent_is_kept_and_old_unrelated_saves_preserve_applied_fields()
{
    let root = tempfile::tempdir().unwrap();
    let (db, _fixture, _drive, mut session) = setup(root.path()).await;
    let review = prepare(&db, &mut session).await;
    until(&db, &mut session, review.id, "review").await;
    let stale: Preferences = db.get("preferences").await.unwrap();
    db.save_profile_preferences(stale.clone(), BTreeSet::from([SettingKey::Appearance]))
        .await
        .unwrap();
    session
        .run_enrollment(
            &db,
            Command::Approve {
                id: review.id,
                accounts: false,
                settings: true,
            },
        )
        .await
        .unwrap();
    let complete = until(&db, &mut session, review.id, "complete").await;
    assert_eq!(
        complete.settings_receipt.unwrap()["kept"],
        json!(["setting:appearance"])
    );
    assert_eq!(
        db.get::<Preferences>("preferences")
            .await
            .unwrap()
            .appearance,
        Appearance::System
    );
    let review = prepare(&db, &mut session).await;
    until(&db, &mut session, review.id, "review").await;
    session
        .run_enrollment(
            &db,
            Command::Approve {
                id: review.id,
                accounts: false,
                settings: true,
            },
        )
        .await
        .unwrap();
    until(&db, &mut session, review.id, "complete").await;
    let mut old = stale;
    old.reader_font_size = 22;
    let saved = db
        .save_profile_preferences(old, BTreeSet::new())
        .await
        .unwrap();
    assert_eq!(saved.value.appearance, Appearance::Dark);
    assert_eq!(saved.value.reader_font_size, 22);
    assert_eq!(saved.value.google_connection_id, "drive:fixture");
    assert!(db.get::<Vec<Account>>("accounts").await.unwrap().is_empty());
    session.close().await.unwrap();
}

#[tokio::test]
async fn matching_account_keeps_local_metadata_mail_drafts_and_newer_connection_intent() {
    let root = tempfile::tempdir().unwrap();
    let (db, _fixture, _drive, mut session) = setup(root.path()).await;
    let account:Account=serde_json::from_value(json!({"id":Uuid::from_u128(10000).to_string(),"name":"My local name","email":"shared-0@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"shared-0","smtp_host":"smtp.example.test","smtp_port":465})).unwrap();
    db.save_account(account.clone()).await.unwrap();
    let mail=crate::model::parse_mail(&account.id,"1","Inbox",b"From: friend@example.test\r\nTo: shared-0@example.test\r\nSubject: Keep this cached message\r\n\r\nLocal mail".to_vec(),true,false).unwrap();
    db.upsert(vec![mail.clone()]).await.unwrap();
    let draft = crate::model::Draft {
        id: Uuid::new_v4().to_string(),
        account_id: account.id.clone(),
        body: "Keep this unsent draft".into(),
        ..Default::default()
    };
    db.save_draft(draft.clone()).await.unwrap();
    let before = db.workspace().await.unwrap();
    let review = prepare(&db, &mut session).await;
    until(&db, &mut session, review.id, "review").await;
    session
        .run_enrollment(
            &db,
            Command::Approve {
                id: review.id,
                accounts: true,
                settings: false,
            },
        )
        .await
        .unwrap();
    let complete = until(&db, &mut session, review.id, "complete").await;
    assert_eq!(complete.applied, 1);
    assert_eq!(
        db.get::<Vec<Account>>("accounts").await.unwrap(),
        vec![account.clone()]
    );
    assert!(
        !db.profile_reconnect_required(account.id.clone())
            .await
            .unwrap()
    );
    assert_eq!(db.export().await.unwrap()[0].raw, mail.raw);
    assert_eq!(db.workspace().await.unwrap().drafts, before.drafts);
    let review = prepare(&db, &mut session).await;
    until(&db, &mut session, review.id, "review").await;
    session
        .run_enrollment(
            &db,
            Command::Approve {
                id: review.id,
                accounts: true,
                settings: false,
            },
        )
        .await
        .unwrap();
    let mut changed = account;
    changed.host = "new-local.example.test".into();
    db.save_account(changed.clone()).await.unwrap();
    let complete = until(&db, &mut session, review.id, "complete").await;
    assert_eq!(complete.kept, 1);
    assert_eq!(complete.applied, 0);
    assert_eq!(
        db.get::<Vec<Account>>("accounts").await.unwrap(),
        vec![changed]
    );
    session.close().await.unwrap();
}

#[tokio::test]
async fn differing_connection_requires_explicit_separate_account_and_removal_after_approval_wins() {
    let root = tempfile::tempdir().unwrap();
    let (db, _fixture, _drive, mut session) = setup(root.path()).await;
    let account:Account=serde_json::from_value(json!({"id":Uuid::from_u128(10000).to_string(),"name":"Local","email":"shared-0@example.test","protocol":"Imap","host":"local.example.test","port":993,"username":"shared-0","smtp_host":"smtp.example.test","smtp_port":465})).unwrap();
    db.save_account(account.clone()).await.unwrap();
    for remove_after_approval in [false, true] {
        let review = prepare(&db, &mut session).await;
        until(&db, &mut session, review.id, "review").await;
        let rows = session.observe(None).await.unwrap().enrollment.rows;
        let row = rows.iter().find(|r| r.kind == "account").unwrap();
        if !remove_after_approval {
            assert!(!row.selected);
            assert!(row.reason.as_ref().unwrap().contains("connection differs"));
        }
        let o = session
            .run_enrollment(
                &db,
                Command::Choose {
                    id: review.id,
                    position: row.position,
                    selected: true,
                },
            )
            .await
            .unwrap();
        assert!(o.error.is_none());
        session
            .run_enrollment(
                &db,
                Command::Approve {
                    id: review.id,
                    accounts: true,
                    settings: false,
                },
            )
            .await
            .unwrap();
        if remove_after_approval {
            let target = crate::store::ConnectionRef {
                kind: crate::store::ConnectionKind::Account,
                id: row.local_id.clone().unwrap(),
            };
            let preview = db.removal_preview(target).await.unwrap();
            db.remove_connection(preview, false).await.unwrap();
        }
        let complete = until(&db, &mut session, review.id, "complete").await;
        let accounts: Vec<Account> = db.get("accounts").await.unwrap();
        assert!(accounts.contains(&account));
        if remove_after_approval {
            assert_eq!(complete.kept, 1);
            assert_eq!(accounts.len(), 1);
        } else {
            assert_eq!(complete.applied, 1);
            assert_eq!(accounts.len(), 2);
            let imported = accounts.iter().find(|a| a.id != account.id).unwrap();
            assert_eq!(imported.host, "imap.example.test");
            assert!(
                db.profile_reconnect_required(imported.id.clone())
                    .await
                    .unwrap()
            );
        }
    }
    session.close().await.unwrap();
}
#[tokio::test]
async fn independent_offline_history_is_retained_and_large_review_pages_keep_unsupported_settings()
{
    use shep_profile_core::{Action, history::LocalEdit};
    let root = tempfile::tempdir().unwrap();
    let (db, _fixture, _drive, mut session) = setup(root.path()).await;
    let original = prepare(&db, &mut session).await;
    let original = until(&db, &mut session, original.id, "review").await;
    let base = session
        .observe(None)
        .await
        .unwrap()
        .enrollment
        .rows
        .into_iter()
        .find_map(|r| r.account)
        .unwrap();
    session
        .run_enrollment(&db, Command::Cancel { id: original.id })
        .await
        .unwrap();
    let history = Worker::open(
        root.path().join("discovery/histories").join(format!(
            "{}.sqlite",
            original.binding.storage_key().unwrap()
        )),
        original.binding.clone(),
    )
    .await
    .unwrap();
    for n in 0..76 {
        let Reply::State(state) = history.request(HistoryCommand::State).await.unwrap() else {
            panic!()
        };
        let changes = if n == 75 {
            vec![crate::profiles::publication::change(Action::Setting {
                key: SettingKey::LeftSwipe,
                value: json!("archive"),
            })]
        } else {
            let mut candidate = base.clone();
            candidate.id = format!("offline-account-{n}");
            candidate.name = format!("Offline account {n}");
            shep_mail_core::profiles::export_account(&candidate, Uuid::from_u128(20000 + n))
                .unwrap()
        };
        history
            .request(HistoryCommand::Edit {
                edit: LocalEdit {
                    operation: Uuid::new_v4(),
                    expected_revision: state.revision,
                    changes,
                    resolutions: vec![],
                },
            })
            .await
            .unwrap();
    }
    let Reply::State(before) = history.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(before.queued, 76);
    history.close().await.unwrap();
    let review = prepare(&db, &mut session).await;
    let review = until(&db, &mut session, review.id, "review").await;
    assert_eq!(review.rows, 78);
    assert_eq!(review.copied, 3);
    let first = session.observe(None).await.unwrap().enrollment.rows;
    assert_eq!(first.len(), 50);
    let o = session
        .run_enrollment(
            &db,
            Command::Rows {
                id: review.id,
                after: first.last().unwrap().position,
            },
        )
        .await
        .unwrap();
    assert_eq!(o.enrollment.rows.len(), 28);
    let unsupported = o
        .enrollment
        .rows
        .iter()
        .find(|r| r.target == "setting:left_swipe")
        .unwrap();
    assert!(!unsupported.available && !unsupported.selected);
    let o = session
        .run_enrollment(
            &db,
            Command::Choose {
                id: review.id,
                position: unsupported.position,
                selected: true,
            },
        )
        .await
        .unwrap();
    assert!(o.error.is_some());
    let history = Worker::open(
        root.path()
            .join("discovery/histories")
            .join(format!("{}.sqlite", review.binding.storage_key().unwrap())),
        review.binding.clone(),
    )
    .await
    .unwrap();
    let Reply::State(after) = history.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(after.queued, before.queued);
    assert_eq!(after.device, before.device);
    assert_eq!(after.operations, before.operations);
    history.close().await.unwrap();
    assert!(db.get::<Vec<Account>>("accounts").await.unwrap().is_empty());
    session.close().await.unwrap();
}
