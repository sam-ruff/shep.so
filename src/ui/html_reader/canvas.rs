use super::{Message as HtmlMessage, State};
use crate::{
    html_render::{Input, Pointer, Viewport},
    ui::Message,
};
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, image, layout, mouse, renderer,
    widget::{Tree, tree},
};
use iced::{Element, Event, Length, Point, Rectangle, Renderer, Size, Theme};
use image::Renderer as _;
use renderer::Renderer as _;

#[derive(Default)]
struct NativeState {
    generation: u64,
    geometry: Option<(Viewport, f32)>,
    focused: bool,
    dragging: bool,
}
pub(super) struct Canvas<'a> {
    state: &'a State,
    enabled: bool,
    scale: f32,
}
impl<'a> Canvas<'a> {
    pub fn new(state: &'a State, enabled: bool, scale: f32) -> Self {
        Self {
            state,
            enabled,
            scale,
        }
    }
}
impl Widget<Message, Theme, Renderer> for Canvas<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<NativeState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(NativeState::default())
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(Size::new(
            limits.max().width,
            self.state
                .frame
                .as_ref()
                .map_or(400., |f| f.content_height.max(40.)),
        ))
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<NativeState>();
        if state.generation != self.state.generation {
            *state = NativeState {
                generation: self.state.generation,
                ..Default::default()
            };
        }
        let bounds = layout.bounds();
        let visible = bounds.intersection(viewport);
        {
            let size = Viewport {
                width: bounds.width.ceil().max(1.) as u32,
                height: viewport.height.ceil().max(1.) as u32,
                scale: self.scale,
            };
            let top = (viewport.y - bounds.y).max(0.).floor();
            if state.geometry != Some((size, top)) {
                state.geometry = Some((size, top));
                shell.publish(Message::Html(HtmlMessage::Input(Input::View(
                    self.state.generation,
                    size,
                    top,
                ))));
            }
        }
        if !self.enabled {
            state.focused = false;
            state.dragging = false;
            return;
        }
        let over = visible.is_some_and(|clip| cursor.is_over(clip));
        let position = cursor.position().unwrap_or(Point::ORIGIN) - layout.position();
        let pointer = match event {
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                state.focused = over;
                state.dragging = over;
                over.then_some(Pointer::Down)
            }
            Event::Mouse(mouse::Event::CursorMoved { .. }) if over || state.dragging => {
                Some(Pointer::Move)
            }
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) if state.dragging => {
                state.dragging = false;
                Some(Pointer::Up)
            }
            Event::Window(iced::window::Event::Unfocused) => {
                state.focused = false;
                state.dragging = false;
                Some(Pointer::Leave)
            }
            _ => None,
        };
        if let Some(kind) = pointer {
            shell.publish(Message::Html(HtmlMessage::Input(Input::Pointer(
                self.state.generation,
                kind,
                position.x + self.state.frame.as_ref().map_or(0., |frame| frame.pan),
                position.y,
            ))));
            if !matches!(kind, Pointer::Leave | Pointer::Move) || state.dragging {
                shell.capture_event();
            }
        }
        if state.focused
            && let Event::Keyboard(iced::keyboard::Event::KeyPressed { key, modifiers, .. }) = event
            && modifiers.command()
            && let iced::keyboard::Key::Character(key) = key
        {
            let command = match key.as_str() {
                "c" | "C" => Some(Input::Copy(self.state.generation)),
                "a" | "A" => Some(Input::SelectAll(self.state.generation)),
                _ => None,
            };
            if let Some(command) = command {
                shell.publish(Message::Html(HtmlMessage::Input(command)));
                shell.capture_event();
            }
        }
        if state.focused
            && let Event::Keyboard(iced::keyboard::Event::KeyPressed {
                key: iced::keyboard::Key::Named(key),
                modifiers,
                ..
            }) = event
            && !modifiers.alt()
            && !modifiers.shift()
        {
            use iced::keyboard::key::Named;
            let id = self.state.generation;
            let navigation = match key {
                Named::ArrowUp if !modifiers.command() => Some(HtmlMessage::Scroll(id, -40.)),
                Named::ArrowDown if !modifiers.command() => Some(HtmlMessage::Scroll(id, 40.)),
                Named::PageUp => Some(HtmlMessage::Scroll(id, -viewport.height * 0.8)),
                Named::PageDown => Some(HtmlMessage::Scroll(id, viewport.height * 0.8)),
                Named::Home => Some(HtmlMessage::ScrollEnd(id, false)),
                Named::End => Some(HtmlMessage::ScrollEnd(id, true)),
                _ => None,
            };
            if let Some(message) = navigation {
                shell.publish(Message::Html(message));
                shell.capture_event();
            }
        }
    }
    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let Some(clip) = bounds.intersection(viewport) else {
            return;
        };
        if let (Some(frame), Some(handle)) = (&self.state.frame, &self.state.handle)
            && frame.viewport.width == bounds.width.ceil().max(1.) as u32
            && (frame.viewport.scale - self.scale).abs() < 0.001
        {
            renderer.with_layer(clip, |renderer| {
                renderer.draw_image(
                    image::Image::new(handle.clone()),
                    Rectangle {
                        x: bounds.x,
                        y: bounds.y + frame.scroll,
                        width: frame.viewport.width as f32,
                        height: frame.viewport.height as f32,
                    },
                    clip,
                );
                for &[x, y, width, height] in &self.state.rectangles {
                    if let Some(bounds) = (Rectangle {
                        x: bounds.x + x - frame.pan,
                        y: bounds.y + y,
                        width,
                        height,
                    })
                    .intersection(&clip)
                    {
                        renderer.fill_quad(
                            renderer::Quad {
                                bounds,
                                ..Default::default()
                            },
                            super::super::components::colors(theme)
                                .accent
                                .scale_alpha(0.3),
                        );
                    }
                }
            });
        } else {
            // Loading occupies the body itself; it must not add/remove a row
            // above the document when the first frame arrives.
            renderer.with_layer(clip, |renderer| {
                for (i, fraction) in [0.66, 0.9, 0.78].into_iter().enumerate() {
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: Rectangle {
                                x: bounds.x,
                                y: bounds.y + 12. + i as f32 * 24.,
                                width: bounds.width * fraction,
                                height: 7.,
                            },
                            border: iced::Border {
                                radius: 3.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                        super::super::components::colors(theme)
                            .muted
                            .scale_alpha(0.12),
                    );
                }
            });
        }
    }
    fn mouse_interaction(
        &self,
        _tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if self.enabled
            && layout
                .bounds()
                .intersection(viewport)
                .is_some_and(|r| cursor.is_over(r))
        {
            if self.state.link {
                mouse::Interaction::Pointer
            } else {
                mouse::Interaction::Text
            }
        } else {
            mouse::Interaction::default()
        }
    }
}
impl<'a> From<Canvas<'a>> for Element<'a, Message> {
    fn from(canvas: Canvas<'a>) -> Self {
        Self::new(canvas)
    }
}
