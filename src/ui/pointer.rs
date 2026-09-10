//! Preserve event positions before iced applies scroll/overlay coordinates.
use super::{Message, Theme};
use iced::advanced::{
    Clipboard, Layout, Overlay, Shell, layout, mouse, overlay, renderer, widget::Operation,
};
use iced::{Event, Point, Renderer, Size};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Default)]
pub(super) struct Tracker(Rc<Cell<Option<Point>>>);

impl Tracker {
    pub fn clear(&self) {
        self.0.set(None);
    }

    pub fn update(&self, event: &Event, cursor: mouse::Cursor) -> mouse::Cursor {
        match event {
            Event::Mouse(mouse::Event::CursorMoved { position }) => self.0.set(Some(*position)),
            Event::Mouse(mouse::Event::CursorLeft)
            | Event::Window(iced::window::Event::Unfocused) => self.clear(),
            _ => {}
        }
        match (cursor, self.0.get()) {
            (mouse::Cursor::Available(_), Some(position)) => mouse::Cursor::Available(position),
            // Never bypass the runtime's overlay/scrollbar exclusion.
            _ => cursor,
        }
    }

    pub fn wrap<'a>(
        &self,
        content: overlay::Element<'a, Message, Theme, Renderer>,
    ) -> overlay::Element<'a, Message, Theme, Renderer> {
        overlay::Element::new(Box::new(TrackedOverlay {
            content,
            tracker: self.clone(),
        }))
    }
}

struct TrackedOverlay<'a> {
    content: overlay::Element<'a, Message, Theme, Renderer>,
    tracker: Tracker,
}

impl Overlay<Message, Theme, Renderer> for TrackedOverlay<'_> {
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        self.content.as_overlay_mut().layout(renderer, bounds)
    }
    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.content
            .as_overlay()
            .draw(renderer, theme, style, layout, cursor);
    }
    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.content
            .as_overlay_mut()
            .operate(layout, renderer, operation);
    }
    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        let cursor = self.tracker.update(event, cursor);
        self.content
            .as_overlay_mut()
            .update(event, layout, cursor, renderer, clipboard, shell);
    }
    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content
            .as_overlay()
            .mouse_interaction(layout, cursor, renderer)
    }
    fn overlay<'a>(
        &'a mut self,
        layout: Layout<'a>,
        renderer: &Renderer,
    ) -> Option<overlay::Element<'a, Message, Theme, Renderer>> {
        let tracker = self.tracker.clone();
        self.content
            .as_overlay_mut()
            .overlay(layout, renderer)
            .map(|content| tracker.wrap(content))
    }
    fn index(&self) -> f32 {
        self.content.as_overlay().index()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CapturingPopup;
    impl Overlay<Message, Theme, Renderer> for CapturingPopup {
        fn layout(&mut self, _: &Renderer, bounds: Size) -> layout::Node {
            layout::Node::new(bounds)
        }
        fn draw(
            &self,
            _: &mut Renderer,
            _: &Theme,
            _: &renderer::Style,
            _: Layout<'_>,
            _: mouse::Cursor,
        ) {
        }
        fn update(
            &mut self,
            event: &Event,
            _: Layout<'_>,
            cursor: mouse::Cursor,
            _: &Renderer,
            _: &mut dyn Clipboard,
            shell: &mut Shell<'_, Message>,
        ) {
            if matches!(event, Event::Mouse(mouse::Event::ButtonPressed(_))) {
                shell.publish(Message::CheckMail(format!("{:?}", cursor.position())));
            }
            shell.capture_event();
        }
        fn index(&self) -> f32 {
            7.0
        }
    }

    #[test]
    fn captured_overlay_motion_survives_popup_close_and_redraw() {
        let tracker = Tracker::default();
        let first = Point::new(10., 20.);
        let second = Point::new(100., 200.);
        let final_cursor = mouse::Cursor::Available(second);
        let mut renderer = Renderer::new(iced::Font::DEFAULT, iced::Pixels(16.));
        let mut messages = Vec::new();
        {
            let mut overlay = tracker.wrap(overlay::Element::new(Box::new(CapturingPopup)));
            assert_eq!(overlay.as_overlay().index(), 7.0);
            let node = overlay
                .as_overlay_mut()
                .layout(&renderer, Size::new(500., 500.));
            for position in [first, second] {
                for event in [
                    Event::Mouse(mouse::Event::CursorMoved { position }),
                    Event::Window(iced::window::Event::RedrawRequested(
                        iced::time::Instant::now(),
                    )),
                    Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)),
                ] {
                    let mut shell = Shell::new(&mut messages);
                    overlay.as_overlay_mut().update(
                        &event,
                        Layout::new(&node),
                        final_cursor,
                        &renderer,
                        &mut iced::advanced::clipboard::Null,
                        &mut shell,
                    );
                    assert_eq!(shell.event_status(), iced::event::Status::Captured);
                    overlay.as_overlay().draw(
                        &mut renderer,
                        &Theme::Light,
                        &renderer::Style::default(),
                        Layout::new(&node),
                        final_cursor,
                    );
                }
            }
        }
        let clicks: Vec<_> = messages
            .iter()
            .filter_map(|message| match message {
                Message::CheckMail(value) => Some(value.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            clicks,
            [format!("{:?}", Some(first)), format!("{:?}", Some(second))]
        );
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        // The base did not see captured events; closing the popup still leaves
        // its latest motion available for the next click.
        assert_eq!(
            tracker.update(&press, mouse::Cursor::Available(first)),
            final_cursor
        );
        assert_eq!(
            tracker.update(&press, mouse::Cursor::Unavailable),
            mouse::Cursor::Unavailable
        );
        tracker.update(&Event::Mouse(mouse::Event::CursorLeft), final_cursor);
        assert_eq!(
            tracker
                .update(&press, mouse::Cursor::Available(first))
                .position(),
            Some(first)
        );
    }
}
