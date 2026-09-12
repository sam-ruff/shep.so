use super::*;
use crate::mail_actions::{Fingerprint, MoveReceipt};

fn record(stage: MoveStage) -> Arc<MoveRecord> {
    let stored = parse_mail(
        "work",
        "42.7",
        "INBOX",
        b"Subject: Keepsake\r\n\r\nUnchanged original".to_vec(),
        true,
        false,
    )
    .unwrap();
    let receipt = MoveReceipt::server(
        &stored.summary,
        "work",
        "Keep",
        None,
        Fingerprint::of(&stored.raw),
    );
    let mut record = MoveRecord::new(stored.summary, receipt);
    record.stage = stage;
    Arc::new(record)
}
fn review(record: Arc<MoveRecord>) -> (App, tokio::sync::mpsc::Receiver<Command>) {
    let (mut app, _) = App::new();
    let (sender, commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    app.dialog = Some(Dialog::MoveRecovery);
    app.choose_recovery(Some(record));
    (app, commands)
}

#[tokio::test]
async fn retry_move_runs_without_a_review_and_coalesces_repeated_clicks() {
    let record = record(MoveStage::Started);
    let (mut app, mut commands) = review(record.clone());
    app.dialog = None;
    app.handle_move_recovery(Message::Retry(record.clone()));
    app.handle_move_recovery(Message::Retry(record.clone()));
    let Command::RecoverMailMove(_, saved, action, confirmed) = commands.try_recv().unwrap() else {
        panic!("Expected move retry");
    };
    assert_eq!(saved.token, record.token);
    assert_eq!(action, RecoveryAction::Retry);
    assert!(!confirmed);
    assert_eq!(app.dialog, None);
    assert_eq!(app.move_recovery.pending.len(), 1);
    assert!(commands.try_recv().is_err());
}

#[tokio::test]
async fn pending_move_accepts_a_new_destination_but_not_flags_or_group_owned_mail() {
    let record = record(MoveStage::Started);
    let (mut app, mut commands) = review(record.clone());
    app.dialog = None;
    app.selected = Some(record.original.id.clone());
    let mut row = record.original.clone();
    row.remote_id.clear();
    let mut page = MailPage::default();
    page.rows.push(row);
    page.move_placeholders.insert(record.original.id.clone());
    page.move_recovery
        .insert(record.original.id.clone(), (*record).clone());
    app.page = Arc::new(page);
    app.mail_actions.base_page = app.page.clone();
    assert!(app.action_mail().is_none());
    assert_eq!(
        app.move_action_mail().map(|m| &m.id),
        Some(&record.original.id)
    );
    Arc::make_mut(&mut app.page)
        .bulk_pending
        .insert(record.original.id.clone());
    assert!(app.move_action_mail().is_none());
    Arc::make_mut(&mut app.page).bulk_pending.clear();
    let _ = app.handle(super::super::Message::Move("Elsewhere".into()));
    let mut submitted = None;
    while let Ok(command) = commands.try_recv() {
        if let Command::Move(_, source, destination) = command {
            submitted = Some((source.remote_id, destination));
        }
    }
    assert_eq!(
        submitted,
        Some((record.original.remote_id.clone(), "Elsewhere".into()))
    );
    assert_eq!(app.dialog, None);
}

#[tokio::test]
async fn recovery_confirmation_prevents_enter_yes_and_duplicate_submission_and_allows_cancel() {
    let record = record(MoveStage::Started);
    let (mut app, mut commands) = review(record.clone());
    for key in [
        Key::Named(keyboard::key::Named::Enter),
        Key::Character("y".into()),
    ] {
        let _ = app.key(key, keyboard::Modifiers::empty(), false);
        assert!(commands.try_recv().is_err());
    }
    let _ = app.key(
        Key::Character("n".into()),
        keyboard::Modifiers::empty(),
        false,
    );
    assert_eq!(app.dialog, None);
    assert!(commands.try_recv().is_err());
    app.dialog = Some(Dialog::MoveRecovery);
    app.handle_move_recovery(Message::Confirm(true));
    let _ = app.key(
        Key::Named(keyboard::key::Named::Enter),
        keyboard::Modifiers::empty(),
        false,
    );
    let Command::RecoverMailMove(request, saved, action, confirmed) = commands.try_recv().unwrap()
    else {
        panic!("Expected recovery");
    };
    assert_eq!(saved.token, record.token);
    assert_eq!(action, RecoveryAction::UseExistingCopy);
    assert!(confirmed);
    app.handle_move_recovery(Message::Submit);
    assert!(commands.try_recv().is_err());
    assert_eq!(app.move_recovery.pending[&record.token], request);
    let _ = app.key(
        Key::Named(keyboard::key::Named::Escape),
        keyboard::Modifiers::empty(),
        false,
    );
    assert_eq!(app.dialog, None);
    assert_eq!(
        app.move_recovery.pending.len(),
        1,
        "Closing the form does not cancel the receipt"
    );
    let _ = app.move_recovery_finished(request + 1, record.token.clone(), Err("Old error".into()));
    assert_eq!(app.move_recovery.pending.len(), 1);
    app.dialog = Some(Dialog::Sender);
    let _ = app.move_recovery_finished(
        request,
        record.token.clone(),
        Err("Destination unavailable".into()),
    );
    assert!(app.move_recovery.pending.is_empty());
    assert_eq!(app.dialog, Some(Dialog::Sender));
    assert!(
        app.notice
            .as_ref()
            .unwrap()
            .0
            .contains("Destination unavailable")
    );
}

#[tokio::test]
async fn recovery_pages_reject_stale_results_preserve_failure_and_reconfirm_changed_stage() {
    let record = record(MoveStage::Started);
    let (mut app, _) = review(record.clone());
    let (sender, mut reads) = engine::CommandSender::foreground_test_channel();
    app.tx = Some(sender);
    app.load_move_recoveries(None);
    let Command::MoveRecoveries(first, _) = reads.try_recv().unwrap() else {
        panic!()
    };
    app.load_move_recoveries(Some("after-token".into()));
    let Command::MoveRecoveries(second, after) = reads.try_recv().unwrap() else {
        panic!()
    };
    assert_eq!(after.as_deref(), Some("after-token"));
    app.move_recoveries_loaded(first, Err("Stale page".into()));
    assert!(app.move_recovery.loading);
    app.move_recovery.confirmed = true;
    app.move_recovery.error = Some("Cleanup failed; retry".into());
    let mut copied = (*record).clone();
    copied.stage = MoveStage::Copied;
    app.move_recoveries_loaded(second, Ok(Arc::new(vec![copied])));
    assert!(!app.move_recovery.loading);
    assert!(!app.move_recovery.confirmed);
    assert_eq!(app.move_recovery.action, Some(RecoveryAction::Retry));
    assert_eq!(
        app.move_recovery.error.as_deref(),
        Some("Cleanup failed; retry")
    );
    // A reader can open a record beyond the first metadata page. Listing the
    // first page must not silently replace that explicitly chosen message.
    app.load_move_recoveries(None);
    let current = app.move_recovery.page_request;
    app.move_recoveries_loaded(current, Ok(Arc::new(vec![])));
    assert_eq!(
        app.move_recovery.selected.as_ref().unwrap().token,
        record.token
    );
    app.dialog = None;
    app.move_recoveries_loaded(current, Err("Closed form".into()));
    assert_eq!(
        app.move_recovery.error.as_deref(),
        Some("Cleanup failed; retry")
    );
}

#[tokio::test]
async fn recovery_backpressure_and_close_failure_leave_a_retryable_review() {
    let record = record(MoveStage::Committed);
    let (mut app, mut commands) = review(record.clone());
    for _ in 0..CHANNEL_CAPACITY {
        app.tx
            .as_ref()
            .unwrap()
            .try_send(Command::SyncCalendar)
            .unwrap();
    }
    app.handle_move_recovery(Message::Submit);
    assert!(app.move_recovery.pending.is_empty());
    assert!(
        app.move_recovery
            .error
            .as_ref()
            .unwrap()
            .contains("queue is full")
    );
    while commands.try_recv().is_ok() {}
    app.handle_move_recovery(Message::Submit);
    let Command::RecoverMailMove(request, ..) = commands.try_recv().unwrap() else {
        panic!()
    };
    app.bulk.stopped = true;
    let window = iced::window::Id::unique();
    let _ = app.handle(super::super::Message::WindowClose(window));
    assert_eq!(app.pending_close, Some(window));
    let _ = app.move_recovery_finished(
        request,
        record.token.clone(),
        Err("Source still needs cleanup".into()),
    );
    assert!(app.pending_close.is_none());
    assert_eq!(app.dialog, Some(Dialog::MoveRecovery));
    assert!(app.move_recovery.error.is_some());
}
