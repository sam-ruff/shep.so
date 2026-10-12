//! Recipient chips for the To, Cc and Bcc fields.
//!
//! Drafts keep the comma-separated string; chips are only the editing model.
//! `shared/recipient-cases.json` is the splitting contract shared with the
//! browser and Flutter clients.

#[derive(Debug, Clone, PartialEq)]
pub(in crate::ui) struct Chip {
    pub text: String,
    /// Whether Send would accept this address.
    pub valid: bool,
}

impl Chip {
    fn new(text: String) -> Self {
        let valid =
            crate::compose::recipients(&text, "").is_ok_and(|mailboxes| mailboxes.len() == 1);
        Self { text, valid }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::ui) struct Recipients {
    pub chips: Vec<Chip>,
    /// The unfinished text in the field's input.
    pub entry: String,
}

impl Recipients {
    pub fn parse(value: &str) -> Self {
        let mut recipients = Self::default();
        recipients.input(value);
        recipients.commit();
        recipients
    }

    /// The draft value: every chip, then the unfinished entry.
    pub fn value(&self) -> String {
        let mut parts: Vec<&str> = self.chips.iter().map(|chip| chip.text.as_str()).collect();
        if !self.entry.is_empty() {
            parts.push(&self.entry);
        }
        parts.join(", ")
    }

    /// Replaces the entry text, turning each finished address into a chip.
    pub fn input(&mut self, text: &str) {
        let (finished, entry) = split(text);
        self.chips.extend(finished.into_iter().map(Chip::new));
        self.entry = entry;
    }

    /// Turns the unfinished entry into a chip. Returns whether anything changed.
    pub fn commit(&mut self) -> bool {
        let text = self.entry.trim();
        if text.is_empty() {
            return !std::mem::take(&mut self.entry).is_empty();
        }
        let chip = Chip::new(text.to_owned());
        self.chips.push(chip);
        self.entry.clear();
        true
    }

    /// Removes a chip if it still holds the text the view showed.
    pub fn remove(&mut self, index: usize, text: &str) -> bool {
        if self.chips.get(index).is_none_or(|chip| chip.text != text) {
            return false;
        }
        self.chips.remove(index);
        true
    }

    /// Moves the last chip back into an empty entry for editing.
    pub fn reopen_last(&mut self) -> bool {
        if !self.entry.is_empty() {
            return false;
        }
        let Some(chip) = self.chips.pop() else {
            return false;
        };
        self.entry = chip.text;
        true
    }
}

impl super::history::Weight for Recipients {
    fn weight(&self) -> usize {
        self.entry.len() + self.chips.iter().map(|chip| chip.text.len()).sum::<usize>()
    }
}

/// Splits typed or pasted text into finished addresses and the unfinished rest.
///
/// Commas, semicolons and line breaks separate addresses, except inside a
/// quoted display name or `<...>`. A space also finishes a complete bare
/// address or one ending in `>`, so pasted space-separated lists split too.
pub(in crate::ui) fn split(text: &str) -> (Vec<String>, String) {
    let mut finished = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    let mut depth = 0usize;
    for c in text.chars() {
        if quoted {
            current.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        match c {
            '"' => {
                quoted = true;
                current.push(c);
            }
            '<' => {
                depth += 1;
                current.push(c);
            }
            '>' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' | ';' | '\n' | '\r' if depth == 0 => finish(&mut finished, &mut current),
            c if c.is_whitespace() && depth == 0 && complete(current.trim()) => {
                finish(&mut finished, &mut current)
            }
            c => current.push(c),
        }
    }
    (finished, current.trim_start().to_owned())
}

fn finish(finished: &mut Vec<String>, current: &mut String) {
    let text = current.trim();
    if !text.is_empty() {
        finished.push(text.to_owned());
    }
    current.clear();
}

fn complete(text: &str) -> bool {
    if text.ends_with('>') {
        return true;
    }
    let Some((local, domain)) = text.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !text.contains(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '(' | ')'))
}

/// Applies the input widget's own edit, from `before` to `after`, to `entry`.
///
/// Within one batch of events the native input keeps editing the text it was
/// last given, which trails the model once chips have been split off. Such an
/// edit is applied at the end of the entry, where typing and pasting happen.
pub(in crate::ui) fn apply_input(entry: &str, before: &str, after: &str) -> String {
    if before == entry {
        return after.to_owned();
    }
    let prefix: usize = before
        .chars()
        .zip(after.chars())
        .take_while(|(old, new)| old == new)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let suffix: usize = before[prefix..]
        .chars()
        .rev()
        .zip(after[prefix..].chars().rev())
        .take_while(|(old, new)| old == new)
        .map(|(c, _)| c.len_utf8())
        .sum();
    let removed = before[prefix..before.len() - suffix].chars().count();
    let kept = entry.chars().count().saturating_sub(removed);
    let mut text: String = entry.chars().take(kept).collect();
    text.push_str(&after[prefix..after.len() - suffix]);
    text
}

/// Keeps a copied column of addresses apart: text inputs drop line breaks.
pub(in crate::ui) fn paste_lines(text: &str) -> String {
    if !text.contains(['\n', '\r', '\t']) {
        return text.to_owned();
    }
    text.replace('\t', " ")
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn cases(section: &str) -> Vec<Value> {
        let cases: Value =
            serde_json::from_str(include_str!("../../../shared/recipient-cases.json"))
                .expect("recipient cases parse");
        cases[section].as_array().cloned().unwrap_or_default()
    }

    fn strings(value: &Value) -> Vec<String> {
        value
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn texts(recipients: &Recipients) -> Vec<String> {
        recipients
            .chips
            .iter()
            .map(|chip| chip.text.clone())
            .collect()
    }

    #[test]
    fn shared_split_cases() {
        let cases = cases("split");
        assert!(cases.len() >= 20);
        for case in cases {
            let (chips, entry) = split(case["input"].as_str().unwrap_or_default());
            assert_eq!(chips, strings(&case["chips"]), "{}", case["name"]);
            assert_eq!(
                entry,
                case["entry"].as_str().unwrap_or_default(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn shared_stored_cases() {
        for case in cases("stored") {
            let recipients = Recipients::parse(case["value"].as_str().unwrap_or_default());
            assert_eq!(
                texts(&recipients),
                strings(&case["chips"]),
                "{}",
                case["name"]
            );
            assert_eq!(recipients.entry, "", "{}", case["name"]);
        }
    }

    #[test]
    fn shared_value_cases() {
        for case in cases("value") {
            let recipients = Recipients {
                chips: strings(&case["chips"]).into_iter().map(Chip::new).collect(),
                entry: case["entry"].as_str().unwrap_or_default().to_owned(),
            };
            assert_eq!(
                recipients.value(),
                case["value"].as_str().unwrap_or_default(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn shared_paste_cases() {
        for case in cases("paste") {
            assert_eq!(
                paste_lines(case["clipboard"].as_str().unwrap_or_default()),
                case["text"].as_str().unwrap_or_default(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn chips_are_marked_as_send_would_treat_them() {
        for case in cases("valid") {
            let chip = Chip::new(case["chip"].as_str().unwrap_or_default().to_owned());
            assert_eq!(Some(chip.valid), case["valid"].as_bool(), "{}", chip.text);
        }
    }

    #[test]
    fn typing_then_committing_keeps_the_draft_value() {
        let mut recipients = Recipients::default();
        recipients.input("a@example.com; b@exa");
        assert_eq!(recipients.value(), "a@example.com, b@exa");
        recipients.input("b@example.com");
        assert_eq!(recipients.value(), "a@example.com, b@example.com");
        assert!(recipients.commit());
        assert_eq!(texts(&recipients), ["a@example.com", "b@example.com"]);
        assert_eq!(recipients.value(), "a@example.com, b@example.com");
        assert!(!recipients.commit());
    }

    #[test]
    fn semicolon_lists_become_sendable() {
        let mut recipients = Recipients::default();
        recipients.input("a@example.com;b@example.com");
        recipients.commit();
        let mailboxes = crate::compose::recipients(&recipients.value(), "To").expect("sendable");
        assert_eq!(mailboxes.len(), 2);
    }

    #[test]
    fn remove_checks_the_chip_it_was_shown() {
        let mut recipients = Recipients::parse("a@example.com, b@example.com");
        assert!(!recipients.remove(0, "b@example.com"));
        assert!(!recipients.remove(5, "a@example.com"));
        assert!(recipients.remove(1, "b@example.com"));
        assert_eq!(recipients.value(), "a@example.com");
    }

    #[test]
    fn reopening_moves_only_the_last_chip_into_an_empty_entry() {
        let mut recipients = Recipients::parse("a@example.com, b@example.com");
        recipients.entry = "c".into();
        assert!(!recipients.reopen_last());
        recipients.entry.clear();
        assert!(recipients.reopen_last());
        assert_eq!(recipients.entry, "b@example.com");
        assert_eq!(texts(&recipients), ["a@example.com"]);
        assert_eq!(recipients.value(), "a@example.com, b@example.com");
        assert!(!Recipients::default().reopen_last());
    }

    #[test]
    fn input_edits_apply_to_the_entry_even_when_the_widget_trails_it() {
        // In step with the model, the widget's text is the new entry.
        assert_eq!(apply_input("sam", "sam", "samx"), "samx");
        assert_eq!(apply_input("sam", "sam", "sxam"), "sxam");
        // The widget still shows text that has since become a chip.
        assert_eq!(
            apply_input("", "alex@example.com;", "alex@example.com; "),
            " "
        );
        assert_eq!(
            apply_input("s", "alex@example.com; s", "alex@example.com; sa"),
            "sa"
        );
        assert_eq!(apply_input("nobody", "", "x"), "nobodyx");
        assert_eq!(apply_input("ab", "zzab;", "zzab"), "a");
        assert_eq!(apply_input("é", "x; é", "x; éü"), "éü");
    }

    #[test]
    fn whitespace_only_entries_clear_without_a_chip() {
        let mut recipients = Recipients {
            entry: "  ".into(),
            ..Default::default()
        };
        assert!(recipients.commit());
        assert!(recipients.chips.is_empty());
        assert_eq!(recipients.entry, "");
    }
}
