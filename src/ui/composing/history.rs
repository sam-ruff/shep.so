//! Bounded undo/redo for one composer field.
//!
//! iced's text editor and inputs have no undo, so each draft session keeps
//! snapshots of its fields. Typing coalesces into word-sized steps.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

const STEPS: usize = 100;
const BYTES: usize = 4 << 20;
const PAUSE: Duration = Duration::from_secs(2);

pub(in crate::ui) trait Weight {
    fn weight(&self) -> usize;
}

impl Weight for String {
    fn weight(&self) -> usize {
        self.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum Edit {
    Insert {
        space: bool,
    },
    Delete,
    /// Paste, cut, a line break or a structural change: always its own step.
    Other,
}

impl Edit {
    /// Classifies a single-line field change by comparing its values.
    pub fn between(before: &str, after: &str) -> Self {
        let (old, new) = (before.chars().count(), after.chars().count());
        if new == old + 1 && after.starts_with(before) {
            let space = after.chars().last().is_some_and(char::is_whitespace);
            Edit::Insert { space }
        } else if new == old + 1 {
            Edit::Insert { space: false }
        } else if new + 1 == old {
            Edit::Delete
        } else {
            Edit::Other
        }
    }

    fn continues(self, next: Self) -> bool {
        match (self, next) {
            // A word ends where a space follows it.
            (Edit::Insert { space: false }, Edit::Insert { space: true }) => false,
            (Edit::Insert { .. }, Edit::Insert { .. }) | (Edit::Delete, Edit::Delete) => true,
            _ => false,
        }
    }
}

pub(in crate::ui) struct History<T> {
    undo: VecDeque<T>,
    redo: Vec<T>,
    bytes: usize,
    open: Option<(Edit, Instant)>,
}

impl<T> Default for History<T> {
    fn default() -> Self {
        Self {
            undo: VecDeque::new(),
            redo: Vec::new(),
            bytes: 0,
            open: None,
        }
    }
}

impl<T: Weight> History<T> {
    /// Saves the state before an edit unless the edit continues the open step.
    pub fn record(&mut self, edit: Edit, now: Instant, before: impl FnOnce() -> T) {
        let continues = self.open.is_some_and(|(last, at)| {
            now.saturating_duration_since(at) < PAUSE && last.continues(edit)
        });
        for state in self.redo.drain(..) {
            self.bytes -= state.weight();
        }
        self.open = (edit != Edit::Other).then_some((edit, now));
        if !continues {
            self.push(before());
        }
    }

    /// Ends the open step, so the next edit starts a new one.
    pub fn seal(&mut self) {
        self.open = None;
    }

    pub fn undo(&mut self, current: impl FnOnce() -> T) -> Option<T> {
        let previous = self.undo.pop_back()?;
        self.bytes -= previous.weight();
        let current = current();
        self.bytes += current.weight();
        self.redo.push(current);
        self.open = None;
        Some(previous)
    }

    pub fn redo(&mut self, current: impl FnOnce() -> T) -> Option<T> {
        let next = self.redo.pop()?;
        self.bytes -= next.weight();
        self.push(current());
        self.open = None;
        Some(next)
    }

    fn push(&mut self, state: T) {
        self.bytes += state.weight();
        self.undo.push_back(state);
        while self.undo.len() > STEPS || self.bytes > BYTES && self.undo.len() > 1 {
            if let Some(oldest) = self.undo.pop_front() {
                self.bytes -= oldest.weight();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(history: &mut History<String>, text: &mut String, typed: &str, now: Instant) {
        for c in typed.chars() {
            let before = text.clone();
            history.record(
                Edit::Insert {
                    space: c.is_whitespace(),
                },
                now,
                || before,
            );
            text.push(c);
        }
    }

    #[test]
    fn typing_undoes_a_word_at_a_time() {
        let now = Instant::now();
        let mut history = History::default();
        let mut text = String::new();
        typed(&mut history, &mut text, "hello big world", now);
        let current = text.clone();
        assert_eq!(history.undo(|| current).as_deref(), Some("hello big"));
        assert_eq!(
            history.undo(|| "hello big".into()).as_deref(),
            Some("hello")
        );
        assert_eq!(history.undo(|| "hello".into()).as_deref(), Some(""));
        assert_eq!(history.undo(String::new), None);
    }

    #[test]
    fn redo_replays_undone_steps_until_a_new_edit() {
        let now = Instant::now();
        let mut history = History::default();
        let mut text = String::new();
        typed(&mut history, &mut text, "one two", now);
        assert_eq!(history.undo(|| "one two".into()).as_deref(), Some("one"));
        assert_eq!(history.redo(|| "one".into()).as_deref(), Some("one two"));
        assert_eq!(history.undo(|| "one two".into()).as_deref(), Some("one"));
        history.record(Edit::Insert { space: false }, now, || "one".into());
        assert_eq!(history.redo(|| "one!".into()), None);
        assert_eq!(history.undo(|| "one!".into()).as_deref(), Some("one"));
    }

    #[test]
    fn a_pause_or_a_different_edit_starts_a_new_step() {
        let start = Instant::now();
        let mut history = History::default();
        let mut text = String::new();
        typed(&mut history, &mut text, "abc", start);
        typed(&mut history, &mut text, "def", start + PAUSE);
        history.record(Edit::Delete, start + PAUSE, || "abcdef".into());
        history.record(Edit::Delete, start + PAUSE, || "abcde".into());
        history.record(Edit::Other, start + PAUSE, || "abcd".into());
        history.record(Edit::Other, start + PAUSE, || "pasted".into());
        let undone: Vec<_> = std::iter::from_fn(|| history.undo(String::new)).collect();
        assert_eq!(undone, ["pasted", "abcd", "abcdef", "abc", ""]);
    }

    #[test]
    fn sealing_ends_the_open_step() {
        let now = Instant::now();
        let mut history = History::default();
        let mut text = String::new();
        typed(&mut history, &mut text, "ab", now);
        history.seal();
        typed(&mut history, &mut text, "cd", now);
        assert_eq!(history.undo(|| "abcd".into()).as_deref(), Some("ab"));
    }

    #[test]
    fn history_is_bounded_by_steps_and_bytes() {
        let now = Instant::now();
        let mut history = History::default();
        for step in 0..STEPS + 20 {
            history.record(Edit::Other, now, || step.to_string());
        }
        assert_eq!(history.undo.len(), STEPS);
        assert_eq!(history.undo.front().map(String::as_str), Some("20"));

        let mut large = History::default();
        let chunk = "x".repeat(BYTES / 3);
        for _ in 0..5 {
            large.record(Edit::Other, now, || chunk.clone());
        }
        assert!(large.bytes <= BYTES);
        assert_eq!(large.undo.len(), 3);
        large.undo(|| chunk.clone());
        large.record(Edit::Other, now, || chunk.clone());
        assert_eq!(
            large.bytes,
            large.undo.iter().map(String::len).sum::<usize>()
        );
    }

    #[test]
    fn field_changes_are_classified_by_their_values() {
        assert_eq!(Edit::between("ab", "abc"), Edit::Insert { space: false });
        assert_eq!(Edit::between("ab", "ab "), Edit::Insert { space: true });
        assert_eq!(Edit::between("ab", "axb"), Edit::Insert { space: false });
        assert_eq!(Edit::between("abc", "ab"), Edit::Delete);
        assert_eq!(Edit::between("ab", "ab, cd@example.com"), Edit::Other);
        assert_eq!(Edit::between("ab", "xy"), Edit::Other);
        assert_eq!(Edit::between("é", "éü"), Edit::Insert { space: false });
    }
}
