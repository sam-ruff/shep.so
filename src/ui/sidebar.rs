use super::*;
use iced::{
    Alignment, Length,
    widget::{button, column, container, image, row, scrollable, space, text},
};

pub(super) struct SidebarItem {
    pub label: String,
    pub icon: &'static str,
    pub action: Message,
    pub active: bool,
    pub depth: u16,
    pub section: bool,
}
impl App {
    pub(super) fn sidebar_items(&self) -> Vec<SidebarItem> {
        let mut items = Vec::new();
        if self.preferences.unified_inbox {
            items.push(SidebarItem {
                label: "Inbox".into(),
                icon: "inbox",
                action: Message::AccountFolderUnified,
                active: self.query.folder == "INBOX" && self.query.account.is_none(),
                depth: 0,
                section: false,
            });
            if self.inbox_expanded {
                for account in &self.workspace.accounts {
                    items.push(SidebarItem {
                        label: account.email.clone(),
                        icon: "mail",
                        action: Message::AccountFolder(account.id.clone(), "INBOX".into()),
                        active: self.query.folder == "INBOX"
                            && self.query.account.as_deref() == Some(&account.id),
                        depth: 1,
                        section: false,
                    });
                }
            }
        }
        for (label, icon, folder) in [
            ("Flagged", "flag", ""),
            ("Sent", "send", "Sent"),
            ("Archive", "archive", "Archive"),
            ("Trash", "trash", "Trash"),
        ] {
            items.push(SidebarItem {
                label: label.into(),
                icon,
                action: if folder.is_empty() {
                    Message::Starred
                } else {
                    Message::Folder(folder.into())
                },
                active: if folder.is_empty() {
                    self.query.starred_only
                } else {
                    self.query.folder == folder
                },
                depth: 0,
                section: false,
            });
        }
        for draft in &self.workspace.drafts {
            items.push(SidebarItem {
                label: if draft.subject.is_empty() {
                    "Untitled draft".into()
                } else {
                    draft.subject.clone()
                },
                icon: "file",
                action: Message::Draft(draft.id.clone()),
                active: false,
                depth: 0,
                section: false,
            });
        }
        for account in &self.workspace.accounts {
            items.push(SidebarItem {
                label: account.email.clone(),
                icon: "mail",
                action: Message::AccountFolder(account.id.clone(), "INBOX".into()),
                active: !self.preferences.unified_inbox
                    && self.query.folder == "INBOX"
                    && self.query.account.as_deref() == Some(&account.id),
                depth: 0,
                section: true,
            });
            let folders = self.workspace.account_folders.get(&account.id);
            let default = vec!["INBOX".into()];
            for folder in folders.unwrap_or(&default) {
                if self.preferences.unified_inbox
                    && matches!(folder.as_str(), "INBOX" | "Sent" | "Archive" | "Trash")
                {
                    continue;
                }
                items.push(SidebarItem {
                    label: if folder == "INBOX" {
                        "Inbox".into()
                    } else {
                        folder.clone()
                    },
                    icon: "folder",
                    action: Message::AccountFolder(account.id.clone(), folder.clone()),
                    active: self.query.account.as_deref() == Some(&account.id)
                        && self.query.folder == *folder,
                    depth: 1,
                    section: false,
                });
            }
        }
        items
    }
    pub(super) fn sidebar(&self) -> Element<'_, Message> {
        let brand = row![
            image(if self.dark() {
                self.dark_logo.clone()
            } else {
                self.light_logo.clone()
            })
            .width(38)
            .height(38),
            text("shep").size(29).font(BOLD)
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        let mut content = column![
            container(brand).padding([6, 7]),
            space().height(14),
            nav(
                "mail",
                "Mail",
                self.tab == Tab::Mail,
                Message::Tab(Tab::Mail),
                None
            ),
            nav(
                "calendar",
                "Calendar",
                self.tab == Tab::Calendar,
                Message::Tab(Tab::Calendar),
                None
            ),
            space().height(14),
            button(
                row![
                    icon_bright("compose", 20.),
                    text("New message").size(13).line_height(1.)
                ]
                .spacing(10)
                .align_y(Alignment::Center)
            )
            .width(Length::Fill)
            .padding([12, 14])
            .style(primary)
            .on_press(Message::Open(Dialog::Compose)),
            space().height(12)
        ]
        .spacing(3);
        for (index, item) in self.sidebar_items().into_iter().enumerate() {
            if item.section {
                content = content.push(space().height(17));
            }
            let focus = self.sidebar_focus && self.sidebar_index == index;
            let mut label = row![
                icon(item.icon, 18.),
                text(super::views::truncate(
                    &item.label,
                    if item.depth > 0 { 23 } else { 25 }
                ))
                .size(if item.section { 11 } else { 12 })
                .line_height(1.)
            ]
            .spacing(9)
            .align_y(Alignment::Center);
            if self.preferences.unified_inbox && index == 0 {
                label = label.push(space().width(Length::Fill)).push(
                    button(icon(
                        if self.inbox_expanded {
                            "down"
                        } else {
                            "chevron"
                        },
                        16.,
                    ))
                    .padding(4)
                    .style(ghost)
                    .on_press(Message::ToggleInboxExpanded),
                );
            }
            content = content.push(
                container(
                    button(label)
                        .width(Length::Fill)
                        .padding([9, 10])
                        .style(move |t, status| {
                            let mut style = if item.active {
                                selected(t, status)
                            } else {
                                ghost(t, status)
                            };
                            if focus {
                                style.border = iced::Border {
                                    color: colors(t).accent,
                                    width: 1.,
                                    radius: 7.into(),
                                };
                            }
                            style
                        })
                        .on_press(Message::SidebarAction(index)),
                )
                .padding(iced::Padding {
                    left: f32::from(item.depth) * 12.,
                    ..Default::default()
                }),
            );
        }
        content = content
            .push(space().height(12))
            .push(action("Add account", Message::Open(Dialog::Account)));
        container(
            column![
                scrollable(content).height(Length::Fill),
                line(),
                button(
                    row![
                        icon("settings", 20.),
                        text("Preferences").size(12).line_height(1.)
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                )
                .padding([12, 10])
                .width(Length::Fill)
                .style(ghost)
                .on_press(Message::Tab(Tab::Preferences))
            ]
            .spacing(12),
        )
        .padding([19, 14])
        .width(if self.size.width < 1100. { 200. } else { 222. })
        .height(Length::Fill)
        .style(|t| container::Style {
            background: Some(colors(t).bg.into()),
            ..Default::default()
        })
        .into()
    }
}
