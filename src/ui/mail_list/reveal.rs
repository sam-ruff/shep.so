//! Reveal a virtual row using the native viewport, guarded by its current scope
//! and selection. Delayed operations cannot scroll a newer selection or folder.
use super::super::{App, Message};
use super::ROW_HEIGHT;
use iced::advanced::widget::{
    Id, Operation,
    operation::{Outcome, Scrollable},
};
use iced::{Rectangle, Task, Vector};

#[derive(Clone, Debug)]
pub struct Revealed {
    pub context: String,
    pub offset: f32,
    pub height: f32,
}

struct Reveal {
    context: String,
    index: usize,
    current: bool,
    result: Option<Revealed>,
}
impl Operation<Message> for Reveal {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Message>)) {
        operate(self);
    }
    fn container(&mut self, id: Option<&Id>, _: Rectangle) {
        if id == Some(&Id::from(self.context.clone())) {
            self.current = true;
        }
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        if !self.current || id != Some(&Id::new("inbox-list")) {
            return;
        }
        let top = self.index as f32 * ROW_HEIGHT;
        let offset = if top < translation.y {
            top
        } else if top + ROW_HEIGHT > translation.y + bounds.height {
            top + ROW_HEIGHT - bounds.height
        } else {
            translation.y
        }
        .clamp(0., (content.height - bounds.height).max(0.));
        state.scroll_to(iced::widget::scrollable::AbsoluteOffset {
            x: None,
            y: Some(offset),
        });
        self.result = Some(Revealed {
            context: self.context.clone(),
            offset,
            height: bounds.height,
        });
    }
    fn finish(&self) -> Outcome<Message> {
        self.result
            .clone()
            .map(|result| Outcome::Some(Message::InboxRevealed(result)))
            .unwrap_or(Outcome::None)
    }
}
impl App {
    pub(in crate::ui) fn inbox_context(&self) -> String {
        format!(
            "inbox-context/{}/{}/{:?}",
            self.list_revision,
            self.selected.as_deref().unwrap_or(""),
            self.page
                .rows
                .iter()
                .position(|mail| Some(&mail.id) == self.selected.as_ref())
        )
    }
    pub(in crate::ui) fn reveal_inbox_selection(&self, index: usize) -> Task<Message> {
        iced::advanced::widget::operate(Reveal {
            context: self.inbox_context(),
            index,
            current: false,
            result: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::scrollable::{AbsoluteOffset, RelativeOffset};
    #[derive(Default)]
    struct Scroll(Option<f32>);
    impl Scrollable for Scroll {
        fn snap_to(&mut self, _: RelativeOffset<Option<f32>>) {
            panic!("unexpected snap");
        }
        fn scroll_by(&mut self, _: AbsoluteOffset, _: Rectangle, _: Rectangle) {
            panic!("unexpected relative scroll");
        }
        fn scroll_to(&mut self, offset: AbsoluteOffset<Option<f32>>) {
            self.0 = offset.y;
        }
    }
    #[test]
    fn actual_viewport_reveals_complete_rows_and_preserves_visible_selection() {
        for (index, offset, height, expected) in [
            (12, 80., 668., 112.),
            (0, 112., 668., 0.),
            (5, 80., 668., 80.),
            (12, 80., 370., 410.),
            (49, 0., 668., 2332.),
        ] {
            let bounds = Rectangle {
                x: 275.,
                y: 194.,
                width: 350.,
                height,
            };
            let content = Rectangle {
                height: 3000.,
                ..bounds
            };
            let mut reveal = Reveal {
                context: "current".into(),
                index,
                current: false,
                result: None,
            };
            let mut scroll = Scroll::default();
            reveal.container(Some(&Id::new("current")), bounds);
            reveal.scrollable(
                Some(&Id::new("inbox-list")),
                bounds,
                content,
                Vector::new(0., offset),
                &mut scroll,
            );
            assert_eq!(scroll.0, Some(expected));
            let result = reveal.result.unwrap();
            assert_eq!(result.height, height);
            assert!(index as f32 * ROW_HEIGHT >= result.offset);
            assert!((index + 1) as f32 * ROW_HEIGHT <= result.offset + height);
        }
    }
    #[test]
    fn stale_scope_selection_and_other_scrollers_are_never_moved() {
        for (marker, scroller) in [
            ("new selection", "inbox-list"),
            ("current", "sidebar-folders"),
        ] {
            let mut reveal = Reveal {
                context: "current".into(),
                index: 20,
                current: false,
                result: None,
            };
            let mut scroll = Scroll::default();
            let bounds = Rectangle::new(iced::Point::ORIGIN, iced::Size::new(350., 400.));
            reveal.container(Some(&Id::new(marker)), bounds);
            reveal.scrollable(
                Some(&Id::new(scroller)),
                bounds,
                Rectangle {
                    height: 3000.,
                    ..bounds
                },
                Vector::ZERO,
                &mut scroll,
            );
            assert_eq!(scroll.0, None);
            assert!(matches!(reveal.finish(), Outcome::None));
        }
    }
}
