use super::{Message as Find, State};
use crate::ui::Message;
use iced::advanced::{
    Clipboard, Layout, Shell, Widget, layout, mouse, renderer,
    widget::{Operation, Tree, tree},
};
use iced::{Element, Event, Length, Rectangle, Renderer, Size, Theme};
use renderer::Renderer as _;

#[derive(Default)]
struct NativeState {
    key: Option<(String, usize)>,
    width: f32,
    jump: Option<(u64, u64)>,
}
struct Highlights<'a> {
    child: Element<'a, Message>,
    find: &'a State,
    id: &'a str,
    block: usize,
    pan: Option<f32>,
    content_width: Option<f32>,
    ready: bool,
}
pub(super) fn wrap<'a>(
    child: Element<'a, Message>,
    find: &'a State,
    id: &'a str,
    block: usize,
    pan: Option<f32>,
    content_width: Option<f32>,
    ready: bool,
) -> Element<'a, Message> {
    Element::new(Highlights {
        child,
        find,
        id,
        block,
        pan,
        content_width,
        ready,
    })
}
impl Widget<Message, Theme, Renderer> for Highlights<'_> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<NativeState>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(NativeState::default())
    }
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.child)]
    }
    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.child));
    }
    fn size(&self) -> Size<Length> {
        self.child.as_widget().size()
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.child
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        self.child
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }
    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.child.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        let state = tree.state.downcast_mut::<NativeState>();
        if !self.ready {
            return;
        }
        let bounds = layout.bounds();
        if state
            .key
            .as_ref()
            .is_none_or(|(id, block)| id != self.id || *block != self.block)
        {
            state.key = Some((self.id.into(), self.block));
            state.width = 0.;
            state.jump = None;
        }
        if self.pan.is_none() && (state.width - bounds.width).abs() > 0.1 {
            state.width = bounds.width;
            shell.publish(Message::Find(Find::Width(
                self.id.into(),
                self.block,
                bounds.width,
            )));
        }
        let jump = (self.find.revision, self.find.jump);
        if state.jump != Some(jump)
            && let Some(found) = self
                .find
                .results
                .as_ref()
                .and_then(|r| self.find.active.and_then(|i| r.matches.get(i)))
            && found.block == self.block
            && let Some(&[x, y, width, height]) = found.rectangles.first()
        {
            state.jump = Some(jump);
            let delta = bounds.y + y + height / 2. - (viewport.y + viewport.height / 2.);
            let pan = self.pan.and_then(|pan| {
                (x < pan || x + width > pan + bounds.width)
                    .then_some((x + width / 2. - bounds.width / 2.).max(0.))
            });
            shell.publish(Message::Find(Find::Reveal(jump.0, jump.1, delta, pan)));
        }
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.child.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
        let bounds = layout.bounds();
        let Some(mut clip) = bounds.intersection(viewport) else {
            return;
        };
        if !self.ready {
            return;
        }
        if self
            .content_width
            .is_some_and(|width| width > bounds.width + 1.)
        {
            clip.height = (clip.height - crate::ui::html_reader::SCROLLBAR_SPACE).max(0.);
        }
        let Some(results) = &self.find.results else {
            return;
        };
        renderer.with_layer(clip, |renderer| {
            for highlight in results.visible(
                self.block,
                clip.y - bounds.y,
                clip.y + clip.height - bounds.y,
            ) {
                let [x, y, width, height] = highlight.bounds;
                if let Some(rectangle) = (Rectangle {
                    x: bounds.x + x - self.pan.unwrap_or(0.),
                    y: bounds.y + y,
                    width,
                    height,
                })
                .intersection(&clip)
                {
                    let active = self.find.active == Some(highlight.matched);
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: rectangle,
                            border: iced::Border {
                                color: iced::Color::from_rgba(0.85, 0.47, 0.02, 0.9),
                                width: if active { 1. } else { 0. },
                                radius: 2.into(),
                            },
                            ..Default::default()
                        },
                        iced::Color::from_rgba(1., 0.72, 0.12, if active { 0.42 } else { 0.23 }),
                    );
                }
            }
        });
    }
    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.child.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }
}
