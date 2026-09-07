use std::collections::HashSet;

use rapidfuzz::distance::{lcs_seq, osa};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};

fn normalized(value: &str) -> String {
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
    score_normalized(
        normalized(query.trim()).as_str(),
        normalized(candidate).as_str(),
    )
}

fn score_normalized(query: &str, candidate: &str) -> Option<usize> {
    if query.is_empty() {
        return Some(0);
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
    let words: Vec<_> = candidate
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let query_words: Vec<_> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    if query_words.is_empty() {
        return None;
    }
    let word_score: Option<usize> = query_words
        .iter()
        .map(|needle| {
            let count = needle.chars().count();
            let maximum = if count >= 6 {
                2
            } else if count >= 3 {
                1
            } else {
                0
            };
            words
                .iter()
                .filter_map(|word| {
                    if word.starts_with(needle) {
                        return Some(5);
                    }
                    osa::distance_with_args(
                        needle.chars(),
                        word.chars(),
                        &osa::Args::default().score_cutoff(maximum),
                    )
                    .map(|edits| 20 + edits * 10)
                })
                .min()
        })
        .sum();
    if let Some(value) = word_score {
        return Some(100 + value);
    }
    if lcs_seq::similarity(query.chars(), candidate.chars()) == query.chars().count() {
        let ratio = rapidfuzz::fuzz::ratio(query.chars(), candidate.chars());
        return Some(250 + ((1. - ratio) * 100.).round() as usize);
    }
    None
}

pub fn ranked(query: &str, choices: impl IntoIterator<Item = String>) -> Vec<String> {
    let query = normalized(query.trim());
    let mut matches: Vec<_> = choices
        .into_iter()
        .filter_map(|value| {
            let key = normalized(&value);
            score_normalized(&query, &key).map(|score| (score, key, value))
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

fn search_tokens(input: &str) -> Vec<String> {
    normalized(input)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(12)
        .map(str::to_owned)
        .collect()
}

pub fn mail_query(connection: &rusqlite::Connection, input: &str) -> anyhow::Result<String> {
    let tokens = search_tokens(input);
    let mut groups = Vec::new();
    for token in tokens {
        let mut alternatives = vec![format!(
            "\"{}\"{}",
            token,
            if token.chars().all(char::is_numeric) {
                ""
            } else {
                "*"
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
            let values: Vec<_> = variants.into_iter().collect();
            let placeholders = vec!["?"; values.len()].join(",");
            let mut statement = connection.prepare(&format!("SELECT term FROM mail_vocab WHERE term IN ({placeholders}) ORDER BY doc DESC,term LIMIT 32"))?;
            let mut terms = Vec::new();
            for term in statement.query_map(rusqlite::params_from_iter(&values), |r| {
                r.get::<_, String>(0)
            })? {
                let term = term?;
                if term != token {
                    terms.push(term);
                }
            }
            if token.len() >= 6 {
                let prefix = &token[..2];
                let end = format!("{prefix}~");
                let mut nearby = connection.prepare(
                    "SELECT term FROM mail_vocab WHERE term>=? AND term<? ORDER BY term LIMIT 256",
                )?;
                for word in nearby.query_map([prefix, &end], |r| r.get::<_, String>(0))? {
                    let word = word?;
                    if osa::distance_with_args(
                        token.chars(),
                        word.chars(),
                        &osa::Args::default().score_cutoff(2),
                    )
                    .is_some()
                    {
                        terms.push(word);
                    }
                }
            }
            terms.sort_by_cached_key(|term| {
                (
                    distance(&token, term),
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
