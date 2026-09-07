//! Literal Unicode-aware search over displayed text, prepared off the UI thread.
//! Whitespace normalization bridges wrapped lines without losing source offsets.
use std::ops::Range;
pub mod plain;

#[derive(Debug, Clone)]
pub struct Match {
    pub block: usize,
    pub rectangles: Vec<[f32; 4]>,
}
#[derive(Debug, Clone)]
pub struct Highlight {
    pub matched: usize,
    pub block: usize,
    pub bounds: [f32; 4],
}
#[derive(Debug, Default)]
pub struct Results {
    pub matches: Vec<Match>,
    rectangles: Vec<Highlight>,
    max_height: f32,
}
impl Results {
    pub fn new(mut matches: Vec<Match>) -> Self {
        for hit in &mut matches {
            let mut merged: Vec<[f32; 4]> = Vec::new();
            for rect in hit.rectangles.drain(..) {
                if let Some(last) = merged.last_mut()
                    && (last[1] - rect[1]).abs() < 0.5
                    && (last[3] - rect[3]).abs() < 0.5
                    && rect[0] <= last[0] + last[2] + 3.
                    && last[0] <= rect[0] + rect[2] + 3.
                {
                    let right = (last[0] + last[2]).max(rect[0] + rect[2]);
                    last[0] = last[0].min(rect[0]);
                    last[2] = right - last[0];
                } else {
                    merged.push(rect);
                }
            }
            hit.rectangles = merged;
        }
        let mut rectangles: Vec<_> = matches
            .iter()
            .enumerate()
            .flat_map(|(matched, hit)| {
                hit.rectangles.iter().map(move |&bounds| Highlight {
                    matched,
                    block: hit.block,
                    bounds,
                })
            })
            .collect();
        rectangles.sort_by(|a, b| {
            a.block
                .cmp(&b.block)
                .then(a.bounds[1].total_cmp(&b.bounds[1]))
        });
        let max_height = rectangles.iter().map(|r| r.bounds[3]).fold(0., f32::max);
        Self {
            matches,
            rectangles,
            max_height,
        }
    }
    pub fn visible(&self, block: usize, top: f32, bottom: f32) -> &[Highlight] {
        let start = self.rectangles.partition_point(|r| {
            r.block < block || (r.block == block && r.bounds[1] + self.max_height < top)
        });
        let end = self
            .rectangles
            .partition_point(|r| r.block < block || (r.block == block && r.bounds[1] <= bottom));
        &self.rectangles[start..end]
    }
}

#[derive(Debug, Default)]
pub struct TextIndex {
    text: String,
    shifts: Vec<(usize, usize)>,
}
impl TextIndex {
    pub fn new(source: &str) -> Self {
        let mut index = Self::default();
        let mut whitespace = false;
        for (offset, character) in source.char_indices() {
            if character.is_whitespace() {
                if !whitespace {
                    index.text.push(' ');
                    whitespace = true;
                }
            } else {
                if whitespace {
                    index.shift(offset);
                    whitespace = false;
                }
                index.text.push(character);
            }
        }
        if whitespace {
            index.shift(source.len());
        }
        index
    }
    fn shift(&mut self, original_end: usize) {
        let delta = original_end - self.text.len();
        if self.shifts.last().map_or(0, |(_, shift)| *shift) != delta {
            self.shifts.push((self.text.len(), delta));
        }
    }
    fn original(&self, offset: usize) -> usize {
        let count = self.shifts.partition_point(|(end, _)| *end <= offset);
        offset + count.checked_sub(1).map_or(0, |i| self.shifts[i].1)
    }
    pub fn find(&self, query: &str, match_case: bool) -> Result<Vec<Range<usize>>, regex::Error> {
        self.find_while(query, match_case, || true)
    }
    pub fn find_while(
        &self,
        query: &str,
        match_case: bool,
        current: impl Fn() -> bool,
    ) -> Result<Vec<Range<usize>>, regex::Error> {
        let query = Self::new(query);
        if query.text.trim().is_empty() || !current() {
            return Ok(Vec::new());
        }
        let pattern = regex::RegexBuilder::new(&regex::escape(&query.text))
            .case_insensitive(!match_case)
            .build()?;
        let mut matches = Vec::new();
        for found in pattern.find_iter(&self.text) {
            if !current() {
                return Ok(Vec::new());
            }
            matches.push(self.original(found.start())..self.original(found.end()));
        }
        Ok(matches)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_unicode_case_and_original_whitespace_offsets() {
        let source = "Before\r\n  CAFÉ\u{a0}\tplan [a.*] café plan Σ σ ς 日本語";
        let index = TextIndex::new(source);
        let found = index.find("café plan", false).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(&source[found[0].clone()], "CAFÉ\u{a0}\tplan");
        assert_eq!(&source[found[1].clone()], "café plan");
        assert_eq!(index.find("café plan", true).unwrap().len(), 1);
        assert_eq!(index.find("[a.*]", false).unwrap().len(), 1);
        assert_eq!(index.find("σ", false).unwrap().len(), 3);
        assert_eq!(index.find("日本", false).unwrap().len(), 1);
        assert!(index.find("not present", false).unwrap().is_empty());
    }

    #[test]
    fn trailing_whitespace_empty_input_and_large_text_have_correct_ranges() {
        for source in ["word\r\n\t", "\u{a0}word\t\t", "word", "", "  "] {
            let index = TextIndex::new(source);
            for query in ["word", "word ", " word"] {
                for range in index.find(query, false).unwrap() {
                    assert!(source.get(range).is_some());
                }
            }
            assert!(index.find("\r\n", false).unwrap().is_empty());
        }
        let source = format!("{}end marker", "Long letter.\n".repeat(10_000));
        assert_eq!(
            TextIndex::new(&source).find("end marker", false).unwrap(),
            vec![source.len() - 10..source.len()]
        );
    }

    #[test]
    fn cancelled_search_does_not_publish_partial_results() {
        let index = TextIndex::new("one one one one");
        let calls = std::cell::Cell::new(0);
        let found = index
            .find_while("one", false, || {
                calls.set(calls.get() + 1);
                calls.get() < 3
            })
            .unwrap();
        assert!(found.is_empty());
    }
}
