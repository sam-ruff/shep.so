//! Prepare the tiny taskbar image on the count worker, never iced update/view.
use ab_glyph::{Font, FontRef, ScaleFont, point};
use futures::SinkExt;
use std::sync::Arc;
use tokio::sync::watch;

pub const SIZE: usize = 32;
#[derive(Debug)]
pub struct Frame {
    pub count: u64,
    pub rgba: Vec<u8>,
    pub description: String,
}
struct Renderer(FontRef<'static>);
impl Renderer {
    fn new() -> Self {
        Self(
            FontRef::try_from_slice(include_bytes!("../../assets/NotoSans-SemiBold.ttf"))
                .expect("bundled badge font"),
        )
    }
    fn render(&self, count: u64) -> Frame {
        let mut rgba = vec![0; SIZE * SIZE * 4];
        if count > 0 {
            for y in 0..SIZE {
                for x in 0..SIZE {
                    let radius =
                        ((x as f32 + 0.5 - 16.).powi(2) + (y as f32 + 0.5 - 16.).powi(2)).sqrt();
                    let alpha = (15.5 - radius).clamp(0., 1.);
                    rgba[(y * SIZE + x) * 4..(y * SIZE + x) * 4 + 4].copy_from_slice(&[
                        213,
                        38,
                        56,
                        (alpha * 255.) as u8,
                    ]);
                }
            }
            let label = if count > 99 {
                "99+".into()
            } else {
                count.to_string()
            };
            let scale = if label.len() == 3 { 19. } else { 25. };
            let font = self.0.as_scaled(scale);
            let width: f32 = label
                .chars()
                .map(|ch| font.h_advance(font.glyph_id(ch)))
                .sum();
            let mut x = (SIZE as f32 - width) / 2.;
            let mut outlines = Vec::new();
            for ch in label.chars() {
                let id = font.glyph_id(ch);
                if let Some(glyph) = self
                    .0
                    .outline_glyph(id.with_scale_and_position(scale, point(x, 0.)))
                {
                    outlines.push(glyph);
                }
                x += font.h_advance(id);
            }
            let top = outlines
                .iter()
                .map(|g| g.px_bounds().min.y)
                .fold(f32::INFINITY, f32::min);
            let bottom = outlines
                .iter()
                .map(|g| g.px_bounds().max.y)
                .fold(f32::NEG_INFINITY, f32::max);
            let shift = ((SIZE as f32 - (bottom - top)) / 2. - top).round() as i32;
            for glyph in outlines {
                let bounds = glyph.px_bounds();
                glyph.draw(|x, y, coverage| {
                    let x = x as i32 + bounds.min.x as i32;
                    let y = y as i32 + bounds.min.y as i32 + shift;
                    if (0..SIZE as i32).contains(&x) && (0..SIZE as i32).contains(&y) {
                        let pixel = &mut rgba[(y as usize * SIZE + x as usize) * 4..][..4];
                        for channel in &mut pixel[..3] {
                            *channel = (*channel as f32 + (255. - *channel as f32) * coverage)
                                .round() as u8;
                        }
                    }
                });
            }
        }
        Frame {
            count,
            rgba,
            description: format!("{count} unread emails"),
        }
    }
}

pub(super) async fn run(
    mut counts: watch::Receiver<u64>,
    mut output: futures::channel::mpsc::Sender<super::Event>,
) {
    let renderer = Renderer::new();
    loop {
        let count = *counts.borrow_and_update();
        if output
            .send(super::Event::Overlay(Arc::new(renderer.render(count))))
            .await
            .is_err()
        {
            return;
        }
        if counts.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    #[test]
    fn taskbar_badge_is_transparent_at_zero_and_bounded_for_large_counts() {
        let renderer = Renderer::new();
        assert!(renderer.render(0).rgba.iter().all(|byte| *byte == 0));
        for count in [1, 9, 10, 99, 100, u64::MAX] {
            let frame = renderer.render(count);
            assert_eq!(frame.rgba.len(), SIZE * SIZE * 4);
            assert_eq!(frame.rgba[3], 0);
            assert!(
                frame
                    .rgba
                    .chunks_exact(4)
                    .any(|p| p[0] > 245 && p[1] > 245 && p[2] > 245 && p[3] == 255)
            );
            assert_eq!(frame.description, format!("{count} unread emails"));
            if let Some(directory) = std::env::var_os("SHEP_BADGE_EVIDENCE") {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                image::save_buffer_with_format(
                    directory.join(format!("taskbar-{count}.webp")),
                    &frame.rgba,
                    SIZE as u32,
                    SIZE as u32,
                    image::ColorType::Rgba8,
                    image::ImageFormat::WebP,
                )
                .unwrap();
            }
        }
        assert_eq!(renderer.render(100).rgba, renderer.render(u64::MAX).rgba);
    }
    #[tokio::test]
    async fn busy_taskbar_bridge_keeps_latest_count_without_unbounded_queue() {
        let (tx, rx) = watch::channel(1);
        let (output, mut events) = futures::channel::mpsc::channel(0);
        let worker = run(rx, output);
        tokio::pin!(worker);
        assert!(futures::poll!(&mut worker).is_pending());
        let super::super::Event::Overlay(first) = events.next().await.unwrap() else {
            panic!("overlay")
        };
        assert_eq!(first.count, 1);
        tx.send_replace(2);
        assert!(futures::poll!(&mut worker).is_pending());
        // This send is held in the actual production zero-capacity output sink.
        for count in 3..100 {
            tx.send_replace(count);
        }
        tx.send_replace(0);
        assert!(futures::poll!(&mut worker).is_pending());
        let super::super::Event::Overlay(held) = events.next().await.unwrap() else {
            panic!("overlay")
        };
        assert_eq!(held.count, 2);
        assert!(futures::poll!(&mut worker).is_pending());
        let super::super::Event::Overlay(latest) = events.next().await.unwrap() else {
            panic!("overlay")
        };
        assert_eq!(latest.count, 0);
        drop(tx);
        worker.await;
    }
}
