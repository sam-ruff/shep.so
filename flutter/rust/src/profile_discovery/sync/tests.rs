use super::{
    super::enrollment::{
        tests::{enrolled, preferences, prepare_only, scope},
        transfer::Records,
    },
    *,
};
use crate::tests::profile;
use shep_profile_core::{
    SettingKey,
    drive::catalog::Profile,
    history::{Journal, LocalEdit},
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

/// A second enrolled device: its journal is the shared profile as Drive would
/// hold it. Publishing imports our exact bytes there; its edits reach us as
/// original records through the same export path the catalog uses.
struct Device {
    binding: Binding,
    journal: Mutex<Journal>,
    device: Mutex<Uuid>,
    complete: AtomicBool,
    pulls: AtomicUsize,
    uploads: AtomicUsize,
}
fn change(action: Action) -> Change {
    Change {
        action,
        extra: Default::default(),
    }
}
impl Device {
    fn new() -> Self {
        let scope = scope();
        let binding = Binding {
            namespace: scope.namespace,
            principal: scope.principal,
            profile: Uuid::new_v4(),
            generation: Uuid::new_v4(),
        };
        let journal = Journal::memory(binding.clone()).unwrap();
        let device = Self {
            binding,
            journal: Mutex::new(journal),
            device: Mutex::new(Uuid::new_v4()),
            complete: AtomicBool::new(true),
            pulls: AtomicUsize::new(0),
            uploads: AtomicUsize::new(0),
        };
        device.edit(vec![Action::ProfileSetup { complete: false }]);
        device.edit(vec![
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
        ]);
        device.edit(vec![Action::ProfileSetup { complete: true }]);
        device
    }
    fn edit(&self, actions: Vec<Action>) -> Uuid {
        let mut journal = self.journal.lock().unwrap();
        let revision = journal.state().unwrap().revision;
        let operation = Uuid::new_v4();
        journal
            .execute(HistoryCommand::Edit {
                edit: LocalEdit {
                    operation,
                    expected_revision: revision,
                    changes: actions.into_iter().map(change).collect(),
                    resolutions: vec![],
                },
            })
            .unwrap();
        operation
    }
    fn set(&self, key: SettingKey, value: Value) -> Uuid {
        self.edit(vec![Action::Setting { key, value }])
    }
    fn current(&self, field: &str) -> Vec<Value> {
        let mut journal = self.journal.lock().unwrap();
        let Reply::Versions(versions) = journal
            .execute(HistoryCommand::Versions {
                target: target(field),
                after: None,
            })
            .unwrap()
        else {
            panic!()
        };
        versions
            .into_iter()
            .map(|v| {
                let Reply::Value(change) = journal
                    .execute(HistoryCommand::Value {
                        target: target(field),
                        operation: v.operation,
                    })
                    .unwrap()
                else {
                    panic!()
                };
                scalar(field, &change).unwrap()
            })
            .collect()
    }
    fn operations(&self) -> u64 {
        self.journal.lock().unwrap().state().unwrap().operations
    }
    fn snapshot_now(&self) -> Snapshot {
        let journal = self.journal.lock().unwrap();
        let overview = journal.overview().unwrap();
        Snapshot {
            binding: self.binding.clone(),
            profile: Profile {
                profile: self.binding.profile,
                generation: self.binding.generation,
                name: overview.name,
                name_conflict: overview.name_conflict,
                accounts: overview.accounts,
                settings: overview.settings,
                operations: overview.state.operations,
                waiting: overview.state.waiting,
                ready: overview.state.ready,
                conflicts: overview.state.conflicts,
                removed: overview.state.removed,
                initialized: overview.state.initialized,
                revision: overview.state.revision,
            },
        }
    }
}
#[async_trait::async_trait]
impl Records for Device {
    async fn export(&self, source: Snapshot, after: u64) -> Result<Option<Record>> {
        Source::export(self, source, after).await
    }
}
#[async_trait::async_trait]
impl Source for Device {
    async fn pull(&self) -> Result<bool> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        Ok(self.complete.load(Ordering::SeqCst))
    }
    async fn snapshot(&self, profile: Uuid, generation: Uuid) -> Result<Snapshot> {
        ensure!(
            profile == self.binding.profile && generation == self.binding.generation,
            "unknown profile"
        );
        Ok(self.snapshot_now())
    }
    async fn source_device(&self, _source: Snapshot) -> Result<Uuid> {
        Ok(*self.device.lock().unwrap())
    }
    async fn export(&self, source: Snapshot, after: u64) -> Result<Option<Record>> {
        let journal = self.journal.lock().unwrap();
        ensure!(
            source.profile.revision == journal.state().unwrap().revision,
            "The profile changed. Snapshot it again."
        );
        Ok(journal.export_record(source.profile.revision, after)?)
    }
    async fn publish(&self, worker: &Worker) -> Result<Option<Uuid>> {
        let Reply::Upload(Some(upload)) = worker.request(HistoryCommand::NextUpload).await? else {
            return Ok(None);
        };
        let file_id = upload
            .file_id
            .clone()
            .unwrap_or_else(|| format!("file-{}", upload.operation));
        worker
            .request(HistoryCommand::Reserve {
                operation: upload.operation,
                file_id: file_id.clone(),
            })
            .await?;
        self.journal
            .lock()
            .unwrap()
            .execute(HistoryCommand::Import {
                record: upload.record.clone(),
            })?;
        self.uploads.fetch_add(1, Ordering::SeqCst);
        worker
            .request(HistoryCommand::Confirm {
                operation: upload.operation,
                file_id,
                sha256: upload.sha256,
            })
            .await?;
        Ok(Some(upload.operation))
    }
}

fn snapshot(values: Value, revisions: &[(&str, u64)]) -> Preferences {
    let mut snapshot = preferences();
    for (key, value) in values.as_object().unwrap() {
        snapshot.values.insert(key.clone(), value.clone());
    }
    for (key, revision) in revisions {
        snapshot.revisions.insert((*key).to_owned(), *revision);
    }
    snapshot
}
async fn command(profile: &MobileProfile, device: &Device, command: Command) -> Result<Value> {
    run(profile, scope(), device, command).await
}
/// Enrol with appearance applied at device revision 1 and every other field at 0.
async fn subscribed(profile: &MobileProfile, device: &Device) -> Value {
    let receipt = snapshot(
        serde_json::json!({"appearance":"Dark"}),
        &[("appearance", 1)],
    );
    let review = enrolled(
        profile,
        device.snapshot_now(),
        device,
        preferences(),
        vec!["appearance".into(), "preview_lines".into()],
        Some(receipt.revisions.clone()),
    )
    .await;
    let status = command(
        profile,
        device,
        Command::Subscribe {
            enrollment: review.id,
            snapshot: receipt,
        },
    )
    .await
    .unwrap();
    command(
        profile,
        device,
        Command::Configure {
            expected_revision: status["revision"].as_u64().unwrap(),
            enabled: Some(true),
            field: None,
            selected: None,
        },
    )
    .await
    .unwrap()
}
async fn cycle(profile: &MobileProfile, device: &Device, local: &Preferences) -> Value {
    command(
        profile,
        device,
        Command::Cycle {
            snapshot: local.clone(),
        },
    )
    .await
    .unwrap()
}
fn basis(status: &Value, field: &str) -> Basis {
    serde_json::from_value(status["bases"][field].clone()).unwrap()
}

#[tokio::test]
async fn seeding_uses_original_revisions_and_leaves_kept_and_legacy_fields_pending() {
    let (_dir, profile) = profile().await;
    let device = Device::new();
    let status = subscribed(&profile, &device).await;
    assert_eq!(status["enabled"], true);
    assert_eq!(status["unproven"], 0);
    let appearance = basis(&status, "appearance");
    assert_eq!(appearance.native_revision, Some(1));
    assert_eq!(appearance.value, Value::from("Dark"));
    assert!(appearance.operation.is_some());
    let reset = basis(&status, "preview_lines");
    assert_eq!(reset.value, Value::Null);
    assert!(reset.operation.is_some());
    assert_eq!(reset.native_revision, Some(0));
    // Fields the profile never defined keep proof of an unchanged device.
    let tooltips = basis(&status, "tooltips");
    assert_eq!(tooltips.operation, None);
    assert_eq!(tooltips.native_revision, Some(0));

    // A legacy receipt proves nothing for any field.
    let (_dir, profile) = crate::tests::profile().await;
    let legacy = Device::new();
    let review = enrolled(
        &profile,
        legacy.snapshot_now(),
        &legacy,
        preferences(),
        vec!["appearance".into()],
        None,
    )
    .await;
    let status = command(
        &profile,
        &legacy,
        Command::Subscribe {
            enrollment: review.id,
            snapshot: snapshot(serde_json::json!({}), &[("appearance", 3)]),
        },
    )
    .await
    .unwrap();
    assert_eq!(status["unproven"], 8);
    let appearance = basis(&status, "appearance");
    assert_eq!(appearance.native_revision, None);
    assert_eq!(appearance.observed, 3);
    assert!(appearance.operation.is_some());
}

#[tokio::test]
async fn local_intent_is_admitted_before_pulling_and_published_to_the_other_device() {
    let (_dir, profile) = profile().await;
    let device = Device::new();
    subscribed(&profile, &device).await;
    // Same value at a newer revision is still explicit intent (change and revert).
    let local = snapshot(
        serde_json::json!({"appearance":"Light","tooltips":true}),
        &[("appearance", 2), ("tooltips", 1)],
    );
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["admitted"], 2);
    assert_eq!(status["last"]["published"], 2);
    assert_eq!(status["last"]["remaining"], false);
    assert_eq!(device.current("appearance"), vec![Value::from("Light")]);
    assert_eq!(device.current("tooltips"), vec![Value::from(true)]);
    assert_eq!(basis(&status, "appearance").native_revision, Some(2));
    assert_eq!(basis(&status, "tooltips").native_revision, Some(1));
    assert_eq!(status["staged"], 0);
    // A repeated cycle with unchanged intent publishes nothing more.
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["admitted"], 0);
    assert_eq!(status["last"]["published"], 0);
    assert_eq!(device.uploads.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn remote_changes_apply_through_one_device_receipt_at_a_time_and_retry_exactly() {
    let (dir, profile) = profile().await;
    let device = Device::new();
    subscribed(&profile, &device).await;
    let local = snapshot(
        serde_json::json!({"appearance":"Dark"}),
        &[("appearance", 1)],
    );
    let light = device.set(SettingKey::Appearance, "Light".into());
    device.set(SettingKey::Tooltips, false.into());
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["imported"], 2);
    assert_eq!(status["applications"], 1);
    assert_eq!(status["last"]["remaining"], true);
    let request = command(&profile, &device, Command::Application)
        .await
        .unwrap();
    assert_eq!(
        request["changes"],
        serde_json::json!({"appearance":"Light"})
    );
    assert_eq!(request["baseline"], serde_json::to_value(&local).unwrap());
    let id: Uuid = serde_json::from_value(request["id"].clone()).unwrap();
    // Without the receipt, another cycle after restart stages nothing else and
    // hands the device the same request to retry.
    drop(profile);
    let profile = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["applications"], 1);
    assert_eq!(
        command(&profile, &device, Command::Application)
            .await
            .unwrap()["id"],
        request["id"]
    );
    // Enrollment must wait for the unconfirmed platform receipt.
    assert!(
        prepare_only(&profile, device.snapshot_now())
            .await
            .unwrap_err()
            .to_string()
            .contains("applying")
    );
    let mut revisions = local.revisions.clone();
    revisions.insert("appearance".into(), 2);
    let confirm = |applied: Vec<&str>, kept: Vec<&str>, revisions: BTreeMap<String, u64>| {
        Command::ConfirmApplication {
            id,
            applied: applied.into_iter().map(Into::into).collect(),
            kept: kept.into_iter().map(Into::into).collect(),
            revisions,
        }
    };
    assert!(
        command(
            &profile,
            &device,
            confirm(vec!["tooltips"], vec![], revisions.clone())
        )
        .await
        .is_err()
    );
    let status = command(
        &profile,
        &device,
        confirm(vec!["appearance"], vec![], revisions.clone()),
    )
    .await
    .unwrap();
    let appearance = basis(&status, "appearance");
    assert_eq!(appearance.operation, Some(light));
    assert_eq!(appearance.native_revision, Some(2));
    assert_eq!(status["applications"], 0);
    // The same receipt is idempotent; a different one cannot replace it.
    command(
        &profile,
        &device,
        confirm(vec!["appearance"], vec![], revisions.clone()),
    )
    .await
    .unwrap();
    assert!(
        command(
            &profile,
            &device,
            confirm(vec![], vec!["appearance"], revisions)
        )
        .await
        .is_err()
    );
    let local = snapshot(
        serde_json::json!({"appearance":"Light"}),
        &[("appearance", 2)],
    );
    let status = cycle(&profile, &device, &local).await;
    let request = command(&profile, &device, Command::Application)
        .await
        .unwrap();
    assert_eq!(request["changes"], serde_json::json!({"tooltips":false}));
    assert_eq!(status["reviews"], 0);
    assert_eq!(device.uploads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_kept_receipt_is_newer_local_intent_that_wins_over_the_older_remote_value() {
    let (_dir, profile) = profile().await;
    let device = Device::new();
    subscribed(&profile, &device).await;
    let local = snapshot(
        serde_json::json!({"appearance":"Dark"}),
        &[("appearance", 1)],
    );
    device.set(SettingKey::Appearance, "Light".into());
    cycle(&profile, &device, &local).await;
    let request = command(&profile, &device, Command::Application)
        .await
        .unwrap();
    let id: Uuid = serde_json::from_value(request["id"].clone()).unwrap();
    let mut revisions = local.revisions.clone();
    revisions.insert("appearance".into(), 2);
    let status = command(
        &profile,
        &device,
        Command::ConfirmApplication {
            id,
            applied: vec![],
            kept: vec!["appearance".into()],
            revisions,
        },
    )
    .await
    .unwrap();
    assert_eq!(basis(&status, "appearance").native_revision, Some(1));
    let local = snapshot(
        serde_json::json!({"appearance":"System"}),
        &[("appearance", 2)],
    );
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["admitted"], 1);
    assert_eq!(status["reviews"], 0);
    assert_eq!(device.current("appearance"), vec![Value::from("System")]);
}

#[tokio::test]
async fn concurrent_edits_defer_to_a_checked_review_whose_decisions_converge_both_devices() {
    let (_dir, profile) = profile().await;
    let device = Device::new();
    subscribed(&profile, &device).await;
    let remote = device.set(SettingKey::Appearance, "Light".into());
    let local = snapshot(
        serde_json::json!({"appearance":"System"}),
        &[("appearance", 2)],
    );
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["admitted"], 1);
    assert_eq!(status["reviews"], 1);
    assert_eq!(status["applications"], 0);
    let reviews = command(&profile, &device, Command::Reviews).await.unwrap();
    let review = &reviews[0];
    assert_eq!(review["field"], "appearance");
    assert_eq!(review["kind"], "conflict");
    assert_eq!(review["total"], 2);
    assert_eq!(review["local"], "System");
    assert_eq!(review["deciding"], false);
    let id: Uuid = serde_json::from_value(review["id"].clone()).unwrap();
    let page = command(
        &profile,
        &device,
        Command::ReviewVersions { id, after: None },
    )
    .await
    .unwrap();
    assert_eq!(page.as_array().unwrap().len(), 2);
    let ours: Uuid = serde_json::from_value(page[0]["operation"].clone()).unwrap();
    let last: Uuid = serde_json::from_value(page[1]["operation"].clone()).unwrap();
    assert!(
        page.as_array()
            .unwrap()
            .iter()
            .any(|v| v["operation"] == serde_json::json!(remote))
    );
    assert_eq!(
        command(
            &profile,
            &device,
            Command::ReviewVersions {
                id,
                after: Some(last)
            }
        )
        .await
        .unwrap(),
        serde_json::json!([])
    );
    // Every page must be seen, and the local intent must be unchanged.
    assert!(
        command(
            &profile,
            &device,
            Command::Decide {
                id,
                choice: Choice::Local,
                seen: 1,
                snapshot: local.clone()
            }
        )
        .await
        .is_err()
    );
    let moved = snapshot(
        serde_json::json!({"appearance":"Dark"}),
        &[("appearance", 3)],
    );
    assert!(
        command(
            &profile,
            &device,
            Command::Decide {
                id,
                choice: Choice::Local,
                seen: 2,
                snapshot: moved
            }
        )
        .await
        .is_err()
    );
    assert!(
        command(
            &profile,
            &device,
            Command::Decide {
                id,
                choice: Choice::Shared {
                    operation: Uuid::new_v4()
                },
                seen: 2,
                snapshot: local.clone()
            }
        )
        .await
        .is_err()
    );
    let status = command(
        &profile,
        &device,
        Command::Decide {
            id,
            choice: Choice::Local,
            seen: 2,
            snapshot: local.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(status["reviews"], 0);
    assert_eq!(status["applications"], 0);
    let resolution = basis(&status, "appearance").operation.unwrap();
    assert!(resolution != ours && resolution != remote);
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["published"], 1);
    assert_eq!(device.current("appearance"), vec![Value::from("System")]);

    // Use profile: the other device changes again while we edit again.
    let remote = device.set(SettingKey::Appearance, "Light".into());
    let local = snapshot(
        serde_json::json!({"appearance":"Dark"}),
        &[("appearance", 3)],
    );
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["reviews"], 1);
    let reviews = command(&profile, &device, Command::Reviews).await.unwrap();
    let id: Uuid = serde_json::from_value(reviews[0]["id"].clone()).unwrap();
    let status = command(
        &profile,
        &device,
        Command::Decide {
            id,
            choice: Choice::Shared { operation: remote },
            seen: 2,
            snapshot: local.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(status["reviews"], 0);
    assert_eq!(status["applications"], 1);
    let request = command(&profile, &device, Command::Application)
        .await
        .unwrap();
    assert_eq!(
        request["changes"],
        serde_json::json!({"appearance":"Light"})
    );
    let application: Uuid = serde_json::from_value(request["id"].clone()).unwrap();
    let mut revisions = local.revisions.clone();
    revisions.insert("appearance".into(), 4);
    command(
        &profile,
        &device,
        Command::ConfirmApplication {
            id: application,
            applied: vec!["appearance".into()],
            kept: vec![],
            revisions,
        },
    )
    .await
    .unwrap();
    let local = snapshot(
        serde_json::json!({"appearance":"Light"}),
        &[("appearance", 4)],
    );
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["published"], 1);
    assert_eq!(status["reviews"], 0);
    assert_eq!(device.current("appearance"), vec![Value::from("Light")]);
}

#[tokio::test]
async fn unproven_fields_need_a_review_before_a_differing_remote_value_applies() {
    let (_dir, profile) = profile().await;
    let device = Device::new();
    let review = enrolled(
        &profile,
        device.snapshot_now(),
        &device,
        preferences(),
        vec!["appearance".into(), "preview_lines".into()],
        None,
    )
    .await;
    let local = snapshot(
        serde_json::json!({"appearance":"Dark"}),
        &[("appearance", 1)],
    );
    let status = command(
        &profile,
        &device,
        Command::Subscribe {
            enrollment: review.id,
            snapshot: local.clone(),
        },
    )
    .await
    .unwrap();
    command(
        &profile,
        &device,
        Command::Configure {
            expected_revision: status["revision"].as_u64().unwrap(),
            enabled: Some(true),
            field: None,
            selected: None,
        },
    )
    .await
    .unwrap();
    // Matching values are not acknowledged; nothing is applied or published.
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["reviews"], 0);
    assert_eq!(status["applications"], 0);
    assert_eq!(basis(&status, "appearance").native_revision, None);
    device.set(SettingKey::Appearance, "Light".into());
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["applications"], 0);
    assert_eq!(status["reviews"], 1);
    let reviews = command(&profile, &device, Command::Reviews).await.unwrap();
    assert_eq!(reviews[0]["kind"], "unproven");
    assert_eq!(reviews[0]["total"], 1);
    let id: Uuid = serde_json::from_value(reviews[0]["id"].clone()).unwrap();
    let page = command(
        &profile,
        &device,
        Command::ReviewVersions { id, after: None },
    )
    .await
    .unwrap();
    let shared: Uuid = serde_json::from_value(page[0]["operation"].clone()).unwrap();
    // Using the only shared version stages the device write without a new operation.
    let status = command(
        &profile,
        &device,
        Command::Decide {
            id,
            choice: Choice::Shared { operation: shared },
            seen: 1,
            snapshot: local.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(status["applications"], 1);
    assert_eq!(status["staged"], 0);
    assert_eq!(device.operations(), 4);
    // A later local edit on an unproven field is still published as intent.
    let local = snapshot(
        serde_json::json!({"tooltips":false}),
        &[("appearance", 1), ("tooltips", 2)],
    );
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["admitted"], 1);
    assert_eq!(device.current("tooltips"), vec![Value::from(false)]);
    assert_eq!(basis(&status, "tooltips").native_revision, Some(2));
}

#[tokio::test]
async fn a_lost_history_acknowledgment_retries_the_same_operation_after_restart() {
    let (dir, profile) = profile().await;
    let device = Device::new();
    let status = subscribed(&profile, &device).await;
    let subscription: Uuid = serde_json::from_value(status["id"].clone()).unwrap();
    let binding: Binding = serde_json::from_value(status["binding"].clone()).unwrap();
    // Stage the exact request, commit it to history, then lose the reply.
    let history = worker(&profile.database, binding.clone()).await.unwrap();
    let revision = state(history.request(HistoryCommand::State).await.unwrap())
        .unwrap()
        .revision;
    let operation = Uuid::new_v4();
    let edit = ledger::Edit {
        operation,
        field: "appearance".into(),
        request: LocalEdit {
            operation,
            expected_revision: revision,
            changes: vec![setting_change("appearance", &"Light".into(), None).unwrap()],
            resolutions: vec![],
        },
        native_revision: 2,
        review: None,
        state: "staged".into(),
    };
    let staged = edit.clone();
    profile
        .database
        .write(move |db| ledger::stage_edit(db, subscription, &staged))
        .await
        .unwrap();
    history
        .request(HistoryCommand::Edit {
            edit: edit.request.clone(),
        })
        .await
        .unwrap();
    history.close().await.unwrap();
    drop(profile);
    let reopened = MobileProfile::open(dir.path().join("mail.sqlite3").to_str().unwrap().into())
        .await
        .unwrap();
    let local = snapshot(
        serde_json::json!({"appearance":"Light"}),
        &[("appearance", 2)],
    );
    let status = cycle(&reopened, &device, &local).await;
    assert_eq!(status["last"]["admitted"], 1);
    assert_eq!(status["last"]["published"], 1);
    assert_eq!(status["staged"], 0);
    assert_eq!(basis(&status, "appearance").operation, Some(operation));
    assert_eq!(device.operations(), 4);
    let status = cycle(&reopened, &device, &local).await;
    assert_eq!(status["last"]["admitted"], 0);
    assert_eq!(device.operations(), 4);
    assert_eq!(device.uploads.load(Ordering::SeqCst), 1);

    // A staged edit whose target moved remotely before its retry is deferred to
    // a review that retains the exact local intent, never silently dropped.
    let remote = device.set(SettingKey::Tooltips, false.into());
    let history = worker(&reopened.database, binding).await.unwrap();
    let revision = state(history.request(HistoryCommand::State).await.unwrap())
        .unwrap()
        .revision;
    history.close().await.unwrap();
    let stale = Uuid::new_v4();
    let edit = ledger::Edit {
        operation: stale,
        field: "tooltips".into(),
        request: LocalEdit {
            operation: stale,
            expected_revision: revision,
            changes: vec![setting_change("tooltips", &true.into(), None).unwrap()],
            resolutions: vec![],
        },
        native_revision: 3,
        review: None,
        state: "staged".into(),
    };
    reopened
        .database
        .write(move |db| ledger::stage_edit(db, subscription, &edit))
        .await
        .unwrap();
    let local = snapshot(
        serde_json::json!({"appearance":"Light","tooltips":true}),
        &[("appearance", 2), ("tooltips", 3)],
    );
    // The retry precedes the pull, so the stale request is admitted first and
    // the remote edit then conflicts with it.
    let status = cycle(&reopened, &device, &local).await;
    assert_eq!(status["reviews"], 1);
    let reviews = command(&reopened, &device, Command::Reviews).await.unwrap();
    assert_eq!(reviews[0]["field"], "tooltips");
    assert_eq!(reviews[0]["kind"], "conflict");
    assert!(
        reviews[0]["versions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| *v == serde_json::json!(remote))
    );
}

#[tokio::test]
async fn controls_pause_work_and_a_pending_enrollment_or_rebuilt_source_is_respected() {
    let (_dir, profile) = profile().await;
    let device = Device::new();
    let status = subscribed(&profile, &device).await;
    let local = snapshot(
        serde_json::json!({"appearance":"Dark"}),
        &[("appearance", 1)],
    );
    let stale = status["revision"].as_u64().unwrap();
    let status = command(
        &profile,
        &device,
        Command::Configure {
            expected_revision: stale,
            enabled: None,
            field: Some("tooltips".into()),
            selected: Some(false),
        },
    )
    .await
    .unwrap();
    assert!(
        command(
            &profile,
            &device,
            Command::Configure {
                expected_revision: stale,
                enabled: Some(false),
                field: None,
                selected: None
            }
        )
        .await
        .is_err()
    );
    device.set(SettingKey::Tooltips, false.into());
    cycle(&profile, &device, &local).await;
    let after = cycle(&profile, &device, &local).await;
    assert_eq!(after["applications"], 0);
    assert_eq!(after["reviews"], 0);
    let local_edit = snapshot(
        serde_json::json!({"tooltips":false}),
        &[("appearance", 1), ("tooltips", 5)],
    );
    let after = cycle(&profile, &device, &local_edit).await;
    assert_eq!(after["last"]["admitted"], 0);
    assert_eq!(device.uploads.load(Ordering::SeqCst), 0);
    // Master switch off: the cycle refuses without touching history or Drive.
    let pulls = device.pulls.load(Ordering::SeqCst);
    let status = command(
        &profile,
        &device,
        Command::Configure {
            expected_revision: status["revision"].as_u64().unwrap(),
            enabled: Some(false),
            field: None,
            selected: None,
        },
    )
    .await
    .unwrap();
    assert!(
        command(
            &profile,
            &device,
            Command::Cycle {
                snapshot: local.clone()
            }
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("turned off")
    );
    assert_eq!(device.pulls.load(Ordering::SeqCst), pulls);
    command(
        &profile,
        &device,
        Command::Configure {
            expected_revision: status["revision"].as_u64().unwrap(),
            enabled: Some(true),
            field: None,
            selected: None,
        },
    )
    .await
    .unwrap();
    // An incomplete pull stops before observation; the next cycle resumes.
    device.complete.store(false, Ordering::SeqCst);
    device.set(SettingKey::Appearance, "Light".into());
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["last"]["remaining"], true);
    assert_eq!(status["applications"], 0);
    device.complete.store(true, Ordering::SeqCst);
    // A rebuilt observation history replays originals from the start.
    *device.device.lock().unwrap() = Uuid::new_v4();
    let status = cycle(&profile, &device, &local).await;
    assert_eq!(status["applications"], 1);
    assert_eq!(status["last"]["imported"], 5);
    // A pending enrollment blocks cycles until it completes or is cancelled.
    let request = command(&profile, &device, Command::Application)
        .await
        .unwrap();
    let id: Uuid = serde_json::from_value(request["id"].clone()).unwrap();
    let mut revisions = local.revisions.clone();
    revisions.insert("appearance".into(), 2);
    command(
        &profile,
        &device,
        Command::ConfirmApplication {
            id,
            applied: vec!["appearance".into()],
            kept: vec![],
            revisions,
        },
    )
    .await
    .unwrap();
    prepare_only(&profile, Device::new().snapshot_now())
        .await
        .unwrap();
    assert!(
        command(&profile, &device, Command::Cycle { snapshot: local })
            .await
            .unwrap_err()
            .to_string()
            .contains("enrollment")
    );
}
