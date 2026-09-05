#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplySection {
    pub heading: String,
    pub body: String,
}

/// Separate common plain-text reply conventions without interpreting mail as executable HTML.
pub fn split(body: &str) -> (String, Vec<ReplySection>) {
    let mut latest = Vec::new();
    let mut sections = Vec::new();
    let mut heading = String::new();
    let mut quoted = Vec::new();
    let mut in_quote = false;
    let lines: Vec<_> = body.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start_matches(['>', ' ']);
        let marker = (trimmed.starts_with("On ") && trimmed.ends_with("wrote:"))
            || trimmed.contains("-----Original Message-----")
            || trimmed.contains("Begin forwarded message:")
            || (trimmed.starts_with("From:")
                && lines
                    .iter()
                    .skip(index + 1)
                    .take(4)
                    .any(|line| line.trim_start_matches(['>', ' ']).starts_with("Sent:")));
        if marker && sections.len() < 7 {
            if in_quote && !quoted.is_empty() {
                sections.push(ReplySection {
                    heading: std::mem::take(&mut heading),
                    body: quoted.join("\n"),
                });
                quoted.clear();
            }
            heading = trimmed.into();
            in_quote = true;
        } else if in_quote || line.trim_start().starts_with('>') {
            if !in_quote {
                heading = "Earlier conversation".into();
                in_quote = true;
            }
            quoted.push(trimmed.to_string());
        } else {
            latest.push(line.to_string());
        }
    }
    if in_quote && !quoted.is_empty() {
        sections.push(ReplySection {
            heading,
            body: quoted.join("\n"),
        });
    }
    (latest.join("\n").trim().to_string(), sections)
}
