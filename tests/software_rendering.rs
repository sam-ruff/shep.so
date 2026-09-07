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

fn rotating_svg(renderer: &mut Renderer, angle: f32, clip: Rectangle) {
    use iced::advanced::svg::Renderer as _;
    let handle = iced::widget::svg::Handle::from_memory(
        br#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><path fill="white" d="M3 3h14v5H8v9H3Z"/></svg>"#.as_slice(),
    );
    renderer.draw_svg(
        iced::advanced::svg::Svg {
            handle,
            color: None,
            rotation: iced::Radians(angle),
            opacity: 1.,
        },
        rect(130., 20., 20., 20.),
        clip,
    );
}

#[test]
fn rotated_svg_keeps_its_center_and_honors_viewport_at_fractional_scale() {
    for scale in [1., 1.25] {
        for angle in [
            0.,
            0.4,
            std::f32::consts::FRAC_PI_2,
            std::f32::consts::PI,
            4.7,
        ] {
            let mut renderer = renderer();
            let clip = rect(128., 18., 24., 20.);
            rotating_svg(&mut renderer, angle, clip);
            let mut pixels = tiny_skia::Pixmap::new(200, 80).unwrap();
            let mut mask = tiny_skia::Mask::new(200, 80).unwrap();
            renderer.draw(
                &mut pixels.as_mut(),
                &mut mask,
                &Viewport::with_physical_size(Size::new(200, 80), scale),
                &[rect(0., 0., 200., 80.)],
                Color::TRANSPARENT,
            );
            let visible = clip * scale;
            let mut ink = 0;
            for y in 0..80 {
                for x in 0..200 {
                    if pixels.pixel(x, y).unwrap().alpha() > 0 {
                        assert!(
                            rect(x as f32, y as f32, 1., 1.)
                                .intersection(&visible)
                                .is_some(),
                            "SVG escaped its own viewport at {x},{y}; scale={scale} angle={angle}"
                        );
                        ink += 1;
                    }
                }
            }
            assert!(
                ink > 50,
                "Rotated SVG disappeared or lost its scale: {angle}, {scale}"
            );
        }
    }
}

#[test]
fn rotating_svg_partial_repaint_matches_full_repaint_without_trails() {
    use iced::advanced::Renderer as _;
    let viewport = Viewport::with_physical_size(Size::new(200, 80), 1.);
    let mut renderer = renderer();
    let mut pixels = tiny_skia::Pixmap::new(200, 80).unwrap();
    let mut mask = tiny_skia::Mask::new(200, 80).unwrap();
    let screen = rect(0., 0., 200., 80.);
    renderer.reset(screen);
    rotating_svg(&mut renderer, 0., screen);
    renderer.draw(
        &mut pixels.as_mut(),
        &mut mask,
        &viewport,
        &[screen],
        Color::TRANSPARENT,
    );
    let initial = pixels.clone();
    let mut visibly_rotated = false;
    for angle in [0.3, 0.7, 1.2, 2., 3., 4., 5., 0.] {
        let previous = renderer.layers()[0].clone();
        renderer.reset(screen);
        rotating_svg(&mut renderer, angle, screen);
        let damage = iced_tiny_skia::Layer::damage(&previous, &renderer.layers()[0]);
        renderer.draw(
            &mut pixels.as_mut(),
            &mut mask,
            &viewport,
            &damage,
            Color::TRANSPARENT,
        );
        let mut full = tiny_skia::Pixmap::new(200, 80).unwrap();
        renderer.draw(
            &mut full.as_mut(),
            &mut mask,
            &viewport,
            &[screen],
            Color::TRANSPARENT,
        );
        assert!(
            pixels.data() == full.data(),
            "Rotation left stale pixels at angle {angle}"
        );
        visibly_rotated |= full.data() != initial.data();
    }
    assert!(visibly_rotated, "A static icon is not a rotating icon");
    assert!(
        pixels.data() == initial.data(),
        "Stopping must restore the original icon"
    );
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

fn shadow_quad(bounds: Rectangle) -> iced::advanced::renderer::Quad {
    iced::advanced::renderer::Quad {
        bounds,
        border: iced::Border::default().rounded(8),
        shadow: iced::Shadow {
            color: Color::from_rgba(0., 0., 0., 0.3),
            offset: iced::Vector::new(0., 3.),
            blur_radius: 10.,
        },
        ..Default::default()
    }
}

#[test]
fn shadows_respect_damage_and_layer_clipping_including_shadow_only_regions() {
    use iced::advanced::Renderer as _;
    let clip = rect(0., 0., 100., 70.);
    let quad = shadow_quad(rect(60., 20., 50., 30.));
    for damage in [rect(55., 45., 70., 25.), rect(55., 52., 70., 8.)] {
        let mut renderer = renderer();
        renderer.with_layer(clip, |renderer| renderer.fill_quad(quad, Color::WHITE));
        let pixels = draw(&mut renderer, damage);
        let visible = clip.intersection(&damage).unwrap();
        let mut ink = 0;
        for y in 0..80 {
            for x in 0..200 {
                if pixels.pixel(x, y).unwrap().alpha() != 0 {
                    assert!(
                        visible.contains(Point::new(x as f32, y as f32)),
                        "shadow escaped damage/viewport at {x},{y}"
                    );
                    ink += 1;
                }
            }
        }
        assert!(
            ink > 20,
            "a shadow remains visible outside its casting quad"
        );
    }
}

#[test]
fn moving_and_dismissing_shadowed_controls_repaints_all_old_pixels() {
    use iced::advanced::Renderer as _;
    let viewport = Viewport::with_physical_size(Size::new(200, 80), 1.);
    let mut renderer = renderer();
    let mut pixels = tiny_skia::Pixmap::new(200, 80).unwrap();
    let mut mask = tiny_skia::Mask::new(200, 80).unwrap();
    let background = Color::from_rgb(0.95, 0.95, 0.97);
    renderer.reset(rect(0., 0., 200., 80.));
    renderer.fill_quad(shadow_quad(rect(15., 12., 50., 30.)), Color::WHITE);
    renderer.draw(
        &mut pixels.as_mut(),
        &mut mask,
        &viewport,
        &[rect(0., 0., 200., 80.)],
        background,
    );
    for bounds in [Some(rect(115., 32., 50., 30.)), None] {
        let previous = renderer.layers()[0].clone();
        renderer.reset(rect(0., 0., 200., 80.));
        if let Some(bounds) = bounds {
            renderer.fill_quad(shadow_quad(bounds), Color::WHITE);
        }
        let damage = iced_tiny_skia::Layer::damage(&previous, &renderer.layers()[0]);
        renderer.draw(
            &mut pixels.as_mut(),
            &mut mask,
            &viewport,
            &damage,
            background,
        );
        let mut complete = tiny_skia::Pixmap::new(200, 80).unwrap();
        renderer.draw(
            &mut complete.as_mut(),
            &mut mask,
            &viewport,
            &[rect(0., 0., 200., 80.)],
            background,
        );
        assert!(
            pixels.data() == complete.data(),
            "partial repaint left a shadow trail"
        );
    }
}

#[test]
fn partially_offscreen_shadows_keep_their_original_origin() {
    use iced::advanced::Renderer as _;
    let mut reference = renderer();
    reference.fill_quad(shadow_quad(rect(40., 20., 50., 30.)), Color::WHITE);
    let reference = draw(&mut reference, rect(0., 0., 200., 80.));
    let mut clipped = renderer();
    clipped.fill_quad(shadow_quad(rect(0., 0., 50., 30.)), Color::WHITE);
    let clipped = draw(&mut clipped, rect(0., 0., 200., 80.));
    for y in 0..50 {
        for x in 0..70 {
            assert_eq!(
                clipped.pixel(x, y),
                reference.pixel(x + 40, y + 20),
                "shadow moved at {x},{y}"
            );
        }
    }
}

#[test]
fn reader_damage_absorbs_interleaved_children_without_repainting_distant_controls() {
    use iced_tiny_skia::window::compositor::group_damage;
    let bounds = rect(0., 0., 1440., 920.);
    let reader = rect(614., 131., 808., 671.);
    let sidebar = rect(51., 273., 49., 12.);
    let mut changes = vec![sidebar];
    for y in (210..790).step_by(40) {
        changes.push(rect(650., y as f32, 200., 25.));
        changes.push(rect(250., y as f32, 340., 39.));
    }
    changes.push(reader);
    let grouped = group_damage(changes.clone(), bounds);
    assert!(
        grouped.len() <= 4,
        "child damage must not repeatedly paint the reader: {grouped:?}"
    );
    assert!(
        grouped.contains(&sidebar),
        "distant sidebar damage stays small"
    );
    for changed in changes {
        assert!(grouped.iter().any(|area| changed.is_within(area)));
    }
    assert!(
        grouped
            .iter()
            .all(|area| area.is_within(&bounds) && area.area() < bounds.area() * 0.8)
    );
    assert!(group_damage(vec![], bounds).is_empty());
    assert_eq!(
        group_damage(vec![rect(-20., -20., 30., 30.)], bounds),
        vec![rect(0., 0., 10., 10.)]
    );
}

#[test]
fn coalesced_reader_changes_match_full_paint_with_borders_text_and_shadows() {
    use iced::advanced::Renderer as _;
    use iced_tiny_skia::window::compositor::group_damage;
    let bounds = rect(0., 0., 1000., 700.);
    for scale in [1., 1.5] {
        let width = (bounds.width * scale) as u32;
        let height = (bounds.height * scale) as u32;
        let viewport = Viewport::with_physical_size(Size::new(width, height), scale);
        let mut renderer = renderer();
        let mut pixels = tiny_skia::Pixmap::new(width, height).unwrap();
        let mut mask = tiny_skia::Mask::new(width, height).unwrap();
        let background = Color::from_rgb(0.1, 0.1, 0.12);
        for step in 0..3 {
            let previous = renderer.layers().to_vec();
            renderer.reset(bounds);
            renderer.fill_quad(
                shadow_quad(rect(320., 30., 650., 630.)),
                Color::from_rgb(0.2, 0.2, 0.23),
            );
            for row in 0..12 {
                let y = 45. + row as f32 * 48.;
                label(&mut renderer, Point::new(12., y), rect(0., 0., 280., 640.));
                if row % 3 != step {
                    renderer.fill_quad(
                        shadow_quad(rect(340., y, 450. + step as f32 * 25., 30.)),
                        Color::from_rgba(0.7, 0.5, 0.8, 0.6),
                    );
                    label(
                        &mut renderer,
                        Point::new(350., y),
                        rect(330., 35., 600., 590.),
                    );
                }
            }
            let damage = if step == 0 {
                vec![bounds]
            } else {
                group_damage(
                    iced_tiny_skia::graphics::damage::diff(
                        &previous,
                        renderer.layers(),
                        |layer| vec![layer.bounds],
                        iced_tiny_skia::Layer::damage,
                    ),
                    bounds,
                )
            };
            renderer.draw(
                &mut pixels.as_mut(),
                &mut mask,
                &viewport,
                &damage,
                background,
            );
            let mut full = tiny_skia::Pixmap::new(width, height).unwrap();
            renderer.draw(
                &mut full.as_mut(),
                &mut mask,
                &viewport,
                &[bounds],
                background,
            );
            assert!(
                pixels.data() == full.data(),
                "coalescing left incorrect pixels at scale {scale}, step {step}"
            );
        }
    }
}

#[test]
fn panel_interior_fast_path_matches_general_painter_at_fractional_scale_and_clips() {
    use iced::advanced::Renderer as _;
    for scale in [1., 1.25, 2.] {
        for alpha in [0.5, 1.] {
            let color = Color::from_rgba(1., 0., 0., alpha);
            let solid = iced::Background::Color(color);
            // A constant gradient uses the general path rasterizer and should
            // produce exactly the same fill as an optimized solid panel.
            let general = iced::Background::Gradient(
                iced::gradient::Linear::new(0.)
                    .add_stop(0., color)
                    .add_stop(1., color)
                    .into(),
            );
            for damage in [
                rect(31.2, 23.4, 21.3, 19.8),
                rect(3., 2., 25., 20.),
                rect(0., 0., 200., 80.),
            ] {
                let paint = |background| {
                    let mut renderer = renderer();
                    let mut quad = shadow_quad(rect(2., 1., 190., 75.));
                    quad.shadow = Default::default();
                    quad.border = iced::Border::default()
                        .rounded(8)
                        .width(2)
                        .color(Color::WHITE);
                    renderer.fill_quad(quad, background);
                    let mut pixels =
                        tiny_skia::Pixmap::new((200. * scale) as u32, (80. * scale) as u32)
                            .unwrap();
                    let mut mask = tiny_skia::Mask::new(pixels.width(), pixels.height()).unwrap();
                    renderer.draw(
                        &mut pixels.as_mut(),
                        &mut mask,
                        &Viewport::with_physical_size(
                            Size::new((200. * scale) as u32, (80. * scale) as u32),
                            scale,
                        ),
                        &[damage],
                        Color::TRANSPARENT,
                    );
                    pixels
                };
                let actual = paint(solid);
                let expected = paint(general);
                let differences: Vec<_> = actual
                    .pixels()
                    .iter()
                    .zip(expected.pixels())
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(10)
                    .map(|(i, (a, b))| (i as u32 % actual.width(), i as u32 / actual.width(), a, b))
                    .collect();
                assert!(
                    differences.is_empty(),
                    "interior/edge clip differs: scale={scale}, alpha={alpha}, damage={damage:?}: {differences:?}"
                );
            }
        }
    }
}

#[test]
fn editor_glyphs_cannot_escape_the_viewport_when_its_bounds_fit_the_damage() {
    use iced::advanced::text::Editor as _;
    for height in [17., 29., 41., 59.] {
        let mut renderer = renderer();
        let mut editor = iced_tiny_skia::graphics::text::Editor::with_text(
            "First line\nSecond line\nThird line\nFourth line\nFifth line",
        );
        editor.update(
            Size::new(180., height),
            Font::with_name("Noto Sans"),
            Pixels(20.),
            text::LineHeight::Absolute(Pixels(30.)),
            text::Wrapping::Word,
            &mut text::highlighter::PlainText,
        );
        let clip = rect(0., 0., 180., height);
        renderer.fill_editor(&editor, Point::ORIGIN, Color::WHITE, clip);
        let pixels = draw(&mut renderer, rect(0., 0., 200., 80.));
        assert!(pixels.pixels().iter().any(|p| p.alpha() > 0));
        for y in height.ceil() as u32..80 {
            for x in 0..200 {
                assert_eq!(
                    pixels.pixel(x, y).unwrap().alpha(),
                    0,
                    "Editor ink below viewport at {x},{y} (height {height})"
                );
            }
        }
    }
}
