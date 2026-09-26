//! Scroll a keyboard-focused control into its scroller's view, using actual
//! layout bounds, after Tab moves native focus.
use iced::Task;
use iced::advanced::widget::{
    Id, Operation,
    operation::{Focusable, Outcome, Scrollable},
};
use iced::widget::scrollable::AbsoluteOffset;
use iced::{Rectangle, Vector};

struct Find {
    scroller: Id,
    viewport: Option<(Rectangle, Rectangle, Vector)>,
    focused: Option<Rectangle>,
}
impl Operation for Find {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
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
        if id == Some(&self.scroller) {
            self.viewport = Some((bounds, content, translation));
        }
    }
    fn focusable(&mut self, _: Option<&Id>, bounds: Rectangle, state: &mut dyn Focusable) {
        if state.is_focused() {
            self.focused = Some(bounds);
        }
    }
    fn finish(&self) -> Outcome<()> {
        let (Some((bounds, content, translation)), Some(target)) = (self.viewport, self.focused)
        else {
            return Outcome::None;
        };
        match offset(bounds, content, translation, target) {
            Some(y) => Outcome::Chain(Box::new(Scroll {
                scroller: self.scroller.clone(),
                y,
            })),
            None => Outcome::None,
        }
    }
}

/// The vertical offset that shows `target`, or `None` when it is already
/// visible or lies outside this scroller's content.
fn offset(
    bounds: Rectangle,
    content: Rectangle,
    translation: Vector,
    target: Rectangle,
) -> Option<f32> {
    if !content.intersects(&target) {
        return None;
    }
    const MARGIN: f32 = 12.;
    let top = bounds.y + translation.y;
    let desired = if target.y - MARGIN < top {
        target.y - MARGIN - bounds.y
    } else if target.y + target.height + MARGIN > top + bounds.height {
        target.y + target.height + MARGIN - bounds.y - bounds.height
    } else {
        return None;
    };
    Some(desired.clamp(0., (content.height - bounds.height).max(0.)))
}

struct Scroll {
    scroller: Id,
    y: f32,
}
impl Operation for Scroll {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
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
        if id == Some(&self.scroller) {
            state.scroll_to(AbsoluteOffset {
                x: None,
                y: Some(self.y),
            });
        }
    }
}

pub(super) fn reveal<T: Send + 'static>(scroller: &'static str) -> Task<T> {
    iced::advanced::widget::operate(Find {
        scroller: Id::new(scroller),
        viewport: None,
        focused: None,
    })
    .discard()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(y: f32, height: f32) -> Rectangle {
        Rectangle {
            x: 0.,
            y,
            width: 400.,
            height,
        }
    }

    #[test]
    fn visible_targets_stay_put_and_hidden_ones_scroll_within_bounds() {
        let bounds = rect(100., 400.);
        let content = rect(100., 1500.);
        let at = |scrolled: f32, target: Rectangle| {
            offset(bounds, content, Vector::new(0., scrolled), target)
        };
        assert_eq!(at(0., rect(300., 16.)), None);
        assert_eq!(at(0., rect(700., 16.)), Some(228.));
        assert_eq!(at(500., rect(400., 16.)), Some(288.));
        assert_eq!(at(0., rect(1590., 16.)), Some(1100.));
        assert_eq!(at(0., rect(1700., 16.)), None, "outside this scroller");
    }
}
