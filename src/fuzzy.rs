use std::collections::HashSet;

use nucleo_matcher::{Config, Utf32Str};
use rapidfuzz::distance::osa;
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

pub(crate) fn normalized(value: &str) -> String {
    // Strip Latin accents like SQLite's unicode61 tokenizer. Preserve marks
    // in other scripts and recompose Hangul/Japanese after normalization.
    let mut latin = false;
    value
        .nfd()
        .filter(|c| {
            if is_combining_mark(*c) {
                return !latin;
            }
            latin = c.is_ascii_alphabetic();
            true
        })
        .flat_map(char::to_lowercase)
        .nfc()
        .collect()
}

pub fn distance(a: &str, b: &str) -> usize {
    osa::distance(a.chars(), b.chars())
}

pub fn score(query: &str, candidate: &str) -> Option<usize> {
    Matcher::new(query).score(candidate)
}

struct Term {
    text: String,
    chars: Vec<char>,
    numeric: bool,
}

fn literal_matcher() -> nucleo_matcher::Matcher {
    let mut config = Config::DEFAULT;
    config.normalize = false;
    config.ignore_case = false;
    nucleo_matcher::Matcher::new(config)
}

/// Score already-normalised catalogue words using one query's cached needles.
pub struct WordMatcher {
    terms: Vec<Term>,
    matcher: nucleo_matcher::Matcher,
    candidate_chars: Vec<char>,
}

impl WordMatcher {
    pub fn new(query: &str) -> Self {
        let terms = normalized(query)
            .split(|c: char| !c.is_alphanumeric())
            .filter(|term| !term.is_empty())
            .map(|text| Term {
                text: text.to_owned(),
                chars: text.chars().collect(),
                numeric: text.chars().any(char::is_numeric),
            })
            .collect();
        Self {
            terms,
            matcher: literal_matcher(),
            candidate_chars: Vec::new(),
        }
    }

    pub fn score_normalized(&mut self, query: &str, candidate: &str) -> Option<usize> {
        let term = self.terms.iter().find(|term| term.text == query)?;
        if query == candidate {
            return Some(0);
        }
        if term.numeric {
            return None;
        }
        if candidate.starts_with(query) {
            return Some(2);
        }
        self.candidate_chars.clear();
        self.candidate_chars.extend(candidate.chars());
        if let Some(score) = self.matcher.fuzzy_match(
            Utf32Str::Unicode(&self.candidate_chars),
            Utf32Str::Unicode(&term.chars),
        ) {
            return Some(10 + 64usize.saturating_sub(usize::from(score) / term.chars.len()));
        }
        let cutoff = match term.chars.len() {
            0..=3 => return None,
            4..=5 => 1,
            _ => 2,
        };
        if term.chars.len().abs_diff(self.candidate_chars.len()) > cutoff {
            return None;
        }
        osa::distance_with_args(
            term.chars.iter().copied(),
            self.candidate_chars.iter().copied(),
            &osa::Args::default().score_cutoff(cutoff),
        )
        .map(|edits| 60 + edits * 10)
    }
}

/// Reuse Nucleo's scratch allocation across a complete result batch.
pub struct Matcher {
    query: String,
    terms: Vec<Term>,
    numeric_tokens: Vec<String>,
    matcher: nucleo_matcher::Matcher,
    candidate_chars: Vec<char>,
}

impl Matcher {
    pub fn new(query: &str) -> Self {
        let query = normalized(query.trim());
        let numeric_tokens = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|token| token.chars().any(char::is_numeric))
            .map(str::to_owned)
            .collect();
        let terms = query
            .split_whitespace()
            .map(|text| Term {
                text: text.to_owned(),
                chars: text.chars().collect(),
                numeric: text.chars().any(char::is_numeric),
            })
            .collect();
        Self {
            query,
            terms,
            numeric_tokens,
            matcher: literal_matcher(),
            candidate_chars: Vec::new(),
        }
    }

    pub fn score(&mut self, candidate: &str) -> Option<usize> {
        self.score_normalized(&normalized(candidate))
    }

    fn score_normalized(&mut self, candidate: &str) -> Option<usize> {
        let query = self.query.as_str();
        if query.is_empty() {
            return Some(0);
        }
        if !query.chars().any(char::is_alphanumeric) {
            return None;
        }
        let words: Vec<_> = candidate
            .split(|c: char| !c.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        if self
            .numeric_tokens
            .iter()
            .any(|token| !words.contains(&token.as_str()))
        {
            return None;
        }
        if query == candidate {
            return Some(0);
        }
        let leaf = candidate.rsplit(['/', '\\']).next().unwrap_or(candidate);
        if query == leaf {
            return Some(10);
        }
        if candidate.starts_with(query) {
            return Some(20 + candidate.len().saturating_sub(query.len()).min(20));
        }
        if leaf.starts_with(query) {
            return Some(45 + leaf.len().saturating_sub(query.len()).min(10));
        }
        if let Some(index) = candidate.find(query) {
            return Some(60 + index.min(30));
        }
        self.candidate_chars.clear();
        self.candidate_chars.extend(candidate.chars());
        let mut tier = 0;
        let mut penalty = 0;
        for term in &self.terms {
            if words.contains(&term.text.as_str()) {
                continue;
            }
            if words.iter().any(|word| word.starts_with(&term.text)) {
                penalty += 5;
                continue;
            }
            // Low-level matching keeps punctuation literal instead of parsing
            // Nucleo's optional negation, anchoring or alternation syntax.
            if let Some(score) = self.matcher.fuzzy_match(
                Utf32Str::Unicode(&self.candidate_chars),
                Utf32Str::Unicode(&term.chars),
            ) {
                tier = tier.max(1);
                penalty += 100usize.saturating_sub(usize::from(score) / term.chars.len());
                continue;
            }
            if !term.text.chars().all(char::is_alphabetic) || term.chars.len() < 3 {
                return None;
            }
            let maximum = if term.chars.len() >= 6 { 2 } else { 1 };
            let edits = words
                .iter()
                .filter_map(|word| {
                    osa::distance_with_args(
                        term.chars.iter().copied(),
                        word.chars(),
                        &osa::Args::default().score_cutoff(maximum),
                    )
                    .map(|edits| edits * 20 + word.chars().count().abs_diff(term.chars.len()))
                })
                .min()?;
            tier = tier.max(2);
            penalty += edits;
        }
        Some(100 + tier * 100 + (penalty / self.terms.len()).min(99))
    }
}

pub fn ranked(query: &str, choices: impl IntoIterator<Item = String>) -> Vec<String> {
    ranked_labels(
        query,
        choices.into_iter().map(|value| (value.clone(), value)),
    )
}

/// Rank visible folder names while retaining their exact protocol identifiers.
pub fn ranked_labels(
    query: &str,
    choices: impl IntoIterator<Item = (String, String)>,
) -> Vec<String> {
    let mut matcher = Matcher::new(query);
    let mut matches: Vec<_> = choices
        .into_iter()
        .filter_map(|(value, label)| {
            let key = normalized(&label);
            matcher
                .score_normalized(&key)
                .map(|score| (score, key, value))
        })
        .collect();
    matches.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
    });
    matches.into_iter().map(|(_, _, value)| value).collect()
}

pub fn literal_query(input: &str) -> String {
    search_tokens(input)
        .into_iter()
        .map(|token| format!("\"{token}\""))
        .collect::<Vec<_>>()
        .join(" AND ")
}

pub fn phrase_query(input: &str) -> String {
    format!("\"{}\"", search_tokens(input).join(" "))
}

fn search_tokens(input: &str) -> Vec<String> {
    normalized(input)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(12)
        .map(str::to_owned)
        .collect()
}

pub(crate) fn nearby_terms(
    connection: &rusqlite::Connection,
    token: &str,
    prefix: &str,
    limit: usize,
) -> anyhow::Result<Vec<String>> {
    let end = format!("{prefix}~");
    let mut statement = connection.prepare_cached(
        "SELECT term FROM mail_vocab WHERE term>=?1 AND term<=?2 ORDER BY term LIMIT ?3",
    )?;
    let mut terms = Vec::new();
    let mut read = |lower: &str, upper: &str| -> anyhow::Result<()> {
        let remaining = limit.saturating_sub(terms.len());
        if remaining == 0 {
            return Ok(());
        }
        let remaining = i64::try_from(remaining)?;
        for term in statement.query_map(rusqlite::params![lower, upper, remaining], |row| {
            row.get::<_, String>(0)
        })? {
            terms.push(term?);
        }
        Ok(())
    };
    if token.starts_with(prefix) && token.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        if let Some(last) = token.bytes().last() {
            // Vocab cursors count postings even at an excluded strict boundary.
            // Tokeniser terms cannot contain the noncharacter U+10FFFF.
            let before = format!(
                "{}{}\u{10ffff}",
                &token[..token.len() - 1],
                char::from(last - 1)
            );
            read(prefix, &before)?;
            read(&format!("{token}\0"), &end)?;
        }
    } else {
        read(prefix, &end)?;
    }
    Ok(terms)
}

fn prefix_needed(connection: &rusqlite::Connection, token: &str) -> anyhow::Result<bool> {
    if token.chars().all(char::is_numeric) {
        return Ok(false);
    }
    if !token.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return Ok(true);
    }
    // A missing longer term makes the exact lookup equivalent in this snapshot,
    // without asking FTS5 to merge a common word's prefix posting lists.
    Ok(connection
        .prepare_cached(
            "SELECT EXISTS(SELECT 1 FROM mail_vocab WHERE term>=?1 AND term<=?2 LIMIT 1)",
        )?
        .query_row(
            rusqlite::params![format!("{token}\0"), format!("{token}\u{10ffff}")],
            |row| row.get(0),
        )?)
}

fn retain_possible_variants(
    variants: &mut HashSet<String>,
    prefix: &str,
    nearby: &[String],
    limit: usize,
) {
    if nearby.len() >= limit {
        return;
    }
    let present: HashSet<_> = nearby.iter().map(String::as_str).collect();
    // Only a complete range proves that an unlisted spelling is absent.
    variants.retain(|variant| !variant.starts_with(prefix) || present.contains(variant.as_str()));
}

pub fn mail_query(connection: &rusqlite::Connection, input: &str) -> anyhow::Result<String> {
    let tokens = search_tokens(input);
    let mut groups = Vec::new();
    for token in tokens {
        let mut alternatives = vec![format!(
            "\"{}\"{}",
            token,
            if prefix_needed(connection, &token)? {
                "*"
            } else {
                ""
            }
        )];
        if token.is_ascii()
            && token.chars().all(char::is_alphabetic)
            && (4..=24).contains(&token.len())
        {
            let mut variants = HashSet::new();
            for index in 0..=token.len() {
                for letter in 'a'..='z' {
                    let mut inserted = token.clone();
                    inserted.insert(index, letter);
                    variants.insert(inserted);
                    if index < token.len() {
                        let mut changed = token.clone();
                        changed.replace_range(index..index + 1, &letter.to_string());
                        variants.insert(changed);
                    }
                }
                if index < token.len() {
                    let mut deleted = token.clone();
                    deleted.remove(index);
                    variants.insert(deleted);
                }
                if index + 1 < token.len() {
                    let mut bytes = token.as_bytes().to_vec();
                    bytes.swap(index, index + 1);
                    variants.insert(String::from_utf8(bytes)?);
                }
            }
            // The exact token already has its own literal/prefix alternative.
            variants.remove(&token);
            let prefix = &token[..2];
            let nearby_limit = if token.len() >= 6 { 256 } else { 1 };
            let nearby = nearby_terms(connection, &token, prefix, nearby_limit)?;
            retain_possible_variants(&mut variants, prefix, &nearby, nearby_limit);
            let values: Vec<_> = variants.into_iter().collect();
            let mut statement = connection.prepare_cached(
                "SELECT term FROM mail_vocab WHERE term IN (SELECT value FROM json_each(?)) ORDER BY doc DESC,term LIMIT 32",
            )?;
            let mut terms = Vec::new();
            for term in
                statement.query_map([serde_json::to_string(&values)?], |r| r.get::<_, String>(0))?
            {
                let term = term?;
                if term != token {
                    terms.push(term);
                }
            }
            let comparator = osa::BatchComparator::new(token.chars());
            if token.len() >= 6 {
                for word in nearby {
                    if comparator
                        .distance_with_args(word.chars(), &osa::Args::default().score_cutoff(2))
                        .is_some()
                    {
                        terms.push(word);
                    }
                }
            }
            terms.sort_by_cached_key(|term| {
                (
                    comparator.distance(term.chars()),
                    term.len().abs_diff(token.len()),
                    term.clone(),
                )
            });
            terms.dedup();
            for term in terms.into_iter().filter(|term| term != &token).take(12) {
                alternatives.push(format!("\"{term}\""));
            }
        }
        groups.push(format!("({})", alternatives.join(" OR ")));
    }
    Ok(groups.join(" AND "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_prefix_scan_keeps_corrections_with_other_prefixes() -> anyhow::Result<()> {
        let connection = rusqlite::Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE VIRTUAL TABLE mail_search USING fts5(body);
             CREATE VIRTUAL TABLE mail_vocab USING fts5vocab(mail_search, 'row');
             INSERT INTO mail_search(body) VALUES ('nilestone qlans');",
        )?;
        assert!(mail_query(&connection, "milestone")?.contains("\"nilestone\""));
        assert!(mail_query(&connection, "plans")?.contains("\"qlans\""));
        Ok(())
    }

    #[test]
    fn full_prefix_scan_retains_one_edit_matches_beyond_its_limit() -> anyhow::Result<()> {
        let connection = rusqlite::Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE VIRTUAL TABLE mail_search USING fts5(body);
             CREATE VIRTUAL TABLE mail_vocab USING fts5vocab(mail_search, 'row');
             INSERT INTO mail_search(body) VALUES ('milestone milestnoe');",
        )?;
        for index in 0..256 {
            connection.execute(
                "INSERT INTO mail_search(body) VALUES (?)",
                [format!("mia{index:03}")],
            )?;
        }
        let scanned = nearby_terms(&connection, "milestone", "mi", 256)?;
        assert_eq!(scanned.len(), 256);
        assert!(!scanned.iter().any(|term| term == "milestnoe"));
        assert!(mail_query(&connection, "milestone")?.contains("\"milestnoe\""));
        Ok(())
    }

    #[test]
    fn prefix_proof_observes_vocabulary_inserts_and_deletes() -> anyhow::Result<()> {
        let connection = rusqlite::Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE VIRTUAL TABLE mail_search USING fts5(body);
             CREATE VIRTUAL TABLE mail_vocab USING fts5vocab(mail_search, 'row');
             INSERT INTO mail_search(rowid,body) VALUES (1,'milestone');",
        )?;
        assert!(!prefix_needed(&connection, "milestone")?);
        assert_eq!(mail_query(&connection, "milestone")?, "(\"milestone\")");
        connection.execute(
            "INSERT INTO mail_search(rowid,body) VALUES (2,'milestone世界')",
            [],
        )?;
        assert!(prefix_needed(&connection, "milestone")?);
        assert!(mail_query(&connection, "milestone")?.starts_with("(\"milestone\"*"));
        connection.execute("DELETE FROM mail_search WHERE rowid=2", [])?;
        assert!(!prefix_needed(&connection, "milestone")?);
        assert!(!prefix_needed(&connection, "17")?);
        assert!(prefix_needed(&connection, "s3")?);
        assert!(prefix_needed(&connection, "日本")?);
        Ok(())
    }

    #[test]
    fn vocabulary_budget_counts_neighbours_on_both_sides_of_exact_token() -> anyhow::Result<()> {
        let connection = rusqlite::Connection::open_in_memory()?;
        connection.execute_batch(
            "CREATE VIRTUAL TABLE mail_search USING fts5(body);
             CREATE VIRTUAL TABLE mail_vocab USING fts5vocab(mail_search, 'row');
             INSERT INTO mail_search(body) VALUES ('architectur architecturda architecture architecturea architectureb elsewhere');
             WITH RECURSIVE repeated(n) AS (
                 VALUES(1) UNION ALL SELECT n+1 FROM repeated WHERE n<2000
             ) INSERT INTO mail_search(body) SELECT 'architecture' FROM repeated;",
        )?;
        assert_eq!(
            nearby_terms(&connection, "architecture", "ar", 3)?,
            ["architectur", "architecturda", "architecturea"]
        );
        assert_eq!(
            nearby_terms(&connection, "architecture", "ar", 1)?,
            ["architectur"]
        );
        assert_eq!(
            nearby_terms(&connection, "architecture", "el", 1)?,
            ["elsewhere"]
        );
        assert!(nearby_terms(&connection, "architecture", "ar", 0)?.is_empty());
        let query = mail_query(&connection, "architecture")?;
        assert!(query.starts_with("(\"architecture\"* OR "));
        assert!(!query.contains(" OR \"architecture\""));
        assert!(query.contains("\"architecturea\""));
        Ok(())
    }
}
