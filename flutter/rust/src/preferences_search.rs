use serde::Deserialize;
use shep_mail_content::fuzzy::{WordMatcher, normalized};
use std::collections::BTreeSet;

#[derive(Deserialize)]
struct Entry {
    label: String,
    section: String,
    description: String,
    synonyms: String,
}

fn words(value: &str) -> Vec<String> {
    normalized(value)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn subsequence(term: &str, word: &str) -> bool {
    let mut chars = word.chars();
    term.chars().all(|wanted| chars.any(|c| c == wanted))
}

pub(crate) fn rank(json: &str, query: &str) -> anyhow::Result<Vec<u32>> {
    anyhow::ensure!(
        json.len() <= 2 * 1024 * 1024,
        "Preferences catalogue is too large to search."
    );
    if query.chars().take(257).count() > 256 {
        return Ok(Vec::new());
    }
    let entries: Vec<Entry> = serde_json::from_str(json)?;
    let query = normalized(query.trim());
    let terms: BTreeSet<_> = words(&query).into_iter().collect();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let fields: Vec<_> = entries
        .iter()
        .map(|entry| {
            [
                words(&entry.label),
                words(&entry.section),
                words(&entry.description),
                words(&entry.synonyms),
            ]
        })
        .collect();
    let anchored: Vec<_> = terms
        .iter()
        .map(|term| {
            fields
                .iter()
                .any(|entry| entry.iter().flatten().any(|word| word.starts_with(term)))
        })
        .collect();
    let mut matcher = WordMatcher::new(&query);
    let mut ranked = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        let total: Option<usize> = terms
            .iter()
            .enumerate()
            .map(|(term_index, term)| {
                fields[position]
                    .iter()
                    .enumerate()
                    .flat_map(|(field, words)| {
                        words.iter().map(move |word| (word, [0, 30, 40, 60][field]))
                    })
                    .filter_map(|(word, weight)| {
                        if anchored[term_index] && !subsequence(term, word) {
                            return None;
                        }
                        let score = if term == word {
                            Some(0)
                        } else if term.chars().any(char::is_numeric) {
                            None
                        } else if word.starts_with(term) {
                            Some(2)
                        } else if term.chars().count() <= 64 && word.chars().count() <= 64 {
                            matcher.score_normalized(term, word)
                        } else {
                            None
                        };
                        score.map(|score| score + weight)
                    })
                    .min()
            })
            .sum();
        if let Some(total) = total {
            let label = normalized(&entry.label);
            let tier = if label == query {
                0
            } else if label.starts_with(&query) {
                1
            } else {
                2
            };
            ranked.push((tier, total, label, u32::try_from(position)?));
        }
    }
    ranked.sort_unstable();
    Ok(ranked.into_iter().map(|entry| entry.3).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalogue() -> String {
        serde_json::json!([
            {"label":"Theme", "section":"Appearance", "description":"Light dark system", "synonyms":"colour"},
            {"label":"Saved profiles", "section":"Profiles and sync", "description":"Shared settings", "synonyms":"cloud"},
            {"label":"Office365", "section":"Connections", "description":"Account", "synonyms":""},
            {"label":"ガ 한글 Café", "section":"Reading", "description":"", "synonyms":""}
        ]).to_string()
    }

    #[test]
    fn shared_settings_cases() -> anyhow::Result<()> {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../shared/preferences-search-cases.json"
        ))?;
        let catalogue = serde_json::to_string(&fixture["entries"])?;
        for case in fixture["cases"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Cases missing"))?
        {
            let query = case["query"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Query missing"))?;
            assert_eq!(
                serde_json::to_value(rank(&catalogue, query)?)?,
                case["positions"],
                "{query}"
            );
        }
        Ok(())
    }

    #[test]
    fn uses_shared_accents_abbreviations_typos_and_numeric_contracts() -> anyhow::Result<()> {
        for (query, expected) in [
            ("APPEARÁNCE", vec![0]),
            ("prf", vec![1]),
            ("theem", vec![0]),
            ("office365", vec![2]),
            ("office36", vec![]),
            ("office365x", vec![]),
            ("カ\u{3099}", vec![3]),
            ("한글", vec![3]),
            ("cafe", vec![3]),
            ("shared profiles", vec![1]),
            ("theme office365", vec![]),
        ] {
            assert_eq!(rank(&catalogue(), query)?, expected, "{query}");
        }
        Ok(())
    }

    #[test]
    fn bounds_query_and_fuzzy_words_and_rejects_invalid_catalogues() -> anyhow::Result<()> {
        assert!(rank(&catalogue(), &"a".repeat(257))?.is_empty());
        assert!(rank("invalid", "theme").is_err());
        let json = serde_json::json!([{"label":"a".repeat(5000), "section":"Connections", "description":"", "synonyms":""}]).to_string();
        assert!(rank(&json, "zzzzz")?.is_empty());
        assert_eq!(rank(&json, "aaa")?, vec![0]);
        assert!(rank(&" ".repeat(2 * 1024 * 1024 + 1), "theme").is_err());
        Ok(())
    }
}
