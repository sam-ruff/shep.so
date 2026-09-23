use std::collections::HashSet;

use rapidfuzz::distance::osa;
pub use shep_mail_content::fuzzy::{
    Matcher, WordMatcher, distance, normalized, ranked, ranked_labels, score,
};

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
