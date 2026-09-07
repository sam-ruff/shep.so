//! Owned visible text geometry. No DOM pointers outlive a layout, and hidden
//! quotations/preheaders cannot leak into a selection or clipboard operation.
use litehtml::{Document, FontHandle, Position};
use std::hash::{Hash, Hasher};
type Measure<'a> = dyn Fn(&str, FontHandle) -> f32 + 'a;
struct Run {
    identity: usize,
    text: String,
    boundaries: Vec<usize>,
    font: FontHandle,
    bounds: Position,
    offset: std::ops::Range<usize>,
}
pub(super) struct Anchor {
    identity: usize,
    fingerprint: u64,
    y: f32,
}
fn fingerprint(text: &str) -> u64 {
    let mut hash = std::hash::DefaultHasher::new();
    text.hash(&mut hash);
    hash.finish()
}
#[derive(Default)]
pub(super) struct Selection {
    runs: Vec<Run>,
    spatial: Vec<usize>,
    max_height: f32,
    start: Option<(usize, usize)>,
    end: Option<(usize, usize)>,
    index: crate::message_find::TextIndex,
}
impl Selection {
    pub fn anchor(&self, top: f32, left: f32, width: f32, height: f32) -> Option<Anchor> {
        if top <= 0. {
            return None;
        }
        let begin = self
            .spatial
            .partition_point(|&i| self.runs[i].bounds.y + self.max_height <= top);
        self.spatial[begin..]
            .iter()
            .copied()
            .take_while(|&i| self.runs[i].bounds.y < top + height)
            .find(|&i| {
                let run = &self.runs[i];
                run.bounds.y + run.bounds.height > top
                    && run.bounds.x + run.bounds.width > left
                    && run.bounds.x < left + width
                    && !run.text.trim().is_empty()
            })
            .map(|run| Anchor {
                identity: self.runs[run].identity,
                fingerprint: fingerprint(&self.runs[run].text),
                y: self.runs[run].bounds.y,
            })
    }
    pub fn displacement(&self, anchor: Anchor) -> Option<f32> {
        self.runs
            .binary_search_by_key(&anchor.identity, |run| run.identity)
            .ok()
            .and_then(|index| self.runs.get(index))
            .filter(|run| fingerprint(&run.text) == anchor.fingerprint)
            .map(|run| run.bounds.y - anchor.y)
    }
    pub fn layout(&mut self, document: &Document<'_>) {
        *self = Self::default();
        let mut elements: Vec<_> = document.root().into_iter().collect();
        let mut identity = 0;
        while let Some(element) = elements.pop() {
            if element.is_text() {
                let node = identity;
                identity += 1;
                let bounds = element.placement();
                if bounds.width > 0. && bounds.height > 0. {
                    let text = element.get_text();
                    let boundaries = text
                        .char_indices()
                        .map(|(i, _)| i)
                        .chain(std::iter::once(text.len()))
                        .collect();
                    self.max_height = self.max_height.max(bounds.height);
                    self.runs.push(Run {
                        identity: node,
                        text,
                        boundaries,
                        bounds,
                        font: element.font(),
                        offset: 0..0,
                    });
                }
            }
            for index in (0..element.children_count()).rev() {
                if let Some(child) = element.child_at(index) {
                    elements.push(child);
                }
            }
        }
        self.spatial = (0..self.runs.len()).collect();
        self.spatial
            .sort_by(|&a, &b| self.runs[a].bounds.y.total_cmp(&self.runs[b].bounds.y));
        let mut text = String::new();
        let mut last: Option<Position> = None;
        for run in &mut self.runs {
            if let Some(previous) = last {
                if run.bounds.y >= previous.y + previous.height * 0.8
                    || run.bounds.y + run.bounds.height <= previous.y
                {
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                } else if run.bounds.x > previous.x + previous.width + 2.
                    && !text.ends_with(char::is_whitespace)
                {
                    text.push('\t');
                }
            }
            let start = text.len();
            text.push_str(&run.text);
            run.offset = start..text.len();
            last = Some(run.bounds);
        }
        self.index = crate::message_find::TextIndex::new(&text);
    }
    pub fn find(
        &self,
        query: &str,
        match_case: bool,
        measure: &Measure<'_>,
    ) -> Result<crate::message_find::Results, regex::Error> {
        let mut matches = Vec::new();
        for range in self.index.find(query, match_case)? {
            let first = self
                .runs
                .partition_point(|run| run.offset.end <= range.start);
            let mut rectangles = Vec::new();
            for run in self.runs[first..]
                .iter()
                .take_while(|run| run.offset.start < range.end)
            {
                let from = range.start.saturating_sub(run.offset.start);
                let to = range.end.min(run.offset.end) - run.offset.start;
                let left = measure(&run.text[..from], run.font);
                let right = measure(&run.text[..to], run.font);
                rectangles.push([
                    run.bounds.x + left,
                    run.bounds.y,
                    (right - left).max(1.),
                    run.bounds.height,
                ]);
            }
            if !rectangles.is_empty() {
                matches.push(crate::message_find::Match {
                    block: 0,
                    rectangles,
                });
            }
        }
        Ok(crate::message_find::Results::new(matches))
    }
    fn hit(&self, measure: &Measure<'_>, x: f32, y: f32) -> Option<(usize, usize)> {
        if self.runs.is_empty() {
            return None;
        }
        let begin = self
            .spatial
            .partition_point(|&i| self.runs[i].bounds.y + self.max_height < y)
            .saturating_sub(1);
        let end = (self
            .spatial
            .partition_point(|&i| self.runs[i].bounds.y <= y)
            + 1)
        .min(self.spatial.len());
        let index = *self.spatial[begin..end].iter().min_by(|&&a, &&b| {
            let distance = |i: usize| {
                let p = self.runs[i].bounds;
                let dy = (p.y - y).max(0.) + (y - p.y - p.height).max(0.);
                let dx = (p.x - x).max(0.) + (x - p.x - p.width).max(0.);
                (dy, dx)
            };
            let (ay, ax) = distance(a);
            let (by, bx) = distance(b);
            ay.total_cmp(&by).then(ax.total_cmp(&bx))
        })?;
        let run = &self.runs[index];
        let local = x - run.bounds.x;
        let (mut lo, mut hi) = (0, run.boundaries.len() - 1);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let left = measure(&run.text[..run.boundaries[mid]], run.font);
            let right = measure(&run.text[..run.boundaries[mid + 1]], run.font);
            if local < (left + right) / 2. {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        Some((index, run.boundaries[lo]))
    }
    pub fn start(&mut self, measure: &Measure<'_>, x: f32, y: f32) {
        self.start = self.hit(measure, x, y);
        self.end = self.start;
    }
    pub fn extend(&mut self, measure: &Measure<'_>, x: f32, y: f32) {
        if self.start.is_some() {
            self.end = self.hit(measure, x, y);
        }
    }
    pub fn all(&mut self) {
        if let Some(last) = self.runs.last() {
            self.start = Some((0, 0));
            self.end = Some((self.runs.len() - 1, last.text.len()));
        }
    }
    pub fn result(&self, measure: &Measure<'_>) -> (String, Vec<[f32; 4]>) {
        let (Some(a), Some(b)) = (self.start, self.end) else {
            return (String::new(), Vec::new());
        };
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        let mut text = String::new();
        let mut rectangles = Vec::new();
        let mut last: Option<Position> = None;
        for i in start.0..=end.0 {
            let run = &self.runs[i];
            let from = if i == start.0 { start.1 } else { 0 };
            let to = if i == end.0 { end.1 } else { run.text.len() };
            if from == to {
                continue;
            }
            if let Some(last) = last {
                if run.bounds.y >= last.y + last.height * 0.8
                    || run.bounds.y + run.bounds.height <= last.y
                {
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                } else if run.bounds.x > last.x + last.width + 2.
                    && !text.ends_with(char::is_whitespace)
                {
                    text.push('\t');
                }
            }
            text.push_str(&run.text[from..to]);
            let x = measure(&run.text[..from], run.font);
            let width = measure(&run.text[..to], run.font) - x;
            rectangles.push([
                run.bounds.x + x,
                run.bounds.y,
                width.max(0.),
                run.bounds.height,
            ]);
            last = Some(run.bounds);
        }
        (text, rectangles)
    }
}
