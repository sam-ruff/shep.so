use super::*;

fn draft(id: &str) -> Draft {
    Draft {
        id: id.into(),
        revision: 2,
        to: "friend@example.com".into(),
        body: "Keep my words".into(),
        ..Default::default()
    }
}

#[test]
fn delayed_file_and_workspace_snapshots_preserve_current_edits() {
    let (mut app, _) = App::new();
    let mut saved = draft("current");
    app.load_draft(saved.clone());
    let _ = app.handle(Message::Field("subject", "New thought".into()));
    saved.attachments.push(DraftAttachment {
        content_id: None,
        id: "file".into(),
        name: "notes.txt".into(),
        media_type: "text/plain".into(),
        size: 10,
    });
    app.observe_drafts(&DraftState {
        revision: 3,
        drafts: vec![saved.clone()],
    });
    app.observe_drafts(&DraftState {
        revision: 2,
        drafts: vec![draft("current")],
    });
    let _ = app.handle(Message::Backend(Event::Workspace(Arc::new(Workspace {
        drafts_revision: 1,
        drafts: vec![],
        ..Default::default()
    }))));
    assert_eq!(app.field("subject"), "New thought");
    assert_eq!(app.composer.draft.attachments, saved.attachments);
    assert_eq!(app.workspace.drafts_revision, 3);
    app.load_draft(draft("another"));
    let _ = app.handle(Message::Backend(Event::DraftFiles(
        "current".into(),
        Ok(Arc::new(DraftState {
            revision: 4,
            drafts: vec![saved],
        })),
    )));
    assert!(app.composer.draft.attachments.is_empty());
    assert_eq!(app.draft_id, "another");
}

#[test]
fn failed_save_or_unavailable_queue_keeps_composer_open() {
    let (mut app, _) = App::new();
    app.load_draft(draft("current"));
    let _ = app.handle(Message::Close);
    assert_eq!(app.dialog, Some(Dialog::Compose));
    assert_eq!(app.editor.text().trim(), "Keep my words");
    app.composer.saving = Some(("current".into(), 2, Exit::Dialog));
    let _ = app.draft_saved("current".into(), 2, Err("Disk full".into()));
    assert_eq!(app.dialog, Some(Dialog::Compose));
    assert!(app.draft_dirty.is_some());
    assert!(app.notice.as_ref().unwrap().0.contains("Disk full"));
}

#[test]
fn sent_ack_cannot_close_another_dialog_or_clear_newer_text() {
    let (mut app, _) = App::new();
    app.load_draft(draft("current"));
    let _ = app.handle(Message::Backend(Event::Sent("current".into(), 1)));
    assert_eq!(app.dialog, Some(Dialog::Compose));
    app.open(Dialog::Event);
    app.fields.insert("title", "Calendar edit".into());
    let _ = app.handle(Message::Backend(Event::Sent("current".into(), 2)));
    assert_eq!(app.dialog, Some(Dialog::Event));
    assert_eq!(app.field("title"), "Calendar edit");
}

#[test]
fn discard_cancel_preserves_unsaved_text_and_failure_can_retry() {
    let (mut app, _) = App::new();
    let (sender, mut queue) = crate::engine::CommandSender::persistence_test_channel();
    app.tx = Some(sender);
    app.load_draft(draft("current"));
    let _ = app.handle(Message::Field("subject", "Unsaved thought".into()));
    app.review_discard_draft("current".into());
    assert_eq!(app.dialog, Some(Dialog::DiscardDraft));
    let _ = app.handle(Message::Close);
    assert_eq!(app.dialog, Some(Dialog::Compose));
    assert_eq!(app.field("subject"), "Unsaved thought");
    assert_eq!(app.editor.text().trim(), "Keep my words");
    app.review_discard_draft("current".into());
    app.confirm_discard_draft();
    assert!(matches!(queue.try_recv().unwrap(), Command::DeleteDraft(id) if id=="current"));
    let _ = app.handle(Message::Close);
    assert_eq!(
        app.dialog,
        Some(Dialog::DiscardDraft),
        "Do not resume editing during a pending discard"
    );
    app.draft_deleted("current".into(), Err("Disk unavailable. Try again.".into()));
    assert!(!app.composer.discard_pending);
    assert_eq!(app.field("subject"), "Unsaved thought");
    app.confirm_discard_draft();
    assert!(matches!(queue.try_recv().unwrap(), Command::DeleteDraft(id) if id=="current"));
    app.draft_deleted(
        "current".into(),
        Ok(Arc::new(DraftState {
            revision: 4,
            drafts: vec![],
        })),
    );
    assert_eq!(app.dialog, None);
    assert_eq!(app.editor.text().trim(), "");
    assert!(app.draft_id.is_empty());
    let _ = app.draft_saved(
        "current".into(),
        2,
        Ok(Arc::new(DraftState {
            revision: 3,
            drafts: vec![draft("current")],
        })),
    );
    assert!(app.workspace.drafts.is_empty());
    assert!(app.draft_dirty.is_none());
}

#[test]
fn discard_acknowledgment_never_clears_another_editor() {
    let (mut app, _) = App::new();
    app.load_draft(draft("current"));
    app.review_discard_draft("current".into());
    app.open(Dialog::Event);
    app.fields.insert("title", "Keep this event".into());
    app.draft_deleted(
        "current".into(),
        Ok(Arc::new(DraftState {
            revision: 4,
            drafts: vec![],
        })),
    );
    assert_eq!(app.dialog, Some(Dialog::Event));
    assert_eq!(app.field("title"), "Keep this event");
}

#[test]
fn draft_context_survives_refresh_and_collapse_keeps_account_folders() {
    let (mut app, _) = App::new();
    let workspace = Arc::new(Workspace {
        drafts: vec![draft("one"), draft("two")],
        drafts_revision: 3,
        ..Default::default()
    });
    app.workspace = workspace.clone();
    assert_eq!(
        app.sidebar_items()
            .iter()
            .filter(|i| matches!(i.action, Message::Draft(_)))
            .count(),
        2
    );
    let _ = app.handle(Message::DraftContext(
        "two".into(),
        iced::Point::new(90., 500.),
    ));
    let _ = app.handle(Message::Backend(Event::Workspace(workspace)));
    assert_eq!(app.composer.context.as_ref().unwrap().id, "two");
    let _ = app.handle(Message::DismissContext);
    let _ = app.handle(Message::ToggleDrafts);
    assert!(app.preferences.collapsed_drafts);
    assert!(
        !app.sidebar_items()
            .iter()
            .any(|i| matches!(i.action, Message::Draft(_)))
    );
    assert!(
        app.sidebar_items()
            .iter()
            .any(|i| matches!(i.action, Message::ToggleDrafts))
    );
}

#[test]
fn forward_completion_targets_only_its_request_and_does_not_replace_another_editor() {
    let (mut app, _) = App::new();
    app.selected = Some("original".into());
    app.composer.forward_pending = Some(("forward".into(), "original".into(), app.detail_revision));
    let forward = draft("forward");
    let state = Arc::new(DraftState {
        revision: 2,
        drafts: vec![forward.clone()],
    });
    let _ = app.forward_ready("obsolete".into(), Ok(state.clone()));
    assert!(app.composer.forward_pending.is_some());
    assert!(app.dialog.is_none());
    let _ = app.forward_ready("forward".into(), Ok(state));
    assert_eq!(app.dialog, Some(Dialog::Compose));
    assert_eq!(app.draft_id, "forward");
    assert!(app.composer.forward_pending.is_none());

    app.composer.forward_pending = Some(("older".into(), "original".into(), app.detail_revision));
    app.load_draft(draft("newer"));
    let state = Arc::new(DraftState {
        revision: 3,
        drafts: vec![draft("older"), draft("newer")],
    });
    let _ = app.forward_ready("older".into(), Ok(state));
    assert_eq!(app.draft_id, "newer");
    assert_eq!(app.dialog, Some(Dialog::Compose));
    assert!(app.workspace.drafts.iter().any(|d| d.id == "older"));
}

#[test]
fn failed_or_navigated_forward_keeps_navigation_and_retry_available() {
    let (mut app, _) = App::new();
    app.selected = Some("new-selection".into());
    app.composer.forward_pending = Some((
        "forward".into(),
        "old-selection".into(),
        app.detail_revision,
    ));
    let _ = app.forward_ready(
        "forward".into(),
        Ok(Arc::new(DraftState {
            revision: 1,
            drafts: vec![draft("forward")],
        })),
    );
    assert!(app.dialog.is_none());
    assert_eq!(app.selected.as_deref(), Some("new-selection"));
    assert!(app.notice.as_ref().unwrap().0.contains("Drafts"));
    app.composer.forward_pending =
        Some(("retry".into(), "new-selection".into(), app.detail_revision));
    let _ = app.forward_ready(
        "retry".into(),
        Err("Storage unavailable. Try again.".into()),
    );
    assert!(app.composer.forward_pending.is_none());
    assert!(app.notice.as_ref().unwrap().1);
    assert_eq!(app.workspace.drafts.len(), 1);
}

#[test]
fn forwarding_formatting_survives_native_editor_roundtrip_and_new_note() {
    let (mut app, _) = App::new();
    let (draft, _) = crate::compose::prepare_forward("fwd".into(), "work".into(), b"Subject: Sample\r\nContent-Type: text/html\r\n\r\n<p>First paragraph.</p><p>Second paragraph.</p>").unwrap();
    app.load_draft(draft);
    let draft = app.current_draft();
    assert!(
        draft
            .forward
            .as_ref()
            .unwrap()
            .render(&draft.body)
            .is_some()
    );
    app.editor
        .perform(text_editor::Action::Edit(text_editor::Edit::Paste(
            Arc::new("New note.\n".into()),
        )));
    let draft = app.current_draft();
    assert!(
        draft
            .forward
            .as_ref()
            .unwrap()
            .render(&draft.body)
            .unwrap()
            .contains("New note.")
    );
}
