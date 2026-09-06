use shep::{
    model::*,
    providers::calendar::{encode_ical, parse_caldav},
    store::Store,
};

fn source() -> CalendarSource {
    CalendarSource {
        access: Default::default(),
        id: "home".into(),
        name: "Home".into(),
        kind: CalendarKind::CalDav,
        url: "https://calendar.example/home/".into(),
        username: "test".into(),
    }
}
fn event(source_id: &str, id: &str) -> CalendarEvent {
    let start = chrono::DateTime::parse_from_rfc3339("2026-09-06T09:00:00Z")
        .unwrap()
        .to_utc();
    CalendarEvent {
        id: id.into(),
        source_id: source_id.into(),
        title: source_id.into(),
        start,
        end: start + chrono::Duration::hours(1),
        all_day: false,
        etag: None,
        remote_url: None,
        location: String::new(),
        description: String::new(),
    }
}
fn parse(properties: &str) -> CalendarEvent {
    let source = source();
    let xml = format!(
        r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/home/event.ics</d:href><d:propstat><d:prop><d:getetag>"v1"</d:getetag><c:calendar-data><![CDATA[BEGIN:VCALENDAR
VERSION:2.0
BEGIN:VEVENT
UID:single
{properties}
END:VEVENT
END:VCALENDAR
]]></c:calendar-data></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
    );
    parse_caldav(&xml, &source, &url::Url::parse(&source.url).unwrap())
        .unwrap()
        .remove(0)
}

#[test]
fn caldav_all_day_and_timed_defaults_follow_calendar_duration_semantics() {
    let all_day = parse("DTSTART;VALUE=DATE:20260906");
    assert!(all_day.all_day);
    assert_eq!(all_day.end - all_day.start, chrono::Duration::days(1));
    let timed = parse("DTSTART:20260906T090000Z");
    assert_eq!(timed.start, timed.end);
    let multiday = parse("DTSTART;VALUE=DATE:20260906\nDURATION:P2D");
    assert_eq!(multiday.end - multiday.start, chrono::Duration::days(2));
    let clock = parse("DTSTART:20260906T090000Z\nDURATION:PT1H30M15S");
    assert_eq!((clock.end - clock.start).num_seconds(), 5415);
}

#[test]
fn caldav_nominal_days_cross_dst_but_exact_hours_do_not() {
    let nominal = parse("DTSTART;TZID=Europe/London:20261024T100000\nDURATION:P1D");
    assert_eq!(nominal.end.to_rfc3339(), "2026-10-25T10:00:00+00:00");
    assert_eq!((nominal.end - nominal.start).num_hours(), 25);
    let exact = parse("DTSTART;TZID=Europe/London:20261024T100000\nDURATION:PT24H");
    assert_eq!(exact.end.to_rfc3339(), "2026-10-25T09:00:00+00:00");
}

#[test]
fn calendar_literal_backslashes_round_trip_without_becoming_newlines() {
    let mut event = event("home", "single");
    event.title = r"C:\new\notes, then; a literal \n".into();
    event.description = "Line one\nLine two".into();
    let source = source();
    let xml = format!(
        r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/home/event.ics</d:href><c:calendar-data><![CDATA[{}]]></c:calendar-data></d:response></d:multistatus>"#,
        encode_ical(&event)
    );
    let parsed = parse_caldav(&xml, &source, &url::Url::parse(&source.url).unwrap()).unwrap();
    assert_eq!(parsed[0].title, event.title);
    assert_eq!(parsed[0].description, event.description);
}

#[tokio::test]
async fn calendar_ids_are_scoped_and_sync_rejects_mixed_sources_atomically() {
    let store = Store::memory().unwrap();
    let a = event("home", "same-uid");
    let mut b = event("work", "same-uid");
    store
        .replace_events("home".into(), vec![a.clone()])
        .await
        .unwrap();
    store
        .replace_events("work".into(), vec![b.clone()])
        .await
        .unwrap();
    b.title = "Updated work event".into();
    store.save_event(b.clone()).await.unwrap();
    let events = store.events().await.unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events.iter().find(|e| e.source_id == "home").unwrap().title,
        a.title
    );
    assert!(store.replace_events("home".into(), vec![b]).await.is_err());
    assert_eq!(store.events().await.unwrap().len(), 2);
    store
        .delete_event("work".into(), "same-uid".into())
        .await
        .unwrap();
    let events = store.events().await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].source_id, "home");
    // Delimiters in either opaque identifier must not collide.
    store.save_event(event("one:two", "three")).await.unwrap();
    store.save_event(event("one", "two:three")).await.unwrap();
    assert_eq!(store.events().await.unwrap().len(), 3);
}

#[tokio::test]
async fn calendar_cache_migrates_legacy_keys_once_and_preserves_events() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("legacy.sqlite");
    let old = event("home", "same-uid");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE events(id TEXT PRIMARY KEY, source TEXT NOT NULL, start INTEGER NOT NULL, data TEXT NOT NULL); PRAGMA user_version=1;").unwrap();
        connection
            .execute(
                "INSERT INTO events VALUES(?,?,?,?)",
                rusqlite::params![
                    "home:same-uid",
                    old.source_id,
                    old.start.timestamp(),
                    serde_json::to_string(&old).unwrap()
                ],
            )
            .unwrap();
    }
    let store = Store::open(&path).unwrap();
    let mut updated = old.clone();
    updated.title = "Updated after migration".into();
    store.save_event(updated.clone()).await.unwrap();
    drop(store);
    let store = Store::open(&path).unwrap();
    let events = store.events().await.unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].title, updated.title);
    store
        .run(move |connection| {
            let key: String = connection.query_row("SELECT id FROM events", [], |r| r.get(0))?;
            assert_eq!(key, old.key());
            let version: u32 = connection.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            assert_eq!(version, 2);
            Ok(())
        })
        .await
        .unwrap();
}

#[test]
fn calendar_access_preserves_legacy_sources_and_distinguishes_each_mutation() {
    use shep::model::CalendarAccess;
    let mut source = source();
    let mut json = serde_json::to_value(&source).unwrap();
    json.as_object_mut().unwrap().remove("access");
    let legacy: CalendarSource = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.access, CalendarAccess::default());
    let mut event = event("home", "a");
    event.etag = Some("\"a\"".into());
    event.remote_url = Some("/home/a.ics".into());
    source.access = CalendarAccess {
        create: false,
        update: true,
        delete: false,
    };
    assert!(shep::providers::calendar::ensure_event_access(&source, &event, false).is_ok());
    assert!(shep::providers::calendar::ensure_event_access(&source, &event, true).is_err());
    event.etag = None;
    event.remote_url = None;
    assert!(shep::providers::calendar::ensure_event_access(&source, &event, false).is_err());
    source.access = CalendarAccess::READ_ONLY;
    assert!(shep::providers::calendar::ensure_event_access(&source, &event, false).is_err());
}

#[tokio::test]
async fn google_calendar_permission_refresh_revokes_missing_grants_without_erasing_cached_events() {
    let store = Store::memory().unwrap();
    let home = source();
    let mut google = home.clone();
    google.kind = CalendarKind::Google;
    google.id = "google:work".into();
    google.url = "work".into();
    store
        .save_sources(vec![home.clone(), google.clone()])
        .await
        .unwrap();
    store.save_event(event(&google.id, "cached")).await.unwrap();
    store.refresh_google_sources(Vec::new()).await.unwrap();
    let sources: Vec<CalendarSource> = store.get("calendars").await.unwrap();
    assert_eq!(sources.iter().find(|s| s.id == home.id).unwrap(), &home);
    assert!(
        sources
            .iter()
            .find(|s| s.id == google.id)
            .unwrap()
            .access
            .read_only()
    );
    assert_eq!(store.events().await.unwrap().len(), 1);
    store
        .refresh_google_sources(vec![google.clone()])
        .await
        .unwrap();
    assert!(
        store
            .get::<Vec<CalendarSource>>("calendars")
            .await
            .unwrap()
            .iter()
            .find(|s| s.id == google.id)
            .unwrap()
            .access
            .update
    );
}
