//! Reveal a keyboard target using actual layout, including compact/scaled rows.
//! A missing row is retried by the controller after the new tree is laid out.
use iced::advanced::widget::{
    Id, Operation,
    operation::{Outcome, Scrollable},
};
use iced::{Rectangle, Task, Vector};

pub(super) const SCROLLER: &str = "sidebar-folders";

struct Reveal {
    target: Id,
    viewport: Option<(Rectangle, Rectangle, Vector)>,
    row: Option<Rectangle>,
}
impl Operation<bool> for Reveal {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<bool>)) {
        operate(self);
    }
    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        if id == Some(&self.target) {
            self.row = Some(bounds);
        }
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        _: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new(SCROLLER)) {
            self.viewport = Some((bounds, content, translation));
        }
    }
    fn finish(&self) -> Outcome<bool> {
        let (Some((bounds, content, translation)), Some(row)) = (self.viewport, self.row) else {
            return Outcome::Some(false);
        };
        let top = bounds.y + translation.y;
        let desired = if row.y < top {
            row.y - bounds.y
        } else if row.y + row.height > top + bounds.height {
            row.y + row.height - bounds.y - bounds.height
        } else {
            return Outcome::Some(true);
        }
        .clamp(0., (content.height - bounds.height).max(0.));
        Outcome::Chain(Box::new(Scroll(desired)))
    }
}
struct Scroll(f32);
impl Operation<bool> for Scroll {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<bool>)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _: Rectangle,
        _: Rectangle,
        _: Vector,
        state: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new(SCROLLER)) {
            state.scroll_to(iced::widget::scrollable::AbsoluteOffset {
                x: None,
                y: Some(self.0),
            });
        }
    }
    fn finish(&self) -> Outcome<bool> {
        Outcome::Some(true)
    }
}
pub(super) fn reveal(target: String) -> Task<bool> {
    iced::advanced::widget::operate(Reveal {
        target: target.into(),
        viewport: None,
        row: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::scrollable::{AbsoluteOffset, RelativeOffset};
    #[derive(Default)]
    struct State(Option<f32>);
    impl Scrollable for State {
        fn snap_to(&mut self, _: RelativeOffset<Option<f32>>) {
            panic!("unexpected relative scroll")
        }
        fn scroll_by(&mut self, _: AbsoluteOffset, _: Rectangle, _: Rectangle) {
            panic!("unexpected relative scroll")
        }
        fn scroll_to(&mut self, value: AbsoluteOffset<Option<f32>>) {
            self.0 = value.y;
        }
    }
    #[test]
    fn reveal_preserves_visible_rows_and_scrolls_only_its_own_viewport() {
        let bounds = Rectangle {
            x: 14.,
            y: 19.,
            width: 180.,
            height: 530.,
        };
        let content = Rectangle {
            height: 1100.,
            ..bounds
        };
        for (row_y, offset, expected) in [
            (520., 0., Some(11.)),
            (29., 100., Some(10.)),
            (200., 100., None),
        ] {
            let mut reveal = Reveal {
                target: Id::new("row"),
                viewport: None,
                row: None,
            };
            let mut state = State::default();
            reveal.scrollable(
                Some(&Id::new(SCROLLER)),
                bounds,
                content,
                Vector::new(0., offset),
                &mut state,
            );
            assert!(
                matches!(reveal.finish(), Outcome::Some(false)),
                "a new row needs the next layout"
            );
            reveal.container(
                Some(&Id::new("row")),
                Rectangle {
                    y: row_y,
                    height: 40.,
                    ..bounds
                },
            );
            match reveal.finish() {
                Outcome::Chain(mut scroll) => {
                    scroll.scrollable(
                        Some(&Id::new("inbox-list")),
                        bounds,
                        content,
                        Vector::ZERO,
                        &mut state,
                    );
                    assert_eq!(state.0, None);
                    scroll.scrollable(
                        Some(&Id::new(SCROLLER)),
                        bounds,
                        content,
                        Vector::ZERO,
                        &mut state,
                    );
                    assert_eq!(state.0, expected);
                }
                Outcome::Some(true) => assert_eq!(expected, None),
                _ => panic!("expected a resolved layout"),
            }
        }
    }
}
