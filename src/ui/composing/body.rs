//! The message body: iced-cube's rich text editor with its formatting
//! toolbar above it. A draft keeps the plain-text alternative in `body` and,
//! once anything is formatted, the HTML of what the sender wrote in
//! `body_html`.
use super::*;
use iced_cube::formatting_toolbar::{self, Output, Tool, formatting_toolbar};
use iced_cube::keys::Keymap as Chords;
use iced_cube::rich_text_editor::{
    self as rich, Content, Document, Selection, Shortcut, Shortcuts,
};

pub(in crate::ui) const BODY: &str = "compose-body";
/// Space between the toolbar and the editor.
const GAP: f32 = 6.;

/// The editor for `draft`'s body, with the caret at the start.
pub(in crate::ui) fn load(draft: &Draft) -> Content {
    let mut content = draft
        .body_html
        .as_deref()
        .and_then(|html| formatted(draft, html))
        .unwrap_or_else(|| Content::with_text(&draft.body));
    content.perform(rich::Action::Move(rich::Motion::DocumentStart));
    content
}

/// The saved formatting, unless the plain body has since changed without it.
fn formatted(draft: &Draft, html: &str) -> Option<Content> {
    let written = rich::html::from_html(html);
    let document = match (&draft.forward, draft.forwarded_note()) {
        (Some(quote), Some(_)) => Document::from_blocks(
            written
                .blocks()
                .iter()
                .chain(Document::from_text(&quote.text).blocks())
                .cloned(),
        ),
        _ => written,
    };
    let content = Content::with_document(document);
    (content.plain_text() == draft.body).then_some(content)
}

/// Writes the editor into `draft`: the plain-text alternative, and the HTML
/// of what the sender wrote when any of it is formatted.
pub(in crate::ui) fn write(content: &Content, draft: &mut Draft) {
    draft.body = content.plain_text();
    let document = content.document();
    draft.body_html = match draft.forwarded_note() {
        // The original stays out of the HTML; its own is sent below the note.
        Some(note) => {
            let note = Document::from_blocks(
                document
                    .blocks()
                    .iter()
                    .take(note.matches('\n').count())
                    .cloned(),
            );
            note.has_formatting().then(|| rich::html::to_html(&note))
        }
        None => document
            .has_formatting()
            .then(|| rich::html::to_html(document)),
    };
}

/// What the toolbar was built and last matched to.
#[derive(Default)]
pub(in crate::ui) struct Toolbar {
    pub state: formatting_toolbar::State,
    /// The editor's chords and the shortcuts it handles.
    keys: Option<(Chords<Shortcut>, Shortcuts)>,
    built: Option<(u64, crate::shortcuts::Keymap, bool)>,
    synced: Option<(u64, Selection, u64)>,
}

impl App {
    /// Rebuilds the toolbar for another draft or changed shortcuts, and
    /// matches its buttons to the caret. Returns what the editor's link
    /// shortcut asked for.
    pub(in crate::ui) fn sync_formatting(&mut self) -> Option<Output> {
        let session = &self.composer.current;
        let hints = self.preferences.tooltips && self.preferences.shortcut_tooltips;
        let toolbar = &mut self.composer.toolbar;
        let current = toolbar.built.as_ref().is_some_and(|(key, keys, shown)| {
            *key == session.ui_key && *keys == self.preferences.shortcuts && *shown == hints
        });
        if !current {
            let keys = self.preferences.shortcuts.editor();
            let shown = if hints { keys.0.clone() } else { Chords::new() };
            toolbar.state = formatting_toolbar::State::with_keymap(&shown);
            toolbar.keys = Some(keys);
            toolbar.built = Some((session.ui_key, self.preferences.shortcuts.clone(), hints));
            toolbar.synced = None;
        }
        let editor = &session.editor;
        let at = (
            editor.revision(),
            editor.selection(),
            editor.link_requests(),
        );
        if toolbar.synced == Some(at) {
            return None;
        }
        toolbar.synced = Some(at);
        toolbar.state.sync(editor)
    }

    /// Applies an editor or toolbar action to the body.
    pub(in crate::ui) fn edit_body(&mut self, action: rich::Action) -> Task<Message> {
        if !self.compose_editable() {
            return Task::none();
        }
        if action.is_edit() {
            self.draft_edited();
        }
        self.composer.current.editor.perform(action);
        // Formatting at a caret changes what is typed next, not the revision.
        self.composer.toolbar.synced = None;
        let output = self.sync_formatting();
        self.formatting_output(output)
    }

    pub(in crate::ui) fn format_body(&mut self, event: formatting_toolbar::Event) -> Task<Message> {
        if !self.compose_editable() {
            return Task::none();
        }
        let _ = self.sync_formatting();
        let output = self
            .composer
            .toolbar
            .state
            .update(event, &self.composer.current.editor);
        self.formatting_output(output)
    }

    fn formatting_output(&mut self, output: Option<Output>) -> Task<Message> {
        // A press on the toolbar took focus from the body; it is reported
        // again once the body has it back.
        match output {
            Some(Output::Format(action)) => {
                self.focused_input = None;
                Task::batch([self.edit_body(action), self.focus_after_layout(BODY)])
            }
            Some(Output::EditLink) => {
                self.focused_input = None;
                self.focus_after_layout(formatting_toolbar::LINK_FIELD)
            }
            Some(Output::Closed) => {
                self.focused_input = None;
                self.focus_after_layout(BODY)
            }
            Some(Output::Unfocus) => formatting_toolbar::unfocus(),
            None => Task::none(),
        }
    }

    /// The toolbar above the editor, `height` tall together.
    pub(in crate::ui) fn body_editor(&self, height: f32) -> Element<'_, Message> {
        let session = &self.composer.current;
        let toolbar = &self.composer.toolbar;
        let (chords, shortcuts) = toolbar
            .keys
            .clone()
            .unwrap_or_else(|| self.preferences.shortcuts.editor());
        let bar = formatting_toolbar(&toolbar.state)
            .tooltip(
                self.preferences
                    .tooltips
                    .then_some(iced_cube::tooltip::Position::Bottom),
            )
            .on_event(Message::Formatting);
        let editor = rich::rich_text_editor(&session.editor)
            .id(BODY)
            .placeholder("Write your message…")
            .text_size(13.)
            .padding(15)
            .height((height - TOOLBAR_HEIGHT - GAP).max(rich::MIN_HEIGHT))
            .keymap(chords)
            .shortcuts(shortcuts)
            .on_action(Message::Editor);
        let editor = super::super::text_context::TextContext::rich(
            editor,
            &session.editor,
            session.draft.id.clone(),
        )
        .menu_theme(self.theme());
        widget::themer(
            Some(self.cube_theme()),
            iced::widget::column![bar, editor].spacing(GAP),
        )
        .into()
    }

    pub(in crate::ui) fn formatting_observation(&self) -> serde_json::Value {
        let state = &self.composer.toolbar.state;
        let draft = self.current_draft();
        let saved = self
            .workspace
            .drafts
            .iter()
            .find(|saved| saved.id == draft.id);
        let active: Vec<_> = [
            Tool::Bold,
            Tool::Italic,
            Tool::Underline,
            Tool::Strikethrough,
        ]
        .into_iter()
        .filter(|tool| state.is_on(*tool))
        .map(Tool::label)
        .collect();
        serde_json::json!({
            "active": active,
            "size": state.size().map(rich::Size::label),
            "bullets": state.is_on(Tool::Bullets),
            "numbers": state.is_on(Tool::Numbers),
            "link": state.link(),
            "plain": draft.body,
            "html": draft.body_html,
            "saved_body": saved.map(|saved| &saved.body),
            "saved_html": saved.and_then(|saved| saved.body_html.as_ref()),
        })
    }
}

/// The toolbar's height, which the editor gives up so the controls below
/// keep their place.
pub(in crate::ui) const TOOLBAR_HEIGHT: f32 = 40.;
