use super::*;
use crate::{
    api::MobileProfile,
    operations::{self, Request},
};
use execute::MockCreationApi;
use shep_mail_core::{folder_actions::creation::CreateOutcome, model::Protocol};

async fn setup() -> (tempfile::TempDir, MobileProfile, Creation) {
    let (directory, profile) = crate::tests::profile().await;
    let mut account = crate::tests::account();
    account.protocol = Protocol::Imap;
    let connection = connection_key(&account);
    profile
        .database
        .write(move |db| {
            db.execute(
                "INSERT INTO accounts VALUES(?1,?2)",
                params![account.id, serde_json::to_string(&account)?],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let value = operations::run(
        &profile,
        Request::AdmitFolder {
            id: uuid::Uuid::new_v4().to_string(),
            account: "fixture".into(),
            connection,
            parent: None,
            name: "Projects".into(),
        },
    )
    .await
    .unwrap();
    (directory, profile, serde_json::from_value(value).unwrap())
}

fn planned() -> Mailbox {
    Mailbox::flat("Projects".into())
}

#[tokio::test]
async fn admission_is_local_under_held_account_and_provider_capacity() {
    let (_directory, profile, first) = setup().await;
    let _held = profile.operations.hold_network_capacity().await;
    let _account = profile.operations.account("fixture").await;
    let id = uuid::Uuid::new_v4().to_string();
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        operations::run(
            &profile,
            Request::AdmitFolder {
                id: id.clone(),
                account: first.account,
                connection: first.connection,
                parent: None,
                name: "Later".into(),
            },
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(result["status"], "queued");
    assert_eq!(result["id"], id);
    assert!(result["target"].is_null());
}

#[tokio::test]
async fn acknowledged_create_survives_inspection_failure_and_repairs_without_create() {
    let (_directory, profile, job) = setup().await;
    let mut api = MockCreationApi::new();
    api.expect_plan().times(1).returning(|_, _| Ok(planned()));
    let mut sequence = mockall::Sequence::new();
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(None));
    api.expect_create()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| anyhow::bail!("offline"));
    let repair = execute(&profile.database, &api, job).await.unwrap();
    assert_eq!(repair.status, "repair");
    assert!(repair.acknowledged);
    let mut recovery = MockCreationApi::new();
    recovery.expect_create().times(0);
    recovery.expect_plan().times(0);
    recovery
        .expect_inspect()
        .times(1)
        .returning(|_| Ok(Some(planned())));
    let done = execute(&profile.database, &recovery, repair).await.unwrap();
    assert_eq!(done.status, "succeeded");
    assert!(done.acknowledged);
}

#[tokio::test]
async fn lost_create_reply_cannot_automatically_replay_and_checked_absence_requires_retry() {
    let (_directory, profile, job) = setup().await;
    let mut api = MockCreationApi::new();
    api.expect_plan().times(1).returning(|_, _| Ok(planned()));
    api.expect_inspect().times(1).returning(|_| Ok(None));
    api.expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Uncertain("disconnected".into())));
    let unknown = execute(&profile.database, &api, job).await.unwrap();
    assert_eq!(unknown.status, "uncertain");
    assert!(
        execute(&profile.database, &api, unknown.clone())
            .await
            .is_err()
    );
    let checked = profile
        .database
        .write(move |db| decide(db, &unknown.id, unknown.revision, "check"))
        .await
        .unwrap();
    let mut inspect = MockCreationApi::new();
    inspect.expect_create().times(0);
    inspect.expect_inspect().times(1).returning(|_| Ok(None));
    let absent = execute(&profile.database, &inspect, checked).await.unwrap();
    assert_eq!(absent.status, "rejected");
    assert!(execute(&profile.database, &inspect, absent).await.is_err());
}

#[tokio::test]
async fn cache_failure_retains_receipt_and_repair_needs_no_credentials_or_provider_permit() {
    let (_directory, profile, job) = setup().await;
    profile.database.write(|db| { db.execute_batch("CREATE TRIGGER fail_folder BEFORE INSERT ON folders BEGIN SELECT RAISE(ABORT,'full'); END;")?; Ok(()) }).await.unwrap();
    let mut api = MockCreationApi::new();
    api.expect_plan().times(1).returning(|_, _| Ok(planned()));
    api.expect_inspect()
        .times(1)
        .returning(|_| Ok(Some(planned())));
    api.expect_create().times(0);
    assert!(execute(&profile.database, &api, job.clone()).await.is_err());
    let lookup = job.id.clone();
    let saved = profile
        .database
        .read(move |db| Ok(get(db, &lookup)?.unwrap()))
        .await
        .unwrap();
    assert_eq!(saved.status, "repair");
    assert_eq!(saved.receipt, Some(planned()));
    profile
        .database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_folder")?;
            Ok(())
        })
        .await
        .unwrap();
    let _capacity = profile.operations.hold_network_capacity().await;
    let result = operations::run(
        &profile,
        Request::ExecuteFolder {
            id: job.id,
            credential_slot: None,
            password: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "succeeded");
}

#[tokio::test]
async fn removal_review_fences_new_requests_and_late_receipts_cannot_recreate_account() {
    let (_directory, profile, job) = setup().await;
    let old_review = profile
        .database
        .read(|db| crate::accounts::preview(db, "fixture"))
        .await
        .unwrap();
    let later = job.clone();
    profile
        .database
        .write(move |db| {
            admit(
                db,
                &uuid::Uuid::new_v4().to_string(),
                "fixture",
                &later.connection,
                None,
                "Newer".into(),
            )?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        profile
            .database
            .write(move |db| crate::accounts::remove(db, old_review, true))
            .await
            .is_err()
    );
    profile
        .database
        .write(|db| {
            let review = crate::accounts::preview(db, "fixture")?;
            assert_eq!(review.folder_requests, 2);
            crate::accounts::remove(db, review, true)
        })
        .await
        .unwrap();
    let mut receipt = job;
    receipt.receipt = Some(planned());
    assert!(
        profile
            .database
            .write(move |db| apply_receipt(db, &receipt))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn restart_never_replays_dispatched_create_and_lost_admission_reply_is_idempotent() {
    let (_directory, profile, job) = setup().await;
    let initial = job.clone();
    let duplicate = profile
        .database
        .write(move |db| {
            admit(
                db,
                &initial.id,
                &initial.account,
                &initial.connection,
                initial.parent,
                initial.name,
            )
        })
        .await
        .unwrap();
    assert_eq!(duplicate, job);
    let id = job.id.clone();
    profile
        .database
        .write(move |db| {
            let mut running = job.clone();
            running.status = "running".into();
            running.target = Some(planned());
            save(db, &job, &running)?;
            recover(db)?;
            Ok(())
        })
        .await
        .unwrap();
    let restored = profile
        .database
        .read(move |db| Ok(get(db, &id)?.unwrap()))
        .await
        .unwrap();
    assert_eq!(restored.status, "uncertain");
    let api = MockCreationApi::new();
    assert!(execute(&profile.database, &api, restored).await.is_err());
}

#[tokio::test]
async fn offline_planning_waits_while_definite_create_refusal_rejects() {
    for plan_fails in [true, false] {
        let (_directory, profile, job) = setup().await;
        let mut api = MockCreationApi::new();
        api.expect_plan().times(1).returning(move |_, _| {
            if plan_fails {
                anyhow::bail!("namespace unavailable")
            } else {
                Ok(planned())
            }
        });
        api.expect_inspect()
            .times(if plan_fails { 0 } else { 1 })
            .returning(|_| Ok(None));
        api.expect_create()
            .times(if plan_fails { 0 } else { 1 })
            .returning(|_| Ok(CreateOutcome::Rejected("NO denied".into())));
        let result = execute(&profile.database, &api, job).await.unwrap();
        assert_eq!(
            result.status,
            if plan_fails { "waiting" } else { "rejected" }
        );
        assert!(!result.acknowledged);
    }
}

#[tokio::test]
async fn typed_name_validation_rejects_without_provider_dispatch() {
    let (_directory, profile, job) = setup().await;
    let mut api = MockCreationApi::new();
    api.expect_plan().times(1).returning(|_, _| {
        Err(
            shep_mail_core::folder_actions::creation::PlanRejected("invalid child name".into())
                .into(),
        )
    });
    api.expect_create().times(0);
    api.expect_inspect().times(0);
    let result = execute(&profile.database, &api, job).await.unwrap();
    assert_eq!(result.status, "rejected");
    assert!(result.target.is_none());
}

struct HeldCreate {
    started: tokio::sync::Notify,
    release: tokio::sync::Notify,
    created: std::sync::atomic::AtomicBool,
}
#[async_trait::async_trait]
impl execute::CreationApi for HeldCreate {
    async fn plan(&self, _: Option<String>, _: String) -> Result<Mailbox> {
        Ok(planned())
    }
    async fn inspect(&self, _: Mailbox) -> Result<Option<Mailbox>> {
        Ok(self
            .created
            .load(std::sync::atomic::Ordering::SeqCst)
            .then(planned))
    }
    async fn create(&self, _: Mailbox) -> Result<CreateOutcome> {
        self.started.notify_one();
        self.release.notified().await;
        self.created
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(CreateOutcome::Acknowledged)
    }
}

#[tokio::test]
async fn held_create_excludes_removal_and_reconnect_but_keeps_local_admission_available() {
    let (_directory, profile, job) = setup().await;
    let slot = profile
        .database
        .write(|db| {
            let account = crate::operations::stored_account(db, "fixture")?;
            let prepared = crate::connections::prepare(db, None, account.clone(), Some(account))?;
            Ok(prepared["slot"].as_str().unwrap().to_owned())
        })
        .await
        .unwrap();
    let profile = std::sync::Arc::new(profile);
    let provider = std::sync::Arc::new(HeldCreate {
        started: Default::default(),
        release: Default::default(),
        created: false.into(),
    });
    *profile.operations.folder_provider.lock().unwrap() = Some(provider.clone());
    let running_profile = profile.clone();
    let id = job.id.clone();
    let running = tokio::spawn(async move {
        operations::run(
            &running_profile,
            Request::ExecuteFolder {
                id,
                credential_slot: None,
                password: Some("fixture".into()),
            },
        )
        .await
    });
    provider.started.notified().await;
    let review = profile
        .database
        .read(|db| crate::accounts::preview(db, "fixture"))
        .await
        .unwrap();
    assert!(
        operations::run(
            &profile,
            Request::RemoveAccount {
                review,
                discard_unresolved: true
            }
        )
        .await
        .is_err()
    );
    let mut reconnect = Box::pin(operations::run(&profile, Request::ActivateAccount { slot }));
    use std::{future::Future, task::Poll};
    assert!(
        std::future::poll_fn(|cx| Poll::Ready(reconnect.as_mut().poll(cx)))
            .await
            .is_pending()
    );
    let queued = operations::run(
        &profile,
        Request::AdmitFolder {
            id: uuid::Uuid::new_v4().to_string(),
            account: job.account,
            connection: job.connection,
            parent: None,
            name: "Second".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(queued["status"], "queued");
    for _ in 0..16 {
        let result = operations::run(
            &profile,
            Request::ExecuteFolder {
                id: queued["id"].as_str().unwrap().into(),
                credential_slot: None,
                password: Some("fixture".into()),
            },
        )
        .await
        .unwrap();
        assert_eq!(result["status"], "waiting");
    }
    provider.release.notify_one();
    let done = running.await.unwrap().unwrap();
    assert_eq!(done["status"], "succeeded");
    reconnect.await.unwrap();
}

#[tokio::test]
async fn changed_connection_rejects_queued_work_but_does_not_trap_stop_tracking() {
    let (_directory, profile, job) = setup().await;
    profile
        .database
        .write(|db| {
            let mut account = crate::operations::stored_account(db, "fixture")?;
            account.host = "different.example.test".into();
            db.execute(
                "UPDATE accounts SET settings=?1 WHERE id='fixture'",
                [serde_json::to_string(&account)?],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let result = operations::run(
        &profile,
        Request::ExecuteFolder {
            id: job.id.clone(),
            credential_slot: None,
            password: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "rejected");
    let rejected: Creation = serde_json::from_value(result).unwrap();
    let dismissed = operations::run(
        &profile,
        Request::DecideFolder {
            id: job.id,
            revision: rejected.revision,
            decision: "dismiss".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(dismissed["status"], "dismissed");
}

#[tokio::test]
async fn actual_database_reopen_preserves_queued_work_and_expires_dispatch_authority() {
    let (_directory, profile, job) = setup().await;
    let path = profile.database.path.to_string_lossy().into_owned();
    let id = job.id.clone();
    profile
        .database
        .write(move |db| {
            let mut running = job.clone();
            running.status = "running".into();
            running.target = Some(planned());
            save(db, &job, &running)?;
            admit(
                db,
                &uuid::Uuid::new_v4().to_string(),
                "fixture",
                &job.connection,
                None,
                "Still queued".into(),
            )?;
            Ok(())
        })
        .await
        .unwrap();
    drop(profile);
    let reopened = MobileProfile::open(path).await.unwrap();
    reopened
        .database
        .read(move |db| {
            assert_eq!(get(db, &id)?.unwrap().status, "uncertain");
            assert_eq!(
                history(db)?
                    .iter()
                    .filter(|job| job.status == "queued")
                    .count(),
                1
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn transient_acknowledgment_write_failure_keeps_receipt_and_never_repeats_create() {
    let (_directory, profile, job) = setup().await;
    profile.database.write(|db| {
        db.execute_batch("CREATE TABLE fail_ack_once(n INTEGER); INSERT INTO fail_ack_once VALUES(1); CREATE TRIGGER fail_ack BEFORE UPDATE OF acknowledged ON folder_creations WHEN new.acknowledged=1 AND (SELECT n FROM fail_ack_once)>0 BEGIN UPDATE fail_ack_once SET n=0; SELECT RAISE(FAIL,'receipt write failed'); END;")?;
        Ok(())
    }).await.unwrap();
    let mut api = MockCreationApi::new();
    api.expect_plan().times(1).returning(|_, _| Ok(planned()));
    let mut sequence = mockall::Sequence::new();
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(None));
    api.expect_create()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(Some(planned())));
    let saved = execute(&profile.database, &api, job).await.unwrap();
    assert!(saved.acknowledged);
    assert_eq!(saved.status, "succeeded");
}

#[tokio::test]
async fn imported_reconnect_guard_keeps_creation_waiting_without_provider_access() {
    let (_directory, profile, job) = setup().await;
    profile
        .database
        .write(|db| {
            db.execute(
                "INSERT INTO profile_reconnect VALUES('fixture','Imported account')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    *profile.operations.folder_provider.lock().unwrap() =
        Some(std::sync::Arc::new(MockCreationApi::new()));
    let result = operations::run(
        &profile,
        Request::ExecuteFolder {
            id: job.id,
            credential_slot: None,
            password: Some("fixture".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "waiting");
    assert!(result["target"].is_null());
    assert!(result["error"].as_str().unwrap().contains("Reconnect"));
}

#[tokio::test]
async fn parent_picker_keeps_wire_identity_and_exposes_decoded_catalogue_label() {
    let (_directory, profile, _job) = setup().await;
    profile
        .database
        .write(|db| {
            let mut mailbox = Mailbox::flat("&AMk-tudes".into());
            mailbox.encoding = shep_mail_core::folders::NameEncoding::ImapUtf7;
            mailbox.selectable = false;
            mailbox.delimiter = Some('.');
            save_catalogue(db, "fixture", &[mailbox])?;
            let options = options(db)?;
            assert_eq!(options[0]["parent_labels"]["&AMk-tudes"], "Études");
            assert_eq!(options[0]["catalogue"][0]["name"], "&AMk-tudes");
            assert_eq!(options[0]["catalogue"][0]["selectable"], false);
            Ok(())
        })
        .await
        .unwrap();
}
