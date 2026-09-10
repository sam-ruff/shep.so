#![cfg(feature = "history")]
use shep_profile_core::{Action, Change, Operation, history::*};
use uuid::Uuid;
fn binding() -> Binding {
    Binding {
        namespace: "so.shep.fixture".into(),
        principal: "drive:fixture-owner".into(),
        profile: Uuid::from_u128(1),
        generation: Uuid::from_u128(2),
    }
}
fn edit(journal: &mut Journal, action: Action) -> String {
    let operation = Uuid::new_v4();
    journal
        .edit(LocalEdit {
            operation,
            expected_revision: journal.state().unwrap().revision,
            changes: vec![Change {
                action,
                extra: Default::default(),
            }],
            resolutions: vec![],
        })
        .unwrap();
    let upload = journal.next_upload().unwrap().unwrap();
    assert_eq!(upload.operation, operation);
    journal.reserve(operation, &operation.to_string()).unwrap();
    journal
        .confirm(operation, &operation.to_string(), &upload.sha256)
        .unwrap();
    upload.record
}
#[test]
fn setup_is_not_enrollable_until_the_complete_causal_snapshot_arrives() {
    let mut source = Journal::memory(binding()).unwrap();
    let root = edit(&mut source, Action::ProfileSetup { complete: false });
    let middle = edit(
        &mut source,
        Action::ProfileName {
            name: "Work".into(),
        },
    );
    assert!(!source.state().unwrap().initialized);
    let complete = edit(&mut source, Action::ProfileSetup { complete: true });
    assert!(source.state().unwrap().initialized);
    for record in [&root, &middle, &complete] {
        assert!(
            Operation::decode(record.as_bytes())
                .unwrap()
                .requires
                .contains(&"initialization-v1".into())
        );
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("new-device.sqlite");
    let mut target = Journal::open(&path, binding()).unwrap();
    target.import(complete.as_bytes()).unwrap();
    target.import(root.as_bytes()).unwrap();
    assert!(!target.state().unwrap().initialized);
    assert_eq!(target.state().unwrap().waiting, 1);
    let device = target.state().unwrap().device;
    drop(target);
    let mut target = Journal::open(&path, binding()).unwrap();
    target.import(middle.as_bytes()).unwrap();
    assert!(target.state().unwrap().initialized);
    assert_eq!(target.state().unwrap().device, device);
    assert_ne!(device, source.state().unwrap().device);
    assert_eq!(target.overview().unwrap().name.as_deref(), Some("Work"));
    let delete = edit(&mut source, Action::ProfileRemoved);
    target.import(delete.as_bytes()).unwrap();
    assert!(!target.state().unwrap().initialized);
}
#[test]
fn foreign_completion_and_reset_cannot_turn_partial_history_into_initialized_setup() {
    let mut source = Journal::memory(binding()).unwrap();
    let root = edit(&mut source, Action::ProfileSetup { complete: false });
    let complete = edit(&mut source, Action::ProfileSetup { complete: true });
    let mut target = Journal::memory(binding()).unwrap();
    target.import(root.as_bytes()).unwrap();
    let mut foreign = Operation::decode(complete.as_bytes()).unwrap();
    foreign.device = Uuid::new_v4();
    assert!(target.import(&foreign.encode().unwrap()).is_err());
    assert!(!target.state().unwrap().initialized);
    assert_eq!(target.state().unwrap().operations, 1);
    assert!(matches!(
        target.edit(LocalEdit {
            operation: Uuid::new_v4(),
            expected_revision: target.state().unwrap().revision,
            changes: vec![Change {
                action: Action::ProfileName {
                    name: "Other device".into()
                },
                extra: Default::default()
            }],
            resolutions: vec![]
        }),
        Err(Error::Incomplete)
    ));
    target.import(complete.as_bytes()).unwrap();
    let reset = LocalEdit {
        operation: Uuid::new_v4(),
        expected_revision: target.state().unwrap().revision,
        changes: vec![Change {
            action: Action::ProfileSetup { complete: false },
            extra: Default::default(),
        }],
        resolutions: vec![],
    };
    assert!(target.edit(reset).is_err());
    assert!(target.state().unwrap().initialized);
    let mut legacy = Journal::memory(binding()).unwrap();
    edit(
        &mut legacy,
        Action::ProfileName {
            name: "Legacy".into(),
        },
    );
    assert!(!legacy.state().unwrap().initialized);
}

#[test]
fn original_record_copy_is_revision_fenced_and_keeps_an_independent_device() {
    let mut source = Journal::memory(binding()).unwrap();
    assert!(source.export_record(0, 0).is_err());
    let root = edit(&mut source, Action::ProfileSetup { complete: false });
    assert!(
        source
            .export_record(source.state().unwrap().revision, 0)
            .is_err()
    );
    let name = edit(
        &mut source,
        Action::ProfileName {
            name: "Shared work".into(),
        },
    );
    let complete = edit(&mut source, Action::ProfileSetup { complete: true });
    let revision = source.state().unwrap().revision;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("independent.sqlite");
    let mut target = Journal::open(&path, binding()).unwrap();
    let device = target.state().unwrap().device;
    let mut cursor = 0;
    for original in [&root, &name, &complete] {
        let record = source.export_record(revision, cursor).unwrap().unwrap();
        assert!(record.position > cursor);
        assert_eq!(&record.record, original);
        assert!(
            !serde_json::to_value(&record)
                .unwrap()
                .as_object()
                .unwrap()
                .contains_key("device")
        );
        target.import(record.record.as_bytes()).unwrap();
        cursor = record.position;
    }
    assert!(source.export_record(revision, cursor).unwrap().is_none());
    assert!(source.export_record(revision, u64::MAX).is_err());
    assert!(target.state().unwrap().initialized);
    assert_eq!(target.state().unwrap().queued, 0);
    assert_ne!(device, source.state().unwrap().device);
    drop(target);
    let target = Journal::open(&path, binding()).unwrap();
    assert_eq!(target.state().unwrap().device, device);
    edit(
        &mut source,
        Action::ProfileName {
            name: "Changed while copying".into(),
        },
    );
    assert!(matches!(
        source.export_record(revision, cursor),
        Err(Error::Changed)
    ));
    assert_eq!(
        target.overview().unwrap().name.as_deref(),
        Some("Shared work")
    );
}
