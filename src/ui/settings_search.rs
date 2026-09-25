//! Local, immutable control captions with navigation to their editable section
//! and, where a caption explains the match, to that control itself.
use super::*;
use iced::{
    Alignment, Length,
    widget::{button, column, row, space, text},
};
use std::sync::OnceLock;

mod catalogue;
#[cfg(test)]
mod coverage;
mod highlight;
#[cfg(test)]
mod performance;
pub(super) mod reveal;
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

/// One matching section, with the control whose caption best explains the match.
#[derive(Clone, Copy)]
pub(super) struct Match {
    pub setting: &'static Setting,
    pub control: Option<&'static str>,
}

/// A control being scrolled into view after choosing a search result.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Reveal {
    pub generation: u64,
    pub tab: SettingsTab,
    pub section: &'static str,
    pub control: &'static str,
    pub state: RevealState,
    /// The accent outline shown briefly over the revealed control.
    pub outline: Option<reveal::Outline>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum RevealState {
    Pending,
    /// The caption is not shown in the current state; the section stays open.
    Missing,
    Revealed {
        top: f32,
        focused: bool,
    },
}

/// Layout attempts before accepting the section alone (about 250 ms).
const REVEAL_ATTEMPTS: u8 = 8;

struct Label {
    text: &'static str,
    normalized: String,
    words: Vec<String>,
}

struct IndexedSetting {
    setting: &'static Setting,
    title: String,
    title_words: Vec<String>,
    labels: Vec<Label>,
    words: Vec<(String, usize)>,
}

fn split_words(value: &str) -> Vec<String> {
    crate::fuzzy::normalized(value)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Every control caption in a section, static and generated.
fn captions(setting: &'static Setting) -> Vec<&'static str> {
    let mut captions: Vec<_> = setting
        .labels
        .split('|')
        .filter(|caption| !caption.is_empty())
        .collect();
    captions.extend(catalogue::generated(setting.title));
    captions
}

fn index() -> &'static [IndexedSetting] {
    static INDEX: OnceLock<Vec<IndexedSetting>> = OnceLock::new();
    INDEX.get_or_init(|| {
        SETTINGS
            .iter()
            .map(|setting| {
                let captions = captions(setting);
                let labels = captions
                    .iter()
                    .map(|&text| Label {
                        text,
                        normalized: crate::fuzzy::normalized(text),
                        words: split_words(text),
                    })
                    .collect();
                let mut words = Vec::new();
                for (value, weight) in [
                    (setting.title.to_owned(), 0),
                    (captions.join(" "), 20),
                    (format!("{:?}", setting.tab), 40),
                    (format!("{} {}", setting.description, setting.synonyms), 60),
                ] {
                    words.extend(split_words(&value).into_iter().map(|word| (word, weight)));
                }
                words.sort();
                words.dedup_by(|a, b| a.0 == b.0);
                IndexedSetting {
                    setting,
                    title: crate::fuzzy::normalized(setting.title),
                    title_words: split_words(setting.title),
                    labels,
                    words,
                }
            })
            .collect()
    })
}

pub(super) fn matches(query: &str) -> Vec<Match> {
    let mut matcher = crate::fuzzy::WordMatcher::new(query);
    matches_with(query, &mut |term, candidate| {
        matcher.score_normalized(term, candidate)
    })
}

fn is_subsequence(term: &str, word: &str) -> bool {
    let mut chars = word.chars();
    term.chars().all(|wanted| chars.any(|c| c == wanted))
}

/// How a caption reads the whole query: 0 the same words, 1 the query's words
/// begin the caption's words in order, 2 neither.
fn phrase_tier(caption: &[String], query: &[String]) -> u8 {
    if caption == query {
        0
    } else if query.len() <= caption.len()
        && query
            .iter()
            .zip(caption)
            .all(|(query, caption)| caption.starts_with(query.as_str()))
    {
        1
    } else {
        2
    }
}

/// Sums each term's best weighted word score, or `None` if a term is unmatched.
fn best_in(
    terms: usize,
    words: &[(&str, usize)],
    score_word: &mut dyn FnMut(usize, &str) -> Option<usize>,
) -> Option<usize> {
    (0..terms)
        .map(|term| {
            words
                .iter()
                .filter_map(|(word, weight)| score_word(term, word).map(|score| score + weight))
                .min()
        })
        .sum()
}

/// How many query terms a caption's words match, and the sum of their best
/// scores. `score_word` scores only matches that are not typos.
fn caption_words(
    words: &[String],
    terms: usize,
    score_word: &mut dyn FnMut(usize, &str) -> Option<usize>,
) -> (usize, usize) {
    (0..terms)
        .filter_map(|term| words.iter().filter_map(|word| score_word(term, word)).min())
        .fold((0, 0), |(count, total), score| (count + 1, total + score))
}

/// The caption matching the most query words, then the best phrase and score,
/// then the shortest. It must match at least half of the words and more of them
/// than the section title, which otherwise keeps the whole section.
fn best_control(
    entry: &IndexedSetting,
    phrase: &[String],
    terms: usize,
    score_word: &mut dyn FnMut(usize, &str) -> Option<usize>,
) -> Option<&'static str> {
    let (title, _) = caption_words(&entry.title_words, terms, score_word);
    entry
        .labels
        .iter()
        .enumerate()
        .filter_map(|(position, label)| {
            let (count, score) = caption_words(&label.words, terms, score_word);
            (count * 2 >= terms && count > title).then(|| {
                let tier = phrase_tier(&label.words, phrase);
                (
                    (
                        std::cmp::Reverse(count),
                        tier,
                        score,
                        label.words.len(),
                        position,
                    ),
                    label.text,
                )
            })
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, text)| text)
}

fn matches_with(query: &str, scorer: &mut impl FnMut(&str, &str) -> Option<usize>) -> Vec<Match> {
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
    let phrase = split_words(&query);
    // A term spelled like a real catalogue word or prefix is not also read as a
    // typo of a different word, so "shared" cannot match "saved".
    let anchored: Vec<bool> = terms
        .iter()
        .map(|term| {
            index()
                .iter()
                .any(|entry| entry.words.iter().any(|(word, _)| word.starts_with(term)))
        })
        .collect();
    let mut score_word = |term: usize, word: &str| {
        if anchored[term] && !is_subsequence(terms[term], word) {
            return None;
        }
        scorer(terms[term], word)
    };
    let mut entries: Vec<_> = index()
        .iter()
        .filter_map(|entry| {
            let words: Vec<_> = entry
                .words
                .iter()
                .map(|(word, weight)| (word.as_str(), *weight))
                .collect();
            let score = best_in(terms.len(), &words, &mut score_word)?;
            let tier = if query == entry.title {
                0
            } else if entry.labels.iter().any(|label| label.normalized == query) {
                1
            } else if entry.title.starts_with(&query)
                || entry
                    .labels
                    .iter()
                    .any(|label| label.normalized.starts_with(&query))
            {
                2
            } else {
                3
            };
            // A typo finds the section but does not name a control.
            let mut score_caption = |term: usize, word: &str| {
                let score = score_word(term, word)?;
                is_subsequence(terms[term], word).then_some(score)
            };
            let control = best_control(entry, &phrase, terms.len(), &mut score_caption);
            Some((tier, score, entry, control))
        })
        .collect();
    entries.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.title.cmp(&b.2.title))
    });
    entries
        .into_iter()
        .map(|(_, _, entry, control)| Match {
            setting: entry.setting,
            control,
        })
        .collect()
}

impl App {
    pub(super) fn settings_matches(&self) -> &[Match] {
        &self.settings_search_results
    }
    /// Opens a result's section and starts revealing its control.
    pub(super) fn reveal_setting(
        &mut self,
        tab: SettingsTab,
        section: &'static str,
        control: &'static str,
    ) -> Task<Message> {
        let task = self.handle(Message::FindSetting(tab, section));
        self.settings_reveal_generation += 1;
        self.settings_reveal = Some(Reveal {
            generation: self.settings_reveal_generation,
            tab,
            section,
            control,
            state: RevealState::Pending,
            outline: None,
        });
        task.chain(self.reveal_setting_attempt(self.settings_reveal_generation, 0))
    }
    fn current_reveal(&self, generation: u64) -> Option<&Reveal> {
        self.settings_reveal.as_ref().filter(|reveal| {
            reveal.generation == generation
                && self.tab == Tab::Preferences
                && self.settings_tab == reveal.tab
                && self.settings_group == Some(reveal.section)
                && self.settings_search.is_empty()
        })
    }
    pub(super) fn reveal_setting_attempt(&self, generation: u64, attempt: u8) -> Task<Message> {
        let Some(reveal) = self.current_reveal(generation) else {
            return Task::none();
        };
        reveal::reveal(reveal.control)
            .map(move |found| Message::SettingRevealed(generation, attempt, found))
    }
    pub(super) fn setting_revealed(
        &mut self,
        generation: u64,
        attempt: u8,
        found: reveal::Found,
    ) -> Task<Message> {
        if self.current_reveal(generation).is_none() {
            return Task::none();
        }
        let (state, outline) = match found {
            reveal::Found::Revealed {
                top,
                focused,
                outline,
            } => (RevealState::Revealed { top, focused }, Some(outline)),
            reveal::Found::Missing if attempt + 1 < REVEAL_ATTEMPTS => {
                return Task::perform(
                    tokio::time::sleep(std::time::Duration::from_millis(32)),
                    move |()| Message::RevealSettingAttempt(generation, attempt + 1),
                );
            }
            reveal::Found::Missing => (RevealState::Missing, None),
        };
        if let Some(reveal) = self.settings_reveal.as_mut() {
            reveal.state = state;
            reveal.outline = outline;
        }
        if outline.is_none() {
            return Task::none();
        }
        Task::perform(tokio::time::sleep(highlight::DURATION), move |()| {
            Message::DismissSettingOutline(generation)
        })
    }
    pub(super) fn dismiss_setting_outline(&mut self, generation: u64) {
        if let Some(reveal) = self
            .settings_reveal
            .as_mut()
            .filter(|reveal| reveal.generation == generation)
        {
            reveal.outline = None;
        }
    }
    /// The outline layer drawn above the Preferences content, or nothing.
    pub(super) fn settings_outline(&self) -> Option<Element<'_, Message>> {
        let reveal = self.current_reveal(self.settings_reveal?.generation)?;
        let outline = reveal.outline?;
        Some(
            highlight::Outline {
                bounds: outline.content,
                dismiss: Message::DismissSettingOutline(reveal.generation),
            }
            .into(),
        )
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
        let noun = if results.len() == 1 {
            "section"
        } else {
            "sections"
        };
        let mut content =
            column![text(format!("{} matching {noun}", results.len())).size(13)].spacing(10);
        for result in results {
            let setting = result.setting;
            let (location, message) = match result.control {
                Some(control) => (
                    format!("{:?}  ·  {control}", setting.tab),
                    Message::RevealSetting(setting.tab, setting.title, control),
                ),
                None => (
                    format!("{:?}", setting.tab),
                    Message::FindSetting(setting.tab, setting.title),
                ),
            };
            content = content.push(
                button(
                    row![
                        column![
                            text(setting.title).size(14).font(BOLD),
                            muted(location).size(11)
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
                .on_press(message),
            );
        }
        content.into()
    }
}
