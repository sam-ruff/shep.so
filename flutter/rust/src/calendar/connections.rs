use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use shep_calendar_core::caldav::{CalDavConnection, validate_url};
use shep_calendar_core::http::CalendarProvider;

const ACTIVE_LIMIT: i64 = 32;
const HISTORY_LIMIT: i64 = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionRequest {
    pub connection: CalDavConnection,
    pub credential_slot: String,
    #[serde(default)]
    pub observed_revision: Option<i64>,
    #[serde(default)]
    pub observed_credential_slot: Option<String>,
}

impl ConnectionRequest {
    fn validate(&self, attempt: &str) -> Result<()> {
        anyhow::ensure!(
            !attempt.is_empty() && attempt.len() <= 255,
            "This calendar setup identity is invalid."
        );
        anyhow::ensure!(
            self.connection.is_bounded(),
            "This CalDAV connection is invalid."
        );
        validate_url(&self.connection.url)?;
        anyhow::ensure!(
            self.credential_slot == format!("calendar-{attempt}"),
            "The credential slot does not belong to this setup attempt."
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionAttempt {
    pub id: String,
    pub request: ConnectionRequest,
    pub status: String,
    pub error: Option<String>,
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveConnection {
    pub connection: CalDavConnection,
    pub credential_slot: String,
    pub revision: i64,
}

fn decode_attempt(
    id: String,
    request: String,
    status: String,
    error: Option<String>,
    created: i64,
) -> Result<ConnectionAttempt> {
    Ok(ConnectionAttempt {
        id,
        request: serde_json::from_str(&request)?,
        status,
        error,
        created,
    })
}

pub fn attempt(db: &Connection, id: &str) -> Result<Option<ConnectionAttempt>> {
    let saved = db
        .query_row(
            "SELECT request,status,error,created FROM calendar_connection_attempts WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    saved
        .map(|(request, status, error, created)| {
            decode_attempt(id.to_owned(), request, status, error, created)
        })
        .transpose()
}

pub fn prepare(
    db: &mut Connection,
    id: &str,
    request: &ConnectionRequest,
) -> Result<ConnectionAttempt> {
    request.validate(id)?;
    let tx = db.transaction()?;
    if let Some(saved) = attempt(&tx, id)? {
        anyhow::ensure!(
            saved.request == *request,
            "This calendar setup identity belongs to another request."
        );
        tx.commit()?;
        return Ok(saved);
    }
    let removed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM removed_calendar_connections WHERE id=?1)",
        [&request.connection.id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        !removed,
        "This calendar connection was removed. Add it with a new identity."
    );
    let current = tx
        .query_row(
            "SELECT revision,credential_slot FROM calendar_connections WHERE id=?1",
            [&request.connection.id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    anyhow::ensure!(
        current
            == request
                .observed_revision
                .zip(request.observed_credential_slot.clone()),
        "This calendar connection changed. Review it before reconnecting."
    );
    let count: i64 = tx.query_row(
        "SELECT count(*) FROM calendar_connection_attempts WHERE status IN ('prepared','probing','waiting')",
        [],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        count < ACTIVE_LIMIT,
        "Calendar connections are catching up. Finish or cancel another setup first."
    );
    let created = chrono::Utc::now().timestamp_millis();
    tx.execute(
        "INSERT INTO calendar_connection_attempts(id,connection_id,request,status,error,created) VALUES(?1,?2,?3,'prepared',NULL,?4)",
        params![
            id,
            request.connection.id,
            serde_json::to_string(request)?,
            created
        ],
    )?;
    tx.commit()?;
    Ok(ConnectionAttempt {
        id: id.to_owned(),
        request: request.clone(),
        status: "prepared".into(),
        error: None,
        created,
    })
}

pub fn attempts(db: &Connection) -> Result<Vec<ConnectionAttempt>> {
    let mut statement = db.prepare(
        "SELECT id,request,status,error,created FROM calendar_connection_attempts WHERE status!='cancelled' ORDER BY created DESC,id LIMIT ?1",
    )?;
    statement
        .query_map([HISTORY_LIMIT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .map(|row| {
            let (id, request, status, error, created) = row?;
            decode_attempt(id, request, status, error, created)
        })
        .collect()
}

pub fn pending_attempts(db: &Connection) -> Result<Vec<ConnectionAttempt>> {
    let mut statement = db.prepare(
        "SELECT id,request,status,error,created FROM calendar_connection_attempts INDEXED BY calendar_connection_attempt_status WHERE status IN ('prepared','probing','waiting') ORDER BY status,created,id LIMIT 32",
    )?;
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?
        .map(|row| {
            let (id, request, status, error, created) = row?;
            decode_attempt(id, request, status, error, created)
        })
        .collect()
}

pub fn claim_probe(db: &mut Connection, id: &str) -> Result<Option<ConnectionRequest>> {
    let tx = db.transaction()?;
    let saved = attempt(&tx, id)?.context("This calendar setup no longer exists.")?;
    match saved.status.as_str() {
        "active" => {
            tx.commit()?;
            return Ok(None);
        }
        "probing" => anyhow::bail!("This calendar setup is already being checked."),
        "prepared" | "waiting" => {}
        _ => anyhow::bail!("This calendar setup was cancelled."),
    }
    let changed = tx.execute(
        "UPDATE calendar_connection_attempts SET status='probing',error=NULL WHERE id=?1 AND status IN ('prepared','waiting')",
        [id],
    )?;
    anyhow::ensure!(changed == 1, "This calendar setup changed.");
    tx.commit()?;
    Ok(Some(saved.request))
}

pub fn wait(db: &Connection, id: &str, error: &str) -> Result<()> {
    anyhow::ensure!(error.len() <= 4096, "The connection error is too long.");
    let changed = db.execute(
        "UPDATE calendar_connection_attempts SET status='waiting',error=?2 WHERE id=?1 AND status IN ('prepared','probing','waiting')",
        params![id, error],
    )?;
    anyhow::ensure!(
        changed == 1,
        "This calendar setup changed while it was checked."
    );
    Ok(())
}

pub fn restart(db: &Connection) -> Result<()> {
    db.execute(
        "UPDATE calendar_connection_attempts SET status='waiting',error='The app closed while checking this calendar. Retry with its saved credentials.' WHERE status='probing'",
        [],
    )?;
    Ok(())
}

pub fn activate(
    db: &mut Connection,
    id: &str,
    expected: &ConnectionRequest,
    sources: &[shep_calendar_core::Source],
    events: &[shep_calendar_core::Event],
) -> Result<Option<String>> {
    anyhow::ensure!(
        !sources.is_empty() && sources.len() <= 50 && events.len() <= 5000,
        "CalDAV returned an invalid sync window."
    );
    anyhow::ensure!(
        sources
            .iter()
            .all(|source| source.id == expected.connection.id)
            && events
                .iter()
                .all(|event| event.source_id == expected.connection.id),
        "CalDAV returned data for another connection."
    );
    let tx = db.transaction()?;
    let saved = attempt(&tx, id)?.context("This calendar setup no longer exists.")?;
    anyhow::ensure!(
        saved.status == "probing" && saved.request == *expected,
        "This calendar setup was cancelled or replaced while it was checked."
    );
    let removed: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM removed_calendar_connections WHERE id=?1)",
        [&expected.connection.id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(!removed, "This calendar connection was removed.");
    let current = tx
        .query_row(
            "SELECT revision,credential_slot FROM calendar_connections WHERE id=?1",
            [&expected.connection.id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()?;
    anyhow::ensure!(
        current
            == expected
                .observed_revision
                .zip(expected.observed_credential_slot.clone()),
        "This calendar connection changed while the replacement was checked."
    );
    if current.is_some() {
        let has_actions: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM calendar_intents i JOIN calendar_sources s ON s.id=i.source_id WHERE s.connection_id=?1)",
            [&expected.connection.id],
            |row| row.get(0),
        )?;
        anyhow::ensure!(
            !has_actions,
            "Finish or cancel pending event changes before reconnecting this calendar."
        );
    }
    let prior_slot: Option<String> = tx
        .query_row(
            "SELECT credential_slot FROM calendar_connections WHERE id=?1",
            [&expected.connection.id],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    tx.execute(
        "INSERT INTO calendar_connections(id,config,credential_slot,revision) VALUES(?1,?2,?3,1) ON CONFLICT(id) DO UPDATE SET config=excluded.config,credential_slot=excluded.credential_slot,revision=calendar_connections.revision+1",
        params![
            expected.connection.id,
            serde_json::to_string(&expected.connection)?,
            expected.credential_slot
        ],
    )?;
    tx.execute(
        "DELETE FROM calendar_events WHERE source_id IN (SELECT id FROM calendar_sources WHERE connection_id=?1)",
        [&expected.connection.id],
    )?;
    tx.execute(
        "DELETE FROM calendar_sources WHERE connection_id=?1",
        [&expected.connection.id],
    )?;
    for source in sources {
        tx.execute(
            "INSERT INTO calendar_sources(id,connection_id,source) VALUES(?1,?2,?3)",
            params![
                source.id,
                expected.connection.id,
                serde_json::to_string(source)?
            ],
        )?;
    }
    for event in events {
        tx.execute(
            "INSERT INTO calendar_events(source_id,id,event) VALUES(?1,?2,?3)",
            params![event.source_id, event.id, serde_json::to_string(event)?],
        )?;
    }
    tx.execute(
        "UPDATE calendar_connection_attempts SET status='active',error=NULL WHERE id=?1 AND status='probing'",
        [id],
    )?;
    if let Some(slot) = prior_slot
        .as_deref()
        .filter(|slot| *slot != expected.credential_slot)
    {
        tx.execute(
            "INSERT OR IGNORE INTO calendar_credential_cleanup(slot) VALUES(?1)",
            [slot],
        )?;
    }
    tx.execute(
        "UPDATE calendar_clock SET revision=revision+1 WHERE id=1",
        [],
    )?;
    tx.commit()?;
    Ok(prior_slot.filter(|slot| slot != &expected.credential_slot))
}

pub fn cancel(db: &mut Connection, id: &str) -> Result<String> {
    let tx = db.transaction()?;
    let saved = attempt(&tx, id)?.context("This calendar setup no longer exists.")?;
    anyhow::ensure!(
        matches!(saved.status.as_str(), "prepared" | "probing" | "waiting"),
        "This calendar setup can no longer be cancelled."
    );
    let changed = tx.execute(
        "UPDATE calendar_connection_attempts SET status='cancelled',error=NULL WHERE id=?1 AND status IN ('prepared','probing','waiting')",
        [id],
    )?;
    anyhow::ensure!(changed == 1, "This calendar setup changed.");
    tx.execute(
        "INSERT OR IGNORE INTO calendar_credential_cleanup(slot) VALUES(?1)",
        [&saved.request.credential_slot],
    )?;
    tx.commit()?;
    Ok(saved.request.credential_slot)
}

pub fn active(db: &Connection, id: &str) -> Result<ActiveConnection> {
    db.query_row(
        "SELECT config,credential_slot,revision FROM calendar_connections WHERE id=?1",
        [id],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )
    .map_err(Into::into)
    .and_then(|(config, credential_slot, revision)| {
        Ok(ActiveConnection {
            connection: serde_json::from_str(&config)?,
            credential_slot,
            revision,
        })
    })
}

pub fn active_connections(db: &Connection) -> Result<Vec<ActiveConnection>> {
    let mut statement = db.prepare(
        "SELECT config,credential_slot,revision FROM calendar_connections ORDER BY id LIMIT 50",
    )?;
    statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .map(|row| {
            let (config, credential_slot, revision) = row?;
            Ok(ActiveConnection {
                connection: serde_json::from_str(&config)?,
                credential_slot,
                revision,
            })
        })
        .collect()
}

pub fn remove(db: &mut Connection, id: &str, revision: i64) -> Result<Option<String>> {
    let tx = db.transaction()?;
    let connection = active(&tx, id)?;
    anyhow::ensure!(
        connection.revision == revision,
        "This calendar connection changed. Review it again before removal."
    );
    let has_actions: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM calendar_intents i JOIN calendar_sources s ON s.id=i.source_id WHERE s.connection_id=?1)",
        [id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        !has_actions,
        "Finish or cancel this calendar's pending changes before removing it."
    );
    tx.execute(
        "INSERT INTO removed_calendar_connections(id,revision,credential_slot,cleanup) VALUES(?1,?2,?3,1) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,credential_slot=excluded.credential_slot,cleanup=1",
        params![id, revision, connection.credential_slot],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO calendar_credential_cleanup(slot) VALUES(?1)",
        [&connection.credential_slot],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO calendar_credential_cleanup(slot) SELECT json_extract(request,'$.credential_slot') FROM calendar_connection_attempts WHERE connection_id=?1 AND status IN ('prepared','probing','waiting')",
        [id],
    )?;
    tx.execute(
        "UPDATE calendar_connection_attempts SET status='cancelled',error=NULL WHERE connection_id=?1 AND status IN ('prepared','probing','waiting')",
        [id],
    )?;
    tx.execute("DELETE FROM calendar_connections WHERE id=?1", [id])?;
    tx.execute(
        "UPDATE calendar_clock SET revision=revision+1 WHERE id=1",
        [],
    )?;
    tx.commit()?;
    Ok(Some(connection.credential_slot))
}

pub fn cleanup_slots(db: &Connection) -> Result<Vec<String>> {
    let mut statement =
        db.prepare("SELECT slot FROM calendar_credential_cleanup ORDER BY slot LIMIT 50")?;
    Ok(statement
        .query_map([], |row| row.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn cleanup_done(db: &Connection, slot: &str) -> Result<()> {
    db.execute(
        "DELETE FROM calendar_credential_cleanup WHERE slot=?1",
        [slot],
    )?;
    Ok(())
}

pub async fn probe(
    db: &crate::database::Database,
    provider: &impl CalendarProvider,
    id: String,
    password: String,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
) -> Result<Option<String>> {
    anyhow::ensure!(
        end > start && end - start <= chrono::Duration::days(730),
        "Calendar sync must cover at most two years."
    );
    let lookup = id.clone();
    let Some(request) = db.write(move |db| claim_probe(db, &lookup)).await? else {
        return Ok(None);
    };
    let result = async {
        let sources = provider
            .sources(&password)
            .await
            .map_err(anyhow::Error::new)?;
        let mut events = Vec::new();
        for source in &sources {
            events.extend(
                provider
                    .events(&password, source, start, end)
                    .await
                    .map_err(anyhow::Error::new)?,
            );
            anyhow::ensure!(
                events.len() <= 5000,
                "This calendar window contains more than 5,000 events."
            );
        }
        Ok::<_, anyhow::Error>((sources, events))
    }
    .await;
    let (sources, events) = match result {
        Ok(value) => value,
        Err(error) => {
            let saved = id;
            let message = format!("{error:#}");
            db.write(move |db| wait(db, &saved, &message)).await?;
            return Err(error);
        }
    };
    db.write(move |db| activate(db, &id, &request, &sources, &events))
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockall::predicate::eq;
    use shep_calendar_core::http::MockCalendarProvider;

    fn db() -> Connection {
        let db = Connection::open_in_memory().expect("database");
        db.execute_batch("PRAGMA foreign_keys=ON;")
            .expect("foreign keys");
        db.execute_batch(include_str!("../schema.sql"))
            .expect("schema");
        db
    }

    fn connection_request(attempt: &str, connection: &str) -> ConnectionRequest {
        ConnectionRequest {
            connection: CalDavConnection {
                id: connection.into(),
                url: "https://calendar.example.test/home/".into(),
                username: "sam".into(),
            },
            credential_slot: format!("calendar-{attempt}"),
            observed_revision: None,
            observed_credential_slot: None,
        }
    }

    fn source(id: &str) -> shep_calendar_core::Source {
        shep_calendar_core::Source {
            id: id.into(),
            name: "Home".into(),
            read_only: false,
        }
    }

    #[test]
    fn exact_prepare_is_idempotent_and_duplicate_probe_is_fenced() {
        let mut db = db();
        let request = connection_request("attempt", "caldav-home");
        let first = prepare(&mut db, "attempt", &request).expect("prepare");
        let duplicate = prepare(&mut db, "attempt", &request).expect("lost reply");
        assert_eq!(first, duplicate);
        assert!(prepare(&mut db, "attempt", &connection_request("attempt", "other")).is_err());
        assert_eq!(
            claim_probe(&mut db, "attempt").expect("claim").as_ref(),
            Some(&request)
        );
        assert!(claim_probe(&mut db, "attempt").is_err());
    }

    #[test]
    fn cancellation_during_probe_prevents_late_activation() {
        let mut db = db();
        let request = connection_request("attempt", "caldav-home");
        prepare(&mut db, "attempt", &request).expect("prepare");
        claim_probe(&mut db, "attempt").expect("claim");
        assert_eq!(
            cancel(&mut db, "attempt").expect("cancel"),
            "calendar-attempt"
        );
        assert!(activate(&mut db, "attempt", &request, &[source("caldav-home")], &[]).is_err());
        assert!(active(&db, "caldav-home").is_err());
    }

    #[test]
    fn checked_reconnect_replaces_slot_only_at_activation() {
        let mut db = db();
        let original = connection_request("first", "caldav-home");
        prepare(&mut db, "first", &original).expect("prepare");
        claim_probe(&mut db, "first").expect("claim");
        assert_eq!(
            activate(&mut db, "first", &original, &[source("caldav-home")], &[]).expect("activate"),
            None
        );
        let mut replacement = connection_request("second", "caldav-home");
        replacement.observed_revision = Some(1);
        replacement.observed_credential_slot = Some("calendar-first".into());
        prepare(&mut db, "second", &replacement).expect("prepare replacement");
        claim_probe(&mut db, "second").expect("claim replacement");
        let mut stale = connection_request("third", "caldav-home");
        stale.observed_revision = Some(1);
        stale.observed_credential_slot = Some("calendar-first".into());
        prepare(&mut db, "third", &stale).expect("prepare concurrent replacement");
        claim_probe(&mut db, "third").expect("claim concurrent replacement");
        assert_eq!(
            active(&db, "caldav-home")
                .expect("old connection retained")
                .credential_slot,
            "calendar-first"
        );
        assert_eq!(
            activate(
                &mut db,
                "second",
                &replacement,
                &[source("caldav-home")],
                &[]
            )
            .expect("activate replacement")
            .as_deref(),
            Some("calendar-first")
        );
        assert_eq!(
            active(&db, "caldav-home")
                .expect("replacement")
                .credential_slot,
            "calendar-second"
        );
        assert!(activate(&mut db, "third", &stale, &[source("caldav-home")], &[]).is_err());
        assert_eq!(
            cleanup_slots(&db).expect("durable cleanup"),
            ["calendar-first"]
        );
    }

    #[test]
    fn removal_rechecks_revision_and_pending_event_ownership() {
        let mut db = db();
        let request = connection_request("attempt", "caldav-home");
        prepare(&mut db, "attempt", &request).expect("prepare");
        claim_probe(&mut db, "attempt").expect("claim");
        activate(&mut db, "attempt", &request, &[source("caldav-home")], &[]).expect("activate");
        assert!(remove(&mut db, "caldav-home", 2).is_err());
        db.execute(
            "INSERT INTO calendar_actions(id,status,created,mutation) VALUES('action','queued',1,'{}')",
            [],
        )
        .expect("action");
        db.execute(
            "INSERT INTO calendar_intents(source_id,event_id,action) VALUES('caldav-home','event','action')",
            [],
        )
        .expect("intent");
        assert!(remove(&mut db, "caldav-home", 1).is_err());
        db.execute("DELETE FROM calendar_intents", [])
            .expect("clear intent");
        let mut pending = connection_request("replacement", "caldav-home");
        pending.observed_revision = Some(1);
        pending.observed_credential_slot = Some("calendar-attempt".into());
        prepare(&mut db, "replacement", &pending).expect("pending replacement");
        assert_eq!(
            remove(&mut db, "caldav-home", 1)
                .expect("remove")
                .as_deref(),
            Some("calendar-attempt")
        );
        assert_eq!(
            cleanup_slots(&db).expect("cleanup"),
            ["calendar-attempt", "calendar-replacement"]
        );
        cleanup_done(&db, "calendar-attempt").expect("cleaned");
        cleanup_done(&db, "calendar-replacement").expect("cleaned replacement");
        assert!(cleanup_slots(&db).expect("cleanup complete").is_empty());
    }

    #[test]
    fn reconnect_does_not_retarget_pending_event_actions() {
        let mut db = db();
        let original = connection_request("first", "caldav-home");
        prepare(&mut db, "first", &original).expect("prepare");
        claim_probe(&mut db, "first").expect("claim");
        activate(&mut db, "first", &original, &[source("caldav-home")], &[]).expect("activate");
        db.execute(
            "INSERT INTO calendar_actions(id,status,created,mutation) VALUES('action','queued',1,'{}')",
            [],
        )
        .expect("action");
        db.execute(
            "INSERT INTO calendar_intents(source_id,event_id,action) VALUES('caldav-home','event','action')",
            [],
        )
        .expect("intent");
        let mut replacement = connection_request("second", "caldav-home");
        replacement.observed_revision = Some(1);
        replacement.observed_credential_slot = Some("calendar-first".into());
        prepare(&mut db, "second", &replacement).expect("prepare replacement");
        claim_probe(&mut db, "second").expect("claim replacement");
        assert!(
            activate(
                &mut db,
                "second",
                &replacement,
                &[source("caldav-home")],
                &[]
            )
            .is_err()
        );
        assert_eq!(
            active(&db, "caldav-home")
                .expect("original retained")
                .credential_slot,
            "calendar-first"
        );
    }

    #[test]
    fn pending_attempts_are_not_buried_by_terminal_history() {
        let db = db();
        for number in 0..60 {
            let request = connection_request(&format!("old-{number}"), &format!("old-{number}"));
            db.execute(
                "INSERT INTO calendar_connection_attempts(id,connection_id,request,status,error,created) VALUES(?1,?2,?3,'active',NULL,?4)",
                params![
                    format!("old-{number}"),
                    request.connection.id,
                    serde_json::to_string(&request).expect("request"),
                    100 + number
                ],
            )
            .expect("history");
        }
        let pending = connection_request("pending", "pending");
        db.execute(
            "INSERT INTO calendar_connection_attempts(id,connection_id,request,status,error,created) VALUES('pending','pending',?1,'waiting','Unlock credentials',1)",
            [serde_json::to_string(&pending).expect("pending")],
        )
        .expect("pending row");
        assert!(
            attempts(&db)
                .expect("history page")
                .iter()
                .all(|attempt| attempt.id != "pending")
        );
        let runnable = pending_attempts(&db).expect("pending page");
        assert_eq!(runnable.len(), 1);
        assert_eq!(runnable[0].id, "pending");
    }

    #[test]
    fn restart_makes_an_interrupted_read_only_probe_retryable() {
        let mut db = db();
        let request = connection_request("first", "caldav-home");
        prepare(&mut db, "first", &request).expect("prepare");
        claim_probe(&mut db, "first").expect("claim");

        restart(&db).expect("restart");

        let saved = attempt(&db, "first").expect("read").expect("attempt");
        assert_eq!(saved.status, "waiting");
        assert!(saved.error.expect("reason").contains("closed"));
        claim_probe(&mut db, "first").expect("retry claim");
    }

    #[tokio::test]
    async fn probe_uses_provider_once_and_persists_waiting_failure() {
        let path = std::env::temp_dir().join(format!("shep-caldav-{}.db", uuid::Uuid::new_v4()));
        let database = crate::database::Database::open(path.to_string_lossy().into_owned())
            .await
            .expect("database");
        let request = connection_request("attempt", "caldav-home");
        database
            .write({
                let request = request.clone();
                move |db| prepare(db, "attempt", &request).map(|_| ())
            })
            .await
            .expect("prepare");
        let mut provider = MockCalendarProvider::new();
        provider
            .expect_sources()
            .with(eq("secret"))
            .times(1)
            .returning(|_| {
                Box::pin(async { Err(shep_calendar_core::ProviderFailure::waiting("offline")) })
            });
        provider.expect_events().times(0);
        let start = chrono::Utc::now();
        assert!(
            probe(
                &database,
                &provider,
                "attempt".into(),
                "secret".into(),
                start,
                start + chrono::Duration::days(1),
            )
            .await
            .is_err()
        );
        let saved = database
            .read(|db| attempt(db, "attempt"))
            .await
            .expect("attempt")
            .expect("saved");
        assert_eq!(saved.status, "waiting");
        assert_eq!(saved.error.as_deref(), Some("offline"));
        drop(database);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn schema_twenty_one_migrates_without_binding_legacy_google_rows() {
        let path =
            std::env::temp_dir().join(format!("shep-calendar21-{}.db", uuid::Uuid::new_v4()));
        {
            let db = Connection::open(&path).expect("legacy database");
            db.execute_batch(
                "PRAGMA user_version=21;
                 CREATE TABLE calendar_sources(id TEXT PRIMARY KEY,source TEXT NOT NULL);
                 CREATE TABLE calendar_actions(id TEXT PRIMARY KEY,status TEXT NOT NULL,error TEXT,created INTEGER NOT NULL,mutation TEXT NOT NULL,subject TEXT,admission_request TEXT);
                 INSERT INTO calendar_sources VALUES('primary','{}');
                 INSERT INTO calendar_actions VALUES('legacy','uncertain',NULL,1,'{}',NULL,NULL);",
            )
            .expect("legacy schema");
        }
        let database = crate::database::Database::open(path.to_string_lossy().into_owned())
            .await
            .expect("migrate");
        let state = database
            .read(|db| {
                Ok((
                    db.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?,
                    db.query_row(
                        "SELECT connection_id FROM calendar_sources WHERE id='primary'",
                        [],
                        |row| row.get::<_, Option<String>>(0),
                    )?,
                    db.query_row(
                        "SELECT connection_id,connection_revision,credential_slot FROM calendar_actions WHERE id='legacy'",
                        [],
                        |row| {
                            Ok((
                                row.get::<_, Option<String>>(0)?,
                                row.get::<_, Option<i64>>(1)?,
                                row.get::<_, Option<String>>(2)?,
                            ))
                        },
                    )?,
                ))
            })
            .await
            .expect("state");
        assert_eq!(state, (24, None, (None, None, None)));
        drop(database);
        let _ = std::fs::remove_file(path);
    }
}
