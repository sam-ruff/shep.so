use super::*;

fn event(title: &str) -> CalendarEvent {
    let start = chrono::Utc::now();
    CalendarEvent {
        id: "one".into(),
        source_id: "work".into(),
        title: title.into(),
        start,
        end: start + chrono::Duration::hours(1),
        location: String::new(),
        description: "Retain this description".into(),
        all_day: false,
        etag: Some("original".into()),
        remote_url: Some("https://example.test/one".into()),
    }
}

#[tokio::test]
async fn calendar_save_paints_before_receipt_and_late_rejection_keeps_newer_editor() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    app.calendar_actions.base = Arc::new(vec![event("Original")]);
    app.dialog = Some(Dialog::Event);
    app.begin_calendar_action(event("Requested"), false);
    assert_eq!(app.events[0].title, "Requested");
    assert!(app.dialog.is_none());
    let Command::CalendarAction(request, _, false) = commands.try_recv().unwrap() else {
        panic!("expected save")
    };
    app.dialog = Some(Dialog::Event);
    app.fields.insert("title", "Newer editor text".into());
    app.calendar_action_finished(
        request,
        CalendarActionResult::Rejected("Permission changed".into()),
    );
    assert_eq!(app.events[0].title, "Original");
    assert_eq!(app.field("title"), "Newer editor text");
    assert_eq!(app.dialog, Some(Dialog::Event));
    assert_eq!(
        app.calendar_recovery_event(&event("").key()).unwrap().title,
        "Requested"
    );
    assert!(app.calendar_actions.needs_review());
}

#[tokio::test]
async fn calendar_delete_preserves_prediction_until_authoritative_revision() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    let original = event("Original");
    app.calendar_actions.base = Arc::new(vec![original.clone()]);
    app.begin_calendar_action(original.clone(), true);
    assert!(app.events.is_empty());
    let Command::CalendarAction(request, _, true) = commands.try_recv().unwrap() else {
        panic!("expected delete")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Applied {
            event: Box::new(original),
            revision: Some(5),
            warning: None,
        },
    );
    let _ = app.handle(Message::Backend(Event::Calendar(
        4,
        app.calendar_actions.base.clone(),
    )));
    assert!(app.events.is_empty());
    let _ = app.handle(Message::Backend(Event::Calendar(5, Arc::new(vec![]))));
    assert!(app.events.is_empty());
    assert!(app.calendar_actions.pending.is_empty());
}

#[tokio::test]
async fn uncertain_calendar_write_keeps_content_and_requires_review_before_close() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    app.begin_calendar_action(event("Requested"), false);
    let Command::CalendarAction(request, _, _) = commands.try_recv().unwrap() else {
        panic!("expected save")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Uncertain("Connection lost".into()),
    );
    app.begin_calendar_action(event("Second attempt"), false);
    assert!(commands.try_recv().is_err());
    assert_eq!(app.events[0].title, "Requested");
    let _ = app.handle(Message::WindowClose(iced::window::Id::unique()));
    assert!(app.pending_close.is_none());
    assert!(app.calendar_actions.needs_review());
    app.busy.clear();
    assert!(
        app.has_required_close_work(),
        "Recovery survives provider capacity release"
    );
    app.dismiss_calendar_change(request);
    assert!(app.calendar_actions.needs_review());
    let _ = app.handle(Message::Backend(Event::Calendar(2, Arc::new(vec![]))));
    app.dismiss_calendar_change(request);
    assert!(
        app.calendar_actions.needs_review(),
        "A local calendar snapshot is not a provider observation"
    );
    app.calendar_action_observed(
        request + 1,
        Ok(engine::CalendarObservation {
            revision: 3,
            events: Arc::new(vec![]),
            current: None,
        }),
    );
    app.dismiss_calendar_change(request);
    assert!(
        app.calendar_actions.needs_review(),
        "A different request cannot verify this event"
    );
    app.inspect_calendar_change(request);
    assert!(
        matches!(commands.try_recv().unwrap(), Command::InspectCalendarAction(id, _) if id == request)
    );
    app.dismiss_calendar_change(request);
    assert!(
        app.calendar_actions.needs_review(),
        "Wait for the checked source cache snapshot"
    );
    app.calendar_action_observed(
        request,
        Ok(engine::CalendarObservation {
            revision: 3,
            events: Arc::new(vec![]),
            current: None,
        }),
    );
    app.dismiss_calendar_change(request);
    assert!(!app.calendar_actions.needs_review());
    assert!(app.events.is_empty());
}

#[tokio::test]
async fn exact_calendar_inspection_handles_out_of_window_events_without_replaying_writes() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    app.events_revision = 10;
    let mut requested = event("Requested");
    requested.start += chrono::Duration::days(800);
    requested.end += chrono::Duration::days(800);
    app.begin_calendar_action(requested.clone(), false);
    let Command::CalendarAction(request, _, _) = commands.try_recv().unwrap() else {
        panic!("expected save")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Uncertain("Lost reply".into()),
    );
    app.busy.clear();
    app.inspect_calendar_change(request);
    assert!(
        matches!(commands.try_recv().unwrap(), Command::InspectCalendarAction(_, event) if event.start == requested.start)
    );
    app.calendar_action_observed(request, Err("Server unavailable".into()));
    assert!(!app.can_dismiss_calendar_change(request));
    app.busy.clear();
    app.inspect_calendar_change(request);
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::InspectCalendarAction(_, _)
    ));
    app.calendar_action_observed(
        request,
        Ok(engine::CalendarObservation {
            revision: 11,
            events: Arc::new(vec![]),
            current: None,
        }),
    );
    assert!(app.can_dismiss_calendar_change(request));
    app.dismiss_calendar_change(request);
    assert!(!app.calendar_actions.has_changes());
}

#[tokio::test]
async fn acknowledged_calendar_repair_can_adopt_verified_newer_server_edit_without_etag() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    let requested = event("Requested");
    app.begin_calendar_action(requested.clone(), false);
    let Command::CalendarAction(request, _, _) = commands.try_recv().unwrap() else {
        panic!("expected save")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Applied {
            event: Box::new(requested),
            revision: None,
            warning: Some("Cache failed".into()),
        },
    );
    app.busy.clear();
    app.inspect_calendar_change(request);
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::InspectCalendarAction(_, _)
    ));
    let mut newer = event("Edited on another device");
    newer.etag = None;
    app.calendar_action_observed(
        request,
        Ok(engine::CalendarObservation {
            revision: 5,
            events: Arc::new(vec![newer.clone()]),
            current: Some(newer),
        }),
    );
    assert!(app.can_dismiss_calendar_change(request));
    app.dismiss_calendar_change(request);
    assert_eq!(app.events[0].title, "Edited on another device");
    assert!(!app.calendar_actions.has_changes());
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn unknown_calendar_create_projects_one_row_until_verified_remote_identity_is_adopted() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    app.begin_calendar_action(event("Requested"), false);
    let Command::CalendarAction(request, _, _) = commands.try_recv().unwrap() else {
        panic!("expected save")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Uncertain("Lost create reply".into()),
    );
    app.busy.clear();
    app.inspect_calendar_change(request);
    assert!(matches!(
        commands.try_recv().unwrap(),
        Command::InspectCalendarAction(_, _)
    ));
    let mut remote = event("Verified remote event");
    remote.id = "google-mapped-identity".into();
    app.calendar_action_observed(
        request,
        Ok(engine::CalendarObservation {
            revision: 6,
            events: Arc::new(vec![remote.clone()]),
            current: Some(remote),
        }),
    );
    assert_eq!(app.events.len(), 1);
    assert_eq!(app.events[0].title, "Requested");
    app.dismiss_calendar_change(request);
    assert_eq!(app.events.len(), 1);
    assert_eq!(app.events[0].id, "google-mapped-identity");
    assert!(!app.calendar_actions.has_changes());
}

#[tokio::test]
async fn acknowledged_calendar_change_never_dismisses_to_stale_cache() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    let mut saved = event("Saved");
    saved.etag = Some("new".into());
    app.calendar_actions.base = Arc::new(vec![event("Original")]);
    app.begin_calendar_action(saved.clone(), false);
    let Command::CalendarAction(request, _, _) = commands.try_recv().unwrap() else {
        panic!("expected save")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Applied {
            event: Box::new(saved.clone()),
            revision: None,
            warning: Some("Saved remotely; cache failed".into()),
        },
    );
    app.dismiss_calendar_change(request);
    assert_eq!(app.events[0].title, "Saved");
    assert!(app.calendar_actions.needs_review());
    let _ = app.handle(Message::Backend(Event::Calendar(5, Arc::new(vec![saved]))));
    assert_eq!(app.events[0].title, "Saved");
    assert!(!app.calendar_actions.needs_review());
}

#[tokio::test]
async fn acknowledged_delete_does_not_retire_repair_from_an_old_absent_snapshot() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    app.events_revision = 5;
    let original = event("Original");
    app.begin_calendar_action(original.clone(), true);
    let Command::CalendarAction(request, _, _) = commands.try_recv().unwrap() else {
        panic!("expected delete")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Applied {
            event: Box::new(original),
            revision: None,
            warning: Some("Cache write failed".into()),
        },
    );
    assert!(app.calendar_actions.needs_review());
    let _ = app.handle(Message::Backend(Event::Calendar(5, Arc::new(vec![]))));
    assert!(app.calendar_actions.needs_review());
    let _ = app.handle(Message::Backend(Event::Calendar(6, Arc::new(vec![]))));
    assert!(!app.calendar_actions.needs_review());
}

#[tokio::test]
async fn rejected_calendar_change_can_be_edited_and_retried_without_discarding_recovery() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    app.begin_calendar_action(event("First attempt"), false);
    let Command::CalendarAction(request, original, _) = commands.try_recv().unwrap() else {
        panic!("expected save")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Rejected("Permission changed".into()),
    );
    app.busy.remove(&format!("event:{}", original.key()));
    app.begin_calendar_action(event("Edited retry"), false);
    let Command::CalendarAction(retry, submitted, _) = commands.try_recv().unwrap() else {
        panic!("expected retry")
    };
    assert_ne!(retry, request);
    assert_eq!(submitted.id, original.id);
    assert_eq!(submitted.title, "Edited retry");
    assert_eq!(app.calendar_actions.pending.len(), 1);
    app.calendar_action_finished(
        request,
        CalendarActionResult::Rejected("Late old error".into()),
    );
    assert_eq!(app.events[0].title, "Edited retry");
    assert!(!app.calendar_actions.needs_review());
}

#[tokio::test]
async fn rejected_new_caldav_event_reopens_as_editable_creation_with_the_same_identity() {
    let (mut app, _) = App::new();
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    Arc::make_mut(&mut app.workspace)
        .calendars
        .push(CalendarSource {
            id: "work".into(),
            name: "Work".into(),
            kind: CalendarKind::CalDav,
            url: "https://example.test/calendar".into(),
            username: "fixture".into(),
            access: CalendarAccess {
                create: true,
                update: false,
                delete: false,
            },
        });
    let mut original = event("First attempt");
    original.etag = None;
    original.remote_url = None;
    app.begin_calendar_action(original.clone(), false);
    let Command::CalendarAction(request, _, _) = commands.try_recv().unwrap() else {
        panic!("expected save")
    };
    app.calendar_action_finished(
        request,
        CalendarActionResult::Rejected("Permission changed".into()),
    );
    app.busy.remove(&format!("event:{}", original.key()));
    let _ = app.handle(Message::EditEvent(original.key()));
    assert!(app.event_access().update);
    assert!(!app.event_access().delete);
    app.fields.insert("title", "Edited new event".into());
    let _ = app.handle(Message::SaveEvent);
    let Command::CalendarAction(_, submitted, false) = commands.try_recv().unwrap() else {
        panic!("expected retry")
    };
    assert_eq!(submitted.id, original.id);
    assert_eq!(submitted.title, "Edited new event");
    assert_eq!(submitted.description, original.description);
}
