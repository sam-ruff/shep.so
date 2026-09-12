use super::*;
mod reveal;
use iced::{
    Alignment, Length,
    widget::{button, column, container, image, row, scrollable, space, text},
};

pub(super) struct SidebarItem {
    pub label: String,
    account_email: Option<String>,
    pub icon: &'static str,
    pub action: Message,
    pub active: bool,
    pub depth: usize,
    pub section: bool,
}
impl SidebarItem {
    fn widget_id(&self) -> String {
        format!("sidebar-row:{:?}", self.action)
    }
}
impl App {
    pub(super) fn sidebar_folder_context(&self, action: &Message) -> Option<FolderSelection> {
        // Context actions use the displayed wire path; pending folders remain
        // protected by the ordinary per-account busy guard.
        if let Message::AccountFolder(account, path) | Message::ToggleFolderGroup(account, path) =
            action
        {
            return Some(FolderSelection {
                account: Some(account.clone()),
                folder: path.clone(),
                sent_only: false,
            });
        }
        self.sidebar_folder(action)
    }
    pub(super) fn reveal_sidebar_focus(&self) -> Task<Message> {
        self.sidebar_items()
            .get(self.sidebar_index)
            .map(|item| Task::done(Message::RevealSidebar(item.widget_id(), 0)))
            .unwrap_or_else(Task::none)
    }
    pub(super) fn reveal_sidebar(&self, target: String, attempt: u8) -> Task<Message> {
        if !self.sidebar_focus
            || self.tab != Tab::Mail
            || self.dialog.is_some()
            || self
                .sidebar_items()
                .get(self.sidebar_index)
                .is_none_or(|item| item.widget_id() != target)
        {
            return Task::none();
        }
        reveal::reveal(target.clone()).then(move |found| {
            if found || attempt >= 8 {
                Task::none()
            } else {
                let target = target.clone();
                Task::perform(
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(32)).await;
                        target
                    },
                    move |target| Message::RevealSidebar(target, attempt + 1),
                )
            }
        })
    }
    fn inbox_label(&self, account: Option<&str>, name: &str) -> String {
        let unread = account
            .map(|id| self.page.inbox_unread.get(id).copied().unwrap_or(0))
            .unwrap_or_else(|| self.page.inbox_unread.values().sum());
        if unread == 0 {
            name.into()
        } else {
            format!("{name} ({unread})")
        }
    }
    pub(super) fn sidebar_items(&self) -> Vec<SidebarItem> {
        let mut items = Vec::new();
        let mut email_counts = HashMap::<&str, usize>::new();
        for account in &self.workspace.accounts {
            *email_counts.entry(&account.email).or_default() += 1;
        }
        let duplicate = |account: &Account| {
            email_counts[account.email.as_str()] > 1 && !account.name.trim().is_empty()
        };
        if self.preferences.unified_inbox {
            items.push(SidebarItem {
                account_email: None,
                label: self.inbox_label(None, "Inbox"),
                icon: "inbox",
                action: Message::AccountFolderUnified,
                active: self.query.folder == "INBOX" && self.query.account.is_none(),
                depth: 0,
                section: false,
            });
            if self.inbox_expanded {
                for account in &self.workspace.accounts {
                    items.push(SidebarItem {
                        account_email: duplicate(account).then(|| account.email.clone()),
                        label: self.inbox_label(
                            Some(&account.id),
                            if duplicate(account) {
                                &account.name
                            } else {
                                &account.email
                            },
                        ),
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
                account_email: None,
                label: label.into(),
                icon,
                action: if folder.is_empty() {
                    Message::Starred
                } else if folder == "Sent" {
                    Message::SentFolder
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
        if self.workspace.outgoing_pending > 0 {
            items.push(SidebarItem {
                account_email: None,
                label: format!("Outbox · {}", self.workspace.outgoing_pending),
                icon: "send",
                action: Message::OpenOutbox,
                active: self.dialog == Some(Dialog::Outbox),
                depth: 0,
                section: false,
            });
        }
        if !self.folder_controls.jobs.is_empty() {
            items.push(SidebarItem {
                account_email: None,
                label: "Folder changes".into(),
                icon: "clock",
                action: Message::Folders(folder_controls::Message::History(0)),
                active: self.dialog == Some(Dialog::FolderHistory),
                depth: 0,
                section: false,
            });
        }
        let drafts = self.draft_labels();
        if !drafts.is_empty() {
            items.push(SidebarItem {
                account_email: None,
                label: format!("Drafts ({})", drafts.len()),
                icon: "file",
                action: Message::ToggleDrafts,
                active: false,
                depth: 0,
                section: false,
            });
            if !self.preferences.collapsed_drafts {
                for (id, subject) in drafts {
                    items.push(SidebarItem {
                        account_email: None,
                        label: if subject.is_empty() {
                            "Untitled draft".into()
                        } else {
                            subject.to_owned()
                        },
                        icon: "compose",
                        action: Message::Draft(id.to_owned()),
                        active: self.compose_visible() && self.composer.current.draft.id == id,
                        depth: 1,
                        section: false,
                    });
                }
            }
        }
        for account in &self.workspace.accounts {
            items.push(SidebarItem {
                account_email: duplicate(account).then(|| account.email.clone()),
                label: if duplicate(account) {
                    account.name.clone()
                } else {
                    account.email.clone()
                },
                icon: "mail",
                action: Message::ToggleAccountFolders(account.id.clone()),
                active: false,
                depth: 0,
                section: true,
            });
            if self.preferences.collapsed_accounts.contains(&account.id) {
                continue;
            }
            let fallback =
                crate::folders::Tree::new(&[crate::folders::Mailbox::flat("INBOX".into())]);
            let tree = self
                .folder_tree(&account.id)
                .map(Arc::as_ref)
                .unwrap_or(&fallback);
            for (depth, node) in tree.visible(self.preferences.expanded_folders.get(&account.id)) {
                let folder = &node.mailbox.name;
                if self.preferences.unified_inbox
                    && matches!(folder.as_str(), "INBOX" | "Sent" | "Archive" | "Trash")
                    && node.children.is_empty()
                {
                    continue;
                }
                items.push(SidebarItem {
                    account_email: None,
                    label: if folder == "INBOX" {
                        self.inbox_label(Some(&account.id), "Inbox")
                    } else {
                        node.label.clone()
                    },
                    icon: "folder",
                    action: if node.mailbox.selectable {
                        Message::AccountFolder(account.id.clone(), folder.clone())
                    } else {
                        Message::ToggleFolderGroup(account.id.clone(), node.path.clone())
                    },
                    active: self.query.account.as_deref() == Some(&account.id)
                        && self.query.folder == self.original_folder(&account.id, folder),
                    depth: depth + 1,
                    section: false,
                });
            }
        }
        if !self.workspace.accounts.is_empty() {
            items.push(SidebarItem {
                account_email: None,
                label: "New folder".into(),
                icon: "plus",
                action: Message::FolderCreation(folder_creation::Message::Open),
                active: self.dialog == Some(Dialog::FolderCreation),
                depth: 0,
                section: false,
            });
        }
        if let Some(selected) = &self.query.folders {
            for item in &mut items {
                if let Some(folder) = self.sidebar_folder(&item.action) {
                    item.active = selected.contains(&folder);
                }
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
            .on_press(Message::NewMessage),
            space().height(12)
        ]
        .spacing(3)
        .width(Length::Fill);
        for (index, item) in self.sidebar_items().into_iter().enumerate() {
            let row_id = item.widget_id();
            if item.section {
                content = content.push(space().height(17));
            }
            let focus = self.sidebar_focus && self.sidebar_index == index;
            let label_size = if item.section { 11. } else { 12. };
            let title: Element<'_, Message> = if let Some(email) = &item.account_email {
                // The saved name can contain the only distinction between two
                // endpoints. Wrap it so a suffix such as "(previous setup)"
                // remains visible even at the minimum sidebar width.
                column![
                    text(item.label.clone())
                        .size(label_size)
                        .width(Length::Fill),
                    super::ellipsis::Ellipsis::new(email.clone(), 11.)
                ]
                .spacing(3)
                .width(Length::Fill)
                .into()
            } else {
                super::ellipsis::Ellipsis::new(item.label.clone(), label_size).into()
            };
            let mut label = row![icon(item.icon, 18.), title]
                .spacing(9)
                .align_y(Alignment::Center);
            if self.preferences.unified_inbox && index == 0 {
                label = label.push(
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
            if let Message::ToggleAccountFolders(account) = &item.action {
                label = label.push(icon(
                    if self.preferences.collapsed_accounts.contains(account) {
                        "chevron"
                    } else {
                        "down"
                    },
                    16.,
                ));
            }
            if matches!(item.action, Message::ToggleDrafts) {
                label = label.push(icon(
                    if self.preferences.collapsed_drafts {
                        "chevron"
                    } else {
                        "down"
                    },
                    16.,
                ));
            }
            if let Some((account, path)) = self.sidebar_group(&item.action) {
                label = label.push(
                    button(icon(
                        if self.folder_expanded(&account, &path) {
                            "down"
                        } else {
                            "chevron"
                        },
                        16.,
                    ))
                    .padding(4)
                    .style(ghost)
                    .on_press(Message::ToggleFolderGroup(account, path)),
                );
            }
            let control = button(label.width(Length::Fill))
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
                .on_press_maybe(
                    (!matches!(item.action, Message::Noop))
                        .then_some(Message::SidebarAction(index)),
                );
            let drop_target = self.sidebar_drop_target(&item.action);
            let reveal = self.sidebar_drag_reveal(&item.action);
            let control = if let Message::Draft(id) = item.action {
                super::context_menu::ContextArea::draft(control, id)
            } else if let Some(target) = self.sidebar_folder_context(&item.action) {
                super::context_menu::ContextArea::folder(control, target)
            } else {
                super::context_menu::ContextArea::sidebar(control)
            };
            let control = if let Some(target) = drop_target {
                control.with_drag(super::drag_mail::Region::Target(
                    self.mail_drag.clone(),
                    target,
                    self.drag_rules(),
                ))
            } else {
                control
            };
            let control = if let Some(reveal) = reveal {
                super::context_menu::ContextArea::sidebar(control).with_drag(
                    super::drag_mail::Region::Reveal(self.mail_drag.clone(), reveal),
                )
            } else {
                control
            };
            content = content.push(
                container(control)
                    .id(row_id)
                    .width(Length::Fill)
                    .clip(true)
                    .padding(iced::Padding {
                        left: item.depth.min(5) as f32 * 12.,
                        ..Default::default()
                    }),
            );
        }

        container(
            column![
                scrollable(content)
                    .id(reveal::SCROLLER)
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .direction(scrollable::Direction::Vertical(
                        scrollable::Scrollbar::new()
                            .width(6)
                            .scroller_width(6)
                            .spacing(4)
                    )),
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
        .width(self.sidebar_width())
        .height(Length::Fill)
        .style(|t| container::Style {
            background: Some(colors(t).bg.into()),
            ..Default::default()
        })
        .into()
    }
}

impl App {
    pub(super) fn sidebar_folder(&self, action: &Message) -> Option<FolderSelection> {
        let (account, folder, sent_only) = match action {
            Message::AccountFolder(account, folder) => {
                // Ctrl-selection and active highlights use the same cache
                // identity as ordinary clicks while a folder rename is pending.
                (
                    Some(account.clone()),
                    self.original_folder(account, folder),
                    false,
                )
            }
            Message::AccountFolderUnified => (None, "INBOX".into(), false),
            Message::Folder(folder) => (self.query.account.clone(), folder.clone(), false),
            Message::SentFolder => (self.query.account.clone(), "Sent".into(), true),
            _ => return None,
        };
        Some(FolderSelection {
            account,
            folder,
            sent_only,
        })
    }
    pub(super) fn toggle_folder_selection(&mut self, folder: FolderSelection) {
        self.close_composer();
        let folders = self.query.folders.get_or_insert_with(|| {
            if self.query.folder.is_empty() {
                Vec::new()
            } else {
                vec![FolderSelection {
                    account: self.query.account.clone(),
                    folder: self.query.folder.clone(),
                    sent_only: self.query.sent_only,
                }]
            }
        });
        if folders.contains(&folder) {
            folders.retain(|f| f != &folder);
        } else {
            folders.push(folder);
        }
        self.query.account = None;
        self.query.folder.clear();
        self.query.sent_only = false;
        self.query.offset = 0;
        self.tab = Tab::Mail;
        self.selected = None;
        self.detail = None;
        self.request_page();
    }
}

impl App {
    pub(super) fn folder_expanded(&self, account: &str, path: &str) -> bool {
        self.preferences
            .expanded_folders
            .get(account)
            .is_some_and(|folders| folders.contains(path))
    }
    pub(super) fn set_folder_expanded(&mut self, account: &str, path: &str, expanded: bool) {
        if self
            .folder_tree(account)
            .and_then(|tree| tree.node(path))
            .is_none_or(|node| node.children.is_empty())
        {
            return;
        }
        let folders = self
            .preferences
            .expanded_folders
            .entry(account.into())
            .or_default();
        let changed = if expanded {
            folders.insert(path.into())
        } else {
            folders.remove(path)
        };
        if changed {
            self.save_preferences();
        }
    }
    pub(super) fn sidebar_group(&self, action: &Message) -> Option<(String, String)> {
        let (account, path) = match action {
            Message::AccountFolder(account, path) | Message::ToggleFolderGroup(account, path) => {
                (account, path)
            }
            _ => return None,
        };
        let node = self.folder_tree(account)?.node(path)?;
        (!node.children.is_empty()).then(|| (account.clone(), node.path.clone()))
    }
    pub(super) fn sidebar_tree_key(&mut self, expand: bool) -> Task<Message> {
        let items = self.sidebar_items();
        let Some(item) = items.get(self.sidebar_index) else {
            return Task::none();
        };
        if let Message::ToggleAccountFolders(account) = &item.action {
            let collapsed = self.preferences.collapsed_accounts.contains(account);
            if expand == collapsed {
                return self.handle(item.action.clone());
            }
            if expand
                && items
                    .get(self.sidebar_index + 1)
                    .is_some_and(|next| next.depth > 0)
            {
                self.sidebar_index += 1;
            }
            return Task::none();
        }
        if matches!(item.action, Message::AccountFolderUnified) {
            self.inbox_expanded = expand;
            return Task::none();
        }
        if let Some((account, path)) = self.sidebar_group(&item.action) {
            let expanded = self.folder_expanded(&account, &path);
            if expand != expanded {
                self.set_folder_expanded(&account, &path, expand);
                return Task::none();
            }
            if expand
                && items
                    .get(self.sidebar_index + 1)
                    .is_some_and(|next| next.depth > item.depth)
            {
                self.sidebar_index += 1;
                return Task::none();
            }
        }
        if !expand && item.depth > 0 {
            // The preceding shallower visible row is the parent, including its
            // account heading. Moving focus must not toggle that parent closed.
            if let Some(index) = (0..self.sidebar_index)
                .rev()
                .find(|&i| items[i].depth < item.depth)
            {
                self.sidebar_index = index;
            }
        }
        Task::none()
    }
}

impl App {
    pub(super) fn move_folder_label<'a>(&'a self, folder: &'a str) -> std::borrow::Cow<'a, str> {
        let account = if !self.field("move_account").is_empty() {
            Some(self.field("move_account"))
        } else if self.mail_selection.mode {
            // The reader may belong to another account after a move. Match
            // the selected membership used by move_folders, including encoding.
            self.mail_selection
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.accounts.keys().next())
                .map(String::as_str)
        } else {
            self.action_mail().map(|mail| mail.account_id.as_str())
        };
        self.workspace.folder_label(account, folder)
    }
    pub(super) fn ranked_move_folders(&self) -> Vec<String> {
        crate::fuzzy::ranked_labels(
            self.field("folder_search"),
            self.move_folders().into_iter().map(|folder| {
                let label = self.move_folder_label(&folder).into_owned();
                (folder, label)
            }),
        )
    }
}
