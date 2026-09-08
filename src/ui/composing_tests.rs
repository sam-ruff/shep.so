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
    let _ = app.handle(Message::ComposeField("subject", "New thought".into()));
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
    assert_eq!(app.compose_field("subject"), "New thought");
    assert_eq!(app.composer.current.draft.attachments, saved.attachments);
    assert_eq!(app.workspace.drafts_revision, 3);
    app.load_draft(draft("another"));
    let _ = app.handle(Message::Backend(Event::DraftFiles(
        "current".into(),
        Ok(Arc::new(DraftState {
            revision: 4,
            drafts: vec![saved],
        })),
    )));
    assert!(app.composer.current.draft.attachments.is_empty());
    assert_eq!(app.composer.current.draft.id, "another");
}

#[test]
fn failed_save_or_unavailable_queue_retains_parked_draft_without_blocking_navigation() {
    let (mut app, _) = App::new();
    app.load_draft(draft("current"));
    let _ = app.handle(Message::CloseComposer);
    assert!(!app.compose_visible());
    assert_eq!(
        app.owned_draft("current").unwrap().body.trim(),
        "Keep my words"
    );
    let _ = app.draft_saved("current".into(), 2, Err("Disk full".into()));
    assert!(app.composer.parked["current"].dirty.is_some());
    assert!(app.notice.as_ref().unwrap().0.contains("Disk full"));
    let _ = app.handle(Message::Draft("current".into()));
    assert!(app.compose_visible());
    assert_eq!(app.current_draft().body.trim(), "Keep my words");
}

#[test]
fn sent_ack_cannot_close_another_dialog_or_clear_newer_text() {
    let (mut app, _) = App::new();
    app.load_draft(draft("current"));
    let _ = app.handle(Message::Backend(Event::Sent("current".into(), 1)));
    assert!(app.compose_visible());
    assert!(app.dialog.is_none());
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
    let _ = app.handle(Message::ComposeField("subject", "Unsaved thought".into()));
    app.review_discard_draft("current".into());
    assert_eq!(app.dialog, Some(Dialog::DiscardDraft));
    let _ = app.handle(Message::Close);
    assert!(app.compose_visible());
    assert!(app.dialog.is_none());
    assert_eq!(app.compose_field("subject"), "Unsaved thought");
    assert_eq!(app.composer.current.editor.text().trim(), "Keep my words");
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
    assert_eq!(app.compose_field("subject"), "Unsaved thought");
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
    assert_eq!(app.composer.current.editor.text().trim(), "");
    assert!(app.composer.current.draft.id.is_empty());
    let _ = app.draft_saved(
        "current".into(),
        2,
        Ok(Arc::new(DraftState {
            revision: 3,
            drafts: vec![draft("current")],
        })),
    );
    assert!(app.workspace.drafts.is_empty());
    assert!(app.composer.current.dirty.is_none());
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
    assert!(app.compose_visible());
    assert!(app.dialog.is_none());
    assert_eq!(app.composer.current.draft.id, "forward");
    assert!(app.composer.forward_pending.is_none());

    app.composer.forward_pending = Some(("older".into(), "original".into(), app.detail_revision));
    app.load_draft(draft("newer"));
    let state = Arc::new(DraftState {
        revision: 3,
        drafts: vec![draft("older"), draft("newer")],
    });
    let _ = app.forward_ready("older".into(), Ok(state));
    assert_eq!(app.composer.current.draft.id, "newer");
    assert!(app.compose_visible());
    assert!(app.dialog.is_none());
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
    app.composer
        .current
        .editor
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

#[test]
fn independent_dialog_fields_cannot_replace_draft_recipients_or_cancel_autosave() {
    let (mut app, _) = App::new();
    let (sender, mut queue) = crate::engine::CommandSender::persistence_test_channel();
    app.tx = Some(sender);
    let mut original = draft("current");
    original.account_id = "work".into();
    original.cc = "copy@example.test".into();
    original.bcc = "private@example.test".into();
    original.subject = "Original subject".into();
    app.load_draft(original.clone());
    let _ = app.handle(Message::ComposeField("subject", "New subject".into()));
    let revision = app.composer.current.draft.revision;
    app.composer.current.dirty = Some(Instant::now() - std::time::Duration::from_secs(2));
    app.open(Dialog::Event);
    let _ = app.handle(Message::Field("subject", "Other form".into()));
    let _ = app.handle(Message::Field("title", "Calendar title".into()));
    let _ = app.handle(Message::Tick);
    let Command::AutoSaveDraft(saved) = queue.try_recv().unwrap() else {
        panic!("The draft must save even when another form is visible");
    };
    assert_eq!(saved.id, original.id);
    assert_eq!(saved.account_id, original.account_id);
    assert_eq!(saved.to, original.to);
    assert_eq!(saved.cc, original.cc);
    assert_eq!(saved.bcc, original.bcc);
    assert_eq!(saved.subject, "New subject");
    assert_eq!(saved.body.trim(), original.body);
    assert_eq!(saved.revision, revision);
    assert!(app.composer.current.dirty.is_none());
    let _ = app.draft_saved("current".into(), revision, Err("Disk full".into()));
    assert!(app.composer.current.dirty.is_some());
    assert_eq!(app.dialog, Some(Dialog::Event));
    assert_eq!(app.field("title"), "Calendar title");
    assert_eq!(app.field("subject"), "Other form");
    assert_eq!(app.current_draft().bcc, original.bcc);
}

#[test]
fn late_attachment_picker_uses_latest_owned_draft_even_under_another_dialog() {
    let (mut app, _) = App::new();
    let (sender, mut queue) = crate::engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    app.load_draft(draft("current"));
    let captured = app.current_draft();
    let _ = app.handle(Message::ComposeField("subject", "Latest subject".into()));
    app.open(Dialog::Event);
    let paths = vec![std::path::PathBuf::from("fictional-picked-file.txt")];
    app.attach_chosen(captured, paths.clone());
    let Command::AddDraftFiles(saved, files) = queue.try_recv().unwrap() else {
        panic!("Expected an attachment import for its original draft");
    };
    assert_eq!(saved.id, "current");
    assert_eq!(saved.subject, "Latest subject");
    assert_eq!(saved.body.trim(), "Keep my words");
    assert_eq!(files, paths);
    assert_eq!(app.dialog, Some(Dialog::Event));
}

#[test]
fn draft_submission_retires_hidden_session_without_clearing_another_form() {
    let (mut app, _) = App::new();
    app.load_draft(draft("current"));
    app.composer.current.dirty = Some(Instant::now());
    app.open(Dialog::Event);
    let _ = app.handle(Message::Field("title", "Keep this calendar edit".into()));
    let _ = app.handle(Message::Backend(Event::SubmissionQueued(
        "current".into(),
        1,
    )));
    assert!(
        app.composer.current.dirty.is_some(),
        "Older receipts cannot retire newer edits"
    );
    let _ = app.handle(Message::Backend(Event::SubmissionQueued(
        "current".into(),
        2,
    )));
    assert!(app.composer.current.draft.id.is_empty());
    assert!(app.composer.current.dirty.is_none());
    assert_eq!(app.dialog, Some(Dialog::Event));
    assert_eq!(app.field("title"), "Keep this calendar edit");
    let _ = app.draft_saved("current".into(), 2, Err("Obsolete save".into()));
    assert!(app.composer.current.dirty.is_none());
}

#[test]
fn compose_field_changes_are_isolated_validated_and_blocked_while_sending() {
    let (mut app, _) = App::new();
    app.load_draft(draft("current"));
    let revision = app.composer.current.draft.revision;
    app.composer.current.dirty = None;
    let _ = app.handle(Message::ComposeField("to", "friend@example.com".into()));
    let _ = app.handle(Message::ComposeField(
        "password",
        "Never a draft field".into(),
    ));
    assert_eq!(app.composer.current.draft.revision, revision);
    assert!(app.composer.current.dirty.is_none());
    let _ = app.handle(Message::Field("subject", "Unrelated form state".into()));
    assert!(app.compose_field("subject").is_empty());
    app.busy.insert("send:current".into());
    let _ = app.handle(Message::ComposeField("to", "changed@example.test".into()));
    assert_eq!(app.compose_field("to"), "friend@example.com");
    app.open(Dialog::Event);
    let _ = app.handle(Message::ComposeField("to", "late@example.test".into()));
    let _ = app.handle(Message::Field("title", "Editable while sending".into()));
    assert_eq!(app.compose_field("to"), "friend@example.com");
    assert_eq!(app.field("title"), "Editable while sending");
}

#[test]
fn hidden_draft_close_waits_for_pending_save_then_the_newest_revision() {
    let (mut app, _) = App::new();
    let (sender, mut queue) = crate::engine::CommandSender::persistence_test_channel();
    app.tx = Some(sender);
    app.load_draft(draft("current"));
    let _ = app.handle(Message::ComposeField("subject", "First edit".into()));
    app.composer.current.dirty = Some(Instant::now() - std::time::Duration::from_secs(2));
    app.autosave_draft();
    let Command::AutoSaveDraft(first) = queue.try_recv().unwrap() else {
        panic!("Expected autosave")
    };
    let _ = app.handle(Message::ComposeField("subject", "Newest edit".into()));
    app.composer.current.dirty = Some(Instant::now() - std::time::Duration::from_secs(2));
    app.autosave_draft();
    assert!(
        queue.try_recv().is_err(),
        "Coalesce behind the pending save"
    );
    app.open(Dialog::Event);
    let window = iced::window::Id::unique();
    assert!(app.defer_draft_exit(Exit::Window(window)));
    assert!(
        queue.try_recv().is_err(),
        "Close observes the existing save"
    );
    let _ = app.draft_saved(
        first.id.clone(),
        first.revision,
        Ok(Arc::new(DraftState {
            revision: 1,
            drafts: vec![first],
        })),
    );
    let Command::AutoSaveDraft(newest) = queue.try_recv().unwrap() else {
        panic!("Save latest before close")
    };
    assert_eq!(newest.subject, "Newest edit");
    assert_eq!(app.composer.current.pending, Some(newest.revision));
    let _ = app.draft_saved(
        "current".into(),
        newest.revision - 1,
        Err("Late old error".into()),
    );
    assert_eq!(app.composer.current.pending, Some(newest.revision));
    assert!(app.composer.close.is_some());
    let _ = app.draft_saved("current".into(), newest.revision, Err("Disk full".into()));
    assert!(app.composer.close.is_none(), "Failure cancels close");
    assert!(app.composer.current.dirty.is_some());
    assert_eq!(app.dialog, Some(Dialog::Event));
    assert_eq!(app.current_draft().subject, "Newest edit");
    assert!(app.defer_draft_exit(Exit::Window(window)));
    assert!(matches!(queue.try_recv().unwrap(), Command::AutoSaveDraft(saved) if saved == newest));
}

#[test]
fn switching_drafts_preserves_latest_text_fields_and_pending_save_identity() {
    let (mut app, _) = App::new();
    let (sender, mut queue) = crate::engine::CommandSender::persistence_test_channel();
    app.tx = Some(sender);
    app.load_draft(draft("one"));
    let _ = app.handle(Message::ComposeField("subject", "First edited".into()));
    app.load_draft(draft("two"));
    assert_eq!(app.composer.current.draft.id, "two");
    let Command::AutoSaveDraft(first) = queue.try_recv().unwrap() else {
        panic!("Save parked draft")
    };
    let _ = app.handle(Message::ComposeField("bcc", "private@example.test".into()));
    // A late acknowledgment can update attachment metadata, never the new editor.
    let _ = app.draft_saved(
        "one".into(),
        first.revision,
        Ok(Arc::new(DraftState {
            revision: 1,
            drafts: vec![first.clone()],
        })),
    );
    assert_eq!(app.compose_field("bcc"), "private@example.test");
    app.load_draft(draft("one"));
    assert_eq!(app.compose_field("subject"), "First edited");
    assert_eq!(app.current_draft().body.trim(), "Keep my words");
    assert!(app.composer.parked.contains_key("two"));
    let Command::AutoSaveDraft(second) = queue.try_recv().unwrap() else {
        panic!("Save second parked draft")
    };
    assert_eq!(second.bcc, "private@example.test");
    let _ = app.draft_saved(
        "two".into(),
        second.revision,
        Err("Fixture disk full".into()),
    );
    assert_eq!(app.composer.current.draft.id, "one");
    assert!(app.composer.parked["two"].dirty.is_some());
    let window = iced::window::Id::unique();
    assert!(app.defer_draft_exit(Exit::Window(window)));
    assert!(
        matches!(queue.try_recv().unwrap(), Command::AutoSaveDraft(saved) if saved.id == "two")
    );
}

#[test]
fn changing_tabs_is_immediate_and_returns_to_the_owned_draft_before_save_finishes() {
    let (mut app, _) = App::new();
    let (sender, mut queue) = crate::engine::CommandSender::persistence_test_channel();
    app.tx = Some(sender);
    app.load_draft(draft("one"));
    let _ = app.handle(Message::ComposeField(
        "subject",
        "Unacknowledged edit".into(),
    ));
    let _ = app.handle(Message::Tab(Tab::Calendar));
    assert_eq!(app.tab, Tab::Calendar);
    assert!(!app.compose_visible());
    assert_eq!(app.composer.resume.as_deref(), Some("one"));
    let Command::AutoSaveDraft(saved) = queue.try_recv().unwrap() else {
        panic!("Save parked draft")
    };
    assert_eq!(app.composer.parked["one"].pending, Some(saved.revision));
    let _ = app.handle(Message::Tab(Tab::Mail));
    assert!(app.compose_visible());
    assert_eq!(app.compose_field("subject"), "Unacknowledged edit");
    assert_eq!(app.composer.current.pending, Some(saved.revision));
    let _ = app.draft_saved(
        "one".into(),
        saved.revision,
        Err("Fixture disk full".into()),
    );
    assert!(app.compose_visible());
    assert!(app.composer.current.dirty.is_some());
}

#[test]
fn folder_filter_page_and_selection_navigation_park_the_editor_without_losing_text() {
    for message in [
        Message::Folder("Archive".into()),
        Message::Account(Some("another-account".into())),
        Message::Filter(MailFilter::Unread),
        Message::Sort(MailSort::Oldest),
        Message::NextPage(true),
        Message::Starred,
        Message::ToggleSelection,
    ] {
        let (mut app, _) = App::new();
        let (sender, mut saves) = crate::engine::CommandSender::persistence_test_channel();
        app.tx = Some(sender);
        app.load_draft(draft("owned"));
        let _ = app.handle(Message::ComposeField("subject", "Unsent reply".into()));
        let _ = app.handle(message);
        assert!(!app.compose_visible());
        assert_eq!(app.owned_draft("owned").unwrap().subject, "Unsent reply");
        assert!(
            matches!(saves.try_recv().unwrap(), Command::AutoSaveDraft(saved)
            if saved.id == "owned" && saved.body.trim() == "Keep my words")
        );
    }
}

#[test]
fn late_reader_results_cannot_reopen_a_reply_or_cancel_pending_close() {
    for through_composer in [false, true] {
        let (mut app, _) = App::new();
        let mut reply = draft("reply");
        reply.reply_context = Some(ReplyContext {
            account_id: "account".into(),
            mail_id: "original".into(),
            quote: "Original message".into(),
            include_quote: true,
        });
        app.load_draft(reply);
        app.park_composer();
        app.selected = Some("original".into());
        app.composer.dismissed_for = None;
        let window = iced::window::Id::unique();
        if through_composer {
            app.composer.close = Some(window);
        } else {
            app.pending_close = Some(window);
        }
        app.restore_reply();
        assert!(!app.compose_visible());
        assert!(app.composer.parked.contains_key("reply"));
        assert_eq!(app.composer.close.or(app.pending_close), Some(window));
    }
}

#[test]
fn closing_waits_for_every_parked_draft_and_preserves_the_latest_revision() {
    let (mut app, _) = App::new();
    let (sender, mut saves) = crate::engine::CommandSender::persistence_test_channel();
    app.tx = Some(sender);
    app.bulk.stopped = true;
    for id in ["one", "two", "three", "four"] {
        app.load_draft(draft(id));
    }
    let window = iced::window::Id::unique();
    assert!(app.defer_draft_exit(Exit::Window(window)));
    let mut drafts = vec![];
    while let Ok(command) = saves.try_recv() {
        let Command::AutoSaveDraft(saved) = command else {
            panic!("Expected an autosave")
        };
        drafts.push(saved);
    }
    assert_eq!(drafts.len(), 4);
    for (i, saved) in drafts.iter().enumerate() {
        let _ = app.draft_saved(
            saved.id.clone(),
            saved.revision,
            Ok(Arc::new(DraftState {
                revision: i as u64 + 1,
                drafts: drafts.clone(),
            })),
        );
        if i < drafts.len() - 1 {
            assert_eq!(
                app.composer.close,
                Some(window),
                "An earlier saved draft must not end close"
            );
            assert!(app.composer.pending());
        }
    }
    assert!(!app.composer.pending());
    assert!(app.composer.close.is_none());
    assert!(app.pending_close.is_none());
}

#[test]
fn mailto_opens_an_owned_inline_draft_and_ignores_supplied_headers() {
    let (mut app, _) = App::new();
    app.load_draft(draft("already-open"));
    let revision = app.html_reader.generation;
    let _ = app.handle(Message::Html(html_reader::Message::Backend(crate::html_render::Event::Link(revision,
        "mailto:friend%40example.test?subject=untrusted&bcc=hidden@example.test&attachment=/private/file".into()))));
    assert!(app.compose_visible());
    assert!(app.dialog.is_none());
    assert_ne!(app.composer.current.draft.id, "already-open");
    assert!(app.composer.parked.contains_key("already-open"));
    assert_eq!(app.compose_field("to"), "friend@example.test");
    assert!(app.compose_field("subject").is_empty());
    assert!(app.compose_field("bcc").is_empty());
    assert!(app.composer.current.draft.attachments.is_empty());
    assert!(app.composer.current.dirty.is_some());
}

#[test]
fn explicit_save_waits_for_the_current_revision_and_does_not_acknowledge_other_drafts() {
    let (mut app, _) = App::new();
    let (tx, mut rx) = crate::engine::CommandSender::persistence_test_channel();
    app.tx = Some(tx);
    app.load_draft(draft("first"));
    app.close_composer();
    let Command::AutoSaveDraft(first) = rx.try_recv().unwrap() else {
        panic!("first save");
    };
    app.load_draft(draft("second"));
    app.close_composer();
    let Command::AutoSaveDraft(second) = rx.try_recv().unwrap() else {
        panic!("second save");
    };
    app.load_draft(first.clone());
    app.edit_compose_field("subject", "Latest requested save".into());
    app.save_current_draft();
    assert!(rx.try_recv().is_err());
    let _ = app.draft_saved(
        first.id.clone(),
        first.revision,
        Ok(Arc::new(DraftState {
            revision: 1,
            drafts: vec![first],
        })),
    );
    app.flush_draft_saves(true);
    let Command::SaveDraft(saved) = rx.try_recv().unwrap() else {
        panic!("explicit newest save");
    };
    assert_eq!(saved.subject, "Latest requested save");
    let _ = app.draft_saved(
        second.id.clone(),
        second.revision,
        Err("Storage unavailable".into()),
    );
    app.flush_draft_saves(true);
    assert!(
        matches!(rx.try_recv().unwrap(), Command::AutoSaveDraft(draft) if draft.id == "second")
    );
}

#[test]
fn late_reply_restore_keeps_an_explicit_selection_intact() {
    let (mut app, _) = App::new();
    let mut reply = draft("reply");
    reply.reply_context = Some(ReplyContext {
        account_id: "account".into(),
        mail_id: "original".into(),
        quote: String::new(),
        include_quote: true,
    });
    app.load_draft(reply);
    app.park_composer();
    app.composer.dismissed_for = None;
    app.selected = Some("original".into());
    app.mail_selection.mode = true;
    app.restore_reply();
    assert!(!app.compose_visible());
    assert!(app.mail_selection.mode);
}

#[test]
fn discard_waits_for_attachment_ownership_before_deleting_a_draft() {
    let (mut app, _) = App::new();
    let (tx, mut rx) = crate::engine::CommandSender::persistence_test_channel();
    app.tx = Some(tx);
    app.load_draft(draft("importing"));
    app.composer.io = Some("importing".into());
    app.review_discard_draft("importing".into());
    app.confirm_discard_draft();
    assert!(app.dialog.is_none());
    assert!(rx.try_recv().is_err());
    assert!(app.notice.as_ref().unwrap().0.contains("Finish attaching"));
    let _ = app.handle(Message::Backend(Event::DraftFiles(
        "importing".into(),
        Err("Unreadable file".into()),
    )));
    app.review_discard_draft("importing".into());
    app.confirm_discard_draft();
    assert!(matches!(rx.try_recv().unwrap(), Command::DeleteDraft(id) if id == "importing"));
}

#[test]
fn select_all_from_the_list_parks_its_reply_but_rejected_focus_keeps_the_editor() {
    let (mut app, _) = App::new();
    app.load_draft(draft("reply"));
    let _ = app.select_all_mail();
    assert!(app.compose_visible());
    assert!(!app.mail_selection.mode);
    app.list_focus = true;
    let _ = app.select_all_mail();
    assert!(!app.compose_visible());
    assert!(app.mail_selection.mode);
    assert_eq!(
        app.owned_draft("reply").unwrap().body.trim(),
        "Keep my words"
    );
}
