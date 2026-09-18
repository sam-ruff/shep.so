//! Test-support record of the mail rows the widget tree actually drew in the
//! latest frame, with their window positions, so native flows can detect stale,
//! duplicated or overlapping rows that the controller state alone cannot show.
use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub(super) struct DrawnRow {
    pub id: String,
    pub y: f32,
    pub height: f32,
}

#[derive(Default)]
struct Frames {
    frame: u64,
    rows: Vec<DrawnRow>,
}

/// Shared between the view tree and the controller; the root widget opens a
/// frame and every visible mail row appends itself in draw order.
#[derive(Default)]
pub(super) struct Log {
    frames: Mutex<Frames>,
}

impl Log {
    pub fn begin_frame(&self) {
        let Ok(mut frames) = self.frames.lock() else {
            return;
        };
        frames.frame += 1;
        frames.rows.clear();
    }
    pub fn row(&self, id: &str, bounds: iced::Rectangle) {
        let Ok(mut frames) = self.frames.lock() else {
            return;
        };
        frames.rows.push(DrawnRow {
            id: id.to_owned(),
            y: bounds.y,
            height: bounds.height,
        });
    }
    /// The last frame's counter and rows; draw and update share one thread, so
    /// a frame observed from the controller is complete.
    pub fn snapshot(&self) -> (u64, Vec<DrawnRow>) {
        self.frames
            .lock()
            .map(|frames| (frames.frame, frames.rows.clone()))
            .unwrap_or_default()
    }
}

/// What the drawn rows say about the list the controller holds.
#[derive(Debug, Default, PartialEq, serde::Serialize)]
pub(super) struct Review {
    pub frame: u64,
    pub ids: Vec<String>,
    pub rows: Vec<DrawnRow>,
    /// Ids drawn more than once.
    pub duplicates: Vec<String>,
    /// Ids drawn that the controller's page no longer lists.
    pub stale: Vec<String>,
    /// Drawn ids are not one contiguous slice of the page in page order.
    pub out_of_order: bool,
    /// Two drawn rows share vertical space.
    pub overlapping: bool,
    pub consistent: bool,
}

pub(super) fn review(frame: u64, rows: Vec<DrawnRow>, page: &[String], row_height: f32) -> Review {
    let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
    let mut seen = std::collections::HashSet::new();
    let mut duplicates: Vec<String> = ids
        .iter()
        .filter(|id| !seen.insert(id.as_str()))
        .cloned()
        .collect();
    duplicates.dedup();
    let stale: Vec<String> = ids
        .iter()
        .filter(|id| !page.contains(id))
        .cloned()
        .collect();
    let out_of_order = match ids
        .first()
        .and_then(|first| page.iter().position(|id| id == first))
    {
        Some(start) => page.get(start..start + ids.len()) != Some(ids.as_slice()),
        None => !ids.is_empty(),
    };
    let overlapping = rows
        .windows(2)
        .any(|pair| (pair[1].y - pair[0].y).abs() < row_height - 1. || pair[1].y < pair[0].y);
    let consistent = duplicates.is_empty() && stale.is_empty() && !out_of_order && !overlapping;
    Review {
        frame,
        ids,
        rows,
        duplicates,
        stale,
        out_of_order,
        overlapping,
        consistent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, y: f32) -> DrawnRow {
        DrawnRow {
            id: id.into(),
            y,
            height: 60.,
        }
    }
    fn page(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn a_contiguous_slice_in_page_order_is_consistent() {
        let result = review(
            3,
            vec![row("b", 200.), row("c", 260.), row("d", 320.)],
            &page(&["a", "b", "c", "d", "e"]),
            60.,
        );
        assert!(result.consistent, "{result:?}");
        assert_eq!(result.ids, page(&["b", "c", "d"]));
        assert_eq!(result.frame, 3);
    }

    #[test]
    fn duplicated_stale_reordered_and_overlapping_rows_are_reported() {
        let result = review(
            1,
            vec![row("a", 200.), row("a", 260.), row("z", 320.)],
            &page(&["a", "b"]),
            60.,
        );
        assert_eq!(result.duplicates, page(&["a"]));
        assert_eq!(result.stale, page(&["z"]));
        assert!(result.out_of_order);
        assert!(!result.consistent);
        let result = review(
            1,
            vec![row("a", 200.), row("b", 230.)],
            &page(&["a", "b"]),
            60.,
        );
        assert!(result.overlapping);
        assert!(!result.consistent);
        let result = review(
            1,
            vec![row("b", 200.), row("a", 260.)],
            &page(&["a", "b"]),
            60.,
        );
        assert!(result.out_of_order);
    }

    #[test]
    fn a_new_frame_replaces_the_previous_rows() {
        let log = Log::default();
        log.begin_frame();
        log.row(
            "a",
            iced::Rectangle::new(iced::Point::new(0., 10.), iced::Size::new(1., 60.)),
        );
        log.begin_frame();
        log.row(
            "b",
            iced::Rectangle::new(iced::Point::new(0., 70.), iced::Size::new(1., 60.)),
        );
        let (frame, rows) = log.snapshot();
        assert_eq!(frame, 2);
        assert_eq!(rows, vec![row("b", 70.)]);
        assert!(review(frame, rows, &page(&[]), 60.).stale == page(&["b"]));
    }
}
