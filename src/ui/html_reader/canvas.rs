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
    presentation: Option<([f32; 4], Option<[f32; 4]>)>,
    focused: bool,
    dragging: bool,
    pan_grab: Option<f32>,
    modifiers: iced::keyboard::Modifiers,
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
                .map_or(400., |f| f.content_height.max(40.))
                + super::SCROLLBAR_SPACE,
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
        let rect = |r: Rectangle| [r.x, r.y, r.width, r.height];
        let presentation = (rect(bounds), visible.map(rect));
        if state.presentation != Some(presentation) {
            state.presentation = Some(presentation);
            shell.publish(Message::Html(HtmlMessage::Geometry(
                self.state.generation,
                presentation.0,
                presentation.1,
                viewport.width,
            )));
        }
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
        if shell.is_event_captured() && matches!(event, Event::Keyboard(_)) {
            return;
        }
        if !self.enabled {
            state.focused = false;
            state.dragging = false;
            state.pan_grab = None;
            state.modifiers = Default::default();
            return;
        }
        if let Event::Keyboard(iced::keyboard::Event::ModifiersChanged(modifiers)) = event {
            state.modifiers = *modifiers;
        }
        let bar = self.state.frame.as_ref().and_then(|f| {
            super::pan::Geometry::new(bounds, *viewport, f.content_width, self.state.pan)
        });
        if let Some(bar) = &bar {
            let over_bar = cursor.is_over(bar.track);
            let pointer = cursor.position().unwrap_or(Point::ORIGIN);
            let mut pan = None;
            match event {
                Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) if over_bar => {
                    state.focused = true;
                    state.dragging = false;
                    let grab = if cursor.is_over(bar.thumb) {
                        pointer.x - bar.thumb.x
                    } else {
                        bar.thumb.width / 2.
                    };
                    state.pan_grab = Some(grab);
                    pan = Some(bar.position(pointer.x, grab));
                }
                Event::Mouse(mouse::Event::CursorMoved { .. }) if state.pan_grab.is_some() => {
                    pan = Some(bar.position(pointer.x, state.pan_grab.unwrap()));
                }
                Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                    if state.pan_grab.take().is_some() =>
                {
                    shell.capture_event();
                    return;
                }
                Event::Mouse(mouse::Event::WheelScrolled { delta })
                    if visible.is_some_and(|clip| cursor.is_over(clip)) =>
                {
                    let (x, y, multiplier) = match delta {
                        mouse::ScrollDelta::Lines { x, y } => (*x, *y, 40.),
                        mouse::ScrollDelta::Pixels { x, y } => (*x, *y, 1.),
                    };
                    if x != 0. || state.modifiers.shift() {
                        pan = Some(self.state.pan - if x != 0. { x } else { y } * multiplier);
                    }
                }
                Event::Window(iced::window::Event::Unfocused) => state.pan_grab = None,
                _ => {}
            }
            if let Some(pan) = pan {
                shell.publish(Message::Html(HtmlMessage::Input(Input::Pan(
                    self.state.generation,
                    pan,
                ))));
                shell.capture_event();
                return;
            }
            if over_bar && matches!(event, Event::Mouse(_)) {
                return;
            }
        } else {
            state.pan_grab = None;
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
                position.y - self.state.anchor_shift(bounds.y, viewport.y),
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
                Named::ArrowLeft if !modifiers.command() && bar.is_some() => {
                    Some(HtmlMessage::Input(Input::Pan(id, self.state.pan - 40.)))
                }
                Named::ArrowRight if !modifiers.command() && bar.is_some() => {
                    Some(HtmlMessage::Input(Input::Pan(id, self.state.pan + 40.)))
                }
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
        let bar = self.state.frame.as_ref().and_then(|f| {
            super::pan::Geometry::new(bounds, *viewport, f.content_width, self.state.pan)
        });
        let body_clip = Rectangle {
            height: (clip.height - bar.as_ref().map_or(0., |_| super::SCROLLBAR_SPACE)).max(0.),
            ..clip
        };
        if let (Some(frame), Some(handle)) = (&self.state.frame, &self.state.handle)
            && frame.viewport.width == bounds.width.ceil().max(1.) as u32
            && (frame.viewport.scale - self.scale).abs() < 0.001
        {
            renderer.with_layer(body_clip, |renderer| {
                renderer.draw_image(
                    image::Image::new(handle.clone()),
                    Rectangle {
                        x: bounds.x,
                        y: bounds.y + frame.scroll + self.state.anchor_shift(bounds.y, viewport.y),
                        width: frame.viewport.width as f32,
                        height: frame.viewport.height as f32,
                    },
                    clip,
                );
                for &[x, y, width, height] in &self.state.rectangles {
                    if let Some(bounds) = (Rectangle {
                        x: bounds.x + x - frame.pan,
                        y: bounds.y + y + self.state.anchor_shift(bounds.y, viewport.y),
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
            renderer.with_layer(body_clip, |renderer| {
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
        if let Some(bar) = bar {
            let colors = super::super::components::colors(theme);
            renderer.fill_quad(
                renderer::Quad {
                    bounds: bar.track,
                    ..Default::default()
                },
                colors.surface,
            );
            for (bounds, color) in [(bar.track, colors.subtle), (bar.thumb, colors.muted)] {
                renderer.fill_quad(
                    renderer::Quad {
                        bounds: Rectangle {
                            y: bounds.y + 5.,
                            height: 6.,
                            ..bounds
                        },
                        border: iced::Border {
                            radius: 3.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    color,
                );
            }
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
            && let Some(bar) = self.state.frame.as_ref().and_then(|f| {
                super::pan::Geometry::new(
                    layout.bounds(),
                    *viewport,
                    f.content_width,
                    self.state.pan,
                )
            })
            && cursor.is_over(bar.track)
        {
            return mouse::Interaction::Grab;
        }
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
