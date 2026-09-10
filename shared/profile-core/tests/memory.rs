//! The in-memory journal must derive the same state as the SQLite journal for
//! the same command sequence, and rebuild itself exactly from stored records.
#![cfg(feature = "history")]
use serde_json::json;
use shep_profile_core::{
    Action, Change, Operation, SettingKey,
    history::{
        memory::{MemoryJournal, StoredRecord},
        *,
    },
};
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
            "initialization-v1".into(),
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
fn record(op: &Operation) -> String {
    String::from_utf8(op.encode().unwrap()).unwrap()
}
fn stored(journal: &MemoryJournal, state: &State) -> Vec<StoredRecord> {
    let mut records = Vec::new();
    let mut after = 0;
    // Export walks in sequence order; unsent local edits are included.
    while let Some(next) = match journal
        .clone()
        .execute(Command::ExportRecord {
            expected_revision: state.revision,
            after,
        })
        .unwrap()
    {
        Reply::Record(record) => record,
        _ => unreachable!(),
    } {
        after = next.position;
        records.push(journal.record(next.operation).unwrap());
    }
    records
}
fn fixture_account() -> shep_profile_core::account::Connection {
    let fixture = Operation::decode(include_bytes!("../../profile-operation.json")).unwrap();
    fixture
        .changes
        .iter()
        .find_map(|change| match &change.action {
            Action::AccountConnection { account } => Some(account.clone()),
            _ => None,
        })
        .unwrap()
}

/// Every command runs on both journals; replies and observations must agree.
struct Pair {
    sqlite: Journal,
    memory: MemoryJournal,
}
impl Pair {
    fn new() -> Self {
        let sqlite = Journal::memory(binding()).unwrap();
        let memory = MemoryJournal::new(binding(), sqlite.state().unwrap().device).unwrap();
        Self { sqlite, memory }
    }
    fn run(&mut self, command: Command) -> Result<Reply> {
        let a = self.sqlite.execute(command.clone());
        let b = self.memory.execute(command);
        match (&a, &b) {
            (Ok(a), Ok(b)) => assert_eq!(
                serde_json::to_value(a).unwrap(),
                serde_json::to_value(b).unwrap()
            ),
            (Err(a), Err(b)) => assert_eq!(a.kind(), b.kind()),
            _ => panic!("journals disagree: {a:?} versus {b:?}"),
        }
        self.observe();
        b
    }
    fn observe(&self) {
        assert_eq!(self.sqlite.state().unwrap(), self.memory.state());
        assert_eq!(
            self.sqlite.overview().unwrap(),
            self.memory.overview().unwrap()
        );
        let mut after = None;
        loop {
            let fields = self.sqlite.fields(after.as_deref()).unwrap();
            assert_eq!(fields, self.memory.fields(after.as_deref()));
            for field in &fields {
                let versions = self.sqlite.versions(&field.target, None).unwrap();
                assert_eq!(versions, self.memory.versions(&field.target, None));
                for version in versions {
                    assert_eq!(
                        self.sqlite.value(&field.target, version.operation).unwrap(),
                        self.memory.value(&field.target, version.operation).unwrap()
                    );
                }
            }
            if fields.len() < PAGE_SIZE {
                break;
            }
            after = fields.last().map(|f| f.target.clone());
        }
        assert_eq!(
            self.sqlite.next_upload().unwrap(),
            self.memory.next_upload().unwrap()
        );
    }
    fn import(&mut self, op: &Operation) -> Result<Reply> {
        self.run(Command::Import { record: record(op) })
    }
    fn edit(
        &mut self,
        n: u128,
        changes: Vec<Change>,
        resolutions: Vec<Resolution>,
    ) -> Result<Reply> {
        let expected_revision = self.memory.state().revision;
        self.run(Command::Edit {
            edit: LocalEdit {
                operation: id(n),
                expected_revision,
                changes,
                resolutions,
            },
        })
    }
}

#[test]
fn memory_journal_matches_sqlite_across_merge_conflicts_tombstones_and_uploads() {
    let account = fixture_account();
    let mut pair = Pair::new();
    let root = op(
        10,
        &[],
        vec![change(Action::ProfileSetup { complete: false })],
    );
    // The setup root belongs to the fixture device, not this journal's device.
    pair.import(&root).unwrap();
    assert!(matches!(
        pair.edit(50, vec![setting("Dark")], vec![]),
        Err(Error::Incomplete)
    ));
    let content = op(
        11,
        &[10],
        vec![
            change(Action::ProfileName {
                name: "Work".into(),
            }),
            change(Action::AccountConnection {
                account: account.clone(),
            }),
            setting("Dark"),
        ],
    );
    let done = op(
        12,
        &[11],
        vec![change(Action::ProfileSetup { complete: true })],
    );
    pair.import(&content).unwrap();
    // Arrival before its parent waits, then drains in order.
    let late = op(
        14,
        &[13],
        vec![change(Action::AccountName {
            id: account.id,
            name: "Renamed".into(),
        })],
    );
    pair.import(&late).unwrap();
    assert_eq!(pair.memory.state().waiting, 1);
    pair.import(&done).unwrap();
    assert!(!pair.memory.state().initialized);
    let mid = op(13, &[12], vec![setting("Light")]);
    pair.import(&mid).unwrap();
    assert!(pair.memory.state().initialized);
    // Concurrent versions of the same field are an explicit conflict.
    let other = op(15, &[12], vec![setting("System")]);
    pair.import(&other).unwrap();
    assert_eq!(pair.memory.state().conflicts, 1);
    assert!(matches!(
        pair.edit(20, vec![setting("Dark")], vec![]),
        Err(Error::Conflict)
    ));
    let resolution = Resolution {
        target: "setting:appearance".into(),
        versions: vec![id(13), id(15)],
    };
    pair.edit(20, vec![setting("Dark")], vec![resolution.clone()])
        .unwrap();
    assert_eq!(pair.memory.state().conflicts, 0);
    assert_eq!(pair.memory.state().queued, 1);
    // A lost reply retries the identical request; a changed one is rejected.
    let revision = pair.memory.state().revision;
    let same = LocalEdit {
        operation: id(20),
        expected_revision: revision - 2,
        changes: vec![setting("Dark")],
        resolutions: vec![resolution.clone()],
    };
    pair.run(Command::Edit { edit: same.clone() }).unwrap();
    assert_eq!(pair.memory.state().revision, revision);
    let mut different = same;
    different.changes = vec![setting("Light")];
    assert!(matches!(
        pair.run(Command::Edit { edit: different }),
        Err(Error::Identity)
    ));
    // Upload reservation and confirmation.
    let upload = match pair.run(Command::NextUpload).unwrap() {
        Reply::Upload(Some(upload)) => upload,
        other => panic!("{other:?}"),
    };
    assert_eq!(upload.operation, id(20));
    assert!(matches!(
        pair.run(Command::Confirm {
            operation: id(20),
            file_id: "file-20".into(),
            sha256: upload.sha256.clone(),
        }),
        Err(Error::Identity)
    ));
    pair.run(Command::Reserve {
        operation: id(20),
        file_id: "file-20".into(),
    })
    .unwrap();
    assert!(matches!(
        pair.run(Command::Reserve {
            operation: id(20),
            file_id: "other".into(),
        }),
        Err(Error::Identity)
    ));
    pair.run(Command::Confirm {
        operation: id(20),
        file_id: "file-20".into(),
        sha256: upload.sha256,
    })
    .unwrap();
    assert_eq!(pair.memory.state().queued, 0);
    assert!(matches!(
        pair.run(Command::NextUpload).unwrap(),
        Reply::Upload(None)
    ));
    // Tombstones hide the account's other fields and reject later edits.
    let removal = op(
        16,
        &[13, 15],
        vec![change(Action::AccountRemoved { id: account.id })],
    );
    pair.import(&removal).unwrap();
    assert_eq!(pair.memory.overview().unwrap().accounts, 0);
    assert!(matches!(
        pair.edit(
            21,
            vec![change(Action::AccountName {
                id: account.id,
                name: "Back".into(),
            })],
            vec![]
        ),
        Err(Error::Removed)
    ));
    // Stale export revisions and acknowledged exports.
    assert!(matches!(
        pair.run(Command::ExportRecord {
            expected_revision: 1,
            after: 0,
        }),
        Err(Error::Changed)
    ));
    let revision = pair.memory.state().revision;
    let mut count = 0;
    let mut after = 0;
    while let Reply::Record(Some(next)) = pair
        .run(Command::ExportAcknowledgedRecord {
            expected_revision: revision,
            after,
        })
        .unwrap()
    {
        after = next.position;
        count += 1;
    }
    assert_eq!(count, 8);
    let cycle = op(30, &[31], vec![setting("Dark")]);
    let cycle_back = op(31, &[30], vec![setting("Light")]);
    pair.import(&cycle).unwrap();
    assert!(matches!(pair.import(&cycle_back), Err(Error::Cycle)));
    // Profile removal hides everything.
    let gone = op(17, &[16, 20], vec![change(Action::ProfileRemoved)]);
    pair.import(&gone).unwrap();
    assert!(pair.memory.state().removed);
    // Only the removal marker itself stays visible.
    assert_eq!(pair.memory.state().fields, 1);
    assert_eq!(pair.memory.overview().unwrap().settings, 0);
    assert!(matches!(
        pair.edit(22, vec![setting("Dark")], vec![]),
        Err(Error::Removed)
    ));
}

#[test]
fn memory_journal_restores_exactly_from_stored_records_and_rejects_foreign_local_records() {
    let account = fixture_account();
    let device = id(3);
    let mut journal = MemoryJournal::new(binding(), device).unwrap();
    let import = |journal: &mut MemoryJournal, op: &Operation| {
        journal.import(op.encode().unwrap().as_slice()).unwrap()
    };
    import(
        &mut journal,
        &op(
            10,
            &[],
            vec![change(Action::ProfileSetup { complete: false })],
        ),
    );
    import(
        &mut journal,
        &op(
            11,
            &[10],
            vec![change(Action::AccountConnection {
                account: account.clone(),
            })],
        ),
    );
    import(
        &mut journal,
        &op(
            12,
            &[11],
            vec![change(Action::ProfileSetup { complete: true })],
        ),
    );
    // Local edit from this device (id(3) authored ops 10..12 as well).
    journal
        .edit(LocalEdit {
            operation: id(20),
            expected_revision: journal.state().revision,
            changes: vec![setting("Dark")],
            resolutions: vec![],
        })
        .unwrap();
    journal.reserve(id(20), "file-20").unwrap();
    let upload = journal.next_upload().unwrap().unwrap();
    journal.confirm(id(20), "file-20", &upload.sha256).unwrap();
    journal
        .edit(LocalEdit {
            operation: id(21),
            expected_revision: journal.state().revision,
            changes: vec![change(Action::ProfileName {
                name: "Laptop".into(),
            })],
            resolutions: vec![],
        })
        .unwrap();
    let state = journal.state();
    let records = stored(&journal, &state);
    assert_eq!(records.len(), 5);
    assert_eq!(records[3].file_id.as_deref(), Some("file-20"));
    assert!(records[3].uploaded);
    assert!(records[4].request.is_some());
    assert!(!records[4].uploaded);
    let restored = MemoryJournal::restore(binding(), device, records.clone()).unwrap();
    let mut expected = state.clone();
    // Revision counts steps, and restore replays inserts and applies only.
    expected.revision = restored.state().revision;
    assert_eq!(restored.state(), expected);
    assert_eq!(restored.overview().unwrap().name.as_deref(), Some("Laptop"));
    assert_eq!(restored.next_upload().unwrap().unwrap().operation, id(21));
    assert_eq!(
        restored.record(id(20)).unwrap(),
        StoredRecord {
            seq: records[3].seq,
            ..records[3].clone()
        }
    );
    // Round trip through JSON, as the browser stores it.
    let text = serde_json::to_string(&records).unwrap();
    let parsed: Vec<StoredRecord> = serde_json::from_str(&text).unwrap();
    assert_eq!(parsed, records);
    // Another device cannot restore this device's local records as its own.
    assert!(matches!(
        MemoryJournal::restore(binding(), id(4), records.clone()),
        Err(Error::Binding)
    ));
    // Out-of-order sequences and changed bytes are refused.
    let mut reordered = records.clone();
    reordered.swap(0, 1);
    assert!(matches!(
        MemoryJournal::restore(binding(), device, reordered),
        Err(Error::Storage)
    ));
    let mut tampered = records;
    tampered[1].operation = id(99);
    assert!(matches!(
        MemoryJournal::restore(binding(), device, tampered),
        Err(Error::Identity)
    ));
}

#[test]
fn memory_journal_rejects_wrong_binding_and_reports_stable_error_kinds() {
    let mut journal = MemoryJournal::new(binding(), id(3)).unwrap();
    let mut foreign = op(10, &[], vec![setting("Dark")]);
    foreign.generation = id(77);
    assert!(matches!(
        journal.import(&foreign.encode().unwrap()),
        Err(Error::Binding)
    ));
    assert!(MemoryJournal::new(binding(), Uuid::nil()).is_err());
    assert_eq!(Error::Changed.kind(), "changed");
    assert_eq!(
        Error::Record(shep_profile_core::Error::Upgrade).kind(),
        "upgrade"
    );
    assert!(matches!(
        journal.import(b"not json"),
        Err(Error::Record(shep_profile_core::Error::Invalid))
    ));
}
