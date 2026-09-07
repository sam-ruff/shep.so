//! Apply a layout correction only while the native scroll and document still
//! match the renderer's snapshot. A late image cannot undo newer navigation.
use super::{Message, State};
use crate::html_render::{Frame, Reflow};
use iced::advanced::widget::{
    Id, Operation,
    operation::{Outcome, Scrollable},
};
use iced::{Rectangle, Task, Vector};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

struct Adjust {
    target: Id,
    generation: u64,
    layout: u64,
    version: Arc<AtomicU64>,
    expected_version: u64,
    body_top: f32,
    parent_width: f32,
    height: u32,
    reflow: Reflow,
    actual: Option<f32>,
}
impl Operation<crate::ui::Message> for Adjust {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<crate::ui::Message>)) {
        operate(self);
    }
    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Rectangle,
        translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        if id != Some(&self.target) || self.version.load(Ordering::Relaxed) != self.expected_version
        {
            return;
        }
        let top = (bounds.y + translation.y - self.body_top).max(0.).floor();
        self.actual = Some(top);
        if (top - self.reflow.from).abs() >= 1.
            || (bounds.width - self.parent_width).abs() >= 1.
            || bounds.height.ceil() as u32 != self.height
        {
            return;
        }
        let target = (translation.y + self.reflow.to - self.reflow.from)
            .clamp(0., (content.height - bounds.height).max(0.));
        state.scroll_to(iced::widget::scrollable::AbsoluteOffset {
            x: None,
            y: Some(target),
        });
        self.actual = Some((bounds.y + target - self.body_top).max(0.).floor());
    }
    fn finish(&self) -> Outcome<crate::ui::Message> {
        Outcome::Some(crate::ui::Message::Html(Message::ReflowApplied(
            self.generation,
            self.layout,
            self.actual,
        )))
    }
}
pub(super) fn apply(
    state: &State,
    target: &'static str,
    frame: &Frame,
) -> Task<crate::ui::Message> {
    let Some(bounds) = state.body_bounds else {
        return Task::done(crate::ui::Message::Html(Message::ReflowApplied(
            frame.generation,
            frame.layout_revision,
            None,
        )));
    };
    iced::advanced::widget::operate(Adjust {
        target: Id::new(target),
        generation: frame.generation,
        layout: frame.layout_revision,
        version: state.view_version.clone(),
        expected_version: state.view_version.load(Ordering::Relaxed),
        body_top: bounds[1],
        parent_width: state.parent_width,
        height: frame.viewport.height,
        reflow: frame.reflow.expect("reflow frame"),
        actual: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Scroll {
        moved: Option<f32>,
    }
    impl Scrollable for Scroll {
        fn snap_to(&mut self, _: iced::widget::scrollable::RelativeOffset<Option<f32>>) {
            panic!("Unexpected relative scroll")
        }
        fn scroll_to(&mut self, offset: iced::widget::scrollable::AbsoluteOffset<Option<f32>>) {
            self.moved = offset.y;
        }
        fn scroll_by(
            &mut self,
            _: iced::widget::scrollable::AbsoluteOffset,
            _: Rectangle,
            _: Rectangle,
        ) {
            panic!("Expected guarded absolute scroll")
        }
    }
    fn adjustment() -> Adjust {
        Adjust {
            target: Id::new("reader"),
            generation: 3,
            layout: 2,
            version: Arc::new(AtomicU64::new(1)),
            expected_version: 1,
            body_top: 300.,
            parent_width: 800.,
            height: 600,
            reflow: Reflow {
                from: 500.,
                to: 900.,
            },
            actual: None,
        }
    }
    fn run(adjust: &mut Adjust, translation: f32, width: f32) -> Scroll {
        let mut scroll = Scroll::default();
        adjust.scrollable(
            Some(&Id::new("reader")),
            Rectangle {
                x: 0.,
                y: 100.,
                width,
                height: 600.,
            },
            Rectangle {
                x: 0.,
                y: 100.,
                width,
                height: 3000.,
            },
            Vector::new(0., translation),
            &mut scroll,
        );
        scroll
    }
    #[test]
    fn applies_the_anchor_in_native_coordinates_and_preserves_newer_scrolls() {
        let mut adjust = adjustment();
        assert_eq!(run(&mut adjust, 700., 800.).moved, Some(1100.));
        assert_eq!(adjust.actual, Some(900.));
        let mut adjust = adjustment();
        assert_eq!(run(&mut adjust, 850., 800.).moved, None);
        assert_eq!(
            adjust.actual,
            Some(650.),
            "A newer user scroll is reported, never overwritten"
        );
    }
    #[test]
    fn changed_documents_and_resized_widgets_reject_late_adjustments() {
        let mut adjust = adjustment();
        adjust.version.store(2, Ordering::Relaxed);
        assert!(run(&mut adjust, 700., 800.).moved.is_none());
        assert!(adjust.actual.is_none());
        let mut adjust = adjustment();
        assert!(run(&mut adjust, 700., 500.).moved.is_none());
    }
}
