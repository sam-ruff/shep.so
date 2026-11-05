//! Reader attachment preview dialog. Preparation and temporary copies run in
//! the engine; this module only tracks the current request and draws results.
mod prepared;

use super::*;
use crate::attachment_files::{Block, Content, Preview, Sheet};
use iced::{
    Alignment, Length,
    widget::{button, column, container, image, row, scrollable, space, text},
};

#[derive(Debug, Clone)]
pub enum Message {
    /// The reader message and the attachment's position in it.
    Show(String, usize),
    Step(bool),
    /// Previous or next PDF page.
    Page(bool),
    Sheet(usize),
    /// Fills the window with the preview, or returns to the dialog size.
    FullScreen(bool),
    Retry,
    Open,
    Save,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub source: String,
    pub index: usize,
    pub count: usize,
    pub name: String,
}

#[derive(Default)]
pub(super) struct State {
    request: u64,
    open_request: u64,
    pub target: Option<Target>,
    pub preview: Option<Arc<Preview>>,
    pub loading: bool,
    pub opening: bool,
    pub error: Option<String>,
    pub opened: Option<std::path::PathBuf>,
    /// PDF page currently requested.
    pub page: usize,
    pub sheet: usize,
    /// Previews prepared before a click; kept when the dialog closes.
    pub prepared: prepared::Prepared,
    /// Whether the shown preview came from `prepared`.
    pub from_cache: bool,
    pub full_screen: bool,
}
impl State {
    fn clear(&mut self) {
        self.request += 1;
        self.target = None;
        self.preview = None;
        self.loading = false;
        self.error = None;
        self.page = 0;
        self.sheet = 0;
        self.full_screen = false;
    }
}

impl App {
    pub(super) fn handle_attachment_preview(&mut self, message: Message) -> Task<super::Message> {
        match message {
            Message::Show(source, index) => {
                let Some(detail) = self
                    .detail
                    .as_deref()
                    .filter(|detail| detail.summary.id == source)
                else {
                    return Task::none();
                };
                let Some(attachment) = detail.attachments.get(index) else {
                    return Task::none();
                };
                let target = Target {
                    name: shep_mail_core::attachments::filename(&attachment.name),
                    count: detail.attachments.len(),
                    source,
                    index,
                };
                self.open(Dialog::Attachment);
                self.attachment_preview.opened = None;
                self.request_attachment_preview(target);
            }
            Message::Step(forward) => {
                let Some(target) = self.attachment_preview.target.clone() else {
                    return Task::none();
                };
                let Some(index) = (if forward {
                    target.index.checked_add(1).filter(|i| *i < target.count)
                } else {
                    target.index.checked_sub(1)
                }) else {
                    return Task::none();
                };
                let name = self
                    .attachment_name(&target.source, index)
                    .map(shep_mail_core::attachments::filename)
                    .unwrap_or_else(|| format!("Attachment {} of {}", index + 1, target.count));
                self.request_attachment_preview(Target {
                    index,
                    name,
                    ..target
                });
            }
            Message::Page(forward) => {
                let state = &self.attachment_preview;
                let Some(Content::Pages { page, pages, .. }) =
                    state.preview.as_deref().map(|p| &p.content)
                else {
                    return Task::none();
                };
                let Some(page) = (if forward {
                    page.checked_add(1).filter(|p| p < pages)
                } else {
                    page.checked_sub(1)
                }) else {
                    return Task::none();
                };
                self.request_attachment_page(page);
            }
            Message::Sheet(sheet) => {
                if let Some(Content::Sheets(sheets)) = self
                    .attachment_preview
                    .preview
                    .as_deref()
                    .map(|p| &p.content)
                    && sheet < sheets.len()
                {
                    self.attachment_preview.sheet = sheet;
                }
            }
            Message::FullScreen(full_screen) => {
                if self.dialog == Some(Dialog::Attachment) {
                    self.attachment_preview.full_screen = full_screen;
                }
            }
            Message::Retry => {
                let page = self.attachment_preview.page;
                if self.attachment_preview.target.is_some() {
                    self.request_attachment_page(page);
                }
            }
            Message::Open => {
                let state = &self.attachment_preview;
                // Open need not wait for the preview: the engine applies the
                // same program, macro and script checks before writing a copy.
                if state.opening || state.preview.as_ref().is_some_and(|p| !p.openable()) {
                    return Task::none();
                }
                let Some(target) = state.target.clone() else {
                    return Task::none();
                };
                let request = self.attachment_preview.open_request + 1;
                if self.try_command(Command::OpenAttachment(
                    request,
                    target.source,
                    target.index,
                )) {
                    self.attachment_preview.open_request = request;
                    self.attachment_preview.opening = true;
                }
            }
            Message::Save => {
                let Some(target) = self.attachment_preview.target.clone() else {
                    return Task::none();
                };
                self.attachment_preview.clear();
                return self.handle(super::Message::ExportAttachment(
                    target.source,
                    target.index,
                ));
            }
        }
        Task::none()
    }

    fn request_attachment_preview(&mut self, mut target: Target) {
        let state = &mut self.attachment_preview;
        state.preview = None;
        state.sheet = 0;
        state.from_cache = false;
        if let Some(preview) = state.prepared.get(&target.source, target.index) {
            // Fences any earlier request still in flight.
            state.request += 1;
            state.error = None;
            state.page = 0;
            state.loading = false;
            state.from_cache = true;
            target.name = preview.name.clone();
            state.target = Some(target);
            state.preview = Some(preview);
            return;
        }
        state.target = Some(target);
        self.request_attachment_page(0);
    }

    /// Prepares first-page previews in the background for the open message and
    /// the nearest unread messages with attachments, so the dialog can show
    /// them as soon as it opens.
    pub(super) fn prepare_attachment_previews(&mut self) {
        let Some(selected) = self.selected.clone() else {
            return;
        };
        if self.page.is_transient_placeholder(&selected) {
            return;
        }
        let mut targets = Vec::new();
        if let Some(detail) = &self.detail
            && !detail.attachments.is_empty()
        {
            targets.push((
                detail.summary.id.clone(),
                Some(detail.attachments.clone()),
                detail.attachments.len(),
            ));
        }
        for mail in prepared::nearby(&self.page.rows, &selected) {
            let attachments = self
                .detail_cache
                .iter()
                .find(|detail| detail.summary.id == mail.id)
                .map(|detail| detail.attachments.clone());
            targets.push((mail.id.clone(), attachments, mail.attachment_count));
        }
        for (source, attachments, count) in targets {
            let indices = self.attachment_preview.prepared.wanted(&source, count);
            if indices.is_empty() {
                continue;
            }
            self.attachment_preview
                .prepared
                .requested(&source, &indices);
            // A full queue forgets the request; the next selection retries.
            if !self.try_command(Command::PrepareAttachmentPreviews(
                source,
                attachments,
                indices,
            )) {
                break;
            }
        }
    }

    pub(super) fn attachment_preview_prepared(
        &mut self,
        source: String,
        index: usize,
        preview: Option<Arc<Preview>>,
    ) {
        let open = self.dialog == Some(Dialog::Attachment);
        let state = &mut self.attachment_preview;
        // A click that is still waiting takes the prepared preview instead.
        if let Some(preview) = &preview
            && open
            && state.loading
            && state.page == 0
            && state
                .target
                .as_ref()
                .is_some_and(|t| t.source == source && t.index == index)
        {
            state.request += 1;
            state.loading = false;
            state.from_cache = true;
            if let Some(target) = &mut state.target {
                target.name = preview.name.clone();
            }
            state.preview = Some(preview.clone());
        }
        let protect = self.detail.as_ref().map(|d| d.summary.id.clone());
        self.attachment_preview
            .prepared
            .finished(source, index, preview, protect.as_deref());
    }

    /// Requests a page of the current attachment. The shown page stays until
    /// the new one is ready.
    fn request_attachment_page(&mut self, page: usize) {
        let state = &mut self.attachment_preview;
        let Some(target) = &state.target else {
            return;
        };
        state.request += 1;
        state.error = None;
        state.page = page;
        let command =
            Command::PreviewAttachment(state.request, target.source.clone(), target.index, page);
        let queued = self.try_command(command);
        self.attachment_preview.loading = queued;
        if !queued {
            self.attachment_preview.error =
                Some("Shep is busy preparing other attachments. Try again shortly.".into());
        }
    }

    pub(super) fn attachment_preview_ready(
        &mut self,
        request: u64,
        result: Result<Arc<Preview>, String>,
    ) {
        let state = &mut self.attachment_preview;
        if request != state.request || self.dialog != Some(Dialog::Attachment) {
            return;
        }
        state.loading = false;
        match result {
            Ok(preview) => {
                if let Some(target) = &mut state.target {
                    target.name = preview.name.clone();
                }
                if let Content::Pages { page, .. } = preview.content {
                    state.page = page;
                }
                state.preview = Some(preview);
            }
            Err(error) => state.error = Some(error),
        }
    }

    pub(super) fn attachment_opened(
        &mut self,
        request: u64,
        result: Result<std::path::PathBuf, String>,
    ) {
        if request != self.attachment_preview.open_request {
            return;
        }
        self.attachment_preview.opening = false;
        match result {
            Ok(path) => {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.attachment_preview.opened = Some(path);
                self.notice(
                    format!("Opened {name} from a temporary copy. Changes to it are not saved to the email."),
                    false,
                );
            }
            Err(error) => self.notice(error, true),
        }
    }

    pub(super) fn attachment_name(&self, source: &str, index: usize) -> Option<&str> {
        self.detail
            .as_deref()
            .filter(|detail| detail.summary.id == source)
            .and_then(|detail| detail.attachments.get(index))
            .map(|attachment| attachment.name.as_str())
    }

    #[cfg(feature = "test-support")]
    pub(super) fn attachment_preview_observation(&self) -> serde_json::Value {
        let state = &self.attachment_preview;
        let preview = state.preview.as_deref();
        let (image, text_bytes, truncated) = match preview.map(|p| &p.content) {
            Some(Content::Image { width, height, .. } | Content::Pages { width, height, .. }) => {
                (Some([*width, *height]), None, false)
            }
            Some(Content::Text { text, truncated }) => (None, Some(text.len()), *truncated),
            Some(Content::Document { truncated, .. }) => (None, None, *truncated),
            _ => (None, None, false),
        };
        let pages = match preview.map(|p| &p.content) {
            Some(Content::Pages { page, pages, .. }) => Some([page + 1, *pages]),
            _ => None,
        };
        let blocks = match preview.map(|p| &p.content) {
            Some(Content::Document { blocks, .. }) => Some(
                blocks
                    .iter()
                    .take(20)
                    .map(|block| match block {
                        crate::attachment_files::Block::Heading(text) => format!("# {text}"),
                        crate::attachment_files::Block::Paragraph(text) => text.clone(),
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        let sheets = match preview.map(|p| &p.content) {
            Some(Content::Sheets(sheets)) => Some(
                sheets
                    .iter()
                    .map(|sheet| {
                        serde_json::json!({
                            "name": sheet.name,
                            "rows": sheet.rows.len(),
                            "first_row": sheet.rows.first(),
                            "truncated": sheet.truncated,
                        })
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        };
        serde_json::json!({
            "open": self.dialog == Some(Dialog::Attachment),
            "source": state.target.as_ref().map(|t| &t.source),
            "index": state.target.as_ref().map(|t| t.index),
            "count": state.target.as_ref().map(|t| t.count),
            "name": state.target.as_ref().map(|t| &t.name),
            "kind": preview.map(|p| p.kind),
            "size": preview.map(|p| p.size),
            "openable": preview.map(|p| p.openable()),
            "refusal": preview.and_then(|p| p.refusal).map(|r| format!("{r:?}")),
            "image": image,
            "pages": pages,
            "blocks": blocks,
            "sheets": sheets,
            "sheet": state.sheet,
            "from_cache": state.from_cache,
            "full_screen": state.full_screen,
            "prepared": state.prepared.keys(),
            "text_bytes": text_bytes,
            "truncated": truncated,
            "loading": state.loading,
            "opening": state.opening,
            "error": &state.error,
            "opened": &state.opened,
        })
    }

    pub(super) fn close_attachment_preview(&mut self) {
        self.attachment_preview.clear();
    }

    pub(super) fn attachment_preview_title(&self) -> &str {
        self.attachment_preview
            .target
            .as_ref()
            .map(|target| target.name.as_str())
            .unwrap_or("Attachment")
    }

    /// The full-screen toggle shown beside the dialog's close button.
    pub(super) fn attachment_full_screen_action(&self) -> Element<'_, super::Message> {
        let full_screen = self.attachment_preview.full_screen;
        let (icon, label) = if full_screen {
            ("collapse", "Exit full screen")
        } else {
            ("expand", "Full screen")
        };
        self.icon_action(
            icon,
            label,
            super::Message::AttachmentPreview(Message::FullScreen(!full_screen)),
        )
    }

    /// Leaves full screen; returns false when the preview was not full screen.
    pub(super) fn leave_attachment_full_screen(&mut self) -> bool {
        let state = &mut self.attachment_preview;
        if self.dialog != Some(Dialog::Attachment) || !state.full_screen {
            return false;
        }
        state.full_screen = false;
        true
    }

    pub(super) fn attachment_preview_form(&self) -> Element<'_, super::Message> {
        let state = &self.attachment_preview;
        let truncated = match state.preview.as_deref().map(|p| &p.content) {
            Some(Content::Text { truncated, .. } | Content::Document { truncated, .. }) => {
                *truncated
            }
            Some(Content::Sheets(sheets)) => sheets.get(state.sheet).is_some_and(|s| s.truncated),
            _ => false,
        };
        let refusal = state.preview.as_ref().and_then(|p| p.refusal);
        // Header, actions and notes stay visible without scrolling the dialog.
        let height = if state.full_screen {
            // Title, metadata and actions take about 236 px; each note 32 px.
            let notes = f32::from(u8::from(truncated) + u8::from(refusal.is_some()));
            (self.size.height - 236. - 32. * notes).max(160.)
        } else {
            (self.size.height - 390.).clamp(160., 640.)
        };
        let wrap = |message| super::Message::AttachmentPreview(message);
        let mut body = column![].spacing(14);
        let content: Element<'_, super::Message> = match (&state.preview, &state.error) {
            (_, Some(error)) => column![
                text(error).size(13),
                action("Try again", wrap(Message::Retry)),
            ]
            .spacing(12)
            .into(),
            (Some(preview), None) => {
                body = body.push(
                    muted(format!("{} · {}", preview.kind, size_label(preview.size))).size(12),
                );
                self.preview_content(preview, height)
            }
            (None, None) => container(muted("Preparing preview…").size(13))
                .center(Length::Fill)
                .height(height)
                .into(),
        };
        body = body.push(content);
        if truncated {
            body = body.push(
                muted("Showing the beginning of this file. Open it to see everything.").size(12),
            );
        }
        if let Some(refusal) = refusal {
            body = body.push(muted(refusal.note()).size(12));
        }
        let openable = refusal.is_none() && state.target.is_some();
        let mut actions = row![
            button(
                row![
                    icon_bright("external", 18.),
                    text(if state.opening { "Opening…" } else { "Open" }).size(12)
                ]
                .spacing(8)
                .align_y(Alignment::Center)
            )
            .padding([10, 16])
            .style(primary)
            .on_press_maybe((openable && !state.opening).then_some(wrap(Message::Open))),
            button(
                row![icon("download", 18.), text("Save a copy…").size(12)]
                    .spacing(8)
                    .align_y(Alignment::Center)
            )
            .padding([10, 14])
            .style(outline)
            .on_press_maybe(state.target.is_some().then_some(wrap(Message::Save))),
            space().width(Length::Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if let Some(target) = state.target.as_ref().filter(|t| t.count > 1) {
            actions = actions
                .push(self.icon_action_maybe(
                    "left",
                    "Previous attachment",
                    (target.index > 0).then_some(wrap(Message::Step(false))),
                ))
                .push(muted(format!("{} of {}", target.index + 1, target.count)).size(12))
                .push(self.icon_action_maybe(
                    "chevron",
                    "Next attachment",
                    (target.index + 1 < target.count).then_some(wrap(Message::Step(true))),
                ));
        }
        body.push(actions).into()
    }
}

impl App {
    fn preview_content<'a>(
        &'a self,
        preview: &'a Preview,
        height: f32,
    ) -> Element<'a, super::Message> {
        let wrap = |message| super::Message::AttachmentPreview(message);
        match &preview.content {
            Content::Pages {
                handle,
                page,
                pages,
                ..
            } => {
                let label = if self.attachment_preview.loading {
                    "Preparing page…".to_owned()
                } else {
                    format!("Page {} of {pages}", page + 1)
                };
                column![
                    container(
                        image(handle.clone())
                            .content_fit(iced::ContentFit::Contain)
                            .width(Length::Fill)
                            .height(Length::Fill),
                    )
                    .center(Length::Fill)
                    .height(height - 48.)
                    .padding(8)
                    .style(subtle),
                    row![
                        space().width(Length::Fill),
                        self.icon_action_maybe(
                            "left",
                            "Previous page",
                            (*page > 0).then_some(wrap(Message::Page(false))),
                        ),
                        muted(label).size(12),
                        self.icon_action_maybe(
                            "chevron",
                            "Next page",
                            (page + 1 < *pages).then_some(wrap(Message::Page(true))),
                        ),
                        space().width(Length::Fill),
                    ]
                    .spacing(8)
                    .height(40)
                    .align_y(Alignment::Center),
                ]
                .spacing(8)
                .into()
            }
            Content::Document { blocks, .. } => document(blocks, height),
            Content::Sheets(sheets) => self.sheets(sheets, height),
            _ => preview_content(preview, height),
        }
    }

    fn sheets<'a>(&'a self, sheets: &'a [Sheet], height: f32) -> Element<'a, super::Message> {
        let current = self
            .attachment_preview
            .sheet
            .min(sheets.len().saturating_sub(1));
        let Some(sheet) = sheets.get(current) else {
            return empty_card("This spreadsheet has no sheets to show.", height);
        };
        let mut body = column![].spacing(8);
        if sheets.len() > 1 {
            let tabs = sheets
                .iter()
                .enumerate()
                .fold(row![].spacing(6), |tabs, (i, s)| {
                    let name = if s.name.is_empty() {
                        format!("Sheet {}", i + 1)
                    } else {
                        s.name.clone()
                    };
                    tabs.push(
                        button(text(name).size(12))
                            .padding([6, 12])
                            .style(if i == current { selected } else { ghost })
                            .on_press(super::Message::AttachmentPreview(Message::Sheet(i))),
                    )
                });
            body = body.push(
                scrollable(tabs)
                    .direction(scrollable::Direction::Horizontal(
                        scrollable::Scrollbar::new().width(4).scroller_width(4),
                    ))
                    .spacing(4),
            );
        }
        if sheet.rows.is_empty() {
            return body
                .push(empty_card("This sheet is empty.", height - 48.))
                .into();
        }
        let width = sheet.rows.first().map_or(0, Vec::len);
        let header = (0..width).fold(row![corner_cell()], |header, column| {
            header.push(grid_cell(column_name(column), true))
        });
        let grid = sheet
            .rows
            .iter()
            .enumerate()
            .fold(column![header], |grid, (index, cells)| {
                grid.push(
                    cells
                        .iter()
                        .fold(row![grid_label((index + 1).to_string())], |row, value| {
                            row.push(grid_cell(value.clone(), false))
                        }),
                )
            });
        body.push(
            container(
                scrollable(container(grid).padding(1))
                    .id("attachment-sheet")
                    .direction(scrollable::Direction::Both {
                        vertical: scrollable::Scrollbar::default(),
                        horizontal: scrollable::Scrollbar::default(),
                    })
                    .width(Length::Fill)
                    .height(Length::Fill),
            )
            .height(if sheets.len() > 1 {
                height - 44.
            } else {
                height
            })
            .style(subtle),
        )
        .into()
    }
}

const CELL_WIDTH: f32 = 128.;

fn grid_cell<'a>(value: String, header: bool) -> Element<'a, super::Message> {
    let label = text(value)
        .size(12)
        .wrapping(iced::widget::text::Wrapping::None);
    let label = if header { label.font(BOLD) } else { label };
    container(label)
        .width(CELL_WIDTH)
        .padding([5, 8])
        .clip(true)
        .style(if header { cell_header } else { cell_body })
        .into()
}

fn grid_label<'a>(value: String) -> Element<'a, super::Message> {
    container(muted(value).size(11))
        .width(44)
        .padding([5, 8])
        .style(cell_header)
        .into()
}

fn corner_cell<'a>() -> Element<'a, super::Message> {
    container(space())
        .width(44)
        .height(26)
        .style(cell_header)
        .into()
}

fn cell_body(theme: &iced::Theme) -> container::Style {
    let colors = colors(theme);
    container::Style {
        background: Some(colors.surface.into()),
        text_color: Some(colors.text),
        border: iced::Border {
            color: colors.border,
            width: 0.5,
            radius: 0.into(),
        },
        ..Default::default()
    }
}

fn cell_header(theme: &iced::Theme) -> container::Style {
    let colors = colors(theme);
    container::Style {
        background: Some(colors.subtle.into()),
        text_color: Some(colors.text),
        border: iced::Border {
            color: colors.border,
            width: 0.5,
            radius: 0.into(),
        },
        ..Default::default()
    }
}

fn column_name(mut index: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (index % 26) as u8);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name.reverse();
    String::from_utf8_lossy(&name).into_owned()
}

fn document(blocks: &[Block], height: f32) -> Element<'_, super::Message> {
    if blocks.is_empty() {
        return empty_card("This document has no text to show.", height);
    }
    let page = blocks.iter().fold(column![].spacing(10), |page, block| {
        page.push(match block {
            Block::Heading(title) => text(title).size(16).font(BOLD),
            Block::Paragraph(body) => text(body).size(13),
        })
    });
    container(
        scrollable(container(page).padding([18, 22]).width(Length::Fill))
            .id("attachment-document")
            .height(height),
    )
    .style(subtle)
    .into()
}

fn empty_card(message: &str, height: f32) -> Element<'_, super::Message> {
    container(muted(message).size(13))
        .center(Length::Fill)
        .height(height.min(200.))
        .style(subtle)
        .into()
}

fn preview_content(preview: &Preview, height: f32) -> Element<'_, super::Message> {
    match &preview.content {
        Content::Image { handle, .. } => container(
            image(handle.clone())
                .content_fit(iced::ContentFit::ScaleDown)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .center(Length::Fill)
        .height(height)
        .padding(8)
        .style(subtle)
        .into(),
        Content::Text { text: body, .. } => container(
            scrollable(
                container(text(body).font(iced::Font::MONOSPACE).size(12))
                    .padding(14)
                    .width(Length::Fill),
            )
            .id("attachment-text")
            .height(height),
        )
        .style(subtle)
        .into(),
        // Pages, documents and sheets are drawn by `App::preview_content`.
        _ => container(
            column![
                icon("file", 40.),
                text(preview.kind).size(14).font(BOLD),
                muted(if preview.openable() {
                    "Shep can't show a preview of this file. Open it with your default app or save a copy."
                } else {
                    "Shep can't show a preview of this file."
                })
                .size(12),
            ]
            .spacing(12)
            .align_x(Alignment::Center),
        )
        .center(Length::Fill)
        .height(height.min(260.))
        .padding(20)
        .style(subtle)
        .into(),
    }
}

pub(super) fn size_label(bytes: usize) -> String {
    const KIB: f64 = 1024.;
    let bytes = bytes as f64;
    if bytes < KIB {
        format!("{bytes} bytes")
    } else if bytes < KIB * KIB {
        format!("{:.0} KB", (bytes / KIB).ceil())
    } else {
        format!("{:.1} MB", bytes / (KIB * KIB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_naturally() {
        assert_eq!(size_label(15), "15 bytes");
        assert_eq!(size_label(1025), "2 KB");
        assert_eq!(size_label(5 * 1024 * 1024), "5.0 MB");
    }
}
