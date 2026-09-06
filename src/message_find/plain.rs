//! Independent text layout: never borrow iced's global UI font-system lock.
use super::{Match, Results, TextIndex};
use crate::model::MailDetail;
use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Wrap};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone)]
pub struct Request {
    pub revision: u64,
    pub source: Arc<MailDetail>,
    pub blocks: Vec<(usize, f32)>,
    pub query: String,
    pub match_case: bool,
    pub font_size: u16,
    pub current: Arc<AtomicU64>,
}

pub fn fonts() -> FontSystem {
    let mut fonts = FontSystem::new();
    fonts
        .db_mut()
        .load_font_data(include_bytes!("../../assets/NotoSans-Regular.ttf").to_vec());
    fonts
}

pub fn find(fonts: &mut FontSystem, request: &Request) -> Result<Results, String> {
    let current = || request.current.load(Ordering::Relaxed) == request.revision;
    let mut matches = Vec::new();
    for &(block, width) in &request.blocks {
        if !current() {
            return Ok(Results::default());
        }
        let Some(source) = (if block == 0 {
            Some(request.source.latest_body.as_str())
        } else {
            request
                .source
                .replies
                .get(block - 1)
                .map(|r| r.body.as_str())
        }) else {
            continue;
        };
        if !width.is_finite() || width <= 0. {
            return Err("Message width is not ready.".into());
        }
        let ranges = TextIndex::new(source)
            .find_while(&request.query, request.match_case, current)
            .map_err(|e| e.to_string())?;
        if ranges.is_empty() {
            continue;
        }
        let size = f32::from(request.font_size);
        let mut buffer = Buffer::new(fonts, Metrics::new(size, size * 1.5));
        buffer.set_size(fonts, Some(width), None);
        buffer.set_wrap(fonts, Wrap::WordOrGlyph);
        buffer.set_text(
            fonts,
            source,
            &Attrs::new().family(Family::Name("Noto Sans")),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(fonts, false);
        let mut offset = 0;
        let offsets: Vec<_> = buffer
            .lines
            .iter()
            .map(|line| {
                let start = offset;
                offset += line.text().len() + line.ending().as_str().len();
                start
            })
            .collect();
        let mut hits: Vec<_> = ranges
            .iter()
            .map(|_| Match {
                block,
                rectangles: Vec::new(),
            })
            .collect();
        for run in buffer.layout_runs() {
            if !current() {
                return Ok(Results::default());
            }
            for glyph in run.glyphs {
                let start = offsets[run.line_i] + glyph.start;
                let end = offsets[run.line_i] + glyph.end;
                let first = ranges.partition_point(|range| range.end <= start);
                for (i, _) in ranges
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take_while(|(_, range)| range.start < end)
                {
                    let rect = [
                        glyph.x,
                        run.line_top + 2.,
                        glyph.w,
                        (run.line_height - 4.).max(1.),
                    ];
                    let hit = &mut hits[i];
                    if let Some(last) = hit.rectangles.last_mut()
                        && (last[1] - rect[1]).abs() < 0.1
                        && ((last[0] + last[2] - rect[0]).abs() < 0.5
                            || (rect[0] + rect[2] - last[0]).abs() < 0.5)
                    {
                        let right = (last[0] + last[2]).max(rect[0] + rect[2]);
                        last[0] = last[0].min(rect[0]);
                        last[2] = right - last[0];
                    } else {
                        hit.rectangles.push(rect);
                    }
                }
            }
        }
        matches.extend(hits.into_iter().filter(|hit| !hit.rectangles.is_empty()));
    }
    Ok(Results::new(matches))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn wrapped_plain_matches_use_owned_layout_and_cancel_without_partial_geometry() {
        let store = crate::store::Store::memory().unwrap();
        let mail = crate::model::parse_mail("test", "plain-find", "INBOX", b"From: reader@example.test\r\nSubject: Find\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nA long first line with the project plan across a narrow reader.\r\nA second project plan.".to_vec(), false, false).unwrap();
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await.unwrap();
        let source = Arc::new(store.detail(id).await.unwrap());
        let request = Request {
            revision: 7,
            source,
            blocks: vec![(0, 140.)],
            query: "project plan".into(),
            match_case: false,
            font_size: 14,
            current: Arc::new(AtomicU64::new(7)),
        };
        tokio::task::spawn_blocking(move || {
            let mut fonts = fonts();
            let found = find(&mut fonts, &request).unwrap();
            assert_eq!(found.matches.len(), 2);
            assert!(found.matches[1].rectangles[0][1] > found.matches[0].rectangles[0][1]);
            assert!(found.matches.iter().all(|hit| {
                hit.rectangles
                    .iter()
                    .all(|r| r[0] >= 0. && r[2] > 0. && r[0] + r[2] <= 141.)
            }));
            let wide = find(
                &mut fonts,
                &Request {
                    blocks: vec![(0, 800.)],
                    ..request.clone()
                },
            )
            .unwrap();
            assert_eq!(wide.matches.len(), 2);
            assert!(wide.matches[1].rectangles[0][1] < found.matches[1].rectangles[0][1]);
            request.current.store(8, Ordering::Relaxed);
            assert!(find(&mut fonts, &request).unwrap().matches.is_empty());
        })
        .await
        .unwrap();
    }
}
