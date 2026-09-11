use super::views::{sender_name, truncate};
use super::*;
#[cfg(test)]
#[path = "conversations_tests.rs"]
mod tests;
use crate::store::ConversationPage;
use iced::{
    Alignment, Length,
    widget::{button, column, container, row, scrollable, space, text},
};

#[derive(Default)]
pub(super) struct Conversation {
    pub page: Arc<ConversationPage>,
    pub generation: u64,
    pub focus: Option<String>,
    pub collapsed: bool,
    pub error: Option<String>,
    pub scroll: f32,
}

impl App {
    pub(super) fn action_mail(&self) -> Option<&Mail> {
        let id = self.reader_id()?;
        if self.mail_actions.restoring(id) || self.bulk_owns_mail(id) {
            return None;
        }
        let mail = self
            .detail
            .as_ref()
            .filter(|detail| detail.summary.id == id)
            .map(|detail| &detail.summary)
            .or_else(|| {
                self.conversation
                    .page
                    .rows
                    .iter()
                    .find(|mail| mail.id == id)
            })
            .or_else(|| self.page.rows.iter().find(|mail| mail.id == id))?;
        (!mail.remote_id.is_empty()).then(|| self.mail_actions.effective(mail))
    }

    pub(super) fn reader_id(&self) -> Option<&str> {
        self.selected.as_ref()?;
        self.conversation
            .focus
            .as_deref()
            .or(self.selected.as_deref())
    }
    pub(super) fn request_conversation(&mut self, offset: Option<usize>) {
        self.conversation.generation += 1;
        if !self.preferences.group_conversations
            || self
                .selected
                .as_ref()
                .is_some_and(|id| self.mail_actions.restoring(id) || self.page.is_placeholder(id))
        {
            return;
        }
        if let Some(anchor) = self.selected.clone() {
            self.conversation.error = None;
            self.send(Command::Conversation(
                self.conversation.generation,
                anchor,
                self.reader_id().map(str::to_owned),
                offset,
            ));
        }
    }
    pub(super) fn focus_conversation_message(&mut self, id: String) {
        self.conversation.focus = Some(id.clone());
        self.conversation.collapsed = false;
        self.expanded_replies.clear();
        self.detail = self.cached_detail(&id);
        if self.detail.is_none() {
            self.send(Command::Detail {
                revision: self.detail_revision,
                id: id.clone(),
                prefetch: false,
                body_chars: self.detail_body_chars(&id),
            });
        }
        if let Some(i) = self
            .conversation
            .page
            .rows
            .iter()
            .position(|mail| mail.id == id)
        {
            let ids: Vec<_> = [i.checked_sub(1), Some(i + 1)]
                .into_iter()
                .flatten()
                .filter_map(|i| {
                    self.conversation
                        .page
                        .rows
                        .get(i)
                        .map(|mail| mail.id.clone())
                })
                .collect();
            for id in ids {
                self.preload(id);
            }
        }
        self.load_remote_images();
    }
    pub(super) fn conversation_result(
        &mut self,
        generation: u64,
        anchor: String,
        result: Result<Arc<ConversationPage>, String>,
    ) -> Task<Message> {
        if generation != self.conversation.generation
            || self.selected.as_deref() != Some(&anchor)
            || self.mail_actions.restoring(&anchor)
            || !self.preferences.group_conversations
        {
            return Task::none();
        }
        match result {
            Err(error) => {
                self.conversation.error = Some(error.clone());
                self.notice(
                    format!(
                        "Could not load related messages: {error}. Reopen this email to retry."
                    ),
                    true,
                );
            }
            Ok(page) => {
                let previous_reader = self.reader_id().map(str::to_owned);
                let changed_page =
                    !self.conversation_visible() || self.conversation.page.offset != page.offset;
                self.conversation.page = page;
                if !self
                    .conversation
                    .page
                    .rows
                    .iter()
                    .any(|mail| Some(mail.id.as_str()) == self.reader_id())
                {
                    // A move can replace an expanded reply's server identity while
                    // the selected Inbox anchor remains in this thread. Keep that
                    // anchor on refresh; explicit paging still opens its first row.
                    let fallback = (!changed_page)
                        .then(|| {
                            self.conversation
                                .page
                                .rows
                                .iter()
                                .find(|mail| mail.id == anchor)
                        })
                        .flatten()
                        .or_else(|| self.conversation.page.rows.first());
                    if let Some(mail) = fallback {
                        self.focus_conversation_message(mail.id.clone());
                    }
                } else if let Some(id) = self.reader_id().map(str::to_owned) {
                    // Warm adjacent messages without resetting the open body or quote state.
                    let ids: Vec<_> = self.conversation.page.rows.iter().map(|m| &m.id).collect();
                    if let Some(i) = ids.iter().position(|candidate| **candidate == id) {
                        let neighbors: Vec<_> = [i.checked_sub(1), Some(i + 1)]
                            .into_iter()
                            .flatten()
                            .filter_map(|i| ids.get(i).map(|id| (*id).clone()))
                            .collect();
                        for id in neighbors {
                            self.preload(id);
                        }
                    }
                }
                self.restore_reply();
                // Periodic sync/read/flag refreshes update the same thread.
                // They must not reposition a person already reading further down.
                if self.conversation_visible()
                    && (changed_page || previous_reader.as_deref() != self.reader_id())
                {
                    return Task::perform(
                        async move {
                            tokio::time::sleep(std::time::Duration::from_millis(32)).await;
                            generation
                        },
                        Message::ConversationScroll,
                    );
                }
            }
        }
        Task::none()
    }
    pub(super) fn conversation_visible(&self) -> bool {
        self.preferences.group_conversations
            && self.selected.as_deref() == Some(&self.conversation.page.anchor)
            && self.conversation.page.total > 1
    }
    pub(super) fn conversation_scroll(&self, generation: u64) -> Task<Message> {
        if generation != self.conversation.generation
            || !self.conversation_visible()
            || self.compose_visible()
        {
            return Task::none();
        }
        let i = self
            .conversation
            .page
            .rows
            .iter()
            .position(|mail| Some(mail.id.as_str()) == self.reader_id())
            .unwrap_or(0);
        widget::operation::scroll_to(
            "conversation-reader",
            widget::scrollable::AbsoluteOffset {
                x: 0.,
                y: i.saturating_sub(1) as f32 * 86.,
            },
        )
    }
    pub(super) fn conversation_cards(&self) -> Element<'_, Message> {
        let page = &self.conversation.page;
        let mut cards = column![].spacing(10);
        for original in &page.rows {
            let mail = self.mail_actions.effective(original);
            let active = self.reader_id() == Some(&mail.id);
            let expanded = active && !self.conversation.collapsed;
            let date = chrono::DateTime::from_timestamp(mail.timestamp, 0)
                .unwrap_or_default()
                .with_timezone(&chrono::Local);
            let mut content = column![].spacing(12);
            if expanded {
                content = content.push(
                    row![
                        muted(&mail.folder).size(11),
                        space().width(Length::Fill),
                        self.icon_action(
                            "down",
                            "Collapse message",
                            Message::ConversationMessage(mail.id.clone())
                        ),
                        self.toggle_icon_action(
                            "flag",
                            if mail.starred {
                                "Remove flag"
                            } else {
                                "Flag message"
                            },
                            mail.starred,
                            Message::ConversationFlag(mail.id.clone())
                        )
                    ]
                    .spacing(4)
                    .align_y(Alignment::Center),
                );
                if let Some(detail) = self
                    .detail
                    .as_ref()
                    .filter(|detail| detail.summary.id == mail.id)
                {
                    content = content
                        .push(self.reader_body(detail, false))
                        .push(self.reader_actions(detail));
                } else {
                    content = content.push(muted("Opening message…"));
                }
            } else {
                content = content.push(
                    row![
                        button(
                            column![
                                row![
                                    text(truncate(&sender_name(&mail.sender), 36))
                                        .font(BOLD)
                                        .size(12),
                                    space().width(Length::Fill),
                                    muted(date.format("%d %b · %H:%M").to_string()).size(10)
                                ]
                                .align_y(Alignment::Center),
                                row![
                                    muted(truncate(&mail.preview, 65)).size(11),
                                    space().width(Length::Fill),
                                    muted(&mail.folder).size(10)
                                ]
                                .spacing(8)
                            ]
                            .spacing(7)
                        )
                        .padding(0)
                        .width(Length::Fill)
                        .style(ghost)
                        .on_press(Message::ConversationMessage(mail.id.clone())),
                        self.toggle_icon_action(
                            "flag",
                            if mail.starred {
                                "Remove flag"
                            } else {
                                "Flag message"
                            },
                            mail.starred,
                            Message::ConversationFlag(mail.id.clone())
                        )
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
            let background = expanded
                .then(|| {
                    self.detail
                        .as_ref()
                        .filter(|detail| detail.summary.id == mail.id)
                        .and_then(|detail| self.document_background(detail))
                })
                .flatten();
            cards = cards.push(widget::themer(
                super::reading::document_theme(background),
                container(content)
                    .height(if expanded {
                        Length::Shrink
                    } else {
                        Length::Fixed(76.)
                    })
                    .align_y(Alignment::Center)
                    .padding(14)
                    .width(Length::Fill)
                    .style(move |theme| {
                        let mut style = if active {
                            selected_card(theme)
                        } else {
                            card(theme)
                        };
                        if let Some(background) = background {
                            style.background = Some(background.into());
                            style.text_color = Some(colors(theme).text);
                        }
                        style
                    }),
            ));
        }
        cards.into()
    }

    pub(super) fn conversation_controls(&self) -> Element<'_, Message> {
        let page = &self.conversation.page;
        row![
            text(format!("{} messages", page.total)).size(12),
            space().width(Length::Fill),
            button(text("Newer").size(11))
                .padding([8, 10])
                .style(ghost)
                .on_press_maybe((page.offset > 0).then_some(Message::ConversationPage(false))),
            muted(format!(
                "{}–{}",
                page.offset + 1,
                page.offset + page.rows.len()
            ))
            .size(10),
            button(text("Older").size(11))
                .padding([8, 10])
                .style(ghost)
                .on_press_maybe(
                    (page.offset + page.rows.len() < page.total)
                        .then_some(Message::ConversationPage(true))
                )
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    }

    pub(super) fn conversation_reader(&self) -> Element<'_, Message> {
        let title = self
            .page
            .rows
            .iter()
            .find(|mail| Some(&mail.id) == self.selected.as_ref())
            .map(|mail| mail.subject.as_str())
            .unwrap_or("Conversation");
        let cards = self.conversation_cards();
        let controls = self.conversation_controls();
        let toolbar: Element<'_, Message> = match &self.detail {
            Some(detail) => self.reader_toolbar(detail),
            None => container(muted("Opening message…")).height(36).into(),
        };
        let mut heading = column![text(title).size(23).font(BOLD), controls].spacing(8);
        if self.conversation.error.is_some() {
            heading = heading.push(action(
                "Reload related messages",
                Message::RetryConversation,
            ));
        }
        column![
            container(toolbar).padding([10, 18]),
            line(),
            self.find_bar(),
            container(heading).padding([14, 20]),
            scrollable(container(cards).padding([0, 20]))
                .id("conversation-reader")
                .on_scroll(|viewport| Message::ConversationViewport(viewport.absolute_offset().y))
                .height(Length::Fill),
            container(self.reader_navigation()).padding([8, 20])
        ]
        .height(Length::Fill)
        .width(Length::Fill)
        .into()
    }
}

fn selected_card(theme: &Theme) -> widget::container::Style {
    let mut style = card(theme);
    style.border.color = colors(theme).accent;
    style
}
