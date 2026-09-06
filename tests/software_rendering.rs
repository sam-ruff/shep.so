//! Exercise the production software renderer, including clipping across damage
//! updates. Native equivalents cover real scrollable controls in Preferences.
use iced::advanced::text::{self, Renderer as _};
use iced::{Color, Font, Pixels, Point, Rectangle, Size, alignment};
use iced_tiny_skia::{Renderer, graphics::Viewport};

fn renderer() -> Renderer {
    static FONT: std::sync::Once = std::sync::Once::new();
    FONT.call_once(|| {
        iced_tiny_skia::graphics::text::font_system()
            .write()
            .unwrap()
            .load_font(std::borrow::Cow::Borrowed(include_bytes!(
                "../assets/NotoSans-Regular.ttf"
            )));
    });
    Renderer::new(Font::with_name("Noto Sans"), Pixels(20.))
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rectangle {
    Rectangle {
        x,
        y,
        width,
        height,
    }
}

fn label(renderer: &mut Renderer, position: Point, clip: Rectangle) {
    renderer.fill_text(
        text::Text {
            content: "100 Shortcuts".into(),
            bounds: Size::new(180., 30.),
            size: Pixels(20.),
            line_height: text::LineHeight::Absolute(Pixels(30.)),
            font: Font::with_name("Noto Sans"),
            align_x: text::Alignment::Default,
            align_y: alignment::Vertical::Top,
            shaping: text::Shaping::Advanced,
            wrapping: text::Wrapping::None,
        },
        position,
        Color::WHITE,
        clip,
    );
}

fn draw(renderer: &mut Renderer, damage: Rectangle) -> tiny_skia::Pixmap {
    let mut pixels = tiny_skia::Pixmap::new(200, 80).unwrap();
    let mut mask = tiny_skia::Mask::new(200, 80).unwrap();
    renderer.draw(
        &mut pixels.as_mut(),
        &mut mask,
        &Viewport::with_physical_size(Size::new(200, 80), 1.),
        &[damage],
        Color::TRANSPARENT,
    );
    pixels
}

#[test]
fn cached_glyphs_respect_local_viewport_and_damage_intersection() {
    for damage in [rect(0., 0., 200., 80.), rect(7., 20., 60., 40.)] {
        let mut renderer = renderer();
        let clip = rect(0., 0., 100., 30.);
        label(&mut renderer, Point::new(0., 15.), clip);
        let pixels = draw(&mut renderer, damage);
        let visible = clip.intersection(&damage).unwrap();
        let mut ink = 0;
        for y in 0..80 {
            for x in 0..200 {
                if pixels.pixel(x, y).unwrap().alpha() != 0 {
                    assert!(
                        visible.contains(Point::new(x as f32, y as f32)),
                        "ink escaped at {x},{y}"
                    );
                    ink += 1;
                }
            }
        }
        assert!(ink > 20, "the in-viewport label must still render");
    }
}

#[test]
fn entirely_scrolled_out_cached_label_does_not_draw() {
    let mut renderer = renderer();
    label(&mut renderer, Point::new(0., 40.), rect(0., 0., 200., 30.));
    let pixels = draw(&mut renderer, rect(0., 0., 200., 80.));
    assert!(pixels.pixels().iter().all(|pixel| pixel.alpha() == 0));
}

#[test]
fn raw_text_does_not_inherit_the_preceding_cached_labels_mask() {
    use iced_tiny_skia::graphics::text::{self as graphics_text, Renderer as _};
    let mut renderer = renderer();
    let buffer = {
        let mut fonts = graphics_text::font_system().write().unwrap();
        let fonts = fonts.raw();
        let mut buffer = cosmic_text::Buffer::new(fonts, cosmic_text::Metrics::new(20., 30.));
        buffer.set_size(fonts, Some(180.), Some(30.));
        buffer.set_text(
            fonts,
            "Visible raw text",
            &cosmic_text::Attrs::new().family(cosmic_text::Family::Name("Noto Sans")),
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(fonts, false);
        std::sync::Arc::new(buffer)
    };
    label(&mut renderer, Point::ORIGIN, rect(0., 0., 5., 20.));
    renderer.fill_raw(graphics_text::Raw {
        buffer: std::sync::Arc::downgrade(&buffer),
        position: Point::new(30., 40.),
        color: Color::WHITE,
        clip_bounds: rect(0., 0., 200., 80.),
    });
    let pixels = draw(&mut renderer, rect(0., 0., 80., 80.));
    let mut ink = 0;
    for y in 40..80 {
        for x in 0..200 {
            if pixels.pixel(x, y).unwrap().alpha() != 0 {
                assert!(x < 80, "raw text escaped the damaged region");
                ink += 1;
            }
        }
    }
    assert!(
        ink > 20,
        "a preceding label's narrower mask must not erase raw text"
    );
}
