//! Owned visible text geometry. No DOM pointers outlive a layout, and hidden
//! quotations/preheaders cannot leak into a selection or clipboard operation.
use litehtml::{Document, FontHandle, Position};
type Measure<'a> = dyn Fn(&str, FontHandle) -> f32 + 'a;
struct Run {
    text: String,
    boundaries: Vec<usize>,
    font: FontHandle,
    bounds: Position,
}
#[derive(Default)]
pub(super) struct Selection {
    runs: Vec<Run>,
    spatial: Vec<usize>,
    max_height: f32,
    start: Option<(usize, usize)>,
    end: Option<(usize, usize)>,
}
impl Selection {
    pub fn layout(&mut self, document: &Document<'_>) {
        *self = Self::default();
        let mut elements: Vec<_> = document.root().into_iter().collect();
        while let Some(element) = elements.pop() {
            if element.is_text() {
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
                        text,
                        boundaries,
                        bounds,
                        font: element.font(),
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
