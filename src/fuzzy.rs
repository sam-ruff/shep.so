use std::collections::HashSet;

/// Edit distance with adjacent transpositions, bounded by short search terms.
pub fn distance(a: &str, b: &str) -> usize {
    let a: Vec<_> = a.chars().take(80).collect();
    let b: Vec<_> = b.chars().take(80).collect();
    let mut matrix = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in matrix.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in matrix[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            matrix[i][j] = (matrix[i - 1][j] + 1)
                .min(matrix[i][j - 1] + 1)
                .min(matrix[i - 1][j - 1] + usize::from(a[i - 1] != b[j - 1]));
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                matrix[i][j] = matrix[i][j].min(matrix[i - 2][j - 2] + 1);
            }
        }
    }
    matrix[a.len()][b.len()]
}
pub fn score(query: &str, candidate: &str) -> Option<usize> {
    let query = query.trim().to_lowercase();
    let candidate = candidate.to_lowercase();
    if query.is_empty() {
        return Some(100);
    }
    if query == candidate {
        return Some(0);
    }
    if candidate.starts_with(&query) {
        return Some(5 + candidate.len().saturating_sub(query.len()));
    }
    if let Some(index) = candidate.find(&query) {
        return Some(30 + index);
    }
    let word_distance = candidate
        .split([' ', '/', '-', '_'])
        .map(|word| distance(&query, word))
        .min()
        .unwrap_or(80);
    if query.chars().count() >= 3 && word_distance <= if query.len() >= 6 { 2 } else { 1 } {
        return Some(60 + word_distance * 10);
    }
    let mut needle = query.chars();
    let mut next = needle.next();
    let mut gaps = 0;
    for ch in candidate.chars() {
        if Some(ch) == next {
            next = needle.next();
        } else if next.is_some() {
            gaps += 1;
        }
    }
    if next.is_none() {
        Some(100 + gaps)
    } else {
        None
    }
}
pub fn ranked(query: &str, choices: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut matches: Vec<_> = choices
        .into_iter()
        .filter_map(|value| score(query, &value).map(|score| (score, value)))
        .collect();
    matches.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    matches.into_iter().map(|(_, value)| value).collect()
}

pub fn mail_query(connection: &rusqlite::Connection, input: &str) -> anyhow::Result<String> {
    let tokens: Vec<_> = input
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .take(12)
        .collect();
    let mut groups = Vec::new();
    for token in tokens {
        let token = token.to_lowercase();
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
            let mut statement = connection.prepare(&format!("SELECT term FROM mail_vocab WHERE term IN ({placeholders}) ORDER BY doc DESC LIMIT 8"))?;
            for term in statement.query_map(rusqlite::params_from_iter(&values), |r| {
                r.get::<_, String>(0)
            })? {
                let term = term?;
                if term != token {
                    alternatives.push(format!("\"{term}\""));
                }
            }
        }
        groups.push(format!("({})", alternatives.join(" OR ")));
    }
    Ok(groups.join(" AND "))
}
