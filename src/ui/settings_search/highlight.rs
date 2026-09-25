//! A brief accent outline over a revealed Preferences control. It is drawn in
//! its own layer above the scrolled content, never captures input and asks to
//! be dismissed on the next click, key press or wheel scroll.
use crate::ui::{Message, components::colors};
use iced::advanced::Renderer as _;
use iced::advanced::{Clipboard, Layout, Shell, Widget, layout, mouse, renderer, widget::Tree};
use iced::{Border, Color, Element, Event, Length, Rectangle, Renderer, Size, Theme, Vector};

/// How long the outline stays without further input.
pub(in crate::ui) const DURATION: std::time::Duration = std::time::Duration::from_millis(1800);
const WIDTH: f32 = 2.;
/// Space between the control and the outline.
const GAP: f32 = 4.;

pub(in crate::ui) struct Outline {
    /// Control bounds relative to the scrolled content.
    pub bounds: Rectangle,
    pub dismiss: Message,
}

pub(super) fn dismisses(event: &Event) -> bool {
    matches!(
        event,
        Event::Mouse(mouse::Event::ButtonPressed(_) | mouse::Event::WheelScrolled { .. })
            | Event::Keyboard(iced::keyboard::Event::KeyPressed { .. })
    )
}

/// The outline rectangle in the layer's coordinates.
pub(super) fn frame(bounds: Rectangle, origin: Vector) -> Rectangle {
    Rectangle {
        x: bounds.x + origin.x - GAP,
        y: bounds.y + origin.y - GAP,
        width: bounds.width + 2. * GAP,
        height: bounds.height + 2. * GAP,
    }
}

impl Widget<Message, Theme, Renderer> for Outline {
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }
    fn layout(&mut self, _: &mut Tree, _: &Renderer, limits: &layout::Limits) -> layout::Node {
        layout::Node::new(limits.max())
    }
    fn update(
        &mut self,
        _: &mut Tree,
        event: &Event,
        _: Layout<'_>,
        _: mouse::Cursor,
        _: &Renderer,
        _: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        _: &Rectangle,
    ) {
        if dismisses(event) {
            shell.publish(self.dismiss.clone());
        }
    }
    fn draw(
        &self,
        _: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _: &renderer::Style,
        layout: Layout<'_>,
        _: mouse::Cursor,
        _: &Rectangle,
    ) {
        let accent = colors(theme).accent;
        renderer.fill_quad(
            renderer::Quad {
                bounds: frame(self.bounds, layout.position() - iced::Point::ORIGIN),
                border: Border {
                    color: accent,
                    width: WIDTH,
                    radius: 10.into(),
                },
                ..Default::default()
            },
            Color { a: 0.08, ..accent },
        );
    }
}

impl From<Outline> for Element<'_, Message> {
    fn from(value: Outline) -> Self {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicks_keys_and_wheel_dismiss_while_motion_and_releases_do_not() {
        let press = Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left));
        let wheel = Event::Mouse(mouse::Event::WheelScrolled {
            delta: mouse::ScrollDelta::Lines { x: 0., y: -1. },
        });
        let key = Event::Keyboard(iced::keyboard::Event::KeyPressed {
            key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
            modified_key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
            physical_key: iced::keyboard::key::Physical::Unidentified(
                iced::keyboard::key::NativeCode::Unidentified,
            ),
            location: iced::keyboard::Location::Standard,
            modifiers: iced::keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        });
        assert!(dismisses(&press) && dismisses(&wheel) && dismisses(&key));
        let moved = Event::Mouse(mouse::Event::CursorMoved {
            position: iced::Point::ORIGIN,
        });
        let released = Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left));
        assert!(!dismisses(&moved) && !dismisses(&released));
    }

    #[test]
    fn frame_follows_the_content_origin_with_a_small_gap() {
        let bounds = Rectangle {
            x: 10.,
            y: 400.,
            width: 120.,
            height: 36.,
        };
        assert_eq!(
            frame(bounds, Vector::new(300., 180.)),
            Rectangle {
                x: 306.,
                y: 576.,
                width: 128.,
                height: 44.,
            }
        );
    }
}
