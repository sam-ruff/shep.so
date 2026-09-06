//! Native, read-only text selection. Creating editor buffers happens off-thread;
//! only selection/cursor actions are accepted by the UI.
use super::*;
use iced::{Length, widget::text};

#[derive(Debug, Clone)]
pub struct Content {
    pub source: Arc<MailDetail>,
    pub blocks: Vec<text_editor::Content>,
}
impl Content {
    pub fn new(source: Arc<MailDetail>) -> Self {
        let blocks = std::iter::once(source.latest_body.as_str())
            .chain(source.replies.iter().map(|reply| reply.body.as_str()))
            .map(text_editor::Content::with_text)
            .collect();
        Self { source, blocks }
    }
    pub fn same_text(&self, source: &MailDetail) -> bool {
        self.source.summary.id == source.summary.id
            && self.source.latest_body == source.latest_body
            && self.source.replies == source.replies
    }
    pub fn apply(&mut self, id: &str, index: usize, action: text_editor::Action) {
        if self.source.summary.id == id
            && !action.is_edit()
            && let Some(block) = self.blocks.get_mut(index)
        {
            block.perform(action);
        }
    }
}

impl App {
    pub(super) fn prepare_reader_selection(&mut self) -> Task<Message> {
        let Some(source) = self.detail.as_ref().filter(|d| !self.formatted(d)) else {
            self.reader_preparation = None;
            if self.pending_reader_selection.take().is_some() {
                self.reader_selection_generation += 1;
            }
            return Task::none();
        };
        if let Some(content) = &mut self.reader_selection {
            if Arc::ptr_eq(&content.source, source) {
                return Task::none();
            }
            if content.same_text(source) {
                content.source = source.clone();
                self.reader_preparation = None;
                if self.pending_reader_selection.take().is_some() {
                    self.reader_selection_generation += 1;
                }
                return Task::none();
            }
        }
        if self
            .pending_reader_selection
            .as_ref()
            .is_some_and(|pending| Arc::ptr_eq(pending, source))
        {
            return Task::none();
        }
        self.reader_selection_generation += 1;
        let generation = self.reader_selection_generation;
        self.pending_reader_selection = Some(source.clone());
        let source = source.clone();
        self.reader_preparation = None;
        let (task, handle) = Task::perform(
            async move {
                static PREPARE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
                    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(1)));
                let permit = PREPARE.clone().acquire_owned().await.ok()?;
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    Box::new(Content::new(source))
                })
                .await
                .ok()
            },
            move |content| Message::ReaderSelectionReady(generation, content),
        )
        .abortable();
        self.reader_preparation = Some(handle.abort_on_drop());
        task
    }
    pub(super) fn selectable_body<'a>(
        &'a self,
        source: &'a MailDetail,
        index: usize,
        fallback: &'a str,
    ) -> Element<'a, Message> {
        if let Some(content) = &self.reader_selection
            && std::ptr::eq(content.source.as_ref(), source)
            && let Some(block) = content.blocks.get(index)
        {
            let id = source.summary.id.clone();
            widget::text_editor(block)
                .on_action(move |action| Message::ReaderSelection(id.clone(), index, action))
                .key_binding(read_only_binding)
                .size(u32::from(self.preferences.reader_font_size))
                .line_height(1.5)
                .padding(0)
                .height(Length::Shrink)
                .style(|theme, _| text_editor::Style {
                    background: iced::Color::TRANSPARENT.into(),
                    border: iced::Border::default(),
                    placeholder: colors(theme).muted,
                    value: colors(theme).text,
                    selection: colors(theme).accent.scale_alpha(0.25),
                })
                .into()
        } else {
            text(fallback)
                .size(u32::from(self.preferences.reader_font_size))
                .line_height(1.5)
                .into()
        }
    }
}

fn read_only_binding(key: text_editor::KeyPress) -> Option<text_editor::Binding<Message>> {
    use text_editor::Binding;
    match Binding::from_key_press(key)? {
        binding @ (Binding::Copy
        | Binding::Select(_)
        | Binding::Move(_)
        | Binding::SelectWord
        | Binding::SelectLine
        | Binding::SelectAll
        | Binding::Unfocus) => Some(binding),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn reader_selection_can_copy_but_never_modify_or_target_another_message() {
        let store = crate::store::Store::memory().unwrap();
        let mail = parse_mail(
            "test",
            "1",
            "INBOX",
            b"From: test@example.com\r\nSubject: Select me\r\n\r\nThe original message.".to_vec(),
            false,
            false,
        )
        .unwrap();
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await.unwrap();
        let mut content = Content::new(Arc::new(store.detail(id.clone()).await.unwrap()));
        let before = content.blocks[0].text();
        content.apply(&id, 0, text_editor::Action::SelectAll);
        assert_eq!(
            content.blocks[0].selection().as_deref(),
            Some(before.trim_end_matches('\n'))
        );
        content.apply(
            &id,
            0,
            text_editor::Action::Edit(text_editor::Edit::Insert('X')),
        );
        assert_eq!(content.blocks[0].text(), before);
        content.apply(
            "other-message",
            0,
            text_editor::Action::Move(text_editor::Motion::Left),
        );
        assert_eq!(
            content.blocks[0].selection().as_deref(),
            Some(before.trim_end_matches('\n'))
        );
    }
}
