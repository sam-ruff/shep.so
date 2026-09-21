use super::*;
use crate::{
    api::MobileProfile,
    operations::{self, Request},
};
use execute::{MockChangeApi, execute};
use shep_mail_core::{folder_actions::Outcome, model::Protocol};

fn mailboxes() -> Vec<Mailbox> {
    ["INBOX", "Projects", "Projects/Design", "Elsewhere"]
        .into_iter()
        .map(|name| Mailbox {
            delimiter: Some('/'),
            encoding: NameEncoding::Utf8,
            ..Mailbox::flat(name.into())
        })
        .collect()
}

async fn setup(action: Action) -> (tempfile::TempDir, MobileProfile, Creation) {
    let (directory, profile) = crate::tests::profile().await;
    let mut account = crate::tests::account();
    account.protocol = Protocol::Imap;
    profile.database.write(move |db| {
        db.execute("INSERT INTO accounts VALUES(?1,?2)",params![account.id,serde_json::to_string(&account)?])?;
        super::super::save_catalogue(db,&account.id,&mailboxes())?;
        db.execute("INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES('fixture:Projects:1','fixture','1','Projects','a','b','subject','',1,1,0,0,'body',X'01')",[])?;
        Ok(())
    }).await.unwrap();
    let frozen = profile
        .database
        .read(move |db| review(db, "fixture", "Projects", action))
        .await
        .unwrap();
    let mut job = profile
        .database
        .write(move |db| admit(db, &uuid::Uuid::new_v4().to_string(), frozen))
        .await
        .unwrap();
    while !job.mutation.as_ref().unwrap().prepared {
        job = profile
            .database
            .write(move |db| prepare(db, &job))
            .await
            .unwrap();
    }
    (directory, profile, job)
}

fn rename() -> Action {
    Action::Rename {
        name: "Work".into(),
    }
}

#[tokio::test]
async fn admission_and_cache_repair_bound_frozen_metadata_to_fifty_rows() {
    let (_directory, profile, previous) = setup(rename()).await;
    let mut job=profile.database.write(move |db| {
        super::super::decide(db,&previous.id,previous.revision,"cancel")?;
        db.execute("WITH RECURSIVE seq(n) AS (SELECT 2 UNION ALL SELECT n+1 FROM seq WHERE n<123) INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) SELECT 'fixture:Projects:'||n,'fixture',n,'Projects','a','b','subject','',1,1,0,0,'body',X'01' FROM seq",[])?;
        let review=review(db,"fixture","Projects",rename())?;
        assert_eq!(review.messages,123);
        let job=admit(db,&uuid::Uuid::new_v4().to_string(),review)?;
        assert_eq!(db.query_row("SELECT count(*) FROM folder_change_members WHERE job=?1",[&job.id],|row|row.get::<_,i64>(0))?,0);
        Ok(job)
    }).await.unwrap();
    while !job.mutation.as_ref().unwrap().prepared {
        let before = job.mutation.as_ref().unwrap().prepared_count;
        job = profile
            .database
            .write(move |db| prepare(db, &job))
            .await
            .unwrap();
        assert!(job.mutation.as_ref().unwrap().prepared_count - before <= 50);
    }
    let mut api = MockChangeApi::new();
    api.expect_catalogue()
        .times(1)
        .returning(|| Ok(mailboxes()));
    api.expect_catalogue().times(1).returning(|| {
        Ok(mailboxes()
            .into_iter()
            .map(|mut mailbox| {
                mailbox.name = mailbox.name.replacen("Projects", "Work", 1);
                mailbox
            })
            .collect())
    });
    api.expect_apply()
        .times(1)
        .returning(|_, _| Ok(Outcome::Applied));
    job = execute(&profile.database, Some(&api), job).await.unwrap();
    assert_eq!(job.status, "repair");
    let first: i64 = profile
        .database
        .read(|db| {
            Ok(
                db.query_row("SELECT count(*) FROM mail WHERE folder='Work'", [], |row| {
                    row.get(0)
                })?,
            )
        })
        .await
        .unwrap();
    assert_eq!(first, 50);
    while job.status == "repair" {
        job = execute(&profile.database, None, job).await.unwrap();
    }
    assert_eq!(job.status, "succeeded");
}

#[tokio::test]
async fn stale_review_and_disk_rejection_leave_no_admission_or_projection() {
    let (_directory, profile, job) = setup(rename()).await;
    profile
        .database
        .write(move |db| {
            super::super::decide(db, &job.id, job.revision, "cancel")?;
            Ok(())
        })
        .await
        .unwrap();
    let frozen = profile
        .database
        .read(|db| review(db, "fixture", "Projects", rename()))
        .await
        .unwrap();
    profile
        .database
        .write(|db| {
            db.execute("UPDATE mail SET unread=0", [])?;
            Ok(())
        })
        .await
        .unwrap();
    assert!(
        profile
            .database
            .write(move |db| admit(db, &uuid::Uuid::new_v4().to_string(), frozen))
            .await
            .is_err()
    );
    let fresh = profile
        .database
        .read(|db| review(db, "fixture", "Projects", rename()))
        .await
        .unwrap();
    profile.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_admission BEFORE INSERT ON folder_creations BEGIN SELECT RAISE(ABORT,'disk'); END;")?;Ok(())}).await.unwrap();
    assert!(
        profile
            .database
            .write(move |db| admit(db, &uuid::Uuid::new_v4().to_string(), fresh))
            .await
            .is_err()
    );
    profile
        .database
        .read(|db| {
            available(db, "fixture")?;
            assert_eq!(
                db.query_row(
                    "SELECT count(*) FROM folder_creations WHERE status='queued'",
                    [],
                    |row| row.get::<_, i64>(0)
                )?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn pending_change_fences_provider_binding_settings_and_sent_roles() {
    let (_directory, profile, _job) = setup(rename()).await;
    profile
        .database
        .read(|db| {
            assert!(crate::connections::check_binding(db, "fixture", None).is_err());
            assert!(crate::connections::check_folder_binding(db, "fixture", None).is_ok());
            Ok(())
        })
        .await
        .unwrap();
    for sql in [
        "UPDATE accounts SET settings=json_set(settings,'$.sent_folder','Projects') WHERE id='fixture'",
        "INSERT INTO known_sent_folders VALUES('fixture','Projects')",
        "UPDATE folders SET names='[]' WHERE account_id='fixture'",
    ] {
        assert!(
            profile
                .database
                .write(move |db| {
                    db.execute(sql, [])?;
                    Ok(())
                })
                .await
                .is_err()
        );
    }
    profile
        .database
        .write(|db| {
            let mut other = crate::tests::account();
            other.id = "other".into();
            db.execute(
                "INSERT INTO accounts VALUES(?1,?2)",
                params![other.id, serde_json::to_string(&other)?],
            )?;
            super::super::admit(
                db,
                &uuid::Uuid::new_v4().to_string(),
                &other.id,
                &connection_key(&other),
                None,
                "Independent".into(),
            )?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn known_sent_role_and_pending_outgoing_refuse_frozen_review() {
    let (_directory, profile, job) = setup(rename()).await;
    profile.database.write(move |db|{super::super::decide(db,&job.id,job.revision,"cancel")?;db.execute("INSERT INTO known_sent_folders VALUES('fixture','Projects/Design')",[])?;assert!(review(db,"fixture","Projects",rename()).is_err());db.execute("DELETE FROM known_sent_folders",[])?;db.execute("INSERT INTO outgoing VALUES('send','draft','queued','fixture','message',X'01','{}')",[])?;assert!(review(db,"fixture","Projects",rename()).is_err());Ok(())}).await.unwrap();
}

#[tokio::test]
async fn queued_change_is_local_idempotent_cancelled_and_fences_new_mail() {
    let (_directory, profile, job) = setup(rename()).await;
    let _network = profile.operations.hold_network_capacity().await;
    let _account = profile.operations.account("fixture").await;
    let review = job.mutation.as_ref().unwrap().review.clone();
    let id = job.id.clone();
    let duplicate = profile
        .database
        .write(move |db| admit(db, &id, review))
        .await
        .unwrap();
    assert_eq!(duplicate, job);
    let refused = profile
        .database
        .write(|db| {
            db.execute("UPDATE mail SET unread=0 WHERE account_id='fixture'", [])?;
            Ok(())
        })
        .await;
    assert!(refused.is_err());
    let waiting = operations::run(
        &profile,
        Request::ExecuteFolder {
            id: job.id.clone(),
            credential_slot: None,
            password: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(waiting["status"], "waiting");
    let id = job.id.clone();
    profile
        .database
        .write(move |db| {
            let current = super::super::get(db, &id)?.unwrap();
            super::super::decide(db, &id, current.revision, "cancel")
        })
        .await
        .unwrap();
    profile
        .database
        .write(|db| {
            db.execute("UPDATE mail SET unread=0 WHERE account_id='fixture'", [])?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn acknowledged_rename_keeps_uid_lineage_and_repairs_without_provider() {
    let (_directory, profile, job) = setup(rename()).await;
    let before: String = profile
        .database
        .read(|db| Ok(db.query_row("SELECT token FROM mail_lineage", [], |r| r.get(0))?))
        .await
        .unwrap();
    let mut api = MockChangeApi::new();
    api.expect_catalogue()
        .times(1)
        .returning(|| Ok(mailboxes()));
    api.expect_apply()
        .times(1)
        .returning(|_, _| Ok(Outcome::Applied));
    api.expect_catalogue()
        .times(1)
        .returning(|| anyhow::bail!("offline after acknowledgement"));
    let pending = execute(&profile.database, Some(&api), job).await.unwrap();
    assert_eq!(pending.status, "repair");
    assert!(pending.mutation.as_ref().unwrap().receipt.is_some());
    let mut check = MockChangeApi::new();
    check.expect_catalogue().times(1).returning(|| {
        Ok(mailboxes()
            .into_iter()
            .map(|mut mailbox| {
                mailbox.name = mailbox.name.replacen("Projects", "Work", 1);
                mailbox
            })
            .collect())
    });
    check.expect_apply().times(0);
    let done = execute(&profile.database, Some(&check), pending)
        .await
        .unwrap();
    assert_eq!(done.status, "succeeded");
    let cached = profile
        .database
        .read(|db| {
            Ok(db.query_row(
                "SELECT m.folder,m.remote_id,l.token FROM mail m JOIN mail_lineage l ON l.id=m.id",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                },
            )?)
        })
        .await
        .unwrap();
    assert_eq!(cached, ("Work".into(), "1".into(), before));
}

#[tokio::test]
async fn unknown_rename_checks_without_replay_and_can_stop_tracking() {
    let (_directory, profile, job) = setup(rename()).await;
    let mut api = MockChangeApi::new();
    api.expect_catalogue().returning(|| Ok(mailboxes()));
    api.expect_apply()
        .times(1)
        .returning(|_, _| Ok(Outcome::Uncertain("lost response".into())));
    let uncertain = execute(&profile.database, Some(&api), job).await.unwrap();
    assert_eq!(uncertain.status, "uncertain");
    assert!(
        execute(&profile.database, Some(&api), uncertain.clone())
            .await
            .is_err()
    );
    let checking = profile
        .database
        .write(move |db| super::super::decide(db, &uncertain.id, uncertain.revision, "check"))
        .await
        .unwrap();
    let checked = execute(&profile.database, Some(&api), checking)
        .await
        .unwrap();
    assert!(checked.mutation.as_ref().unwrap().checked);
    profile
        .database
        .write(move |db| super::super::decide(db, &checked.id, checked.revision, "dismiss"))
        .await
        .unwrap();
    let source: String = profile
        .database
        .read(|db| Ok(db.query_row("SELECT folder FROM mail", [], |r| r.get(0))?))
        .await
        .unwrap();
    assert_eq!(source, "Projects");
}

#[tokio::test]
async fn offline_preflight_waits_and_changed_subtree_rejects_before_dispatch() {
    let (_directory, profile, job) = setup(rename()).await;
    let mut api = MockChangeApi::new();
    api.expect_catalogue()
        .times(1)
        .returning(|| anyhow::bail!("offline"));
    api.expect_apply().times(0);
    let waiting = execute(&profile.database, Some(&api), job).await.unwrap();
    assert_eq!(waiting.status, "waiting");
    let mut changed = MockChangeApi::new();
    changed.expect_catalogue().returning(|| {
        let mut list = mailboxes();
        list.push(Mailbox {
            delimiter: Some('/'),
            encoding: NameEncoding::Utf8,
            ..Mailbox::flat("Projects/New".into())
        });
        Ok(list)
    });
    changed.expect_apply().times(0);
    assert_eq!(
        execute(&profile.database, Some(&changed), waiting)
            .await
            .unwrap()
            .status,
        "rejected"
    );
}

#[tokio::test]
async fn delete_retains_completed_steps_and_restart_never_replays_dispatch() {
    let (_directory, profile, job) = setup(Action::Delete).await;
    let mut api = MockChangeApi::new();
    api.expect_catalogue().returning(|| Ok(mailboxes()));
    api.expect_apply()
        .times(1)
        .withf(|_, step| matches!(step,Step::Delete{source} if source=="Projects/Design"))
        .returning(|_, _| Ok(Outcome::Applied));
    let next = execute(&profile.database, Some(&api), job).await.unwrap();
    assert_eq!(next.status, "queued");
    assert_eq!(next.mutation.as_ref().unwrap().completed, 1);
    profile
        .database
        .write(move |db| {
            let mut after = next.clone();
            after.status = "running".into();
            super::super::save(db, &next, &after)?;
            super::super::recover(db)?;
            Ok(())
        })
        .await
        .unwrap();
    let history = profile.database.read(super::super::history).await.unwrap();
    assert_eq!(history[0].status, "uncertain");
    assert_eq!(history[0].mutation.as_ref().unwrap().completed, 1);
}

#[tokio::test]
async fn cache_failure_retains_observed_receipt_and_only_retries_local_transaction() {
    let (_directory, profile, job) = setup(rename()).await;
    profile.database.write(|db|{db.execute_batch("CREATE TRIGGER fail_folder_cache BEFORE UPDATE OF folder ON mail BEGIN SELECT RAISE(ABORT,'disk'); END;")?;Ok(())}).await.unwrap();
    let mut api = MockChangeApi::new();
    api.expect_catalogue()
        .times(1)
        .returning(|| Ok(mailboxes()));
    api.expect_catalogue().times(1).returning(|| {
        Ok(mailboxes()
            .into_iter()
            .map(|mut mailbox| {
                mailbox.name = mailbox.name.replacen("Projects", "Work", 1);
                mailbox
            })
            .collect())
    });
    api.expect_apply()
        .times(1)
        .returning(|_, _| Ok(Outcome::Applied));
    let id = job.id.clone();
    assert!(execute(&profile.database, Some(&api), job).await.is_err());
    profile
        .database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_folder_cache;")?;
            Ok(())
        })
        .await
        .unwrap();
    let saved = profile
        .database
        .read(move |db| super::super::get(db, &id))
        .await
        .unwrap()
        .unwrap();
    assert!(saved.mutation.as_ref().unwrap().observed);
    let _network = profile.operations.hold_network_capacity().await;
    let done = operations::run(
        &profile,
        Request::ExecuteFolder {
            id: saved.id,
            credential_slot: None,
            password: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(done["status"], "succeeded");
}

#[tokio::test]
async fn removal_includes_change_and_rejects_late_receipt() {
    let (_directory, profile, job) = setup(rename()).await;
    profile
        .database
        .write(|db| {
            let review = crate::accounts::preview(db, "fixture")?;
            assert_eq!(review.folder_requests, 1);
            crate::accounts::remove(db, review, true)
        })
        .await
        .unwrap();
    assert!(
        profile
            .database
            .write(move |db| {
                let mut after = job.clone();
                after.acknowledged = true;
                super::super::save(db, &job, &after)
            })
            .await
            .is_err()
    );
}

#[tokio::test]
async fn checked_persistent_cache_failure_can_release_account_without_erasing_receipt() {
    let (_directory, profile, job) = setup(rename()).await;
    profile.database.write(|db|{db.execute_batch("CREATE TRIGGER keep_cache BEFORE UPDATE OF folder ON mail BEGIN SELECT RAISE(ABORT,'disk'); END;")?;Ok(())}).await.unwrap();
    let mut api = MockChangeApi::new();
    api.expect_catalogue()
        .times(1)
        .returning(|| Ok(mailboxes()));
    api.expect_catalogue().times(2).returning(|| {
        Ok(mailboxes()
            .into_iter()
            .map(|mut mailbox| {
                mailbox.name = mailbox.name.replacen("Projects", "Work", 1);
                mailbox
            })
            .collect())
    });
    api.expect_apply()
        .times(1)
        .returning(|_, _| Ok(Outcome::Applied));
    let id = job.id.clone();
    assert!(execute(&profile.database, Some(&api), job).await.is_err());
    let lookup = id.clone();
    let checking = profile
        .database
        .write(move |db| {
            let saved = super::super::get(db, &lookup)?.unwrap();
            super::super::decide(db, &saved.id, saved.revision, "check")
        })
        .await
        .unwrap();
    assert!(
        execute(&profile.database, Some(&api), checking)
            .await
            .is_err()
    );
    profile
        .database
        .write(move |db| {
            let saved = super::super::get(db, &id)?.unwrap();
            assert!(saved.mutation.as_ref().unwrap().checked);
            let retired = super::super::decide(db, &id, saved.revision, "dismiss")?;
            assert!(retired.mutation.as_ref().unwrap().receipt.is_some());
            available(db, "fixture")?;
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn schema_twenty_three_creation_rows_upgrade_without_new_dispatch_authority() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(include_str!("../../schema.sql")).unwrap();
        let account = crate::tests::account();
        db.execute(
            "INSERT INTO accounts VALUES(?1,?2)",
            params![account.id, serde_json::to_string(&account).unwrap()],
        )
        .unwrap();
        db.execute_batch("CREATE TABLE folder_creations(id TEXT PRIMARY KEY,account_id TEXT NOT NULL REFERENCES accounts(id),connection TEXT NOT NULL,parent TEXT,name TEXT NOT NULL,status TEXT NOT NULL,target TEXT,receipt TEXT,acknowledged INTEGER NOT NULL DEFAULT 0,error TEXT,revision INTEGER NOT NULL DEFAULT 1,created INTEGER NOT NULL); PRAGMA user_version=23;").unwrap();
        db.execute("INSERT INTO folder_creations(id,account_id,connection,name,status,created) VALUES('old','fixture','saved','Projects','running',1)",[]).unwrap();
    }
    let profile = MobileProfile::open(path.to_str().unwrap().into())
        .await
        .unwrap();
    profile
        .database
        .read(|db| {
            let saved = super::super::get(db, "old")?.unwrap();
            assert_eq!(saved.status, "uncertain");
            assert!(saved.mutation.is_none());
            assert_eq!(
                db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?,
                24
            );
            Ok(())
        })
        .await
        .unwrap();
}
