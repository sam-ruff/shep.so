//! Reveal one Preferences control by its rendered caption, using actual layout.
//! A caption that is not laid out yet is retried by the caller.
use iced::advanced::widget::{
    Id, Operation,
    operation::{Focusable, Outcome, Scrollable, TextInput},
};
use iced::{Rectangle, Task, Vector};

pub(in crate::ui) const SCROLLER: &str = "settings-scroll";
/// Space kept above a revealed caption so it does not touch the viewport edge.
const MARGIN: f32 = 16.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Found {
    Missing,
    /// Window-space top of the caption after scrolling, and whether the text
    /// field it labels received keyboard focus.
    Revealed {
        top: f32,
        focused: bool,
    },
}

/// Largest gap between a caption and the text field it labels from above.
const FIELD_GAP: f32 = 24.;

struct Locate {
    caption: &'static str,
    viewport: Option<(Rectangle, Rectangle, Vector)>,
    caption_bounds: Option<Rectangle>,
    /// Another caption was laid out after ours, so a field below is not ours.
    intervened: bool,
    /// The first text field after the caption has been considered.
    field_checked: bool,
    input: Option<Rectangle>,
}

impl Locate {
    fn inside(&self, bounds: Rectangle) -> bool {
        self.viewport.is_some_and(|(_, content, _)| {
            bounds.y >= content.y && bounds.y + bounds.height <= content.y + content.height
        })
    }
    /// A field labelled by this caption shares its row, or sits directly below it.
    fn labels(&self, caption: Rectangle, field: Rectangle) -> bool {
        let same_row = field.y < caption.y + caption.height && caption.y < field.y + field.height;
        let below = field.y >= caption.y + caption.height
            && field.y - (caption.y + caption.height) <= FIELD_GAP;
        same_row || (!self.intervened && below)
    }
}

impl Operation<Found> for Locate {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Found>)) {
        operate(self);
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
    fn text(&mut self, _: Option<&Id>, bounds: Rectangle, text: &str) {
        if self.caption_bounds.is_some() {
            self.intervened = true;
            return;
        }
        if text.trim() == self.caption && self.inside(bounds) {
            self.caption_bounds = Some(bounds);
        }
    }
    fn text_input(&mut self, _: Option<&Id>, bounds: Rectangle, _: &mut dyn TextInput) {
        let Some(caption) = self.caption_bounds else {
            return;
        };
        if self.field_checked {
            return;
        }
        self.field_checked = true;
        if self.labels(caption, bounds) {
            self.input = Some(bounds);
        }
    }
    fn finish(&self) -> Outcome<Found> {
        let (Some((bounds, content, translation)), Some(caption)) =
            (self.viewport, self.caption_bounds)
        else {
            return Outcome::Some(Found::Missing);
        };
        let target = self.input.map_or(caption, |input| caption.union(&input));
        let top = bounds.y + translation.y;
        let offset = if target.y >= top && target.y + target.height <= top + bounds.height {
            translation.y
        } else {
            (target.y - bounds.y - MARGIN).clamp(0., (content.height - bounds.height).max(0.))
        };
        Outcome::Chain(Box::new(Apply {
            offset,
            input: self.input,
            top: caption.y - offset,
            focused: false,
        }))
    }
}

struct Apply {
    offset: f32,
    input: Option<Rectangle>,
    top: f32,
    focused: bool,
}

impl Operation<Found> for Apply {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Found>)) {
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
                y: Some(self.offset),
            });
        }
    }
    fn focusable(&mut self, _: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        let Some(input) = self.input else {
            return;
        };
        if bounds == input && !self.focused {
            state.focus();
            self.focused = true;
        } else {
            state.unfocus();
        }
    }
    fn finish(&self) -> Outcome<Found> {
        Outcome::Some(Found::Revealed {
            top: self.top,
            focused: self.focused,
        })
    }
}

pub(super) fn reveal(caption: &'static str) -> Task<Found> {
    iced::advanced::widget::operate(Locate {
        caption,
        viewport: None,
        caption_bounds: None,
        intervened: false,
        field_checked: false,
        input: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::widget::scrollable::{AbsoluteOffset, RelativeOffset};

    #[derive(Default)]
    struct Scroll(Option<f32>);
    impl Scrollable for Scroll {
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
    #[derive(Default)]
    struct Field(bool);
    impl Focusable for Field {
        fn is_focused(&self) -> bool {
            self.0
        }
        fn focus(&mut self) {
            self.0 = true;
        }
        fn unfocus(&mut self) {
            self.0 = false;
        }
    }
    impl TextInput for Field {
        fn text(&self) -> &str {
            ""
        }
        fn move_cursor_to_front(&mut self) {}
        fn move_cursor_to_end(&mut self) {}
        fn move_cursor_to(&mut self, _: usize) {}
        fn select_all(&mut self) {}
        fn select_range(&mut self, _: usize, _: usize) {}
    }

    const VIEWPORT: Rectangle = Rectangle {
        x: 30.,
        y: 200.,
        width: 900.,
        height: 400.,
    };
    const CONTENT: Rectangle = Rectangle {
        x: 30.,
        y: 200.,
        width: 900.,
        height: 2000.,
    };
    fn row(y: f32) -> Rectangle {
        Rectangle {
            x: 60.,
            y,
            width: 200.,
            height: 20.,
        }
    }
    fn locate(caption: &'static str) -> Locate {
        Locate {
            caption,
            viewport: None,
            caption_bounds: None,
            intervened: false,
            field_checked: false,
            input: None,
        }
    }
    fn run(mut chain: Box<dyn Operation<Found>>, fields: &mut [(Rectangle, Field)]) -> Scroll {
        let mut scroll = Scroll::default();
        chain.scrollable(
            Some(&Id::new("other")),
            VIEWPORT,
            CONTENT,
            Vector::ZERO,
            &mut scroll,
        );
        assert_eq!(scroll.0, None, "only the Preferences scroller moves");
        chain.scrollable(
            Some(&Id::new(SCROLLER)),
            VIEWPORT,
            CONTENT,
            Vector::ZERO,
            &mut scroll,
        );
        for (bounds, field) in fields.iter_mut() {
            chain.focusable(None, *bounds, field);
        }
        scroll
    }

    #[test]
    fn caption_below_the_viewport_scrolls_and_focuses_its_own_field() {
        let mut op = locate("Copies to keep (1–100)");
        let mut scroll = Scroll::default();
        let mut target = Field::default();
        op.text(None, row(20.), "Copies to keep (1–100)");
        assert!(
            op.caption_bounds.is_none(),
            "header text is outside the scroller"
        );
        op.scrollable(
            Some(&Id::new(SCROLLER)),
            VIEWPORT,
            CONTENT,
            Vector::new(0., 100.),
            &mut scroll,
        );
        op.text(None, row(1200.), "Copies to keep (1–100)");
        op.text_input(None, row(1228.), &mut target);
        let Outcome::Chain(chain) = op.finish() else {
            panic!("expected a scroll")
        };
        let mut fields = [(row(900.), Field(true)), (row(1228.), Field::default())];
        let scroll = run(chain, &mut fields);
        assert_eq!(scroll.0, Some(1200. - 200. - MARGIN));
        assert!(!fields[0].1.0 && fields[1].1.0);
    }

    #[test]
    fn visible_caption_keeps_its_offset_and_a_later_caption_blocks_focus() {
        let mut op = locate("Compress copies");
        let mut scroll = Scroll::default();
        let mut field = Field::default();
        op.scrollable(
            Some(&Id::new(SCROLLER)),
            VIEWPORT,
            CONTENT,
            Vector::new(0., 50.),
            &mut scroll,
        );
        op.text(None, row(400.), "Compress copies");
        op.text(None, row(430.), "Backup passphrase");
        op.text_input(None, row(436.), &mut field);
        assert_eq!(op.input, None);
        let Outcome::Chain(chain) = op.finish() else {
            panic!("expected a resolved caption")
        };
        let mut fields = [(row(436.), Field(true))];
        let scroll = run(chain, &mut fields);
        assert_eq!(scroll.0, Some(50.));
        assert!(fields[0].1.0, "no field is focused, so focus stays put");
    }

    #[test]
    fn field_on_the_caption_row_is_focused_after_its_help_text() {
        let mut op = locate("Check for new mail");
        let mut scroll = Scroll::default();
        let mut field = Field::default();
        op.scrollable(
            Some(&Id::new(SCROLLER)),
            VIEWPORT,
            CONTENT,
            Vector::ZERO,
            &mut scroll,
        );
        op.text(None, row(300.), "Check for new mail");
        op.text(None, row(322.), "Seconds between background checks");
        let beside = Rectangle {
            x: 700.,
            y: 296.,
            width: 90.,
            height: 40.,
        };
        op.text_input(None, beside, &mut field);
        op.text_input(None, row(344.), &mut field);
        assert_eq!(op.input, Some(beside));
    }

    #[test]
    fn missing_caption_or_scroller_waits_for_the_next_layout() {
        let mut op = locate("Print");
        op.text(None, row(300.), "Print");
        assert!(matches!(op.finish(), Outcome::Some(Found::Missing)));
        let mut scroll = Scroll::default();
        op.scrollable(
            Some(&Id::new(SCROLLER)),
            VIEWPORT,
            CONTENT,
            Vector::ZERO,
            &mut scroll,
        );
        op.text(None, row(300.), "Printer");
        assert!(matches!(op.finish(), Outcome::Some(Found::Missing)));
    }
}
