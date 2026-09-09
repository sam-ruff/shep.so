#![cfg(feature = "history")]
use serde_json::json;
use shep_profile_core::{Action, Change, Operation, SettingKey, history::*};
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n + 1000)
}
fn binding() -> Binding {
    Binding {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture-owner".into(),
        profile: id(1),
        generation: id(2),
    }
}
fn change(action: Action) -> Change {
    Change {
        action,
        extra: Default::default(),
    }
}
fn setting(value: &str) -> Change {
    change(Action::Setting {
        key: SettingKey::Appearance,
        value: json!(value),
    })
}
fn op(n: u128, parents: &[u128], changes: Vec<Change>) -> Operation {
    let b = binding();
    Operation {
        format: shep_profile_core::FORMAT.into(),
        major: 1,
        minor: 0,
        requires: vec![
            "causal-v1".into(),
            "accounts-v1".into(),
            "settings-v1".into(),
        ],
        namespace: b.namespace,
        profile: b.profile,
        generation: b.generation,
        device: id(3 + n % 2),
        operation: id(n),
        parents: parents.iter().map(|n| id(*n)).collect(),
        changes,
        extra: Default::default(),
    }
}
fn import(j: &mut Journal, op: &Operation) -> State {
    j.import(&op.encode().unwrap()).unwrap()
}
fn edit(j: &Journal, n: u128, changes: Vec<Change>) -> LocalEdit {
    LocalEdit {
        operation: id(n),
        expected_revision: j.state().unwrap().revision,
        changes,
        resolutions: vec![],
    }
}
fn value(j: &Journal, key: &str) -> Change {
    let versions = j.versions(key, None).unwrap();
    assert_eq!(versions.len(), 1);
    j.value(key, versions[0].operation).unwrap()
}

#[test]
fn overview_counts_account_definitions_and_setting_intents_with_explicit_name_conflicts() {
    let fixture = Operation::decode(include_bytes!("../../profile-operation.json")).unwrap();
    let account = fixture
        .changes
        .iter()
        .find_map(|change| match &change.action {
            Action::AccountConnection { account } => Some(account.clone()),
            _ => None,
        })
        .unwrap();
    let mut journal = Journal::memory(binding()).unwrap();
    import(
        &mut journal,
        &op(
            10,
            &[],
            vec![
                change(Action::ProfileName {
                    name: "Work".into(),
                }),
                change(Action::AccountConnection {
                    account: account.clone(),
                }),
                setting("Dark"),
            ],
        ),
    );
    let overview = journal.overview().unwrap();
    assert_eq!(overview.accounts, 1);
    assert_eq!(overview.settings, 1);
    assert_eq!(overview.name.as_deref(), Some("Work"));
    assert!(!overview.name_conflict);
    import(
        &mut journal,
        &op(
            11,
            &[10],
            vec![change(Action::AccountName {
                id: id(999),
                name: "Name without a definition".into(),
            })],
        ),
    );
    import(
        &mut journal,
        &op(
            12,
            &[10],
            vec![change(Action::ProfileName {
                name: "Personal".into(),
            })],
        ),
    );
    import(
        &mut journal,
        &op(
            13,
            &[10],
            vec![change(Action::ProfileName {
                name: "Another".into(),
            })],
        ),
    );
    let overview = journal.overview().unwrap();
    assert_eq!(overview.accounts, 1);
    assert!(overview.name_conflict);
    assert!(overview.name.is_none());
    import(
        &mut journal,
        &op(
            14,
            &[11, 12, 13],
            vec![
                change(Action::AccountRemoved { id: account.id }),
                change(Action::SettingRemoved {
                    key: SettingKey::Appearance,
                }),
            ],
        ),
    );
    let overview = journal.overview().unwrap();
    assert_eq!((overview.accounts, overview.settings), (0, 1));
    assert!(matches!(
        value(&journal, "setting:appearance").action,
        Action::SettingRemoved { .. }
    ));
    assert!(overview.name_conflict);
    import(
        &mut journal,
        &op(15, &[14], vec![change(Action::ProfileRemoved)]),
    );
    let overview = journal.overview().unwrap();
    assert!(overview.state.removed);
    assert!(!overview.name_conflict);
    assert!(overview.name.is_none());
    assert_eq!((overview.accounts, overview.settings), (0, 0));
}

#[test]
fn independent_devices_merge_fields_and_review_conflicts_without_clock_winners() {
    let a = op(
        10,
        &[],
        vec![
            change(Action::ProfileName {
                name: "Personal".into(),
            }),
            setting("Light"),
        ],
    );
    let b = op(11, &[10], vec![setting("Dark")]);
    let c = op(
        12,
        &[10],
        vec![change(Action::Setting {
            key: SettingKey::UnifiedInbox,
            value: json!(true),
        })],
    );
    let mut d = op(13, &[10], vec![setting("System")]);
    d.changes[0]
        .extra
        .insert("future_display_hint".into(), json!({"preserve":true}));
    let mut desktop = Journal::memory(binding()).unwrap();
    let mut phone = Journal::memory(binding()).unwrap();
    for item in [&a, &b, &c, &d] {
        import(&mut desktop, item);
    }
    for item in [&d, &c, &b, &a] {
        import(&mut phone, item);
    }
    for j in [&desktop, &phone] {
        assert_eq!(j.state().unwrap().conflicts, 1);
        assert_eq!(j.state().unwrap().fields, 3);
        assert_eq!(j.state().unwrap().waiting, 0);
        assert_eq!(
            j.versions("setting:appearance", None)
                .unwrap()
                .iter()
                .map(|v| v.operation)
                .collect::<Vec<_>>(),
            vec![id(11), id(13)]
        );
        assert_eq!(
            value(j, "setting:unified_inbox").action,
            c.changes[0].action
        );
        assert_eq!(
            j.value("setting:appearance", id(13)).unwrap().extra,
            d.changes[0].extra
        );
    }
    let mut resolution = edit(&desktop, 20, vec![setting("Light")]);
    assert!(matches!(
        desktop.edit(resolution.clone()),
        Err(Error::Conflict)
    ));
    resolution.resolutions.push(Resolution {
        target: "setting:appearance".into(),
        versions: vec![id(11)],
    });
    assert!(matches!(
        desktop.edit(resolution.clone()),
        Err(Error::Changed)
    ));
    resolution.resolutions[0].versions.push(id(13));
    desktop.edit(resolution.clone()).unwrap();
    let upload = desktop.next_upload().unwrap().unwrap();
    assert_eq!(
        Operation::decode(upload.record.as_bytes()).unwrap().parents,
        vec![id(11), id(12), id(13)]
    );
    phone.import(upload.record.as_bytes()).unwrap();
    assert_eq!(desktop.state().unwrap().conflicts, 0);
    assert_eq!(phone.state().unwrap().conflicts, 0);
    assert_eq!(
        value(&desktop, "setting:appearance"),
        value(&phone, "setting:appearance")
    );
    // A lost local reply retries the frozen request, despite a newer revision.
    let revision = desktop.state().unwrap().revision;
    desktop.edit(resolution.clone()).unwrap();
    assert_eq!(desktop.state().unwrap().revision, revision);
    resolution.changes = vec![setting("Dark")];
    assert!(matches!(desktop.edit(resolution), Err(Error::Identity)));
}

#[test]
fn missing_ancestry_survives_restart_and_drains_in_bounded_batches_and_pages() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.sqlite");
    let mut j = Journal::open(&path, binding()).unwrap();
    for n in 100..200 {
        import(
            &mut j,
            &op(
                n,
                &[10],
                vec![change(Action::AccountName {
                    id: id(n + 1000),
                    name: format!("Account {n}"),
                })],
            ),
        );
    }
    assert_eq!(j.state().unwrap().waiting, 100);
    assert!(j.fields(None).unwrap().is_empty());
    let pending_edit = edit(&j, 20, vec![setting("Light")]);
    assert!(matches!(j.edit(pending_edit), Err(Error::Incomplete)));
    drop(j);
    let mut j = Journal::open(&path, binding()).unwrap();
    assert_eq!(j.state().unwrap().waiting, 100);
    let ready = import(
        &mut j,
        &op(
            10,
            &[],
            vec![change(Action::ProfileName {
                name: "Work".into(),
            })],
        ),
    );
    assert_eq!(ready.waiting, 101 - APPLY_BATCH as u64);
    let mut previous = ready.waiting;
    while previous > 0 {
        let state = j.drain().unwrap();
        assert!(previous - state.waiting <= APPLY_BATCH as u64);
        assert!(state.waiting < previous);
        previous = state.waiting;
    }
    assert_eq!(j.state().unwrap().fields, 101);
    let first = j.fields(None).unwrap();
    assert_eq!(first.len(), PAGE_SIZE);
    let second = j.fields(Some(&first.last().unwrap().target)).unwrap();
    assert_eq!(second.len(), PAGE_SIZE);
    let third = j.fields(Some(&second.last().unwrap().target)).unwrap();
    assert_eq!(third.len(), 1);
}

#[test]
fn tombstones_dominate_concurrent_and_descendant_edits_in_either_arrival_order() {
    let root = op(
        10,
        &[],
        vec![change(Action::AccountName {
            id: id(30),
            name: "Old name".into(),
        })],
    );
    let remove = op(
        11,
        &[10],
        vec![change(Action::AccountRemoved { id: id(30) })],
    );
    let stale = op(
        12,
        &[10],
        vec![change(Action::AccountName {
            id: id(30),
            name: "Offline edit".into(),
        })],
    );
    let later = op(
        13,
        &[11, 12],
        vec![change(Action::AccountName {
            id: id(30),
            name: "Invalid resurrection".into(),
        })],
    );
    for order in [
        [&root, &remove, &stale, &later],
        [&later, &stale, &remove, &root],
    ] {
        let mut j = Journal::memory(binding()).unwrap();
        for item in order {
            import(&mut j, item);
        }
        let state = j.state().unwrap();
        assert_eq!((state.fields, state.conflicts, state.waiting), (1, 0, 0));
        assert_eq!(
            j.fields(None).unwrap()[0].target,
            format!("account:{}:removed", id(30))
        );
        assert!(
            j.value(&format!("account:{}:name", id(30)), id(10))
                .is_err()
        );
        let invalid = edit(
            &j,
            40,
            vec![change(Action::AccountName {
                id: id(30),
                name: "Again".into(),
            })],
        );
        assert!(matches!(j.edit(invalid), Err(Error::Removed)));
        let fresh = edit(
            &j,
            41,
            vec![change(Action::AccountName {
                id: id(31),
                name: "New identity".into(),
            })],
        );
        j.edit(fresh).unwrap();
        assert_eq!(j.state().unwrap().fields, 2);
        import(&mut j, &op(50, &[13], vec![change(Action::ProfileRemoved)]));
        import(&mut j, &op(51, &[10], vec![setting("Dark")]));
        let state = j.state().unwrap();
        assert!(state.removed);
        assert_eq!(state.fields, 1);
        assert_eq!(j.fields(None).unwrap()[0].target, "profile:removed");
        let fresh = edit(&j, 52, vec![setting("Light")]);
        assert!(matches!(j.edit(fresh), Err(Error::Removed)));
    }
}

#[test]
fn immutable_records_cycles_bindings_and_future_versions_cannot_replace_history() {
    let mut j = Journal::memory(binding()).unwrap();
    let root = op(10, &[], vec![setting("Light")]);
    let raw = root.encode().unwrap();
    j.import(&raw).unwrap();
    let original = j.state().unwrap();
    j.import(&raw).unwrap();
    assert_eq!(j.state().unwrap().revision, original.revision);
    let mut alternate = raw.clone();
    alternate.push(b'\n');
    assert!(matches!(j.import(&alternate), Err(Error::Identity)));
    let mut wrong = root.clone();
    wrong.operation = id(20);
    wrong.generation = id(777);
    assert!(matches!(
        j.import(&wrong.encode().unwrap()),
        Err(Error::Binding)
    ));
    let mut future: serde_json::Value = serde_json::from_slice(&raw).unwrap();
    future["major"] = json!(99);
    assert!(matches!(
        j.import(&serde_json::to_vec(&future).unwrap()),
        Err(Error::Record(shep_profile_core::Error::Upgrade))
    ));
    import(&mut j, &op(30, &[31], vec![setting("Dark")]));
    assert!(matches!(
        j.import(&op(31, &[30], vec![setting("System")]).encode().unwrap()),
        Err(Error::Cycle)
    ));
    assert_eq!(j.state().unwrap().operations, 2);
    assert_eq!(j.state().unwrap().waiting, 1);
    assert_eq!(
        value(&j, "setting:appearance").action,
        root.changes[0].action
    );
}

#[test]
fn concurrent_deletions_remain_tombstones_without_an_unresolvable_value_conflict() {
    let mut j = Journal::memory(binding()).unwrap();
    import(
        &mut j,
        &op(
            10,
            &[],
            vec![change(Action::AccountName {
                id: id(30),
                name: "Shared".into(),
            })],
        ),
    );
    import(
        &mut j,
        &op(
            11,
            &[10],
            vec![change(Action::AccountRemoved { id: id(30) })],
        ),
    );
    import(
        &mut j,
        &op(
            12,
            &[10],
            vec![change(Action::AccountRemoved { id: id(30) })],
        ),
    );
    assert_eq!(j.state().unwrap().conflicts, 0);
    assert_eq!(
        j.versions(&format!("account:{}:removed", id(30)), None)
            .unwrap()
            .len(),
        2
    );
    import(&mut j, &op(13, &[11], vec![change(Action::ProfileRemoved)]));
    import(&mut j, &op(14, &[12], vec![change(Action::ProfileRemoved)]));
    assert!(j.state().unwrap().removed);
    assert_eq!(j.state().unwrap().conflicts, 0);
    assert_eq!(j.state().unwrap().fields, 1);
}

#[test]
fn reserved_upload_identity_and_exact_bytes_survive_lost_replies_and_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.sqlite");
    let mut j = Journal::open(&path, binding()).unwrap();
    let mut change = setting("Light");
    change
        .extra
        .insert("extension".into(), json!({"retained":[1,2,3]}));
    let request = edit(&j, 10, vec![change]);
    j.edit(request.clone()).unwrap();
    let upload = j.next_upload().unwrap().unwrap();
    let device = j.state().unwrap().device;
    drop(j);
    let mut j = Journal::open(&path, binding()).unwrap();
    assert_eq!(j.state().unwrap().device, device);
    j.edit(request).unwrap();
    assert_eq!(j.state().unwrap().operations, 1);
    j.reserve(upload.operation, "reserved_google_id").unwrap();
    let version = j.state().unwrap().revision;
    j.reserve(upload.operation, "reserved_google_id").unwrap();
    assert_eq!(j.state().unwrap().revision, version);
    assert!(matches!(
        j.reserve(upload.operation, "replacement_id"),
        Err(Error::Identity)
    ));
    drop(j);
    let mut j = Journal::open(&path, binding()).unwrap();
    let recovered = j.next_upload().unwrap().unwrap();
    assert_eq!(recovered.record, upload.record);
    assert_eq!(recovered.sha256, upload.sha256);
    assert_eq!(recovered.file_id.as_deref(), Some("reserved_google_id"));
    assert!(matches!(
        j.confirm(upload.operation, "reserved_google_id", "wrong"),
        Err(Error::Identity)
    ));
    assert_eq!(j.state().unwrap().queued, 1);
    j.confirm(upload.operation, "reserved_google_id", &upload.sha256)
        .unwrap();
    let version = j.state().unwrap().revision;
    j.confirm(upload.operation, "reserved_google_id", &upload.sha256)
        .unwrap();
    assert_eq!(j.state().unwrap().revision, version);
    assert!(j.next_upload().unwrap().is_none());
    drop(j);
    let mut other = binding();
    other.principal = "drive:different-owner".into();
    assert!(matches!(Journal::open(&path, other), Err(Error::Binding)));
    assert_eq!(
        Journal::open(&path, binding())
            .unwrap()
            .state()
            .unwrap()
            .queued,
        0
    );
}

#[test]
fn local_edits_preserve_optional_fields_and_only_reject_changed_targets() {
    let mut j = Journal::memory(binding()).unwrap();
    let mut root = op(10, &[], vec![setting("Light")]);
    root.changes[0]
        .extra
        .insert("future_options".into(), json!({"contrast":"preserve"}));
    import(&mut j, &root);
    let mut frozen = edit(&j, 20, vec![setting("Dark")]);
    assert!(matches!(
        j.edit(frozen.clone()),
        Err(Error::Record(shep_profile_core::Error::Upgrade))
    ));
    frozen.changes[0].extra = root.changes[0].extra.clone();
    import(
        &mut j,
        &op(
            11,
            &[10],
            vec![change(Action::Setting {
                key: SettingKey::UnifiedInbox,
                value: json!(true),
            })],
        ),
    );
    // Independent remote values do not invalidate the captured appearance edit.
    j.edit(frozen).unwrap();
    assert_eq!(value(&j, "setting:appearance").extra, root.changes[0].extra);
    assert_eq!(
        value(&j, "setting:unified_inbox").action,
        Action::Setting {
            key: SettingKey::UnifiedInbox,
            value: json!(true)
        }
    );
    let mut older = edit(&j, 21, vec![setting("System")]);
    older.changes[0].extra = root.changes[0].extra.clone();
    let mut remote = op(22, &[20], vec![setting("Light")]);
    remote.changes[0].extra = root.changes[0].extra.clone();
    import(&mut j, &remote);
    assert!(matches!(j.edit(older), Err(Error::Changed)));
    assert_eq!(
        value(&j, "setting:appearance").action,
        remote.changes[0].action
    );
}

#[test]
fn ready_work_is_indexed_and_private_files_do_not_weaken_other_database_ownership() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.sqlite");
    let j = Journal::open(&path, binding()).unwrap();
    let fixture = rusqlite::Connection::open(&path).unwrap();
    let plan=fixture.prepare("EXPLAIN QUERY PLAN SELECT raw FROM operations WHERE applied=0 AND remaining=0 ORDER BY seq LIMIT 1").unwrap().query_map([],|r|r.get::<_,String>(3)).unwrap().collect::<std::result::Result<Vec<_>,_>>().unwrap();
    assert!(
        plan.iter().any(|step| step.contains("ready_operations")),
        "{plan:?}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
    assert_eq!(j.state().unwrap().ready, 0);
    drop(j);
    fixture.execute_batch("PRAGMA user_version=99").unwrap();
    assert!(matches!(
        Journal::open(&path, binding()),
        Err(Error::Record(shep_profile_core::Error::Upgrade))
    ));
    let version: i64 = fixture
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .unwrap();
    assert_eq!(version, 99);
}

#[test]
fn failed_sql_application_rolls_back_record_heads_versions_and_counters() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.sqlite");
    let mut j = Journal::open(&path, binding()).unwrap();
    let fixture = rusqlite::Connection::open(&path).unwrap();
    fixture.execute_batch("CREATE TRIGGER deny_version BEFORE INSERT ON versions BEGIN SELECT RAISE(ABORT,'fixture failure'); END;").unwrap();
    let frozen = edit(&j, 10, vec![setting("Light")]);
    assert!(matches!(j.edit(frozen.clone()), Err(Error::Storage)));
    assert_eq!(
        (
            j.state().unwrap().operations,
            j.state().unwrap().queued,
            j.state().unwrap().fields
        ),
        (0, 0, 0)
    );
    assert!(j.next_upload().unwrap().is_none());
    fixture.execute_batch("DROP TRIGGER deny_version;").unwrap();
    j.edit(frozen).unwrap();
    assert_eq!(j.state().unwrap().operations, 1);
    let count: i64 = fixture
        .query_row("SELECT count(*) FROM heads", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1);
    let integrity: String = fixture
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

#[test]
fn independent_process_and_path_alias_cannot_own_the_same_journal() {
    if let Ok(path) = std::env::var("SHEP_HISTORY_LOCK_CHILD") {
        assert!(matches!(
            Journal::open(std::path::Path::new(&path), binding()),
            Err(Error::Owned)
        ));
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profile.sqlite");
    let _owner = Journal::open(&path, binding()).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "independent_process_and_path_alias_cannot_own_the_same_journal",
            "--nocapture",
        ])
        .env("SHEP_HISTORY_LOCK_CHILD", &path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    #[cfg(unix)]
    {
        let alias = dir.path().join("alias.sqlite");
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        assert!(matches!(
            Journal::open(&alias, binding()),
            Err(Error::Owned)
        ));
    }
}

#[test]
fn acknowledged_export_omits_pending_local_records_and_tracks_confirmation() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("history.sqlite");
    let mut history = Journal::open(&path, binding()).unwrap();
    // The export boundary requires a complete setup, even for this queue test.
    let mut source = Journal::memory(binding()).unwrap();
    for (number, complete) in [(690, false), (691, true)] {
        source
            .edit(edit(
                &source,
                number,
                vec![change(Action::ProfileSetup { complete })],
            ))
            .unwrap();
    }
    let source_revision = source.state().unwrap().revision;
    let mut baseline = 0;
    while let Some(record) = source.export_record(source_revision, baseline).unwrap() {
        baseline = record.position;
        history.import(record.record.as_bytes()).unwrap();
    }
    history
        .edit(edit(&history, 700, vec![setting("Dark")]))
        .unwrap();
    let original = op(
        701,
        &[700],
        vec![change(Action::Setting {
            key: SettingKey::Tooltips,
            value: json!(false),
        })],
    );
    import(&mut history, &original);
    let before = history.state().unwrap();
    let imported = history
        .export_acknowledged_record(before.revision, baseline)
        .unwrap()
        .unwrap();
    assert_eq!(imported.operation, id(701));
    assert_eq!(imported.record.as_bytes(), original.encode().unwrap());
    assert!(
        history
            .export_acknowledged_record(before.revision, imported.position)
            .unwrap()
            .is_none()
    );
    let upload = history.next_upload().unwrap().unwrap();
    history
        .reserve(upload.operation, "fixture-confirmed-file")
        .unwrap();
    assert!(matches!(
        history.export_acknowledged_record(before.revision, baseline),
        Err(Error::Changed)
    ));
    history
        .confirm(upload.operation, "fixture-confirmed-file", &upload.sha256)
        .unwrap();
    drop(history);
    let history = Journal::open(&path, binding()).unwrap();
    let revision = history.state().unwrap().revision;
    let first = history
        .export_acknowledged_record(revision, baseline)
        .unwrap()
        .unwrap();
    assert_eq!(first.operation, id(700));
    assert_eq!(first.record, upload.record);
    let second = history
        .export_acknowledged_record(revision, first.position)
        .unwrap()
        .unwrap();
    assert_eq!(second.operation, id(701));
    assert!(
        history
            .export_acknowledged_record(revision, second.position)
            .unwrap()
            .is_none()
    );
    assert!(matches!(
        history.export_acknowledged_record(revision, u64::MAX),
        Err(Error::Changed)
    ));
    let db = rusqlite::Connection::open(&path).unwrap();
    let plans:Vec<String>=db.prepare("EXPLAIN QUERY PLAN SELECT seq,id,raw FROM operations WHERE seq>? AND (local=0 OR uploaded=1) ORDER BY seq LIMIT 1").unwrap().query_map([0],|row|row.get(3)).unwrap().collect::<rusqlite::Result<_>>().unwrap();
    assert!(
        plans
            .iter()
            .any(|plan| plan.contains("acknowledged_operations")),
        "{plans:?}"
    );
    assert!(!plans.iter().any(|plan| plan.contains("SCAN")), "{plans:?}");
}
