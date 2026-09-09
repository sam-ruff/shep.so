//! Compact, mouse-friendly conversation rows shared by every mail scope.
pub(super) mod reveal;
use super::ellipsis::Ellipsis;
use super::views::{sender_name, truncate};
use super::*;
use iced::{
    Alignment, Border, Color, Length,
    widget::{button, checkbox, column, container, opaque, row, space},
};

pub(super) const ROW_HEIGHT: f32 = 60.;

fn blend(base: Color, accent: Color, amount: f32) -> Color {
    Color::from_rgb(
        base.r + (accent.r - base.r) * amount,
        base.g + (accent.g - base.g) * amount,
        base.b + (accent.b - base.b) * amount,
    )
}

fn row_style(theme: &Theme, status: button::Status, selected: bool, unread: bool) -> button::Style {
    let p = colors(theme);
    let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
    let background = if selected {
        p.tint
    } else if unread {
        blend(p.surface, p.accent, if hovered { 0.13 } else { 0.065 })
    } else if hovered {
        p.subtle
    } else {
        p.surface
    };
    button::Style {
        background: Some(background.into()),
        text_color: p.text,
        ..Default::default()
    }
}

impl App {
    pub(super) fn mail_list_width(&self) -> f32 {
        self.size.width / (self.preferences.interface_scale as f32 / 100.)
            - self.sidebar_width()
            - 41.
            - self.reader_width()
    }

    pub(super) fn mail_row(&self, mail: &Mail, active: bool) -> Element<'_, Message> {
        let sender = sender_name(&mail.sender);
        let date = chrono::DateTime::from_timestamp(mail.timestamp, 0)
            .unwrap_or_default()
            .with_timezone(&chrono::Local);
        let date = if date.date_naive() == chrono::Local::now().date_naive() {
            date.format("%H:%M").to_string()
        } else {
            date.format("%d %b").to_string()
        };
        let unread = mail.unread;
        let leading: Element<'_, Message> =
            if self.mail_selection.mode {
                super::context_menu::ContextArea::sidebar(
                    button(
                        checkbox(self.mail_selection.visible.contains(&mail.id))
                            .size(18)
                            .on_toggle({
                                let id = mail.id.clone();
                                move |_| Message::CheckMail(id.clone())
                            }),
                    )
                    .width(40)
                    .height(44)
                    .padding(11)
                    .style(ghost)
                    .on_press(Message::CheckMail(mail.id.clone())),
                )
                .with_drag(super::drag_mail::Region::Block(self.mail_drag.clone()))
                .into()
            } else {
                container(container(space()).width(7).height(7).style(move |theme| {
                    container::Style {
                        background: unread.then(|| colors(theme).accent.into()),
                        border: Border {
                            radius: 4.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }
                }))
                .width(16)
                .height(44)
                .center_x(16)
                .center_y(44)
                .into()
            };
        let flag = super::context_menu::ContextArea::sidebar(opaque(
            button(flag_icon(mail.starred, 18.))
                .width(40)
                .height(44)
                .padding(11)
                .style(if mail.starred { flagged } else { ghost })
                .on_press_maybe(
                    (!self.mail_actions.restoring(&mail.id) && !self.bulk_owns_mail(&mail.id))
                        .then(|| Message::FlagRow(mail.id.clone())),
                ),
        ))
        .with_drag(super::drag_mail::Region::Block(self.mail_drag.clone()));
        let subject = Ellipsis::new(truncate(&mail.subject, 120), 12.).font(if unread {
            BOLD
        } else {
            iced::Font::DEFAULT
        });
        let preview = container(Ellipsis::new(truncate(&mail.preview, 160), 11.)).style(|theme| {
            container::Style {
                text_color: Some(colors(theme).muted),
                ..Default::default()
            }
        });
        let mut metadata = row![].spacing(5).align_y(Alignment::Center);
        if self.mail_actions.restoring(&mail.id) {
            metadata = metadata.push(muted("Restoring…").size(10));
        }
        if self.query.searches_all_folders() {
            let folder = self
                .workspace
                .folder_label(Some(mail.account_id.as_str()), &mail.folder);
            metadata = metadata.push(
                container(muted(truncate(&folder, 15)).size(10))
                    .padding([1, 4])
                    .style(subtle),
            );
        }
        if mail.attachment_count > 0 {
            metadata = metadata.push(icon("clip", 14.));
        }
        metadata = metadata.push(muted(date).size(10));
        let content: Element<'_, Message> = if self.mail_list_width() >= 640. {
            row![
                container(Ellipsis::new(sender.clone(), 12.))
                    .width(Length::FillPortion(2))
                    .clip(true),
                container(subject).width(Length::FillPortion(3)).clip(true),
                container(preview).width(Length::FillPortion(3)).clip(true),
                metadata,
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into()
        } else {
            column![
                row![
                    container(Ellipsis::new(sender.clone(), 12.))
                        .width(Length::Fill)
                        .clip(true),
                    metadata
                ]
                .spacing(6)
                .align_y(Alignment::Center),
                row![
                    container(subject).width(Length::FillPortion(3)).clip(true),
                    container(preview).width(Length::FillPortion(2)).clip(true)
                ]
                .spacing(8),
            ]
            .spacing(5)
            .into()
        };
        button(
            row![
                leading,
                container(content).width(Length::Fill).clip(true),
                flag
            ]
            .spacing(5)
            .align_y(Alignment::Center),
        )
        .padding(iced::Padding {
            top: 7.,
            right: 18.,
            bottom: 7.,
            left: 8.,
        })
        .height(ROW_HEIGHT - 1.)
        .width(Length::Fill)
        .style(move |theme, status| row_style(theme, status, active, unread))
        .on_press(Message::Select(mail.id.clone()))
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unread_selected_and_hovered_rows_have_distinct_surfaces_in_both_themes() {
        for theme in [Theme::Light, Theme::Dark] {
            let normal = row_style(&theme, button::Status::Active, false, false).background;
            let unread = row_style(&theme, button::Status::Active, false, true).background;
            let selected = row_style(&theme, button::Status::Active, true, true).background;
            let hover = row_style(&theme, button::Status::Hovered, false, true).background;
            assert_ne!(unread, normal);
            assert_ne!(unread, selected);
            assert_ne!(unread, hover);
            assert_ne!(selected, hover);
        }
    }
}
