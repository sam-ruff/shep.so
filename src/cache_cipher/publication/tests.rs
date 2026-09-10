use super::*;
use crate::cache_cipher::migration::{Candidate, stage_plaintext};
use rusqlite::{Connection, config::DbConfig};

const MAIN: &str = "shep.sqlite";
const LOCK: &str = ".cache-encryption.lock";
const BODIES: [&str; 2] = [
    "Fictional checkpointed body",
    "Fictional body only in the write-ahead log",
];

/// A WAL plaintext cache whose newest row lives only in the uncheckpointed log.
fn fixture(dir: &Path) -> PathBuf {
    let main = dir.join(MAIN);
    let c = Connection::open(&main).unwrap();
    c.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA user_version=4; PRAGMA application_id=1397245264;
        CREATE TABLE messages(id INTEGER PRIMARY KEY, body TEXT);",
    )
    .unwrap();
    c.execute("INSERT INTO messages(body) VALUES(?)", [BODIES[0]])
        .unwrap();
    c.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
    c.set_db_config(DbConfig::SQLITE_DBCONFIG_NO_CKPT_ON_CLOSE, true)
        .unwrap();
    c.execute("INSERT INTO messages(body) VALUES(?)", [BODIES[1]])
        .unwrap();
    drop(c);
    assert!(fs::metadata(sidecar(&main, "-wal")).unwrap().len() > 0);
    main
}

fn stage(main: &Path, key: &Key) -> Candidate {
    let (_send, receive) = tokio::sync::oneshot::channel();
    stage_plaintext(main, main.parent().unwrap(), key, receive).unwrap()
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name != LOCK)
        .collect();
    names.sort();
    names
}

fn bodies(c: &Connection) -> Vec<String> {
    c.prepare("SELECT body FROM messages ORDER BY id")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn assert_encrypted_complete(dir: &Path, key: &Key) {
    assert_eq!(names(dir), [MAIN]);
    let main = dir.join(MAIN);
    assert!(!starts_with_plaintext_header(&main).unwrap());
    let c = key.open(&main, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(bodies(&c), BODIES);
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        4
    );
}

fn assert_plaintext_intact(dir: &Path) {
    let main = dir.join(MAIN);
    assert!(starts_with_plaintext_header(&main).unwrap());
    for name in names(dir) {
        assert!(
            [MAIN, "shep.sqlite-wal", "shep.sqlite-shm"].contains(&name.as_str()),
            "unexpected leftover {name}"
        );
    }
    let c = Connection::open_with_flags(&main, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(bodies(&c), BODIES);
}

#[test]
fn publication_replaces_plaintext_after_checkpoint_and_authenticated_read() {
    let dir = tempfile::tempdir().unwrap();
    let main = fixture(dir.path());
    let key = Key::generate().unwrap();
    let guard = Guard::migration(dir.path()).unwrap();
    let candidate = stage(&main, &key);
    publish(&guard, candidate, &main, &key).unwrap();
    assert_encrypted_complete(dir.path(), &key);
    let plain = Connection::open_with_flags(&main, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(
        plain
            .query_row("SELECT count(*) FROM sqlite_schema", [], |_| Ok(()))
            .is_err()
    );
    drop(plain);
    let report = recover(&guard, &main, &key).unwrap();
    assert_eq!(
        report,
        Report {
            outcome: Outcome::Idle,
            removed_orphans: 0,
            kept: Vec::new()
        }
    );
    assert_encrypted_complete(dir.path(), &key);
}

#[test]
fn publication_rejects_a_stale_candidate_and_succeeds_on_retry() {
    let dir = tempfile::tempdir().unwrap();
    let main = fixture(dir.path());
    let key = Key::generate().unwrap();
    let guard = Guard::migration(dir.path()).unwrap();
    let candidate = stage(&main, &key);
    let c = Connection::open(&main).unwrap();
    c.execute(
        "INSERT INTO messages(body) VALUES('Fictional row after staging')",
        [],
    )
    .unwrap();
    c.execute(
        "DELETE FROM messages WHERE body='Fictional row after staging'",
        [],
    )
    .unwrap();
    drop(c);
    let error = publish(&guard, candidate, &main, &key).unwrap_err();
    assert!(error.to_string().contains("changed after"), "{error}");
    assert_plaintext_intact(dir.path());
    let candidate = stage(&main, &key);
    publish(&guard, candidate, &main, &key).unwrap();
    assert_encrypted_complete(dir.path(), &key);
}

#[test]
fn publication_refuses_while_another_connection_holds_the_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let main = fixture(dir.path());
    let key = Key::generate().unwrap();
    let guard = Guard::migration(dir.path()).unwrap();
    let candidate = stage(&main, &key);
    let other = Connection::open(&main).unwrap();
    other
        .execute_batch("BEGIN; SELECT count(*) FROM messages;")
        .unwrap();
    let error = publish(&guard, candidate, &main, &key).unwrap_err();
    assert!(error.to_string().contains("still open"), "{error}");
    assert!(fs::metadata(sidecar(&main, "-wal")).unwrap().len() > 0);
    other.execute_batch("COMMIT").unwrap();
    drop(other);
    assert_plaintext_intact(dir.path());
    let candidate = stage(&main, &key);
    publish(&guard, candidate, &main, &key).unwrap();
    assert_encrypted_complete(dir.path(), &key);
}

#[test]
fn publication_requires_exclusive_ownership_of_the_data_root() {
    let dir = tempfile::tempdir().unwrap();
    let main = fixture(dir.path());
    let key = Key::generate().unwrap();
    let reader = Guard::reader(dir.path()).unwrap();
    assert!(Guard::migration(dir.path()).is_err());
    let elsewhere = tempfile::tempdir().unwrap();
    let foreign = Guard::migration(elsewhere.path()).unwrap();
    let candidate = stage(&main, &key);
    let error = publish(&foreign, candidate, &main, &key).unwrap_err();
    assert!(error.to_string().contains("outside the owned"), "{error}");
    assert!(recover(&foreign, &main, &key).is_err());
    assert!(clean_orphans(&foreign, dir.path()).is_err());
    assert_plaintext_intact(dir.path());
    drop(reader);
    let guard = Guard::migration(dir.path()).unwrap();
    let candidate = stage(&main, &key);
    publish(&guard, candidate, &main, &key).unwrap();
    assert_encrypted_complete(dir.path(), &key);
}

fn interrupted(step: Step) -> (tempfile::TempDir, PathBuf, Key, Guard) {
    let dir = tempfile::tempdir().unwrap();
    let main = fixture(dir.path());
    let key = Key::generate().unwrap();
    let guard = Guard::migration(dir.path()).unwrap();
    let candidate = stage(&main, &key);
    let error = publish_with(&guard, candidate, &main, &key, &|at| at == step).unwrap_err();
    assert!(
        error.to_string().contains("interrupted"),
        "{step:?}: {error}"
    );
    (dir, main, key, guard)
}

fn plaintext_copy(dir: &Path) -> Option<PathBuf> {
    [MAIN, "shep.sqlite.plaintext-recovery"]
        .into_iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists() && starts_with_plaintext_header(path).unwrap())
}

#[test]
fn interrupted_publication_resumes_or_rolls_back_deterministically_at_every_step() {
    for step in Step::ALL {
        let (dir, main, key, guard) = interrupted(step);
        let listing = names(dir.path());
        let journalled = listing.iter().any(|name| name.ends_with(JOURNAL_SUFFIX));
        assert_eq!(journalled, step > Step::Journal, "{step:?}: {listing:?}");
        if step <= Step::Retire {
            assert!(starts_with_plaintext_header(&main).unwrap(), "{step:?}");
        }
        if step < Step::Finish {
            let copy =
                plaintext_copy(dir.path()).unwrap_or_else(|| panic!("{step:?}: {listing:?}"));
            let c = Connection::open_with_flags(&copy, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            assert_eq!(bodies(&c), BODIES, "{step:?}");
        }
        let report = recover(&guard, &main, &key).unwrap();
        let expected = if step <= Step::Journal {
            Outcome::Idle
        } else {
            Outcome::Completed
        };
        assert_eq!(report.outcome, expected, "{step:?}: {listing:?}");
        assert_eq!(report.removed_orphans, 0, "{step:?}");
        assert!(report.kept.is_empty(), "{step:?}");
        if expected == Outcome::Idle {
            assert_plaintext_intact(dir.path());
        } else {
            assert_encrypted_complete(dir.path(), &key);
        }
        // A second recovery is a no-op, as on every later startup.
        assert_eq!(recover(&guard, &main, &key).unwrap().outcome, Outcome::Idle);
    }
}

#[test]
fn interrupted_publication_with_the_wrong_key_restores_the_plaintext() {
    for step in [Step::Retire, Step::Publish, Step::Verify, Step::Dispose] {
        let (dir, main, _key, guard) = interrupted(step);
        let wrong = Key::generate().unwrap();
        let report = recover(&guard, &main, &wrong).unwrap();
        assert_eq!(report.outcome, Outcome::RolledBack, "{step:?}");
        assert_plaintext_intact(dir.path());
        assert_eq!(names(dir.path()), [MAIN], "{step:?}");
        // The plaintext is complete, so the migration can simply be retried.
        let retry = Key::generate().unwrap();
        let candidate = stage(&main, &retry);
        publish(&guard, candidate, &main, &retry).unwrap();
        assert_encrypted_complete(dir.path(), &retry);
    }
    // Once the plaintext is disposed of, a wrong key must keep everything.
    let (dir, main, key, guard) = interrupted(Step::Finish);
    let wrong = Key::generate().unwrap();
    let error = recover(&guard, &main, &wrong).unwrap_err();
    assert!(
        error.to_string().contains("could not be unlocked"),
        "{error}"
    );
    assert_eq!(names(dir.path()), [MAIN, "shep.sqlite.encryption-journal"]);
    assert_eq!(
        recover(&guard, &main, &key).unwrap().outcome,
        Outcome::Completed
    );
    assert_encrypted_complete(dir.path(), &key);
}

#[test]
fn recovery_restores_the_plaintext_when_a_journalled_candidate_is_lost() {
    for step in [Step::Retire, Step::Publish] {
        let (dir, main, key, guard) = interrupted(step);
        let journal =
            fs::read_to_string(dir.path().join("shep.sqlite.encryption-journal")).unwrap();
        let journal: Journal = serde_json::from_str(&journal).unwrap();
        fs::remove_file(dir.path().join(&journal.candidate)).unwrap();
        let report = recover(&guard, &main, &key).unwrap();
        assert_eq!(report.outcome, Outcome::RolledBack, "{step:?}");
        assert_plaintext_intact(dir.path());
        assert_eq!(names(dir.path()), [MAIN], "{step:?}");
    }
}

#[test]
fn recovery_refuses_states_it_did_not_create_and_keeps_every_file() {
    let (dir, main, key, guard) = interrupted(Step::Publish);
    // Candidate, retired plaintext and a main file together are ambiguous.
    fs::write(&main, b"unrelated").unwrap();
    let before = names(dir.path());
    let error = recover(&guard, &main, &key).unwrap_err();
    assert!(error.to_string().contains("did not create"), "{error}");
    assert_eq!(names(dir.path()), before);
    fs::remove_file(&main).unwrap();
    assert_eq!(
        recover(&guard, &main, &key).unwrap().outcome,
        Outcome::Completed
    );
    assert_encrypted_complete(dir.path(), &key);

    // A recovery file with no journal is not ours to delete.
    let dir = tempfile::tempdir().unwrap();
    let main = fixture(dir.path());
    let stray = dir.path().join("shep.sqlite.plaintext-recovery");
    fs::write(&stray, b"older plaintext").unwrap();
    let guard = Guard::migration(dir.path()).unwrap();
    let report = recover(&guard, &main, &key).unwrap();
    assert_eq!(report.outcome, Outcome::Idle);
    assert_eq!(report.kept, [stray.canonicalize().unwrap()]);
    assert!(stray.exists());
    let candidate = stage(&main, &key);
    let error = publish(&guard, candidate, &main, &key).unwrap_err();
    assert!(error.to_string().contains("recovery file"), "{error}");
    assert_eq!(fs::read(&stray).unwrap(), b"older plaintext");
}

#[test]
fn orphan_cleanup_keeps_journalled_candidates_and_foreign_files() {
    let (dir, main, key, guard) = interrupted(Step::Retire);
    let journal = fs::read_to_string(dir.path().join("shep.sqlite.encryption-journal")).unwrap();
    let journal: Journal = serde_json::from_str(&journal).unwrap();
    let live = dir.path().join(&journal.candidate);
    let stale = dir.path().join(".shep-encrypted-stale.partial");
    fs::write(&stale, b"").unwrap();
    fs::write(sidecar(&stale, "-journal"), b"").unwrap();
    let scratch = dir.path().join(".shep-cache-scratch-old");
    fs::create_dir(&scratch).unwrap();
    fs::write(scratch.join("scratch.sqlite"), b"").unwrap();
    let import = dir.path().join(".shep-import-held.sqlite");
    fs::write(&import, b"").unwrap();
    assert_eq!(clean_orphans(&guard, dir.path()).unwrap(), 3);
    assert!(live.exists());
    assert!(import.exists());
    assert!(!stale.exists() && !scratch.exists());
    assert_eq!(clean_orphans(&guard, dir.path()).unwrap(), 0);
    let report = recover(&guard, &main, &key).unwrap();
    assert_eq!(report.outcome, Outcome::Completed);
    fs::remove_file(&import).unwrap();
    assert_encrypted_complete(dir.path(), &key);
    assert_eq!(
        candidate_base(".shep-encrypted-a.partial-wal"),
        Some(".shep-encrypted-a.partial")
    );
    assert_eq!(candidate_base(".shep-encrypted-a.partial.bak"), None);
    assert_eq!(candidate_base("shep.sqlite"), None);
}

#[test]
fn recovery_of_a_fresh_or_delete_mode_cache_is_idle() {
    let dir = tempfile::tempdir().unwrap();
    let key = Key::generate().unwrap();
    let guard = Guard::migration(dir.path()).unwrap();
    let main = dir.path().join(MAIN);
    assert_eq!(recover(&guard, &main, &key).unwrap().outcome, Outcome::Idle);
    let c = Connection::open(&main).unwrap();
    c.execute_batch("CREATE TABLE messages(id INTEGER PRIMARY KEY, body TEXT);")
        .unwrap();
    for body in BODIES {
        c.execute("INSERT INTO messages(body) VALUES(?)", [body])
            .unwrap();
    }
    drop(c);
    assert!(!sidecar(&main, "-wal").exists());
    let candidate = stage(&main, &key);
    publish(&guard, candidate, &main, &key).unwrap();
    let c = key.open(&main, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(bodies(&c), BODIES);
    assert_eq!(names(dir.path()), [MAIN]);
}
