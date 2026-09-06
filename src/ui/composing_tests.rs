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
