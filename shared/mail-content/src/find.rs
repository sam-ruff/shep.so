//! Literal search matching the desktop TextIndex contract. Off-thread callers
//! receive UTF-16 offsets for Flutter/DOM selection without changing source text.
use serde::{Deserialize, Serialize};
use std::ops::Range;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hit {
    pub block: usize,
    pub start: usize,
    pub end: usize,
}
pub fn find(blocks: &[String], query: &str, match_case: bool) -> Result<Vec<Hit>, regex::Error> {
    let mut hits = Vec::new();
    for (block, source) in blocks.iter().enumerate() {
        let ranges = TextIndex::new(source).find(query, match_case)?;
        // Sorted matches allow a single UTF-8 -> UTF-16 pass, even when every
        // character matches. Repeated prefix scans would be quadratic.
        let mut chars = source.char_indices().peekable();
        let mut utf16 = 0;
        for range in ranges {
            while chars.peek().is_some_and(|(byte, _)| *byte < range.start) {
                utf16 += chars.next().unwrap().1.len_utf16();
            }
            let start = utf16;
            while chars.peek().is_some_and(|(byte, _)| *byte < range.end) {
                utf16 += chars.next().unwrap().1.len_utf16();
            }
            hits.push(Hit {
                block,
                start,
                end: utf16,
            });
        }
    }
    Ok(hits)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_contract_preserves_utf16_ranges_for_shared_client_cases() {
        let cases: serde_json::Value =
            serde_json::from_str(include_str!("../../find-cases.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let blocks: Vec<String> = serde_json::from_value(case["blocks"].clone()).unwrap();
            let actual = find(
                &blocks,
                case["query"].as_str().unwrap(),
                case["match_case"].as_bool().unwrap(),
            )
            .unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                case["hits"],
                "{case}"
            );
        }
    }
    #[test]
    fn repeated_matches_and_cancellation_do_not_change_offsets() {
        let text = "😀a ".repeat(20_000);
        let hits = find(&[text], "a", false).unwrap();
        assert_eq!(hits.len(), 20_000);
        assert_eq!(hits.last().unwrap().start, 79_998);
        assert!(
            TextIndex::new("match match")
                .find_while("match", false, || false)
                .unwrap()
                .is_empty()
        );
    }
}
