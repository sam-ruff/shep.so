//! Local, immutable control captions with navigation to their editable section.
use super::*;
use iced::{
    Alignment, Length,
    widget::{button, column, row, space, text},
};
use std::sync::OnceLock;

mod catalogue;
#[cfg(test)]
mod performance;
#[cfg(test)]
mod tests;
use catalogue::SETTINGS;

pub(super) struct Setting {
    pub title: &'static str,
    pub tab: SettingsTab,
    labels: &'static str,
    description: &'static str,
    synonyms: &'static str,
}

struct IndexedSetting {
    setting: &'static Setting,
    title: String,
    labels: Vec<String>,
    words: Vec<(String, usize)>,
}

fn index() -> &'static [IndexedSetting] {
    static INDEX: OnceLock<Vec<IndexedSetting>> = OnceLock::new();
    INDEX.get_or_init(|| {
        SETTINGS
            .iter()
            .map(|setting| {
                let title = crate::fuzzy::normalized(setting.title);
                let labels: Vec<_> = setting
                    .labels
                    .split('|')
                    .map(crate::fuzzy::normalized)
                    .collect();
                let mut words = Vec::new();
                for (value, weight) in [
                    (title.clone(), 0),
                    (labels.join(" "), 20),
                    (format!("{:?}", setting.tab), 40),
                    (format!("{} {}", setting.description, setting.synonyms), 60),
                ] {
                    words.extend(
                        crate::fuzzy::normalized(&value)
                            .split(|c: char| !c.is_alphanumeric())
                            .filter(|word| !word.is_empty())
                            .map(|word| (word.to_owned(), weight)),
                    );
                }
                words.sort();
                words.dedup_by(|a, b| a.0 == b.0);
                IndexedSetting {
                    setting,
                    title,
                    labels,
                    words,
                }
            })
            .collect()
    })
}

/// Pure matching boundary shared by section titles, captions and supporting text.
fn word_score(query: &str, candidate: &str) -> Option<usize> {
    if query.len() > candidate.len().saturating_mul(4).saturating_add(8) {
        return None;
    }
    if query == candidate {
        return Some(0);
    }
    if query.chars().any(char::is_numeric) {
        return None;
    }
    if candidate.starts_with(query) {
        return Some(2);
    }
    let count = query.chars().count();
    let cutoff = match count {
        0..=3 => 0,
        4..=5 => 1,
        _ => 2,
    };
    if count.abs_diff(candidate.chars().count()) > cutoff {
        return None;
    }
    rapidfuzz::distance::osa::distance_with_args(
        query.chars(),
        candidate.chars(),
        &rapidfuzz::distance::osa::Args::default().score_cutoff(cutoff),
    )
    .map(|distance| 10 + distance * 10)
}

pub(super) fn matches(query: &str) -> Vec<&'static Setting> {
    matches_with(query, &mut word_score)
}

fn matches_with(
    query: &str,
    scorer: &mut impl FnMut(&str, &str) -> Option<usize>,
) -> Vec<&'static Setting> {
    let query = crate::fuzzy::normalized(query.trim());
    let mut terms: Vec<_> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| !term.is_empty())
        .collect();
    terms.sort_unstable();
    terms.dedup();
    if terms.is_empty() {
        return Vec::new();
    }
    let mut entries: Vec<_> = index()
        .iter()
        .filter_map(|entry| {
            let score: usize = terms
                .iter()
                .map(|term| {
                    entry
                        .words
                        .iter()
                        .filter_map(|(word, weight)| scorer(term, word).map(|score| score + weight))
                        .min()
                })
                .sum::<Option<usize>>()?;
            let tier = if query == entry.title {
                0
            } else if entry.labels.contains(&query) {
                1
            } else if entry.title.starts_with(&query)
                || entry.labels.iter().any(|label| label.starts_with(&query))
            {
                2
            } else {
                3
            };
            Some((tier, score, entry))
        })
        .collect();
    entries.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.title.cmp(&b.2.title))
    });
    entries
        .into_iter()
        .map(|(_, _, entry)| entry.setting)
        .collect()
}

impl App {
    pub(super) fn settings_matches(&self) -> &[&'static Setting] {
        &self.settings_search_results
    }
    pub(super) fn settings_results(&self) -> Element<'_, Message> {
        let results = self.settings_matches();
        if results.is_empty() {
            return column![
                text("No matching settings").size(16).font(BOLD),
                muted("Try a control name, its purpose or a different spelling.").size(13)
            ]
            .spacing(10)
            .into();
        }
        let mut content =
            column![text(format!("{} matching sections", results.len())).size(13)].spacing(10);
        for result in results {
            content = content.push(
                button(
                    row![
                        column![
                            text(result.title).size(14).font(BOLD),
                            muted(format!("{:?}", result.tab)).size(11)
                        ]
                        .spacing(5),
                        space().width(Length::Fill),
                        icon("chevron", 18.)
                    ]
                    .align_y(Alignment::Center),
                )
                .padding(16)
                .width(Length::Fill)
                .style(outline)
                .on_press(Message::FindSetting(result.tab, result.title)),
            );
        }
        content.into()
    }
}
