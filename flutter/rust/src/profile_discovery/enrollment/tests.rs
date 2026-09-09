use super::*;
use crate::tests::{account, profile, seed};
use shep_profile_core::{
    SettingKey,
    history::{Journal, LocalEdit, Record},
};
fn preferences() -> Preferences {
    let values = serde_json::from_value(serde_json::json!({"appearance":"System","left_swipe":"archive","right_swipe":"read","preview_lines":2,"sender_pictures":true,"unified_inbox":true,"reply_display":"Collapsed","tooltips":true})).unwrap();
    Preferences {
        values,
        revisions: SETTINGS.into_iter().map(|s| (s.into(), 0)).collect(),
    }
}
fn scope() -> Scope {
    Scope {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture-owner".into(),
    }
}
struct Source {
    snapshot: Snapshot,
    records: Vec<Record>,
    device: Uuid,
}
#[async_trait::async_trait]
impl transfer::Records for Source {
    async fn export(&self, source: Snapshot, after: u64) -> Result<Option<Record>> {
        ensure!(
            source.binding == self.snapshot.binding && source.profile == self.snapshot.profile,
            "Changed source"
        );
        Ok(self.records.iter().find(|r| r.position > after).cloned())
    }
}
fn append(journal: &mut Journal, actions: Vec<Action>) {
    let revision = journal.state().unwrap().revision;
    journal
        .execute(HistoryCommand::Edit {
            edit: LocalEdit {
                operation: Uuid::new_v4(),
                expected_revision: revision,
                changes: actions
                    .into_iter()
                    .map(|action| Change {
                        action,
                        extra: Default::default(),
                    })
                    .collect(),
                resolutions: vec![],
            },
        })
        .unwrap();
}
fn source(accounts: usize) -> Source {
    let scope = scope();
    let binding = Binding {
        namespace: scope.namespace,
        principal: scope.principal,
        profile: Uuid::new_v4(),
        generation: Uuid::new_v4(),
    };
    let mut journal = Journal::memory(binding.clone()).unwrap();
    append(&mut journal, vec![Action::ProfileSetup { complete: false }]);
    append(
        &mut journal,
        vec![
            Action::ProfileName {
                name: "Work profile".into(),
            },
            Action::Setting {
                key: SettingKey::Appearance,
                value: "Dark".into(),
            },
            Action::SettingRemoved {
                key: SettingKey::PreviewLines,
            },
        ],
    );
    for i in 0..accounts {
        let mut account = account();
        account.id = Uuid::from_u128(1000 + i as u128).to_string();
        account.name = format!("Team {i:03}");
        let actions = shep_mail_core::profiles::export_account(
            &account,
            Uuid::parse_str(&account.id).unwrap(),
        )
        .unwrap()
        .into_iter()
        .map(|c| c.action)
        .collect();
        append(&mut journal, actions);
    }
    append(&mut journal, vec![Action::ProfileSetup { complete: true }]);
    let state = journal.state().unwrap();
    let mut records = vec![];
    while let Some(record) = journal
        .export_record(
            state.revision,
            records.last().map_or(0, |r: &Record| r.position),
        )
        .unwrap()
    {
        records.push(record);
    }
    let profile = shep_profile_core::drive::catalog::Profile {
        profile: binding.profile,
        generation: binding.generation,
        name: Some("Work profile".into()),
        name_conflict: false,
        accounts: accounts as u64,
        settings: 2,
        operations: state.operations,
        waiting: state.waiting,
        ready: state.ready,
        conflicts: state.conflicts,
        removed: state.removed,
        initialized: state.initialized,
        revision: state.revision,
    };
    Source {
        snapshot: Snapshot { binding, profile },
        records,
        device: state.device,
    }
}
async fn prepared(profile: &MobileProfile, source: &Source) -> Review {
    prepared_preferences(profile, source, preferences()).await
}
async fn prepared_preferences(
    profile: &MobileProfile,
    source: &Source,
    preferences: Preferences,
) -> Review {
    let id = Uuid::new_v4();
    let snapshot = source.snapshot.clone();
    let key = scope().storage_key().unwrap();
    let mut review = profile
        .database
        .write(move |db| store::prepare(db, &key, id, snapshot, preferences))
        .await
        .unwrap();
    let history = worker(&profile.database, review.binding.clone())
        .await
        .unwrap();
    let Reply::State(state) = history.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_ne!(state.device, source.device);
    // Import can commit before the mail-cache copy cursor. Retrying must not
    // duplicate the operation or enqueue it as an authored local upload.
    history
        .request(HistoryCommand::Import {
            record: source.records[0].record.clone(),
        })
        .await
        .unwrap();
    let mut steps = 0;
    while review.phase != "review" {
        let before = review.rows;
        review = transfer::step(
            &profile.database,
            &scope().storage_key().unwrap(),
            review,
            source.snapshot.clone(),
            source,
            &history,
        )
        .await
        .unwrap();
        assert!(review.rows - before <= 50);
        steps += 1;
        assert!(steps < 1000);
    }
    let Reply::State(state) = history.request(HistoryCommand::State).await.unwrap() else {
        panic!()
    };
    assert_eq!(state.operations, source.records.len() as u64);
    assert_eq!(state.queued, 0);
    history.close().await.unwrap();
    review
}
#[tokio::test]
async fn enrollment_copies_original_history_pages_accounts_and_applies_reconnect_receipts_without_touching_mail()
 {
    let (dir, profile) = profile().await;
    seed(&profile, 5).await;
    profile.database.write(|db| { db.execute("INSERT INTO drafts VALUES('local-draft',1,'{\"account_id\":\"fixture\",\"body\":\"Keep my work\"}')", [])?; Ok(()) }).await.unwrap();
    let source = source(75);
    let mut review = prepared(&profile, &source).await;
    assert_eq!(review.rows, 77);
    let id = review.id;
    let key = scope().storage_key().unwrap();
    let k = key.clone();
    let first = profile
        .database
        .read(move |db| store::rows(db, &k, id, 0))
        .await
        .unwrap();
    assert_eq!(first.as_array().unwrap().len(), 50);
    assert!(first[0].get("slot").is_none());
    let after = first[49]["position"].as_u64().unwrap();
    let k = key.clone();
    assert_eq!(
        profile
            .database
            .read(move |db| store::rows(db, &k, id, after))
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        27
    );
    let k = key.clone();
    review = profile
        .database
        .write(move |db| store::approve(db, &k, id, true, true))
        .await
        .unwrap();
    // Held provider capacity cannot prevent independent cache-only enrollment.
    let held = profile.operations.hold_network_capacity().await;
    while review.phase == "applying" {
        review = apply::step(&profile, &key, review).await.unwrap();
    }
    drop(held);
    assert_eq!(review.applied, 75);
    let k = key.clone();
    let request = profile
        .database
        .read(move |db| apply::settings(db, &k, id))
        .await
        .unwrap();
    assert_eq!(
        request["changes"],
        serde_json::json!({"appearance":"Dark","preview_lines":null})
    );
    profile
        .database
        .read(|db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get::<_, i64>(0))?,
                76
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM mail", [], |r| r.get::<_, i64>(0))?,
                5
            );
            assert_eq!(
                db.query_row(
                    "SELECT content FROM drafts WHERE id='local-draft'",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                "{\"account_id\":\"fixture\",\"body\":\"Keep my work\"}"
            );
            let id: String = db.query_row(
                "SELECT account_id FROM profile_reconnect LIMIT 1",
                [],
                |r| r.get(0),
            )?;
            let account = crate::operations::stored_account(db, &id)?;
            assert!(
                crate::connections::target(db, account)
                    .unwrap_err()
                    .to_string()
                    .contains("passwords")
            );
            assert!(crate::connections::check_binding(db, &id, None).is_err());
            Ok(())
        })
        .await
        .unwrap();
    // Reopen the actual database before acknowledging the platform setting write.
    drop(profile);
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let k = key.clone();
    let completed = reopened
        .database
        .write(move |db| {
            apply::confirm(
                db,
                &k,
                id,
                vec!["appearance".into()],
                vec!["preview_lines".into()],
                None,
            )
        })
        .await
        .unwrap();
    assert_eq!(completed.phase, "complete");
    let k = key.clone();
    assert_eq!(
        reopened
            .database
            .write(move |db| apply::confirm(
                db,
                &k,
                id,
                vec!["appearance".into()],
                vec!["preview_lines".into()],
                None
            ))
            .await
            .unwrap()
            .phase,
        "complete"
    );
    let k = key.clone();
    assert!(
        reopened
            .database
            .write(move |db| apply::confirm(
                db,
                &k,
                id,
                vec!["appearance".into(), "preview_lines".into()],
                vec![],
                None
            ))
            .await
            .is_err()
    );
}
#[tokio::test]
async fn changed_endpoint_requires_a_separate_account_and_stale_reviews_do_not_apply() {
    let (_dir, profile) = profile().await;
    let source = source(1);
    let shared = Uuid::from_u128(1000);
    profile
        .database
        .write(move |db| {
            let mut account = account();
            account.id = shared.to_string();
            account.host = "original.example.test".into();
            db.execute(
                "INSERT INTO accounts VALUES(?,?)",
                params![account.id, serde_json::to_string(&account)?],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let review = prepared(&profile, &source).await;
    let id = review.id;
    let key = scope().storage_key().unwrap();
    let k = key.clone();
    let rows = profile
        .database
        .read(move |db| store::rows(db, &k, id, 0))
        .await
        .unwrap();
    assert_eq!(rows[0]["selected"], false);
    assert_eq!(rows[0]["local"]["host"], "original.example.test");
    let k = key.clone();
    profile
        .database
        .write(move |db| store::choose(db, &k, id, 1, true))
        .await
        .unwrap();
    profile
        .database
        .write(|db| {
            db.execute(
                "UPDATE accounts SET settings=json_set(settings,'$.name','Later name')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let k = key.clone();
    assert!(
        profile
            .database
            .write(move |db| store::approve(db, &k, id, true, true))
            .await
            .is_err()
    );
    profile
        .database
        .write(move |db| {
            let mut account = account();
            account.id = shared.to_string();
            account.host = "original.example.test".into();
            db.execute(
                "UPDATE accounts SET settings=?",
                [serde_json::to_string(&account)?],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let k = key.clone();
    let review = profile
        .database
        .write(move |db| store::approve(db, &k, id, true, false))
        .await
        .unwrap();
    let applied = apply::step(&profile, &key, review.clone()).await.unwrap();
    assert_eq!(applied.applied, 1);
    // A lost application reply retries from the durable per-row receipt.
    apply::step(&profile, &key, review).await.unwrap();
    profile
        .database
        .read(move |db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get::<_, i64>(0))?,
                2
            );
            assert_eq!(
                crate::operations::stored_account(db, &shared.to_string())?.host,
                "original.example.test"
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM profile_reconnect", [], |r| r
                    .get::<_, i64>(0))?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
}
#[test]
fn device_preferences_refuse_unknown_fields_and_invalid_revisions() {
    let mut baseline = preferences();
    baseline.validate().unwrap();
    baseline
        .values
        .insert("credential_slot".into(), "not-portable".into());
    assert!(baseline.validate().is_err());
    let mut baseline = preferences();
    baseline.revisions.insert("appearance".into(), u64::MAX);
    assert!(baseline.validate().is_err());
}

#[tokio::test]
async fn reconnect_activation_clears_import_guard_and_newer_local_account_changes_are_kept() {
    let (_dir, profile) = profile().await;
    let source = source(1);
    let review = prepared(&profile, &source).await;
    let id = review.id;
    let key = scope().storage_key().unwrap();
    let k = key.clone();
    let review = profile
        .database
        .write(move |db| store::approve(db, &k, id, true, false))
        .await
        .unwrap();
    let review = apply::step(&profile, &key, review).await.unwrap();
    let review = apply::step(&profile, &key, review).await.unwrap();
    assert_eq!(review.phase, "settings");
    let k = key.clone();
    profile
        .database
        .write(move |db| apply::confirm(db, &k, id, vec![], vec![], None))
        .await
        .unwrap();
    profile
        .database
        .write(|db| {
            let id: String =
                db.query_row("SELECT account_id FROM profile_reconnect", [], |r| r.get(0))?;
            let account = crate::operations::stored_account(db, &id)?;
            let old: String = db.query_row(
                "SELECT slot FROM account_credentials WHERE account_id=?",
                [&id],
                |r| r.get(0),
            )?;
            let prepared = crate::connections::prepare(db, account.clone(), Some(account.clone()))?;
            let slot = prepared["slot"].as_str().unwrap();
            assert_ne!(slot, old);
            assert!(crate::connections::target(db, account.clone()).is_err());
            crate::connections::activate(db, slot)?;
            assert_eq!(crate::connections::target(db, account)?, slot);
            crate::connections::check_binding(db, &id, Some(slot))?;
            assert!(crate::connections::check_binding(db, &id, Some(&old)).is_err());
            assert!(crate::connections::cleanup(db)?.contains(&old));
            Ok(())
        })
        .await
        .unwrap();
    let review = prepared(&profile, &source).await;
    let id = review.id;
    let k = key.clone();
    let review = profile
        .database
        .write(move |db| store::approve(db, &k, id, true, false))
        .await
        .unwrap();
    profile
        .database
        .write(|db| {
            db.execute(
                "UPDATE accounts SET settings=json_set(settings,'$.name','Newer local name')",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let review = apply::step(&profile, &key, review).await.unwrap();
    assert_eq!(review.kept, 1);
    assert_eq!(review.applied, 0);
    profile
        .database
        .read(|db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get::<_, i64>(0))?,
                1
            );
            assert_eq!(
                db.query_row(
                    "SELECT json_extract(settings,'$.name') FROM accounts",
                    [],
                    |r| r.get::<_, String>(0)
                )?,
                "Newer local name"
            );
            Ok(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn removed_device_mapping_stays_unselected_and_unknown_connection_fields_cannot_apply() {
    let (_dir, profile) = profile().await;
    let original = source(1);
    let mut review = prepared(&profile, &original).await;
    let id = review.id;
    let key = scope().storage_key().unwrap();
    let k = key.clone();
    review = profile
        .database
        .write(move |db| store::approve(db, &k, id, true, false))
        .await
        .unwrap();
    while review.phase == "applying" {
        review = apply::step(&profile, &key, review).await.unwrap();
    }
    let k = key.clone();
    profile
        .database
        .write(move |db| apply::confirm(db, &k, id, vec![], vec![], None))
        .await
        .unwrap();
    profile
        .database
        .write(|db| {
            let id: String = db.query_row("SELECT id FROM accounts", [], |r| r.get(0))?;
            let removal = crate::accounts::preview(db, &id)?;
            crate::accounts::remove(db, removal, false)?;
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM profile_account_mappings", [], |r| r
                    .get::<_, i64>(
                    0
                ))?,
                1
            );
            Ok(())
        })
        .await
        .unwrap();
    let review = prepared(&profile, &original).await;
    let id = review.id;
    let k = key.clone();
    let rows = profile
        .database
        .read(move |db| store::rows(db, &k, id, 0))
        .await
        .unwrap();
    assert_eq!(rows[0]["selected"], false);
    assert!(rows[0]["reason"].as_str().unwrap().contains("removed here"));
    let k = key.clone();
    let review = profile
        .database
        .write(move |db| store::approve(db, &k, id, true, false))
        .await
        .unwrap();
    let review = apply::step(&profile, &key, review).await.unwrap();
    assert_eq!(review.kept, 1);
    profile
        .database
        .read(|db| {
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM accounts", [], |r| r.get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .await
        .unwrap();

    let (_dir, other) = crate::tests::profile().await;
    let mut future = source(1);
    for record in &mut future.records {
        let mut operation = shep_profile_core::Operation::decode(record.record.as_bytes()).unwrap();
        for change in &mut operation.changes {
            if let Action::AccountConnection { account } = &mut change.action {
                account.extra.insert(
                    "future_connection_option".into(),
                    serde_json::json!({"retained":true}),
                );
            }
        }
        record.record = String::from_utf8(operation.encode().unwrap()).unwrap();
    }
    let review = prepared(&other, &future).await;
    let id = review.id;
    let k = key.clone();
    let rows = other
        .database
        .read(move |db| store::rows(db, &k, id, 0))
        .await
        .unwrap();
    assert_eq!(rows[0]["available"], false);
    assert_eq!(rows[0]["selected"], false);
    assert!(
        rows[0]["reason"]
            .as_str()
            .unwrap()
            .contains("additional connection fields")
    );
    let k = key.clone();
    assert!(
        other
            .database
            .write(move |db| store::choose(db, &k, id, 1, true))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn original_platform_revisions_survive_failed_native_receipts_restart_and_changed_retries() {
    let (dir, profile) = profile().await;
    let mut baseline = preferences();
    baseline.revisions.values_mut().for_each(|value| *value = 3);
    let mut review = prepared_preferences(&profile, &source(0), baseline.clone()).await;
    let key = scope().storage_key().unwrap();
    let id = review.id;
    let k = key.clone();
    review = profile
        .database
        .write(move |db| store::approve(db, &k, id, false, true))
        .await
        .unwrap();
    review = apply::step(&profile, &key, review).await.unwrap();
    assert_eq!(review.phase, "settings");
    let mut revisions = baseline.revisions.clone();
    revisions.insert("appearance".into(), 4);
    // Decode through the production command boundary, including missing legacy proof.
    let command = serde_json::json!({"kind":"confirm_settings", "id":id,
        "applied":["appearance"], "kept":["preview_lines"], "revisions":revisions});
    let Command::ConfirmSettings {
        revisions: decoded, ..
    } = serde_json::from_value(command.clone()).unwrap()
    else {
        panic!()
    };
    assert_eq!(decoded.as_ref(), Some(&revisions));
    for malformed in [
        serde_json::json!({}),
        serde_json::json!({"appearance":-1}),
        serde_json::json!({"appearance":1.5}),
    ] {
        let mut invalid = command.clone();
        invalid["revisions"] = malformed;
        if let Ok(Command::ConfirmSettings { revisions, .. }) = serde_json::from_value(invalid) {
            let k = key.clone();
            assert!(
                profile
                    .database
                    .write(move |db| apply::confirm(
                        db,
                        &k,
                        id,
                        vec!["appearance".into()],
                        vec!["preview_lines".into()],
                        revisions
                    ))
                    .await
                    .is_err()
            );
        }
    }
    let mut missing = revisions.clone();
    missing.remove("tooltips");
    let mut unknown = revisions.clone();
    unknown.remove("tooltips");
    unknown.insert("future_field".into(), 3);
    let mut exhausted = revisions.clone();
    exhausted.insert("tooltips".into(), 9_007_199_254_740_992);
    let mut older = revisions.clone();
    older.insert("tooltips".into(), 2);
    for invalid in [missing, unknown, exhausted, older] {
        let k = key.clone();
        assert!(
            profile
                .database
                .write(move |db| apply::confirm(
                    db,
                    &k,
                    id,
                    vec!["appearance".into()],
                    vec!["preview_lines".into()],
                    Some(invalid)
                ))
                .await
                .is_err()
        );
    }
    profile
        .database
        .write(|db| {
            db.execute_batch(
                "CREATE TRIGGER fail_settings_receipt BEFORE UPDATE ON profile_enrollments
            WHEN NEW.phase='complete' BEGIN SELECT RAISE(ABORT,'isolated receipt failure'); END;",
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let original = revisions.clone();
    let k = key.clone();
    assert!(
        profile
            .database
            .write(move |db| apply::confirm(
                db,
                &k,
                id,
                vec!["appearance".into()],
                vec!["preview_lines".into()],
                Some(original)
            ))
            .await
            .is_err()
    );
    let k = key.clone();
    let pending = profile
        .database
        .read(move |db| read(db, &k, id))
        .await
        .unwrap();
    assert_eq!(pending.phase, "settings");
    assert!(pending.settings_receipt.is_none());
    profile
        .database
        .write(|db| {
            db.execute_batch("DROP TRIGGER fail_settings_receipt")?;
            Ok(())
        })
        .await
        .unwrap();
    drop(profile);
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    // Confirm the original proof after a lost application reply; newer local
    // platform state is deliberately not an argument to this native command.
    let original = revisions.clone();
    let k = key.clone();
    let completed = reopened
        .database
        .write(move |db| {
            apply::confirm(
                db,
                &k,
                id,
                vec!["appearance".into()],
                vec!["preview_lines".into()],
                Some(original),
            )
        })
        .await
        .unwrap();
    assert_eq!(
        completed.settings_receipt.as_ref().unwrap()["revisions"],
        serde_json::json!(revisions)
    );
    let original = revisions.clone();
    let k = key.clone();
    assert_eq!(
        reopened
            .database
            .write(move |db| apply::confirm(
                db,
                &k,
                id,
                vec!["appearance".into()],
                vec!["preview_lines".into()],
                Some(original)
            ))
            .await
            .unwrap()
            .settings_receipt,
        completed.settings_receipt
    );
    let mut newer = revisions;
    newer.insert("appearance".into(), 5);
    for changed in [Some(newer), None] {
        let k = key.clone();
        assert!(
            reopened
                .database
                .write(move |db| apply::confirm(
                    db,
                    &k,
                    id,
                    vec!["appearance".into()],
                    vec!["preview_lines".into()],
                    changed
                ))
                .await
                .is_err()
        );
    }
    let k = key;
    assert_eq!(
        reopened
            .database
            .read(move |db| read(db, &k, id))
            .await
            .unwrap()
            .settings_receipt,
        completed.settings_receipt
    );
}

#[tokio::test]
async fn legacy_platform_receipts_cannot_acquire_later_current_revisions() {
    let (_dir, profile) = profile().await;
    let review = prepared(&profile, &source(0)).await;
    let key = scope().storage_key().unwrap();
    let id = review.id;
    let k = key.clone();
    let review = profile
        .database
        .write(move |db| store::approve(db, &k, id, false, false))
        .await
        .unwrap();
    apply::step(&profile, &key, review).await.unwrap();
    let command = serde_json::json!({"kind":"confirm_settings", "id":id, "applied":[], "kept":[]});
    let Command::ConfirmSettings { revisions, .. } = serde_json::from_value(command).unwrap()
    else {
        panic!()
    };
    assert!(revisions.is_none());
    let k = key.clone();
    let complete = profile
        .database
        .write(move |db| apply::confirm(db, &k, id, vec![], vec![], None))
        .await
        .unwrap();
    assert!(
        complete
            .settings_receipt
            .unwrap()
            .get("revisions")
            .is_none()
    );
    assert!(
        profile
            .database
            .write(move |db| apply::confirm(
                db,
                &key,
                id,
                vec![],
                vec![],
                Some(preferences().revisions)
            ))
            .await
            .is_err()
    );
}
