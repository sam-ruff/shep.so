use super::*;
use iced::{
    Alignment, Length,
    widget::{checkbox, column, container, pick_list, row, space, text},
};

impl App {
    pub(super) fn reading_settings(&self) -> Element<'_, Message> {
        column![
            text("Reading and layout").size(17).font(BOLD),
            row![
                text("Message text size").size(13),
                space().width(Length::Fill),
                pick_list(
                    [11u16, 12, 13, 14, 15, 16, 18, 20, 22, 24, 26],
                    Some(self.preferences.reader_font_size),
                    Message::PrefReaderSize
                )
                .style(select_input)
                .menu_style(select_menu)
                .text_size(12)
                .padding(10)
            ]
            .align_y(Alignment::Center),
            row![
                text("Interface size (%)").size(13),
                space().width(Length::Fill),
                pick_list(
                    [80u16, 90, 100, 110, 120, 130, 140],
                    Some(self.preferences.interface_scale),
                    Message::PrefScale
                )
                .style(select_input)
                .menu_style(select_menu)
                .text_size(12)
                .padding(10)
            ]
            .align_y(Alignment::Center),
            column![
                text("Reply history").size(13),
                pick_list(
                    [
                        ReplyDisplay::Collapsed,
                        ReplyDisplay::Expanded,
                        ReplyDisplay::LatestOnly
                    ],
                    Some(self.preferences.reply_display),
                    Message::PrefReplies
                )
                .style(select_input)
                .menu_style(select_menu)
                .text_size(12)
                .padding(10)
                .width(Length::Fill)
            ]
            .spacing(8),
            checkbox(self.preferences.unified_inbox)
                .label("Show a unified inbox")
                .on_toggle(Message::PrefUnified),
            checkbox(self.preferences.cross_account_moves)
                .label("Allow moving mail between accounts")
                .on_toggle(Message::PrefCrossAccount),
            checkbox(self.preferences.group_conversations)
                .label("Group related messages in the reader")
                .on_toggle(Message::PrefConversations),
        ]
        .spacing(16)
        .into()
    }
    pub(super) fn privacy_settings(&self) -> Element<'_, Message> {
        container(column![
            text("Remote images").size(17).font(BOLD),
            pick_list([ImagePolicy::BlockAll, ImagePolicy::Contacts, ImagePolicy::AllowAll], Some(self.preferences.image_policy), Message::PrefImages).style(select_input).menu_style(select_menu).text_size(12).padding(11).width(Length::Fill),
            muted("Loading an external image lets its server see the request. Sender identity in email is not verified by Shep.").size(12),
            action("Manage contacts", Message::SettingsTab(SettingsTab::Contacts)),
            line(),
            text(format!("Image exceptions: {} messages, {} senders, {} domains", self.preferences.image_messages.len(), self.preferences.image_senders.len(), self.preferences.image_domains.len())).size(12),
            action("Clear image exceptions", Message::ClearImageTrust),
        ].spacing(16)).padding(23).style(card).into()
    }
    pub(super) fn contacts_settings(&self) -> Element<'_, Message> {
        container(column![
            text("Contacts").size(17).font(BOLD),
            muted("Add email addresses, separated by commas. These addresses are used by the Contacts image policy."),
            input("alex@example.com, maya@example.com", self.field("contacts"), |value| Message::Field("contacts", value)).id("contacts"),
            row![action("Save contacts", Message::SavePreferences), action("Image preferences", Message::SettingsTab(SettingsTab::Privacy))].spacing(10).wrap(),
            text(format!("{} saved contacts", self.preferences.contacts.len())).size(12),
        ].spacing(16)).padding(23).style(card).into()
    }
    pub(super) fn image_bar(&self) -> Element<'_, Message> {
        let width = self.size.width / (self.preferences.interface_scale as f32 / 100.);
        let compact = if self.full_reader {
            width < 650.
        } else {
            (width - self.sidebar_width() - 57.) * (1. - self.preferences.reader_split) < 480.
        };
        if compact {
            return container(
                row![
                    icon("image", 16.),
                    text("Images blocked").size(11),
                    space().width(Length::Fill),
                    pick_list(
                        [ImageScope::Email, ImageScope::Sender, ImageScope::Domain],
                        None::<ImageScope>,
                        |scope| Message::AllowImages(scope as u8)
                    )
                    .placeholder("Show images")
                    .style(select_input)
                    .menu_style(select_menu)
                    .text_size(11)
                    .padding(7)
                ]
                .spacing(6)
                .align_y(Alignment::Center),
            )
            .padding(8)
            .width(Length::Fill)
            .style(subtle)
            .into();
        }
        container(
            column![
                row![
                    icon("image", 18.),
                    text("External images are blocked").size(12)
                ]
                .spacing(8)
                .align_y(Alignment::Center),
                row![
                    action("This email", Message::AllowImages(0)),
                    action("This sender", Message::AllowImages(1)),
                    action("This domain", Message::AllowImages(2))
                ]
                .spacing(6)
                .wrap(),
            ]
            .spacing(8),
        )
        .padding(12)
        .style(subtle)
        .into()
    }
    pub(super) fn sender_dialog(&self) -> Element<'_, Message> {
        let Some(detail) = &self.detail else {
            return space().into();
        };
        let address = crate::remote_images::sender_address(&detail.summary.sender)
            .unwrap_or_else(|| detail.summary.sender.clone());
        let domain =
            crate::remote_images::sender_domain(&detail.summary.sender).unwrap_or_default();
        column![
            text(&detail.summary.sender).size(15).font(BOLD),
            text("Email address").size(12),
            text(address.clone()).size(14),
            action("Copy email address", Message::CopyAddress(address)),
            text("Domain").size(12),
            text(domain.clone()).size(14),
            action("Copy domain", Message::CopyAddress(domain)),
        ]
        .spacing(12)
        .into()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageScope {
    Email,
    Sender,
    Domain,
}
impl std::fmt::Display for ImageScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Email => "This email",
            Self::Sender => "This sender",
            Self::Domain => "This domain",
        })
    }
}
