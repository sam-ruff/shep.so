//! Compare table reuse with the original measurement algorithm on one thread.
//! Both modes include the inline/caption offset corrections, checked below.
//! The comparison switch is a dev-dependency feature, absent in shipped builds.
use super::*;
use litehtml_sys::table_layout_test::Mode;

#[derive(Debug, PartialEq)]
struct Rendering {
    dimensions: (f32, f32),
    pixels: [u8; 32],
    selection: (String, Vec<[f32; 4]>),
}

fn sequence(html: &str, fonts: Fonts, cached: bool, reflow: bool) -> (Vec<Rendering>, (u64, u64)) {
    let mode = Mode::new(cached);
    let surface = container::Surface::new(740, 700, 1., fonts);
    let mut container = surface.clone();
    let source = format!(
        "<style>body{{font-family:sans-serif;font-size:14px;line-height:1.5;background:#fff;margin:0}}</style>{html}"
    );
    let mut doc = Document::from_html(&source, &mut container, None, None).unwrap();
    let measure = surface.0.borrow().text_measure_fn();
    let mut result = Vec::new();
    let views: &[(u32, u32, f32)] = if reflow {
        &[
            (740, 700, 1.),
            (740, 700, 1.),
            (320, 520, 1.),
            (940, 620, 1.25),
            (740, 700, 1.),
        ]
    } else {
        &[(740, 700, 1.)]
    };
    for (index, &(width, height, scale)) in views.iter().enumerate() {
        if index == 1 {
            assert!(surface.load_remote_image(
                "https://example.test/item.webp",
                Arc::from(include_bytes!("../../assets/logo-light.webp").as_slice())
            ));
        }
        surface
            .0
            .borrow_mut()
            .resize_with_scale(width, height, scale);
        doc.media_changed();
        let _ = doc.render(width as f32);
        let mut selection = Selection::default();
        selection.layout(&doc);
        selection.all();
        surface.begin_draw();
        doc.draw(
            DrawContext::default(),
            0.,
            0.,
            Some(Position {
                x: 0.,
                y: 0.,
                width: width as f32,
                height: height as f32,
            }),
        );
        result.push(Rendering {
            dimensions: (doc.width(), doc.height()),
            pixels: <sha2::Sha256 as sha2::Digest>::digest(surface.0.borrow().pixels()).into(),
            selection: selection.result(&measure),
        });
    }
    (result, mode.counts())
}

#[test]
fn deeply_nested_tables_preserve_pixels_and_avoid_exponential_layout_work() {
    let fonts = fonts();
    let source = complex_fixture::letter(14);
    let (original, (original_work, original_hits)) = sequence(&source, fonts.clone(), false, false);
    let (optimized, (work, hits)) = sequence(&source, fonts, true, false);
    assert_eq!(original, optimized);
    assert_eq!(original_hits, 0);
    assert!(
        original_work > 10_000,
        "The fixture must reproduce repeated subtree work: {original_work}"
    );
    assert!(
        work < 200 && hits > 10,
        "Expected bounded repeated-layout work, got {work} layouts/{hits} hits"
    );
}

#[test]
fn table_reuse_preserves_constraints_positioned_content_selection_and_image_reflows() {
    let fonts = fonts();
    let content = "<div style='position:relative;left:5px'><div style='position:absolute;right:2px;top:1px'>Badge</div></div><img src='https://example.test/item.webp' style='width:32px;height:auto'><span>Selectable table content wraps across lines.</span>";
    for (index, style) in [
        "width:100%;border-collapse:collapse;border:2px solid #8260c4",
        "width:auto;border-spacing:7px;padding:3%",
        "width:70%;height:50%;min-height:160px;max-width:450px",
        "width:340px;position:relative;left:9px;top:5px",
        "width:80%;margin-left:auto;margin-right:auto",
    ]
    .iter()
    .enumerate()
    {
        let inner = format!(
            "<table style='{style}'><tr><td rowspan='2' style='vertical-align:bottom'>{content}</td><td style='width:35%;padding:7px'><div style='float:right;width:40px'>Float</div>Text beside a float</td></tr><tr><td>Second row</td></tr><tr><td colspan='2' style='height:90px;vertical-align:middle'>Spanned row <span>inline text</span></td></tr></table>"
        );
        let html = format!(
            "<div style='position:relative;height:600px'><table width='100%'><tr><td><table width='100%'><tr><td>{inner}</td></tr></table></td></tr></table><p>Following text</p></div>"
        );
        let (original, _) = sequence(&html, fonts.clone(), false, true);
        let (optimized, (_, hits)) = sequence(&html, fonts.clone(), true, true);
        assert_eq!(original, optimized, "Table variant {index}");
        assert!(hits > 0, "Variant {index} must exercise reuse");
        assert_ne!(
            optimized[0].pixels, optimized[1].pixels,
            "Image arrival must change pixels"
        );
        assert_eq!(
            optimized[1], optimized[4],
            "Returning to a prior size must preserve loaded images and layout"
        );
    }
}

#[test]
fn captions_and_relative_inlines_remain_stable_with_table_reuse() {
    let fonts = fonts();
    for inner in [
        "<caption>Top caption</caption><tr><td>Body</td></tr><caption style='caption-side:bottom'>Bottom caption</caption>",
        "<tr><td><span style='position:relative;left:5px'>Relative text</span><span style='position:absolute;left:12px;top:4px'>Positioned</span></td></tr>",
    ] {
        let html = format!(
            "<div style='position:relative'><table width='100%'><tr><td><table width='100%'>{inner}</table></td></tr></table></div>"
        );
        let (original, _) = sequence(&html, fonts.clone(), false, true);
        let (optimized, (_, hits)) = sequence(&html, fonts.clone(), true, true);
        assert_eq!(original, optimized);
        assert!(hits > 0);
        assert_eq!(optimized[1], optimized[4]);
    }
}

#[test]
fn media_queries_reflow_the_same_document_without_reusing_old_styles() {
    let fonts = fonts();
    let html = "<style>@media (max-width:500px){.shift{position:relative;left:5px}}</style><table width='100%'><tr><td><table width='100%'><tr><td><span class='shift'>Responsive inline text</span></td></tr></table></td></tr></table>";
    let (original, _) = sequence(html, fonts.clone(), false, true);
    let (optimized, (_, hits)) = sequence(html, fonts, true, true);
    assert_eq!(original, optimized);
    assert!(hits > 0);
}

#[test]
fn unshifted_relative_inline_boxes_can_be_reused_without_changing_their_pixels() {
    let fonts = fonts();
    for style in [
        "position:relative",
        "position:relative;left:0;right:8px;top:0;bottom:6px",
        "position:relative;right:0;bottom:0",
    ] {
        let html = format!(
            "<table width='100%'><tr><td><table width='100%'><tr><td><span style='{style}'>Unshifted inline text</span></td></tr></table></td></tr></table>"
        );
        let (original, _) = sequence(&html, fonts.clone(), false, true);
        let (optimized, (_, hits)) = sequence(&html, fonts.clone(), true, true);
        assert_eq!(original, optimized);
        assert_eq!(optimized[1], optimized[4]);
        assert!(hits > 0);
    }
}

#[test]
fn inline_offsets_apply_once_to_wrapped_fragments_and_superscripts() {
    let fonts = fonts();
    for tag in ["span", "sup"] {
        let make = |left, top| {
            format!(
                "<table width='180'><tr><td><table width='100%'><tr><td><{tag} style='position:relative;left:{left}px;top:{top}px'>Several wrapped words carry exactly one relative offset each</{tag}></td></tr></table></td></tr></table>"
            )
        };
        let (plain, _) = sequence(&make(0, 0), fonts.clone(), true, true);
        let (shifted, (_, hits)) = sequence(&make(5, 2), fonts.clone(), true, true);
        assert!(hits > 0);
        assert_eq!(shifted[1], shifted[4]);
        assert_eq!(plain[0].selection.0, shifted[0].selection.0);
        let base = &plain[0].selection.1;
        let moved = &shifted[0].selection.1;
        assert_eq!(base.len(), moved.len());
        assert!(base.len() > 10, "Exercise wrapped fragments");
        for (base, moved) in base.iter().zip(moved) {
            assert!(
                (moved[0] - base[0] - 5.).abs() < 0.01,
                "{tag}: horizontal offset accumulated"
            );
            assert!(
                (moved[1] - base[1] - 2.).abs() < 0.01,
                "{tag}: vertical offset accumulated"
            );
            assert_eq!(base[2..], moved[2..]);
        }
    }
}

#[test]
fn captions_displace_cell_text_once_including_when_height_equals_border() {
    let fonts = fonts();
    for (height, border) in [(20, 0), (20, 2), (2, 2)] {
        let make = |caption| {
            let caption = if caption {
                format!(
                    "<caption style='height:{height}px;font-size:1px;line-height:1px'></caption>"
                )
            } else {
                String::new()
            };
            format!(
                "<table width='300' cellspacing='0' cellpadding='0' style='border:{border}px solid black'>{caption}<tr><td>Body text</td></tr></table>"
            )
        };
        let (plain, _) = sequence(&make(false), fonts.clone(), true, true);
        let (captioned, _) = sequence(&make(true), fonts.clone(), true, true);
        assert_eq!(captioned[1], captioned[4]);
        assert_eq!(plain[0].selection.0, captioned[0].selection.0);
        let base = &plain[0].selection.1;
        let moved = &captioned[0].selection.1;
        assert_eq!(base.len(), moved.len());
        assert!(!base.is_empty());
        for (base, moved) in base.iter().zip(moved) {
            assert_eq!(base[0], moved[0]);
            assert!(
                (moved[1] - base[1] - height as f32).abs() < 0.01,
                "Caption height {height}/border {border}: expected one displacement, got {}",
                moved[1] - base[1]
            );
            assert_eq!(base[2..], moved[2..]);
        }
        assert_eq!(
            captioned[0].dimensions.1 - plain[0].dimensions.1,
            height as f32
        );
    }
}
