use super::*;
use iced::advanced::widget::operation::{self, focusable};
use iced::widget::column;

struct Native {
    element: Element<'static, Message>,
    renderer: Renderer,
    tree: Tree,
    node: layout::Node,
    messages: Vec<Message>,
}

impl Native {
    fn new(mut element: Element<'static, Message>) -> Self {
        let renderer = Renderer::new(iced::Font::DEFAULT, Pixels(16.));
        let mut tree = Tree::new(element.as_widget());
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &layout::Limits::new(Size::ZERO, Size::new(400., 300.)),
        );
        Self {
            element,
            renderer,
            tree,
            node,
            messages: vec![],
        }
    }

    fn viewport() -> Rectangle {
        Rectangle::with_size(Size::new(400., 300.))
    }

    fn event(&mut self, event: Event, cursor: Point) {
        self.element.as_widget_mut().update(
            &mut self.tree,
            &event,
            Layout::new(&self.node),
            mouse::Cursor::Available(cursor),
            &self.renderer,
            &mut iced::advanced::clipboard::Null,
            &mut Shell::new(&mut self.messages),
            &Self::viewport(),
        );
    }

    fn hover(&mut self, at: Point) {
        self.event(Event::Mouse(mouse::Event::CursorMoved { position: at }), at);
    }

    fn press(&mut self, at: Point) {
        self.event(
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
            at,
        );
    }

    fn escape(&mut self) {
        let key = keyboard::Key::Named(keyboard::key::Named::Escape);
        self.event(
            Event::Keyboard(keyboard::Event::KeyPressed {
                modified_key: key.clone(),
                key,
                physical_key: keyboard::key::Physical::Unidentified(
                    keyboard::key::NativeCode::Unidentified,
                ),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::default(),
                text: None,
                repeat: false,
            }),
            Point::ORIGIN,
        );
    }

    fn operate<T>(&mut self, mut operation: impl Operation<T>) -> Option<T> {
        self.element.as_widget_mut().operate(
            &mut self.tree,
            Layout::new(&self.node),
            &self.renderer,
            &mut operation::black_box::<T, ()>(&mut operation),
        );
        match operation.finish() {
            operation::Outcome::Some(value) => Some(value),
            _ => None,
        }
    }

    /// Runs an operation and any operation it chains to, as the runtime does.
    fn run(&mut self, operation: impl Operation<()> + 'static) {
        let mut next: Option<Box<dyn Operation<()>>> = Some(Box::new(operation));
        while let Some(mut operation) = next.take() {
            self.element.as_widget_mut().operate(
                &mut self.tree,
                Layout::new(&self.node),
                &self.renderer,
                operation.as_mut(),
            );
            if let operation::Outcome::Chain(chained) = operation.finish() {
                next = Some(chained);
            }
        }
    }

    fn focusables(&mut self) -> usize {
        self.operate(focusable::count())
            .map_or(0, |count| count.total)
    }

    fn focused(&mut self) -> Option<Id> {
        self.operate(focusable::find_focused())
    }

    fn tip_open(&mut self, viewport: Rectangle) -> bool {
        self.element
            .as_widget_mut()
            .overlay(
                &mut self.tree,
                Layout::new(&self.node),
                &self.renderer,
                &viewport,
                Vector::ZERO,
            )
            .is_some()
    }

    /// The first help icon's centre, found through its row layout.
    fn icon(&self) -> Point {
        fn find(layout: Layout<'_>) -> Option<Rectangle> {
            let bounds = layout.bounds();
            if layout.children().next().is_none()
                && bounds.width == TARGET
                && bounds.height == TARGET
            {
                return Some(bounds);
            }
            layout.children().find_map(find)
        }
        find(Layout::new(&self.node)).map_or(Point::ORIGIN, |icon| icon.center())
    }
}

fn labelled(topic: &'static Topic) -> Element<'static, Message> {
    row![text("Setting").size(13), HelpTip::new(topic)]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

#[test]
fn help_text_is_short_plain_and_uniquely_identified() {
    let mut ids = std::collections::HashSet::new();
    for topic in ALL {
        assert!(ids.insert(topic.id), "{} repeated", topic.id);
        assert!(topic.id.starts_with("help-"), "{}", topic.id);
        let length = topic.text.chars().count();
        assert!((40..=200).contains(&length), "{} is {length}", topic.id);
        assert!(topic.text.ends_with('.'), "{}", topic.id);
        for dash in ['\u{2014}', '\u{2013}'] {
            assert!(!topic.text.contains(dash), "{} has a long dash", topic.id);
        }
        for american in ["ize", "yze", "color", "behavior", "center"] {
            assert!(
                !topic.text.to_lowercase().contains(american),
                "{} uses US spelling {american}",
                topic.id
            );
        }
    }
}

#[test]
fn tips_open_below_and_stay_inside_the_window() {
    let window = Size::new(900., 640.);
    let tip = Size::new(300., 60.);
    let icon = Size::new(TARGET, TARGET);
    let anchor = Rectangle::new(Point::new(400., 100.), icon);
    let placed = place(anchor, tip, window);
    assert_eq!(placed.y, anchor.y + TARGET + GAP);
    assert_eq!(placed.center_x(), anchor.center_x());

    let right = Rectangle::new(Point::new(880., 100.), icon);
    let placed = place(right, tip, window);
    assert_eq!(placed.x + placed.width, window.width - MARGIN);

    let left = Rectangle::new(Point::new(2., 100.), icon);
    assert_eq!(place(left, tip, window).x, MARGIN);

    let bottom = Rectangle::new(Point::new(400., 600.), icon);
    let placed = place(bottom, tip, window);
    assert_eq!(placed.y + placed.height, bottom.y - GAP, "flips above");

    assert_eq!(place(bottom, Size::new(300., 700.), window).y, MARGIN);
}

#[test]
fn hovering_the_icon_or_its_margin_reveals_help_and_leaving_hides_it() {
    let mut native = Native::new(labelled(&CLOSE_TO_TRAY));
    let icon = native.icon();
    assert!(!native.tip_open(Native::viewport()));

    native.hover(icon);
    assert!(native.tip_open(Native::viewport()));

    native.hover(Point::new(icon.x + TARGET / 2. + REACH - 1., icon.y));
    assert!(
        native.tip_open(Native::viewport()),
        "within the 24px target"
    );

    native.hover(Point::new(icon.x + 40., icon.y));
    assert!(!native.tip_open(Native::viewport()));
}

#[test]
fn keyboard_focus_reveals_help_until_focus_moves_or_escape() {
    let mut native = Native::new(
        column![
            text("Mail & performance"),
            labelled(&CHECK_INTERVAL),
            labelled(&CROSS_ACCOUNT_MOVES)
        ]
        .into(),
    );
    assert_eq!(native.focusables(), 2, "each icon is a Tab stop");

    native.run(focusable::focus_next());
    assert_eq!(native.focused(), Some(Id::new(CHECK_INTERVAL.id)));
    assert!(native.tip_open(Native::viewport()));

    native.run(focusable::focus_next());
    assert_eq!(native.focused(), Some(Id::new(CROSS_ACCOUNT_MOVES.id)));

    native.escape();
    assert_eq!(native.focused(), None);
    assert!(!native.tip_open(Native::viewport()));
}

#[test]
fn clicking_pins_help_and_clicking_elsewhere_releases_it() {
    let mut native = Native::new(labelled(&BACKUP_ENCRYPTION));
    let icon = native.icon();
    let away = Point::new(350., 250.);
    native.press(icon);
    native.hover(away);
    assert!(
        native.tip_open(Native::viewport()),
        "pinned after the pointer leaves"
    );

    native.press(away);
    assert!(!native.tip_open(Native::viewport()));

    native.press(icon);
    native.press(icon);
    native.hover(away);
    assert!(
        !native.tip_open(Native::viewport()),
        "a second click unpins it"
    );
}

#[test]
fn scrolled_away_icons_keep_their_tip_hidden() {
    let mut native = Native::new(labelled(&BACKUP_COMPRESSION));
    native.press(native.icon());
    let below = Rectangle::new(Point::new(0., 500.), Size::new(400., 100.));
    assert!(!native.tip_open(below));
    assert!(native.tip_open(Native::viewport()));
}

#[test]
fn the_mark_is_smaller_than_its_target_and_raised_to_the_top() {
    let target = Rectangle::new(Point::new(100., 40.), Size::new(TARGET, TARGET));
    let mark = mark_bounds(target);
    assert_eq!(mark.size(), Size::new(MARK, MARK));
    assert!(mark.width < target.width);
    assert_eq!(mark.center_x(), target.center_x());
    assert!(
        mark.center_y() < target.center_y(),
        "raised like a footnote"
    );
    let ring = mark.expand(RING_GAP);
    assert!(ring.y >= target.y && ring.y + ring.height <= target.y + TARGET);
}

#[tokio::test]
async fn the_help_icon_setting_alone_removes_the_icons_and_their_tab_stops() {
    let (mut app, _) = App::new();
    assert!(app.preferences.help_icons);
    let mut shown = Native::new(app.with_help(text("Setting"), &SYNCED_PASSWORDS));
    assert_eq!(shown.focusables(), 1);
    shown.hover(shown.icon());
    assert!(shown.tip_open(Native::viewport()));

    app.preferences.tooltips = false;
    let mut kept = Native::new(app.with_help(text("Setting"), &SYNCED_PASSWORDS));
    assert_eq!(kept.focusables(), 1, "icon tooltips do not control help");
    kept.hover(kept.icon());
    assert!(kept.tip_open(Native::viewport()));

    app.preferences.tooltips = true;
    app.preferences.help_icons = false;
    let mut hidden = Native::new(app.with_help(text("Setting"), &SYNCED_PASSWORDS));
    assert_eq!(hidden.focusables(), 0);
    assert!(!hidden.tip_open(Native::viewport()));
}
