use super::super::context_menu::ContextArea;
use super::*;
use crate::store::{MailSelectionId, Store};
use iced::advanced::{Layout, Shell, Widget, layout, mouse, widget::Tree};
use iced::{Point, Rectangle, Renderer};

async fn fixture() -> (App, Store, tokio::sync::mpsc::Receiver<Command>) {
    let store = Store::memory().unwrap();
    for id in ["a", "b"] {
        let account:Account=serde_json::from_value(serde_json::json!({"id":id,"name":id,"email":format!("{id}@example.test"),"protocol":"Imap","host":"imap.example.test","port":993,"username":id,"smtp_host":"smtp.example.test","smtp_port":465})).unwrap();
        store.save_account(account).await.unwrap();
        store
            .save_folders(
                id.into(),
                vec!["INBOX".into(), "Archive".into(), "Projects".into()],
            )
            .await
            .unwrap();
        store
            .upsert(vec![
                parse_mail(
                    id,
                    "1",
                    "INBOX",
                    format!("Subject: {id}\r\n\r\nBody {id}").into_bytes(),
                    true,
                    false,
                )
                .unwrap(),
            ])
            .await
            .unwrap();
    }
    let (sender, commands) = engine::CommandSender::selection_test_channel();
    let (mut app, _) = App::new();
    app.tx = Some(sender);
    app.workspace = Arc::new(store.workspace().await.unwrap());
    app.page = Arc::new(store.query(MailQuery::default()).await.unwrap());
    app.mail_actions.base_page = app.page.clone();
    app.selected = Some(app.page.rows[0].id.clone());
    (app, store, commands)
}
fn target(account: Option<&str>, folder: &str) -> Target {
    Target {
        account: account.map(str::to_owned),
        folder: folder.into(),
    }
}
async fn select_all(app: &mut App, store: &Store) -> Arc<crate::store::SelectionSnapshot> {
    let snapshot = Arc::new(
        store
            .capture_selection(
                MailSelectionId::default(),
                0,
                MailQuery::default(),
                true,
                app.page.rows.iter().map(|m| m.id.clone()).collect(),
            )
            .await
            .unwrap(),
    );
    app.mail_selection.mode = true;
    app.mail_selection.count = snapshot.selected;
    app.mail_selection.visible = snapshot.visible.clone();
    app.mail_selection.snapshot = Some(snapshot.clone());
    snapshot
}
#[tokio::test]
async fn destination_rules_cover_mixed_accounts_missing_folders_pop3_and_inbox_aliases() {
    let (mut app, store, _) = fixture().await;
    assert!(app.sidebar_drop_target(&Message::SentFolder).is_none());
    assert!(app.sidebar_drop_target(&Message::Starred).is_none());
    assert_eq!(
        app.sidebar_drop_target(&Message::AccountFolder("a".into(), "Sent Items".into())),
        Some(target(Some("a"), "Sent Items"))
    );
    let snapshot = select_all(&mut app, &store).await;
    let group = Payload::Group(snapshot);
    assert!(
        app.drag_rules()
            .check(&group, &target(None, "Archive"))
            .is_ok()
    );
    assert!(
        app.drag_rules()
            .check(&group, &target(None, "Inbox"))
            .unwrap_err()
            .contains("already")
    );
    assert!(
        app.drag_rules()
            .check(&group, &target(Some("b"), "Projects"))
            .unwrap_err()
            .contains("Preferences")
    );
    app.preferences.cross_account_moves = true;
    assert!(
        app.drag_rules()
            .check(&group, &target(Some("b"), "Projects"))
            .is_ok()
    );
    assert!(
        app.drag_rules()
            .check(&group, &target(Some("b"), "Missing"))
            .is_err()
    );
    Arc::make_mut(&mut app.workspace)
        .accounts
        .iter_mut()
        .find(|a| a.id == "a")
        .unwrap()
        .protocol = Protocol::Pop3;
    assert!(
        app.drag_rules()
            .check(&group, &target(Some("b"), "Projects"))
            .unwrap_err()
            .contains("IMAP")
    );
    assert!(
        app.drag_rules()
            .check(&group, &target(None, "Archive"))
            .is_ok(),
        "POP3 supports moves within its local account"
    );
}
#[tokio::test]
async fn drop_uses_dragged_metadata_while_another_body_is_open_and_projects_immediately() {
    let (mut app, _, _) = fixture().await;
    let (sender, mut commands) = engine::CommandSender::network_test_channel();
    app.tx = Some(sender);
    let source = app.page.rows[1].clone();
    let selected = app.selected.clone();
    app.drop_mail(
        Arc::new(Payload::Single(Box::new(source.clone()))),
        Some(target(None, "Archive")),
    );
    assert_eq!(app.selected, selected);
    assert_eq!(app.page.total, 1);
    assert!(app.page.rows.iter().all(|m| m.id != source.id));
    assert_eq!(
        app.action_toasts.current.as_ref().unwrap().label(),
        "Archived 1 message"
    );
    assert!(
        matches!(commands.try_recv().unwrap(),Command::Move(_,mail,folder) if mail.id==source.id && folder=="Archive")
    );
}
#[tokio::test]
async fn cancelled_rejected_and_stale_drops_preserve_mail_and_selection() {
    let (mut app, store, mut commands) = fixture().await;
    let snapshot = select_all(&mut app, &store).await;
    let payload = Arc::new(Payload::Group(snapshot));
    app.drop_mail(payload.clone(), None);
    app.drop_mail(payload.clone(), Some(target(Some("b"), "Archive")));
    assert_eq!(app.page.total, 2);
    assert_eq!(app.mail_selection.count, 2);
    assert!(commands.try_recv().is_err());
    app.preferences.cross_account_moves = true;
    Arc::make_mut(app.mail_selection.snapshot.as_mut().unwrap()).revision += 1;
    app.drop_mail(payload, Some(target(Some("b"), "Archive")));
    assert!(app.notice.as_ref().unwrap().0.contains("selection changed"));
    assert!(commands.try_recv().is_err());
    let mail = app.page.rows[0].clone();
    Arc::make_mut(&mut app.page)
        .rows
        .retain(|m| m.id != mail.id);
    app.drop_mail(
        Arc::new(Payload::Single(Box::new(mail))),
        Some(target(None, "Archive")),
    );
    assert!(app.notice.as_ref().unwrap().0.contains("message changed"));
    assert!(commands.try_recv().is_err());
}
#[tokio::test]
async fn group_drop_freezes_exact_membership_and_uses_the_existing_confirmation() {
    let (mut app, store, mut commands) = fixture().await;
    let snapshot = select_all(&mut app, &store).await;
    app.preferences.cross_account_moves = true;
    app.drop_mail(
        Arc::new(Payload::Group(snapshot.clone())),
        Some(target(Some("b"), "Projects")),
    );
    assert_eq!(app.dialog, Some(Dialog::BulkReview));
    assert!(
        commands.try_recv().is_err(),
        "Dropping must not bypass the review"
    );
    app.pump_bulk();
    let Command::ReviewSelection(serial, id, revision, _) = commands.try_recv().unwrap() else {
        panic!("Expected review");
    };
    assert_eq!((id, revision), (snapshot.id, snapshot.revision));
    let review = Arc::new(store.freeze_selection(id, revision).await.unwrap());
    app.bulk_event(Event::BulkReview(serial, Ok(review.clone())));
    let _ = app.handle_bulk(bulk::Message::Confirm);
    let Command::BulkStart(_, id, crate::bulk::Action::Move { account, folder }) =
        commands.try_recv().unwrap()
    else {
        panic!("Expected confirmed group");
    };
    assert_eq!(id, review.id);
    assert_eq!(account.as_deref(), Some("b"));
    assert_eq!(folder, "Projects");
    assert_eq!(review.selected, 2);
    assert!(app.action_toasts.current.is_some());
}

fn widget_tree(handle: Handle, payload: Arc<Payload>, rules: Rules) -> ContextArea<'static> {
    use iced::widget::{button, column, row};
    let source = ContextArea::new(
        button(
            row![
                "Message",
                ContextArea::sidebar(
                    button("Flag")
                        .width(60)
                        .on_press(Message::FlagRow("id".into()))
                )
                .with_drag(Region::Block(handle.clone()))
            ]
            .spacing(40),
        )
        .width(200)
        .height(50)
        .on_press(Message::Select("id".into())),
        "id".into(),
    )
    .with_drag(Region::Source(handle.clone(), Some(payload)));
    let destination = ContextArea::sidebar(
        button("Archive")
            .width(200)
            .height(50)
            .on_press(Message::SidebarAction(0)),
    )
    .with_drag(Region::Target(
        handle.clone(),
        target(None, "Archive"),
        rules.clone(),
    ));
    ContextArea::root(column![source, destination].spacing(20), 100)
        .with_drag(Region::Root(handle, true, rules))
}
fn input(
    area: &mut ContextArea<'_>,
    tree: &mut Tree,
    renderer: &Renderer,
    node: &layout::Node,
    event: iced::Event,
    messages: &mut Vec<Message>,
) {
    // Simulate iced's final cursor for an entire input batch, including a
    // redraw after each movement. Root tracking must retain the actual origin.
    area.update(
        tree,
        &event,
        Layout::new(node),
        mouse::Cursor::Available(Point::new(20., 90.)),
        renderer,
        &mut iced::advanced::clipboard::Null,
        &mut Shell::new(messages),
        &Rectangle::with_size(Size::new(220., 180.)),
    );
}
#[tokio::test]
async fn native_widget_drag_survives_redraws_and_does_not_click_the_source_or_destination() {
    let (app, _, _) = fixture().await;
    let payload = Arc::new(Payload::Single(Box::new(app.page.rows[0].clone())));
    let handle = Handle::default();
    let mut area = widget_tree(handle.clone(), payload, app.drag_rules());
    let renderer = Renderer::new(iced::Font::DEFAULT, 16.into());
    let mut tree = Tree::new(&area as &dyn Widget<Message, Theme, Renderer>);
    let node = area.layout(
        &mut tree,
        &renderer,
        &layout::Limits::new(Size::ZERO, Size::new(220., 180.)),
    );
    let mut messages = Vec::new();
    for event in [
        iced::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(20., 20.),
        }),
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
        iced::Event::Mouse(mouse::Event::CursorMoved {
            position: Point::new(20., 90.),
        }),
        iced::Event::Window(iced::window::Event::RedrawRequested(Instant::now())),
        iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
    ] {
        input(&mut area, &mut tree, &renderer, &node, event, &mut messages);
    }
    assert_eq!(
        messages
            .iter()
            .filter(|m| matches!(m,Message::DropMail(_,Some(t)) if t.folder=="Archive"))
            .count(),
        1
    );
    assert!(
        messages
            .iter()
            .all(|m| !matches!(m, Message::SelectClick(..) | Message::SidebarClick(..)))
    );
    assert!(!handle.holding());
}
#[tokio::test]
async fn nested_flag_and_small_pointer_jitter_never_start_a_drag() {
    let (app, _, _) = fixture().await;
    for (origin, end) in [
        (Point::new(150., 20.), Point::new(20., 90.)),
        (Point::new(20., 20.), Point::new(22., 21.)),
    ] {
        let handle = Handle::default();
        let mut area = widget_tree(
            handle.clone(),
            Arc::new(Payload::Single(Box::new(app.page.rows[0].clone()))),
            app.drag_rules(),
        );
        let renderer = Renderer::new(iced::Font::DEFAULT, 16.into());
        let mut tree = Tree::new(&area as &dyn Widget<Message, Theme, Renderer>);
        let node = area.layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(220., 180.)),
        );
        let mut messages = Vec::new();
        for event in [
            iced::Event::Mouse(mouse::Event::CursorMoved { position: origin }),
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::CursorMoved { position: end }),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ] {
            input(&mut area, &mut tree, &renderer, &node, event, &mut messages);
        }
        assert!(messages.iter().all(|m| !matches!(m, Message::DropMail(..))));
        if end.y < 50. {
            assert!(
                messages
                    .iter()
                    .any(|m| matches!(m, Message::SelectClick(..)))
            );
        }
        assert!(!handle.holding());
    }
}

#[tokio::test]
async fn cancelling_at_window_edges_focus_loss_or_right_click_does_not_become_a_row_click() {
    let (app, _, _) = fixture().await;
    for cancellation in [
        iced::Event::Mouse(mouse::Event::CursorLeft),
        iced::Event::Window(iced::window::Event::Unfocused),
        iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Right)),
    ] {
        let handle = Handle::default();
        let mut area = widget_tree(
            handle.clone(),
            Arc::new(Payload::Single(Box::new(app.page.rows[0].clone()))),
            app.drag_rules(),
        );
        let renderer = Renderer::new(iced::Font::DEFAULT, 16.into());
        let mut tree = Tree::new(&area as &dyn Widget<Message, Theme, Renderer>);
        let node = area.layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(220., 180.)),
        );
        let mut messages = Vec::new();
        for event in [
            iced::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(20., 20.),
            }),
            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            iced::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(20., 30.),
            }),
            cancellation,
            iced::Event::Mouse(mouse::Event::CursorMoved {
                position: Point::new(20., 20.),
            }),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)),
        ] {
            input(&mut area, &mut tree, &renderer, &node, event, &mut messages);
        }
        assert!(messages.iter().all(|m| !matches!(
            m,
            Message::DropMail(..)
                | Message::SelectClick(..)
                | Message::SidebarClick(..)
                | Message::MailContext(..)
        )));
        assert!(!handle.holding());
    }
}
