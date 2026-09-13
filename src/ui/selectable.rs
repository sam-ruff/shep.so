//! Native, read-only text selection. Creating editor buffers happens off-thread;
//! only selection/cursor actions are accepted by the UI.
use super::*;
use iced::{Length, widget::text};

#[derive(Debug, Clone)]
pub struct Content {
    pub source: Arc<MailDetail>,
    pub blocks: Vec<text_editor::Content>,
    pub title: text_editor::Content,
    pub conversation_title: Option<(String, text_editor::Content)>,
    pub plain: bool,
}
pub(super) const TITLE: usize = usize::MAX;
const CONVERSATION_TITLE: usize = usize::MAX - 1;
impl Content {
    pub fn new(source: Arc<MailDetail>, plain: bool) -> Self {
        let blocks = std::iter::once(source.latest_body.as_str())
            .chain(source.replies.iter().map(|reply| reply.body.as_str()))
            .filter(|_| plain)
            .map(text_editor::Content::with_text)
            .collect();
        let title = text_editor::Content::with_text(&source.summary.subject);
        Self {
            source,
            blocks,
            title,
            conversation_title: None,
            plain,
        }
    }
    pub fn same_text(&self, source: &MailDetail) -> bool {
        self.source.summary.id == source.summary.id
            && self.source.summary.subject == source.summary.subject
            && self.source.latest_body == source.latest_body
            && self.source.replies == source.replies
    }
    pub fn apply(&mut self, id: &str, index: usize, action: text_editor::Action) {
        if self.source.summary.id == id
            && !action.is_edit()
            && let Some(block) = if index == TITLE {
                Some(&mut self.title)
            } else if index == CONVERSATION_TITLE {
                self.conversation_title.as_mut().map(|(_, content)| content)
            } else {
                self.blocks.get_mut(index)
            }
        {
            block.perform(action);
        }
    }
}

impl App {
    pub(super) fn prepare_reader_selection(&mut self) -> Task<Message> {
        let Some(source) = self.detail.as_ref() else {
            self.reader_preparation = None;
            if self.pending_reader_selection.take().is_some() {
                self.reader_selection_generation += 1;
            }
            return Task::none();
        };
        let plain = !self.formatted(source);
        if let Some(content) = &mut self.reader_selection
            && content.plain == plain
            && (Arc::ptr_eq(&content.source, source) || content.same_text(source))
        {
            content.source = source.clone();
            self.reader_preparation = None;
            if self.pending_reader_selection.take().is_some() {
                self.reader_selection_generation += 1;
            }
            return Task::none();
        }
        if self
            .pending_reader_selection
            .as_ref()
            .is_some_and(|(pending, pending_plain)| {
                *pending_plain == plain && Arc::ptr_eq(pending, source)
            })
        {
            return Task::none();
        }
        self.reader_selection_generation += 1;
        let generation = self.reader_selection_generation;
        self.pending_reader_selection = Some((source.clone(), plain));
        let source = source.clone();
        let conversation_title = self
            .page
            .rows
            .iter()
            .find(|mail| Some(&mail.id) == self.selected.as_ref())
            .map(|mail| mail.subject.clone());
        self.reader_preparation = None;
        let (task, handle) = Task::perform(
            async move {
                static PREPARE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
                    std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(1)));
                let permit = PREPARE.clone().acquire_owned().await.ok()?;
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let mut content = Content::new(source, plain);
                    content.conversation_title = conversation_title.map(|title| {
                        let editor = text_editor::Content::with_text(&title);
                        (title, editor)
                    });
                    Box::new(content)
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
            super::text_context::TextContext::editor(
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
                    }),
                block,
                false,
                format!("{}:{index}", source.summary.id),
            )
            .menu_theme(self.theme())
            .into()
        } else {
            text(fallback)
                .size(u32::from(self.preferences.reader_font_size))
                .line_height(1.5)
                .into()
        }
    }

    pub(super) fn selectable_title<'a>(&'a self, source: &'a MailDetail) -> Element<'a, Message> {
        let size = if self.compact_reader() { 21 } else { 25 };
        let Some(content) = self.reader_selection.as_ref().filter(|content| {
            content.source.summary.id == source.summary.id
                && content.source.summary.subject == source.summary.subject
        }) else {
            return text(&source.summary.subject).size(size).font(BOLD).into();
        };
        let id = source.summary.id.clone();
        self.title_editor(&content.title, id, TITLE, size)
    }

    pub(super) fn selectable_conversation_title<'a>(
        &'a self,
        title: &'a str,
    ) -> Element<'a, Message> {
        if let Some(content) = &self.reader_selection
            && let Some((saved, editor)) = &content.conversation_title
            && saved == title
            && self.reader_id() == Some(&content.source.summary.id)
        {
            return self.title_editor(
                editor,
                content.source.summary.id.clone(),
                CONVERSATION_TITLE,
                23,
            );
        }
        text(title).size(23).font(BOLD).into()
    }

    fn title_editor<'a>(
        &'a self,
        content: &'a text_editor::Content,
        id: String,
        index: usize,
        size: u32,
    ) -> Element<'a, Message> {
        let context = format!("{id}:title:{index}");
        super::text_context::TextContext::editor(
            widget::text_editor(content)
                .on_action(move |action| Message::ReaderSelection(id.clone(), index, action))
                .key_binding(read_only_binding)
                .size(size)
                .font(BOLD)
                .padding(0)
                .height(Length::Shrink)
                .style(|theme, _| text_editor::Style {
                    background: iced::Color::TRANSPARENT.into(),
                    border: iced::Border::default(),
                    placeholder: colors(theme).muted,
                    value: colors(theme).text,
                    selection: colors(theme).accent.scale_alpha(0.25),
                }),
            content,
            false,
            context,
        )
        .menu_theme(self.theme())
        .into()
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
    async fn quick_reader_mode_return_preserves_selection_and_rejects_late_preparation()
    -> anyhow::Result<()> {
        let store = crate::store::Store::memory()?;
        let mail = parse_mail(
            "test", "1", "INBOX",
            b"Subject: Keep selected title\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Keep selected body.</p>".to_vec(),
            false, false,
        )?;
        let id = mail.summary.id.clone();
        store.upsert(vec![mail]).await?;
        let source = Arc::new(store.detail(id.clone()).await?);
        for plain in [true, false] {
            for refreshed in [false, true] {
                let (mut app, _) = App::new();
                app.selected = Some(id.clone());
                app.detail = Some(source.clone());
                let _ = app.handle_html(html_reader::Message::Plain(plain));
                let mut content = Content::new(source.clone(), plain);
                content.apply(&id, TITLE, text_editor::Action::SelectAll);
                content.apply(&id, 0, text_editor::Action::SelectAll);
                let body_selection = content.blocks.first().and_then(|body| body.selection());
                app.reader_selection = Some(Box::new(content));

                let _ = app.handle_html(html_reader::Message::Plain(!plain));
                let pending = app.prepare_reader_selection();
                let generation = app.reader_selection_generation;
                assert!(app.pending_reader_selection.is_some());
                assert!(app.reader_preparation.is_some());

                if refreshed {
                    app.detail = Some(Arc::new(source.as_ref().clone()));
                }
                let _ = app.handle_html(html_reader::Message::Plain(plain));
                let _ = app.prepare_reader_selection();
                assert!(
                    app.pending_reader_selection.is_none(),
                    "plain={plain}, refreshed={refreshed}"
                );
                assert!(app.reader_preparation.is_none());
                assert!(app.reader_selection_generation > generation);

                let _ = app.handle(Message::ReaderSelectionReady(
                    generation,
                    Some(Box::new(Content::new(source.clone(), !plain))),
                ));
                let retained = app
                    .reader_selection
                    .as_ref()
                    .expect("existing reader content");
                assert_eq!(retained.plain, plain);
                assert_eq!(
                    retained.title.selection().as_deref(),
                    Some("Keep selected title")
                );
                assert_eq!(
                    retained.blocks.first().and_then(|body| body.selection()),
                    body_selection
                );
                assert!(Arc::ptr_eq(
                    &retained.source,
                    app.detail.as_ref().expect("current detail")
                ));
                drop(pending);
            }
        }
        Ok(())
    }

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
        let source = Arc::new(store.detail(id.clone()).await.unwrap());
        let title_only = Content::new(source.clone(), false);
        assert!(title_only.blocks.is_empty());
        assert_eq!(title_only.title.text().trim_end(), "Select me");
        let mut content = Content::new(source, true);
        let before = content.blocks[0].text();
        content.apply(&id, TITLE, text_editor::Action::SelectAll);
        assert_eq!(content.title.selection().as_deref(), Some("Select me"));
        content.apply(
            &id,
            TITLE,
            text_editor::Action::Edit(text_editor::Edit::Insert('X')),
        );
        assert_eq!(content.title.text().trim_end(), "Select me");
        content.apply(
            "other-message",
            TITLE,
            text_editor::Action::Move(text_editor::Motion::Left),
        );
        assert_eq!(content.title.selection().as_deref(), Some("Select me"));
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
