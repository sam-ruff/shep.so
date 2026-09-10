use super::*;

fn review() -> Arc<Review> {
    Arc::new(Review {
        bytes: 100,
        messages: 2,
        accounts: vec![],
        calendars: 0,
        drafts: 1,
        pending_outgoing: 1,
        pending_bulk: 0,
        pending_folders: 0,
        pending_moves: 0,
        pending_credentials: 0,
    })
}

#[test]
fn review_requires_confirmation_and_the_exact_preference_ack_before_installing() {
    let (mut app, _) = App::new();
    let (tx, mut database, mut persistence) = engine::CommandSender::database_test_channels();
    app.tx = Some(tx);
    let _ = app.database_import_action(Action::Begin);
    let id = app.database_import.serial;
    let _ = app.database_import_action(Action::Path(
        id,
        Some(PathBuf::from("/fixture/import.sqlite")),
    ));
    assert!(matches!(
        database.try_recv().unwrap(),
        Command::Database(Request::Import { .. })
    ));
    let _ = app.database_import_update(id, Update::Review(review()));
    let _ = app.database_import_action(Action::Install);
    assert!(
        persistence.try_recv().is_err(),
        "Unfinished work needs explicit review"
    );
    let _ = app.database_import_action(Action::ConfirmReview(true));
    let _ = app.database_import_action(Action::Name("  ".into()));
    let _ = app.database_import_action(Action::Install);
    assert!(persistence.try_recv().is_err());
    assert!(
        app.database_import
            .pending
            .as_ref()
            .unwrap()
            .review
            .is_some()
    );
    let _ = app.database_import_action(Action::Name("Shared workspace".into()));
    let _ = app.database_import_action(Action::Install);
    let Command::SavePreferences(generation, _) = persistence.try_recv().unwrap() else {
        panic!("Save preferences first")
    };
    app.database_preferences_saved(generation + 1);
    assert!(database.try_recv().is_err());
    app.database_preferences_saved(generation);
    assert!(
        matches!(database.try_recv().unwrap(),Command::Database(Request::Install {request, name}) if request == id && name == "Shared workspace")
    );
    app.advance_database_import();
    assert!(
        database.try_recv().is_err(),
        "Do not duplicate a pending installation"
    );
    let _ = app.database_import_update(id, Update::ReviewError("Choose another name".into()));
    let p = app.database_import.pending.as_ref().unwrap();
    assert!(p.review.is_some() && p.installing.is_none() && p.save.is_none());
}

#[test]
fn cancel_retries_a_full_queue_and_a_saved_receipt_wins_over_cancel_intent() {
    let (mut app, _) = App::new();
    let (tx, mut database, _persistence) = engine::CommandSender::database_test_channels();
    app.tx = Some(tx);
    let _ = app.database_import_action(Action::Begin);
    let id = app.database_import.serial;
    let _ = app.database_import_action(Action::Path(
        id,
        Some(PathBuf::from("/fixture/import.sqlite")),
    ));
    let _ = app.database_import_action(Action::Cancel);
    assert!(app.database_import.pending.as_ref().unwrap().cancelling);
    assert!(!app.database_import.pending.as_ref().unwrap().cancel_sent);
    database.try_recv().unwrap();
    app.advance_database_import();
    assert!(
        matches!(database.try_recv().unwrap(),Command::Database(Request::Cancel(request)) if request == id)
    );
    let saved = Installed {
        id: crate::profiles::Id::Imported(uuid::Uuid::new_v4()),
        name: "Kept copy".into(),
        path: "/fixture/profile/shep.sqlite".into(),
        warning: None,
        registered: true,
    };
    let _ = app.database_import_update(id, Update::ImportFinished(Ok(Some(saved.clone()))));
    assert!(!app.database_import.pending());
    assert_eq!(app.database_import.saved.as_ref().unwrap().id, saved.id);
    let _ = app.database_import_update(id, Update::ImportFinished(Ok(None)));
    assert_eq!(app.database_import.saved.as_ref().unwrap().id, saved.id);
}

#[test]
fn stale_picker_and_backend_results_cannot_change_a_new_import() {
    let (mut app, _) = App::new();
    let (tx, mut database, _persistence) = engine::CommandSender::database_test_channels();
    app.tx = Some(tx);
    let _ = app.database_import_action(Action::Begin);
    let old = app.database_import.serial;
    let _ = app.database_import_action(Action::Cancel);
    let _ = app.database_import_action(Action::Begin);
    let current = app.database_import.serial;
    let _ = app.database_import_action(Action::Path(
        old,
        Some(PathBuf::from("/fixture/old.sqlite")),
    ));
    assert!(database.try_recv().is_err());
    let _ = app.database_import_action(Action::Path(
        current,
        Some(PathBuf::from("/fixture/current.sqlite")),
    ));
    database.try_recv().unwrap();
    let _ = app.database_import_update(
        old,
        Update::ImportFinished(Err("Old validation failure".into())),
    );
    let _ = app.database_import_update(old, Update::Review(review()));
    assert!(app.database_import.error.is_none());
    assert!(
        app.database_import
            .pending
            .as_ref()
            .unwrap()
            .review
            .is_none()
    );
    let _ = app.database_import_update(current, Update::Review(review()));
    assert!(
        app.database_import
            .pending
            .as_ref()
            .unwrap()
            .review
            .is_some()
    );
}
