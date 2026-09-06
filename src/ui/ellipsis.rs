//! A single-line label measured against its actual available width. Cache shaping
//! in the widget tree so folder names cannot widen every row in a scrollable.
use super::Message;
use iced::advanced::{
    Layout, Widget, layout, mouse, renderer, text,
    widget::{Tree, tree},
};
use iced::{Element, Font, Length, Rectangle, Renderer, Size, Theme};
use text::Renderer as _;

type Paragraph = <Renderer as text::Renderer>::Paragraph;
#[derive(Default)]
struct State {
    paragraph: text::paragraph::Plain<Paragraph>,
    original: String,
    width: f32,
    size: f32,
    font: Font,
}
pub(super) struct Ellipsis {
    label: String,
    size: f32,
}
impl Ellipsis {
    pub fn new(label: String, size: f32) -> Self {
        Self { label, size }
    }
}
impl Widget<Message, Theme, Renderer> for Ellipsis {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }
    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }
    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<State>();
        let width = limits.max().width;
        if state.original != self.label
            || state.width != width
            || state.size != self.size
            || state.font != renderer.default_font()
        {
            let shape = |state: &mut State, label: &str| {
                state.paragraph.update(text::Text {
                    content: label,
                    bounds: Size::INFINITE,
                    size: self.size.into(),
                    line_height: text::LineHeight::Relative(1.),
                    font: renderer.default_font(),
                    align_x: text::Alignment::Left,
                    align_y: iced::alignment::Vertical::Top,
                    shaping: text::Shaping::Advanced,
                    wrapping: text::Wrapping::None,
                });
                state.paragraph.min_bounds().width
            };
            if shape(state, &self.label) > width {
                let ends: Vec<usize> = self
                    .label
                    .char_indices()
                    .map(|(i, _)| i)
                    .chain(std::iter::once(self.label.len()))
                    .collect();
                let (mut lo, mut hi) = (0, ends.len() - 1);
                while lo < hi {
                    let mid = (lo + hi).div_ceil(2);
                    if shape(state, &format!("{}…", &self.label[..ends[mid]])) <= width {
                        lo = mid;
                    } else {
                        hi = mid - 1;
                    }
                }
                shape(state, &format!("{}…", &self.label[..ends[lo]]));
            }
            state.original.clone_from(&self.label);
            state.width = width;
            state.size = self.size;
            state.font = renderer.default_font();
        }
        layout::Node::new(Size::new(width, self.size))
    }
    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if let Some(clip) = layout.bounds().intersection(viewport) {
            renderer.fill_paragraph(
                tree.state.downcast_ref::<State>().paragraph.raw(),
                layout.position(),
                style.text_color,
                clip,
            );
        }
    }
    fn operate(
        &mut self,
        _tree: &mut Tree,
        layout: Layout<'_>,
        _renderer: &Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        operation.text(None, layout.bounds(), &self.label);
    }
}
impl From<Ellipsis> for Element<'_, Message> {
    fn from(label: Ellipsis) -> Self {
        Self::new(label)
    }
}
