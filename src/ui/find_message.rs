mod highlight;
use super::*;
use crate::{
    html_render::Input,
    message_find::{Results, plain},
};
use iced::{
    Alignment, Length,
    widget::{button, column, container, row, space, text},
};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone)]
pub enum Message {
    Open,
    Close,
    Query(String),
    MatchCase,
    Enter(u64, Key, keyboard::Modifiers, bool, bool),
    Next(bool),
    Run(u64),
    Width(String, usize, f32),
    Reveal(u64, u64, f32, Option<f32>),
    GuardedKey(Key, keyboard::Modifiers, bool),
}

#[derive(Debug, Clone, PartialEq)]
struct KeyState {
    id: String,
    query: String,
    match_case: bool,
    html: Option<u64>,
    plain: u64,
    font_size: u16,
    blocks: Vec<(usize, f32)>,
}

#[derive(Default)]
pub(super) struct State {
    pub open: bool,
    pub query: String,
    pub match_case: bool,
    pub revision: u64,
    pub results: Option<Arc<Results>>,
    pub active: Option<usize>,
    pub jump: u64,
    pub pending: bool,
    pub error: Option<String>,
    pub widths: HashMap<(String, usize), f32>,
    key: Option<KeyState>,
    current: Arc<AtomicU64>,
}
impl State {
    pub fn accept(&mut self, revision: u64, result: Result<Arc<Results>, String>) {
        if !self.open || revision != self.revision {
            return;
        }
        self.pending = false;
        match result {
            Ok(results) => {
                self.active = (!results.matches.is_empty())
                    .then(|| self.active.unwrap_or(0).min(results.matches.len() - 1));
                self.results = Some(results);
                self.error = None;
                self.jump += 1;
            }
            Err(error) => {
                self.results = None;
                self.active = None;
                self.error = Some(error);
            }
        }
    }
}

impl App {
    pub(super) fn prepare_find(&mut self) -> Task<super::Message> {
        let detail = self.detail.as_ref().filter(|detail| {
            self.find_message.open
                && self.tab == Tab::Mail
                && self.reader_id() == Some(detail.summary.id.as_str())
        });
        let key = detail.map(|detail| {
            let html = self
                .formatted(detail)
                .then_some(self.html_reader.generation);
            let blocks = if html.is_some() {
                Vec::new()
            } else {
                (0..=detail.replies.len())
                    .filter(|&index| {
                        index == 0
                            || (self.preferences.reply_display != ReplyDisplay::LatestOnly
                                && (self.expanded_replies.contains(&(index - 1))
                                    != (self.preferences.reply_display == ReplyDisplay::Expanded)))
                    })
                    .map(|index| {
                        (
                            index,
                            self.find_message
                                .widths
                                .get(&(detail.summary.id.clone(), index))
                                .copied()
                                .unwrap_or(0.),
                        )
                    })
                    .collect()
            };
            KeyState {
                id: detail.summary.id.clone(),
                query: self.find_message.query.clone(),
                match_case: self.find_message.match_case,
                html,
                plain: self.reader_selection_generation,
                font_size: self.preferences.reader_font_size,
                blocks,
            }
        });
        if self.find_message.key == key {
            return Task::none();
        }
        let state = &mut self.find_message;
        if state
            .key
            .as_ref()
            .map(|k| (&k.id, &k.query, k.match_case, k.html))
            != key
                .as_ref()
                .map(|k| (&k.id, &k.query, k.match_case, k.html))
        {
            state.active = None;
        }
        state.key = key;
        state.revision += 1;
        state.current.store(state.revision, Ordering::Relaxed);
        state.results = None;
        state.error = None;
        state.pending = state
            .key
            .as_ref()
            .is_some_and(|key| !key.query.trim().is_empty());
        if state.pending {
            let revision = state.revision;
            return Task::perform(
                tokio::time::sleep(std::time::Duration::from_millis(60)),
                move |_| super::Message::Find(Message::Run(revision)),
            );
        }
        Task::none()
    }
    pub(super) fn handle_find(&mut self, message: Message) -> Task<super::Message> {
        match message {
            Message::Open
                if self.tab == Tab::Mail && self.reader_id().is_some() && self.dialog.is_none() =>
            {
                self.focused_input = None;
                self.pending_focus = None;
                self.find_message.open = true;
                self.sidebar_focus = false;
                return focus_after_layout("find-message");
            }
            Message::Close => {
                self.focused_input = None;
                self.pending_focus = None;
                self.find_message.open = false;
                self.find_message.widths.clear();
                return widget::operation::focus("unfocused");
            }
            Message::Query(query) => self.find_message.query = query,
            Message::MatchCase => {
                self.find_message.match_case = !self.find_message.match_case;
                return widget::operation::focus("find-message");
            }
            Message::Enter(revision, key, modifiers, captured, focused)
                if self.find_message.open
                    && revision == self.find_message.revision
                    && self.tab == Tab::Mail
                    && self.dialog.is_none() =>
            {
                if focused {
                    return self.handle_find(Message::Next(modifiers.shift()));
                }
                if !self.full_reader {
                    return widget::operation::is_focused("search").map(move |focused| {
                        super::Message::Find(Message::GuardedKey(
                            key.clone(),
                            modifiers,
                            focused || captured,
                        ))
                    });
                }
                return self.key(key, modifiers, captured);
            }
            Message::Next(previous) => {
                if let Some(results) = &self.find_message.results
                    && !results.matches.is_empty()
                {
                    let len = results.matches.len();
                    let active = self.find_message.active.unwrap_or(0);
                    self.find_message.active = Some(if previous {
                        (active + len - 1) % len
                    } else {
                        (active + 1) % len
                    });
                    self.find_message.jump += 1;
                }
                return widget::operation::focus("find-message");
            }
            Message::Run(revision)
                if revision == self.find_message.revision && self.find_message.open =>
            {
                if let Some(key) = &self.find_message.key {
                    let command = if let Some(generation) = key.html {
                        Input::Find(generation, revision, key.query.clone(), key.match_case)
                    } else {
                        if key.blocks.iter().any(|(_, width)| *width <= 0.) {
                            return Task::none();
                        }
                        let Some(source) = self.detail.clone().filter(|d| d.summary.id == key.id)
                        else {
                            return Task::none();
                        };
                        Input::PlainFind(plain::Request {
                            revision,
                            source,
                            blocks: key.blocks.clone(),
                            query: key.query.clone(),
                            match_case: key.match_case,
                            font_size: key.font_size,
                            current: self.find_message.current.clone(),
                        })
                    };
                    return self.handle_html(super::html_reader::Message::Input(command));
                }
            }
            Message::Width(id, index, width)
                if self.reader_id() == Some(id.as_str()) && width.is_finite() && width > 0. =>
            {
                self.find_message
                    .widths
                    .retain(|(message, _), _| message == &id);
                self.find_message.widths.insert((id, index), width);
            }
            Message::Reveal(revision, jump, delta, pan)
                if revision == self.find_message.revision && jump == self.find_message.jump =>
            {
                let target = if self.conversation_visible() {
                    "conversation-reader"
                } else {
                    "message-reader"
                };
                let scroll = widget::operation::scroll_by(
                    target,
                    widget::scrollable::AbsoluteOffset { x: 0., y: delta },
                );
                let horizontal = if let Some(pan) = pan {
                    self.handle_html(super::html_reader::Message::Input(Input::Pan(
                        self.html_reader.generation,
                        pan,
                    )))
                } else {
                    Task::none()
                };
                return Task::batch([scroll, horizontal]);
            }
            Message::GuardedKey(key, modifiers, focused)
                if self.dialog.is_none()
                    && self.tab == Tab::Mail
                    && self.context_menu.is_none()
                    && self.remapping.is_none()
                    && self.composer.context.is_none() =>
            {
                return self.key(key, modifiers, focused);
            }
            _ => {}
        }
        Task::none()
    }
    pub(super) fn find_bar(&self) -> Element<'_, super::Message> {
        if !self.find_message.open {
            return space().height(0).into();
        }
        let state = &self.find_message;
        let status = if state.error.is_some() {
            "Search failed".into()
        } else if state.query.trim().is_empty() {
            String::new()
        } else if state.pending {
            "Searching…".into()
        } else {
            format!(
                "{} / {}",
                state.active.map_or(0, |i| i + 1),
                state.results.as_ref().map_or(0, |r| r.matches.len())
            )
        };
        let controls = row![
            input("Find in message…", &state.query, |q| super::Message::Find(
                Message::Query(q)
            ))
            .id("find-message")
            .width(Length::Fill),
            muted(status).size(11),
            button(text("Aa").size(12))
                .padding(7)
                .style(if state.match_case { primary } else { ghost })
                .on_press(super::Message::Find(Message::MatchCase)),
            self.icon_action(
                "up",
                if self.preferences.shortcut_tooltips {
                    "Previous match · Shift+Enter"
                } else {
                    "Previous match"
                },
                super::Message::Find(Message::Next(true))
            ),
            self.icon_action(
                "down",
                if self.preferences.shortcut_tooltips {
                    "Next match · Enter"
                } else {
                    "Next match"
                },
                super::Message::Find(Message::Next(false))
            ),
            self.icon_action(
                "close",
                if self.preferences.shortcut_tooltips {
                    "Close find · Esc"
                } else {
                    "Close find"
                },
                super::Message::Find(Message::Close)
            ),
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        let mut bar = column![controls].spacing(6);
        if let Some(error) = &state.error {
            bar = bar.push(muted(error).size(11));
        }
        container(bar).padding([7, 12]).into()
    }
    pub(super) fn find_highlights<'a>(
        &'a self,
        detail: &'a MailDetail,
        block: usize,
        child: Element<'a, super::Message>,
    ) -> Element<'a, super::Message> {
        if !self.find_message.open {
            return child;
        }
        highlight::wrap(
            child,
            &self.find_message,
            &detail.summary.id,
            block,
            self.formatted(detail)
                .then(|| self.html_reader.frame.as_ref().map_or(0., |f| f.pan)),
            self.formatted(detail).then(|| {
                self.html_reader
                    .frame
                    .as_ref()
                    .map_or(0., |f| f.content_width)
            }),
            !self.formatted(detail) || !self.html_reader.anchoring(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn find_enter_uses_event_modifiers_and_rejects_old_focus_checks() {
        let (mut app, _) = App::new();
        app.find_message.open = true;
        app.find_message.revision = 9;
        app.find_message.accept(
            9,
            Ok(Arc::new(Results::new(
                (0..3)
                    .map(|i| crate::message_find::Match {
                        block: 0,
                        rectangles: vec![[0., i as f32 * 20., 30., 20.]],
                    })
                    .collect(),
            ))),
        );
        app.modifiers = keyboard::Modifiers::default();
        let _ = app.handle_find(Message::Enter(
            9,
            Key::Named(keyboard::key::Named::Enter),
            keyboard::Modifiers::SHIFT,
            true,
            true,
        ));
        assert_eq!(app.find_message.active, Some(2));
        let _ = app.handle_find(Message::Enter(
            8,
            Key::Named(keyboard::key::Named::Enter),
            keyboard::Modifiers::default(),
            false,
            true,
        ));
        assert_eq!(app.find_message.active, Some(2));
        app.focused_input = Some("find-message");
        app.pending_focus = Some("find-message");
        let _ = app.handle_find(Message::Close);
        assert_eq!(app.focused_input, None);
        assert_eq!(app.pending_focus, None);
        assert!(!app.find_message.open);
    }

    #[test]
    fn stale_or_closed_searches_cannot_restore_matches_or_errors() {
        let mut state = State {
            open: true,
            revision: 9,
            pending: true,
            ..Default::default()
        };
        state.accept(8, Err("stale".into()));
        assert!(state.pending);
        assert!(state.error.is_none());
        state.accept(
            9,
            Ok(Arc::new(Results::new(vec![crate::message_find::Match {
                block: 0,
                rectangles: vec![[0., 0., 20., 20.]],
            }]))),
        );
        assert_eq!(state.active, Some(0));
        assert!(!state.pending);
        state.open = false;
        state.accept(9, Err("closed".into()));
        assert!(state.error.is_none());
    }
}
