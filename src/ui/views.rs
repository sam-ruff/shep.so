use super::*;
use iced::{
    Alignment, Border, Length,
    widget::{
        button, checkbox, column, container, image, mouse_area, opaque, pick_list, row, scrollable,
        space, stack, text,
    },
};

impl App {
    pub(super) fn layout(&self) -> Element<'_, Message> {
        let content = if self.full_reader && self.tab == Tab::Mail {
            container(
                column![
                    row![
                        action("Close reader", Message::ClosePreview),
                        space().width(Length::Fill),
                        muted(self.preferences.shortcuts.key(Action::ClosePreview))
                    ]
                    .align_y(Alignment::Center),
                    self.reader()
                ]
                .spacing(10),
            )
            .padding(16)
            .height(Length::Fill)
            .into()
        } else {
            match self.tab {
                Tab::Mail => self.mail_view(),
                Tab::Calendar => self.calendar_view(),
                Tab::Preferences => self.preferences_view(),
            }
        };
        let mut main = column![content].height(Length::Fill);
        if self.dialog.is_none()
            && let Some((message, error, _)) = &self.notice
        {
            let error = *error;
            main = main.push(
                container(
                    row![
                        text(message).size(12),
                        space().width(Length::Fill),
                        self.icon_action("close", "Dismiss", Message::Dismiss)
                    ]
                    .align_y(Alignment::Center)
                    .spacing(10),
                )
                .padding([4, 14])
                .style(move |t| {
                    let p = colors(t);
                    container::Style {
                        background: Some(
                            if error {
                                if p.bg.r < 0.3 {
                                    hex(0x382325)
                                } else {
                                    hex(0xfff0ef)
                                }
                            } else {
                                p.tint
                            }
                            .into(),
                        ),
                        text_color: Some(p.text),
                        ..Default::default()
                    }
                }),
            );
        }
        let base: Element<_> = container(if self.full_reader && self.tab == Tab::Mail {
            Element::from(main.width(Length::Fill))
        } else {
            Element::from(
                row![
                    self.sidebar(),
                    super::layout::SidebarDivider(self.sidebar_width()),
                    main.width(Length::Fill)
                ]
                .height(Length::Fill),
            )
        })
        .style(|t| container::Style {
            background: Some(colors(t).bg.into()),
            text_color: Some(colors(t).text),
            ..Default::default()
        })
        .into();
        if let Some(dialog) = self.dialog {
            let overlay = container(
                mouse_area(
                    container(space())
                        .width(Length::Fill)
                        .height(Length::Fill)
                        .style(|_| container::Style {
                            background: Some(iced::Color::from_rgba(0.08, 0.08, 0.12, 0.45).into()),
                            ..Default::default()
                        }),
                )
                .on_press(Message::Close),
            )
            .width(Length::Fill)
            .height(Length::Fill);
            let modal = container(opaque(self.dialog_view(dialog)))
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .padding(24);
            stack![base, opaque(overlay), modal].into()
        } else {
            // Keep the base widget tree alive as dialogs open and close. Replacing the
            // root Stack with a Container drops native input focus and shaped text.
            let mut layers = stack![base];
            if self.context_menu.is_some() {
                layers = layers.push(opaque(
                    mouse_area(container(space()).width(Length::Fill).height(Length::Fill))
                        .on_press(Message::DismissContext)
                        .on_right_press(Message::DismissContext),
                ));
                layers = layers.push(self.mail_context_view());
            }
            if self.saved_toast.is_some() {
                layers = layers.push(
                    container(opaque(
                        container(
                            row![
                                icon("check", 18.),
                                text("Changes saved").size(13),
                                self.icon_action("close", "Dismiss", Message::DismissToast)
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                        )
                        .padding([8, 14])
                        .style(card),
                    ))
                    .align_right(Length::Fill)
                    .align_bottom(Length::Fill)
                    .padding(20),
                );
            }
            layers.into()
        }
    }
    fn page_header<'a>(
        &self,
        title: impl Into<std::borrow::Cow<'a, str>>,
        subtitle: impl Into<std::borrow::Cow<'a, str>>,
        right: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let subtitle = subtitle.into();
        let mut title = column![heading(title)].spacing(7);
        if !subtitle.is_empty() {
            title = title.push(muted(subtitle));
        }
        row![title, space().width(Length::Fill), right]
            .align_y(Alignment::Center)
            .spacing(20)
            .into()
    }
    fn mail_view(&self) -> Element<'_, Message> {
        let title = if let Some(folders) = &self.query.folders {
            match folders.as_slice() {
                [] => "No folders selected".to_string(),
                [folder] => {
                    if folder.folder == "INBOX" {
                        "Inbox".into()
                    } else {
                        folder.folder.clone()
                    }
                }
                folders => format!("{} folders", folders.len()),
            }
        } else if self.query.starred_only {
            "Flagged".into()
        } else if self.query.folder == "INBOX" {
            "Inbox".into()
        } else {
            self.query.folder.clone()
        };
        let header = row![
            heading(title),
            muted(format!(
                "{} messages · {} unread",
                self.page.total, self.page.unread
            ))
            .size(12),
            space().width(Length::Fill),
            if self.demo {
                badge("TEST")
            } else {
                space().into()
            },
            self.toggle_icon_action(
                "sync",
                self.shortcut_hint(
                    if self.busy.contains("sync") {
                        "Syncing mail…"
                    } else {
                        "Sync mail"
                    },
                    Action::Sync
                ),
                self.busy.contains("sync"),
                Message::Sync
            )
        ]
        .spacing(14)
        .align_y(Alignment::Center);
        let content = if self.workspace.accounts.is_empty() && self.tx.is_some() {
            self.welcome()
        } else {
            let panes = iced::widget::pane_grid(&self.panes, |_, pane, _| {
                iced::widget::pane_grid::Content::new(match pane {
                    MailPane::Inbox => self.message_list(),
                    MailPane::Reader => self.reader(),
                })
            })
            .spacing(1)
            .min_size(300)
            .on_resize(8, Message::PaneResize)
            .height(Length::Fill)
            .style(|theme| {
                let p = colors(theme);
                iced::widget::pane_grid::Style {
                    hovered_split: iced::widget::pane_grid::Line {
                        color: p.accent,
                        width: 3.,
                    },
                    picked_split: iced::widget::pane_grid::Line {
                        color: p.accent,
                        width: 3.,
                    },
                    ..iced::widget::pane_grid::default(theme)
                }
            });
            container(panes).style(card).height(Length::Fill).into()
        };
        container(column![header, content].spacing(14))
            .padding([16, 20])
            .height(Length::Fill)
            .into()
    }
    fn welcome(&self) -> Element<'_, Message> {
        container(
            column![
                image(if self.dark() {
                    self.dark_logo.clone()
                } else {
                    self.light_logo.clone()
                })
                .width(88)
                .height(88),
                space().height(12),
                text("Add your first account").font(BOLD).size(27),
                muted("Bring your accounts and calendars together in one considered space.")
                    .size(14),
                space().height(8),
                button(text("Add your first account").size(13))
                    .padding([13, 22])
                    .style(primary)
                    .on_press(Message::Open(Dialog::Account)),
            ]
            .spacing(18)
            .align_x(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(card)
        .into()
    }
    fn message_list(&self) -> Element<'_, Message> {
        let filters = row![
            pick_list(MailFilter::ALL, Some(self.mail_filter()), Message::Filter)
                .text_size(11)
                .style(select_input)
                .menu_style(select_menu)
                .padding([9, 8])
                .width(Length::Fill),
            pick_list(MailSort::ALL, Some(self.query.sort), Message::Sort)
                .text_size(11)
                .style(select_input)
                .menu_style(select_menu)
                .padding([9, 8])
                .width(Length::Fill),
        ]
        .spacing(6)
        .align_y(Alignment::Center);
        let search =
            input("Search conversations…", &self.query.search, Message::Query).id("search");
        // Fixed-height rows let us shape only visible text plus a small overscan.
        let row_height = 104.;
        let first = ((self.inbox_scroll / row_height) as usize)
            .saturating_sub(2)
            .min(self.page.rows.len());
        let count = ((self.size.height / (self.preferences.interface_scale as f32 / 100.) - 220.)
            .max(104.)
            / row_height)
            .ceil() as usize
            + 5;
        let end = (first + count).min(self.page.rows.len());
        let mut messages = column![space().height(first as f32 * row_height)].spacing(0);
        for (index, mail) in self.page.rows.iter().enumerate().take(end).skip(first) {
            let active = self.selected.as_deref() == Some(&mail.id);
            let unread = mail.unread;
            let sender = sender_name(&mail.sender);
            let date = chrono::DateTime::from_timestamp(mail.timestamp, 0)
                .unwrap_or_default()
                .with_timezone(&chrono::Local);
            let date = if date.date_naive() == chrono::Local::now().date_naive() {
                date.format("%H:%M").to_string()
            } else {
                date.format("%d %b").to_string()
            };
            let mut top = row![
                avatar(&sender, index, 30.),
                text(truncate(&sender, 23)).size(12).font(if unread {
                    BOLD
                } else {
                    iced::Font::DEFAULT
                }),
                space().width(Length::Fill),
                muted(date).size(10),
                button(flag_icon(mail.starred, 18.))
                    .padding(6)
                    .style(if mail.starred { flagged } else { ghost })
                    .on_press(Message::FlagRow(mail.id.clone()))
            ]
            .spacing(9)
            .align_y(Alignment::Center);
            if unread {
                top = top.push(
                    container(space())
                        .width(5)
                        .height(5)
                        .style(|t| container::Style {
                            background: Some(colors(t).accent.into()),
                            border: Border {
                                radius: 3.into(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                );
            }
            let bottom = row![
                muted(truncate(&mail.preview, 84))
                    .wrapping(text::Wrapping::None)
                    .height(17)
                    .size(12)
                    .width(Length::Fill),
                if mail.attachment_count > 0 {
                    icon("clip", 16.)
                } else {
                    space().into()
                }
            ]
            .spacing(8);
            let entry = button(
                column![
                    top,
                    text(truncate(&mail.subject, 57))
                        .wrapping(text::Wrapping::None)
                        .height(18)
                        .size(12)
                        .font(if unread { BOLD } else { iced::Font::DEFAULT }),
                    bottom
                ]
                .spacing(5),
            )
            .padding([10, 16])
            .height(row_height - 1.)
            .width(Length::Fill)
            .style(move |t, status| {
                let p = colors(t);
                button::Style {
                    background: Some(
                        if active {
                            p.tint
                        } else if matches!(status, button::Status::Hovered) {
                            p.subtle
                        } else {
                            p.surface
                        }
                        .into(),
                    ),
                    text_color: p.text,
                    ..Default::default()
                }
            })
            .on_press(Message::Select(mail.id.clone()));
            messages = messages
                .push(super::context_menu::ContextArea::new(
                    mouse_area(entry)
                        .on_enter(Message::Hover(mail.id.clone()))
                        .on_double_click(Message::OpenMessage(mail.id.clone())),
                    mail.id.clone(),
                ))
                .push(line());
        }
        messages = messages.push(space().height((self.page.rows.len() - end) as f32 * row_height));
        if self.page.rows.is_empty() {
            messages = messages.push(
                container(
                    column![
                        icon("search", 26.),
                        text("Nothing here just yet").size(14).font(BOLD),
                        muted(if self.query.search.is_empty() {
                            "New conversations will appear here."
                        } else {
                            "Try another name, subject, or phrase."
                        })
                    ]
                    .spacing(14)
                    .align_x(Alignment::Center),
                )
                .padding([50, 15])
                .width(Length::Fill),
            );
        }
        let pages = row![
            button(icon("left", 15.))
                .padding(10)
                .style(ghost)
                .on_press_maybe((self.query.offset > 0).then_some(Message::NextPage(false))),
            space().width(Length::Fill),
            muted(format!(
                "{}–{} of {}",
                if self.page.total == 0 {
                    0
                } else {
                    self.query.offset + 1
                },
                (self.query.offset + self.page.rows.len()).min(self.page.total),
                self.page.total
            ))
            .size(10),
            space().width(Length::Fill),
            button(icon("chevron", 15.))
                .padding(10)
                .style(ghost)
                .on_press_maybe(
                    (self.query.offset + PAGE_SIZE < self.page.total)
                        .then_some(Message::NextPage(true))
                )
        ]
        .align_y(Alignment::Center);
        column![
            container(column![filters, search].spacing(15)).padding(18),
            line(),
            iced::widget::keyed_column([(
                self.list_revision,
                scrollable(messages)
                    .id("inbox-list")
                    .height(Length::Fill)
                    .on_scroll(|v| Message::InboxScroll(v.absolute_offset().y))
                    .into()
            )])
            .height(Length::Fill),
            line(),
            container(pages).padding([3, 12])
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
    fn reader(&self) -> Element<'_, Message> {
        if self.conversation_visible() {
            return self.conversation_reader();
        }
        let Some(detail) = &self.detail else {
            return container(
                column![
                    icon("mail", 32.),
                    text(if self.selected.is_some() {
                        "Opening conversation…"
                    } else {
                        "Select a message"
                    })
                    .size(18)
                    .font(BOLD),
                    muted("Choose a message to read it here.")
                ]
                .spacing(18)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        };
        let footer = column![self.reader_actions(detail), self.reader_navigation()].spacing(4);
        column![
            container(self.reader_toolbar(detail)).padding([10, 18]),
            line(),
            scrollable(
                container(self.reader_body(detail, true)).padding(if self.size.width < 1200. {
                    22.
                } else {
                    35.
                })
            )
            .height(Length::Fill),
            container(footer).padding([10, 20])
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
    pub(super) fn reader_toolbar<'a>(&'a self, detail: &'a MailDetail) -> Element<'a, Message> {
        let toolbar = row![
            self.icon_action(
                "archive",
                self.shortcut_hint("Archive", Action::Archive),
                Message::Move("Archive".into())
            ),
            self.icon_action(
                "trash",
                self.shortcut_hint("Move to Trash", Action::Delete),
                Message::Move("Trash".into())
            ),
            self.icon_action(
                "mail",
                if detail.summary.unread {
                    "Mark as read"
                } else {
                    "Mark as unread"
                },
                Message::ToggleRead
            ),
            self.toggle_icon_action(
                "flag",
                self.shortcut_hint(
                    if detail.summary.starred {
                        "Remove flag"
                    } else {
                        "Flag message"
                    },
                    Action::Star
                ),
                detail.summary.starred,
                Message::ToggleStar
            ),
            space().width(Length::Fill),
            if (self.size.width / (self.preferences.interface_scale as f32 / 100.)
                - self.sidebar_width()
                - 57.)
                * (1. - self.preferences.reader_split)
                < 440.
            {
                self.icon_action(
                    "move",
                    self.shortcut_hint("Move to folder", Action::Move),
                    Message::Open(Dialog::Move),
                )
            } else {
                button(
                    row![
                        icon("move", 20.),
                        text("Move").size(12).line_height(1.),
                        muted(self.preferences.shortcuts.key(Action::Move))
                            .size(11)
                            .line_height(1.)
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                )
                .padding([8, 11])
                .style(outline)
                .on_press(Message::Open(Dialog::Move))
                .into()
            },
            self.icon_action(
                "download",
                "Export original email",
                Message::Open(Dialog::Export)
            )
        ]
        .spacing(4)
        .align_y(Alignment::Center);
        toolbar.into()
    }
    pub(super) fn reader_body<'a>(
        &'a self,
        detail: &'a MailDetail,
        subject: bool,
    ) -> iced::widget::Column<'a, Message> {
        let sender = sender_name(&detail.summary.sender);
        let date = chrono::DateTime::from_timestamp(detail.summary.timestamp, 0)
            .unwrap_or_default()
            .with_timezone(&chrono::Local);
        let sender_header = row![
            avatar(&sender, 0, 41.),
            column![
                text(sender.clone()).font(BOLD).size(13),
                muted(&detail.summary.sender).size(10),
                muted(format!("To: {}", detail.summary.recipient)).size(10)
            ]
            .spacing(5)
            .width(Length::Fill),
            column![
                muted(date.format("%d %b %Y").to_string()).size(10),
                muted(date.format("%H:%M").to_string()).size(10)
            ]
            .spacing(5)
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let body = &detail.latest_body;
        let mut reading = column![
            button(sender_header)
                .padding(0)
                .style(ghost)
                .on_press(Message::Open(Dialog::Sender)),
            line()
        ]
        .spacing(18);
        if subject {
            reading =
                column![text(&detail.summary.subject).size(25).font(BOLD), reading].spacing(18);
        }
        if !detail.remote_images.is_empty()
            && !crate::remote_images::allowed(&self.preferences, &detail.summary)
        {
            reading = reading.push(self.image_bar());
        }
        reading = reading.push(self.selectable_body(detail, 0, body));
        if self.preferences.reply_display != ReplyDisplay::LatestOnly {
            for (index, reply) in detail.replies.iter().enumerate() {
                let expanded = self.expanded_replies.contains(&index)
                    != (self.preferences.reply_display == ReplyDisplay::Expanded);
                let mut section = column![
                    button(
                        row![
                            icon(if expanded { "down" } else { "chevron" }, 18.),
                            text(&reply.heading).size(12)
                        ]
                        .spacing(8)
                        .align_y(Alignment::Center)
                    )
                    .padding(8)
                    .style(ghost)
                    .on_press(Message::ToggleReply(index))
                ]
                .spacing(8);
                if expanded {
                    section = section.push(self.selectable_body(detail, index + 1, &reply.body));
                }
                reading = reading.push(
                    container(section)
                        .padding(12)
                        .width(Length::Fill)
                        .style(subtle),
                );
            }
        }
        if crate::remote_images::allowed(&self.preferences, &detail.summary) {
            for remote in &detail.remote_images {
                if let Some((_, handle)) = self
                    .remote_handles
                    .iter()
                    .find(|(url, _)| url == &remote.url)
                {
                    reading = reading.push(
                        image(handle.clone())
                            .width(Length::Fill)
                            .content_fit(iced::ContentFit::Contain),
                    );
                } else {
                    reading = reading.push(
                        muted(
                            self.image_errors
                                .get(&remote.url)
                                .map(String::as_str)
                                .unwrap_or("Loading image…"),
                        )
                        .size(11),
                    );
                }
            }
        }
        if detail.body_truncated {
            reading=reading.push(muted("Showing the first 32,000 characters. Export the original email to read the full message."));
        }
        reading
    }
    pub(super) fn reader_actions<'a>(&'a self, detail: &'a MailDetail) -> Element<'a, Message> {
        let mut footer = row![
            button(
                row![
                    icon_bright("reply", 20.),
                    text("Reply").size(12).line_height(1.)
                ]
                .spacing(8)
                .align_y(Alignment::Center)
            )
            .padding([10, 14])
            .style(primary)
            .on_press(Message::Reply),
            action("Reply all", Message::ReplyAll)
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        for (index, attachment) in detail.attachments.iter().enumerate() {
            footer = footer.push(
                button(
                    row![
                        icon("clip", 18.),
                        text(truncate(&attachment.name, 24))
                            .size(12)
                            .line_height(1.)
                    ]
                    .spacing(7)
                    .align_y(Alignment::Center),
                )
                .padding([10, 12])
                .style(outline)
                .on_press(Message::ExportAttachment(index)),
            );
        }
        footer.wrap().into()
    }
    pub(super) fn reader_navigation(&self) -> Element<'_, Message> {
        row![
            space().width(Length::Fill),
            self.icon_action(
                "left",
                self.shortcut_hint("Previous inbox message", Action::Previous),
                Message::PreviousMessage(true)
            ),
            self.icon_action(
                "chevron",
                self.shortcut_hint("Next inbox message", Action::Next),
                Message::PreviousMessage(false)
            )
        ]
        .spacing(4)
        .into()
    }
    fn calendar_view(&self) -> Element<'_, Message> {
        let header = self.page_header(
            "Calendar",
            "",
            row![
                self.icon_action("sync", "Refresh calendar", Message::SyncCalendar),
                button(text("New event").size(13))
                    .padding([10, 16])
                    .style(primary)
                    .on_press(Message::Open(Dialog::Event))
            ]
            .spacing(10)
            .into(),
        );
        let month_header = row![
            text(self.month.format("%B %Y").to_string())
                .size(20)
                .font(BOLD),
            space().width(Length::Fill),
            action("Today", Message::Today),
            self.icon_action("left", "Previous month", Message::Month(-1)),
            self.icon_action("chevron", "Next month", Message::Month(1))
        ]
        .spacing(9)
        .align_y(Alignment::Center);
        let mut weekdays = row![].spacing(1);
        for d in ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"] {
            weekdays = weekdays.push(
                container(muted(d).size(10).font(BOLD))
                    .center_x(Length::Fill)
                    .padding(10),
            );
        }
        let start =
            self.month - chrono::Duration::days(self.month.weekday().num_days_from_monday() as i64);
        let today = chrono::Local::now().date_naive();
        let mut weeks = column![weekdays].spacing(1);
        for week in 0..6 {
            let mut days = row![].spacing(1);
            for weekday in 0..7 {
                let day = start + chrono::Duration::days(week * 7 + weekday);
                let selected = day == self.day;
                let in_month = day.month() == self.month.month();
                let mut content = column![
                    container(text(day.day().to_string()).size(12).font(if day == today {
                        BOLD
                    } else {
                        iced::Font::DEFAULT
                    }))
                    .padding([2, 3])
                ]
                .spacing(6);
                for event in self.events.iter().filter(|e| event_day(e) == day).take(2) {
                    content = content.push(
                        container(text(truncate(&event.title, 19)).size(9))
                            .padding([4, 5])
                            .width(Length::Fill)
                            .style(|t| {
                                let p = colors(t);
                                container::Style {
                                    background: Some(p.tint.into()),
                                    text_color: Some(p.accent),
                                    border: Border {
                                        radius: 4.into(),
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                }
                            }),
                    );
                }
                days = days.push(
                    mouse_area(
                        button(content)
                            .padding(9)
                            .width(Length::Fill)
                            .height(Length::Fill)
                            .style(move |t, status| {
                                let p = colors(t);
                                button::Style {
                                    background: Some(
                                        if selected {
                                            p.tint
                                        } else if matches!(status, button::Status::Hovered) {
                                            p.subtle
                                        } else {
                                            p.surface
                                        }
                                        .into(),
                                    ),
                                    text_color: if in_month { p.text } else { p.muted },
                                    border: Border {
                                        color: if day == today { p.accent } else { p.border },
                                        width: if day == today { 1.5 } else { 0.5 },
                                        radius: 4.into(),
                                    },
                                    ..Default::default()
                                }
                            })
                            .on_press(Message::Day(day)),
                    )
                    .on_double_click(Message::NewEventOnDay(day)),
                );
            }
            weeks = weeks.push(days.height(Length::Fill));
        }
        let grid = container(
            column![month_header, space().height(8), weeks.height(Length::Fill)].spacing(12),
        )
        .padding(23)
        .style(card)
        .width(Length::Fill)
        .height(Length::Fill);
        let mut agenda = column![
            muted(self.day.format("%A").to_string().to_uppercase())
                .size(10)
                .font(BOLD),
            text(self.day.format("%d %B").to_string())
                .size(24)
                .font(BOLD),
            space().height(12)
        ]
        .spacing(7);
        let events: Vec<_> = self
            .events
            .iter()
            .filter(|e| event_day(e) == self.day)
            .collect();
        if events.is_empty() {
            agenda = agenda.push(
                container(
                    column![
                        icon("sun", 26.),
                        text("No events on this day").size(13).font(BOLD),
                        muted("No events planned for this day.").size(11)
                    ]
                    .spacing(16),
                )
                .padding([25, 0]),
            );
        }
        for event in events {
            agenda = agenda
                .push(
                    button(
                        column![
                            muted(if event.all_day {
                                "All day".into()
                            } else {
                                format!(
                                    "{} – {}",
                                    event.start.with_timezone(&chrono::Local).format("%H:%M"),
                                    event.end.with_timezone(&chrono::Local).format("%H:%M")
                                )
                            })
                            .size(10),
                            text(&event.title).size(13).font(BOLD),
                            muted(&event.location).size(11)
                        ]
                        .spacing(9),
                    )
                    .padding(15)
                    .width(Length::Fill)
                    .style(outline)
                    .on_press(Message::EditEvent(event.key())),
                )
                .push(space().height(6));
        }
        agenda = agenda
            .push(space().height(Length::Fill))
            .push(line())
            .push(muted("YOUR CALENDARS").size(10).font(BOLD))
            .push(space().height(5));
        for source in &self.workspace.calendars {
            agenda =
                agenda.push(row![icon("calendar", 14.), text(&source.name).size(11)].spacing(9));
        }
        agenda = agenda.push(space().height(9)).push(action(
            "Connect a calendar",
            Message::Open(Dialog::Calendar),
        ));
        let body = row![
            grid,
            container(agenda)
                .padding(23)
                .width(if self.size.width < 1200. { 220. } else { 275. })
                .height(Length::Fill)
                .style(card)
        ]
        .spacing(18);
        container(column![header, body.height(Length::Fill)].spacing(23))
            .padding([27, 28])
            .height(Length::Fill)
            .into()
    }
    fn preferences_view(&self) -> Element<'_, Message> {
        let header = self.page_header(
            "Preferences",
            "Make Shep feel like home.",
            row![
                input(
                    "Search settings…",
                    &self.settings_search,
                    Message::SettingsSearch
                )
                .id("settings-search")
                .width(if self.size.width < 1100. { 170 } else { 230 }),
                action("Save changes", Message::SavePreferences)
            ]
            .spacing(10)
            .align_y(Alignment::Center)
            .into(),
        );
        let mut tabs = row![].spacing(7);
        for (tab, label) in [
            (SettingsTab::General, "General"),
            (SettingsTab::Accounts, "Accounts"),
            (SettingsTab::Calendars, "Calendars"),
            (SettingsTab::Backups, "Backups"),
            (SettingsTab::Shortcuts, "Shortcuts"),
            (SettingsTab::Privacy, "Privacy"),
            (SettingsTab::Contacts, "Contacts"),
        ] {
            tabs = tabs.push(
                button(text(label).size(12))
                    .padding([10, 15])
                    .style(if self.settings_tab == tab {
                        selected
                    } else {
                        ghost
                    })
                    .on_press(Message::SettingsTab(tab)),
            );
        }
        let content = if !self.settings_search.trim().is_empty() {
            self.settings_results()
        } else {
            match self.settings_tab {
                SettingsTab::General => self.general_settings(),
                SettingsTab::Accounts => self.account_settings(),
                SettingsTab::Calendars => self.calendar_settings(),
                SettingsTab::Backups => self.backup_settings(),
                SettingsTab::Shortcuts => self.shortcut_settings(),
                SettingsTab::Privacy => self.privacy_settings(),
                SettingsTab::Contacts => self.contacts_settings(),
            }
        };
        let content: Element<'_, Message> = if self.settings_group.is_some() {
            column![action("All settings", Message::ShowAllSettings), content]
                .spacing(12)
                .into()
        } else {
            content
        };
        container(
            column![
                muted("WORKSPACE  /  PREFERENCES").size(10).font(BOLD),
                header,
                tabs.wrap(),
                line(),
                scrollable(container(content).max_width(940).width(Length::Fill))
                    .height(Length::Fill)
            ]
            .spacing(23),
        )
        .padding([27, 32])
        .height(Length::Fill)
        .into()
    }
    fn general_settings(&self) -> Element<'_, Message> {
        let mut choices = row![].spacing(15);
        for (appearance, label, description) in [
            (Appearance::Light, "Light", "A fresh, quiet workspace"),
            (Appearance::Dark, "Dark", "Easy on the eyes"),
            (Appearance::System, "System", "Follow your device"),
        ] {
            let active = self.preferences.appearance == appearance;
            let preview = container(
                row![
                    container(space())
                        .width(20)
                        .height(44)
                        .style(move |_| container::Style {
                            background: Some(
                                if appearance == Appearance::Dark {
                                    hex(0x24242a)
                                } else {
                                    hex(0xefedf4)
                                }
                                .into()
                            ),
                            ..Default::default()
                        }),
                    column![
                        container(space()).height(6).width(55).style(move |_| {
                            container::Style {
                                background: Some(hex(0xa799c4).into()),
                                border: Border {
                                    radius: 3.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }
                        }),
                        container(space()).height(20).width(85).style(move |_| {
                            container::Style {
                                background: Some(
                                    if appearance == Appearance::Dark {
                                        hex(0x323239)
                                    } else {
                                        hex(0xf0eef4)
                                    }
                                    .into(),
                                ),
                                border: Border {
                                    radius: 4.into(),
                                    ..Default::default()
                                },
                                ..Default::default()
                            }
                        })
                    ]
                    .spacing(10)
                ]
                .spacing(12),
            )
            .padding(12)
            .width(Length::Fill)
            .style(move |_| container::Style {
                background: Some(
                    if appearance == Appearance::Dark {
                        hex(0x19191d)
                    } else {
                        iced::Color::WHITE
                    }
                    .into(),
                ),
                border: Border {
                    radius: 6.into(),
                    color: hex(0xdedbe5),
                    width: 1.,
                },
                ..Default::default()
            });
            choices = choices.push(
                button(
                    column![
                        preview,
                        row![
                            text(label).font(BOLD).size(13),
                            space().width(Length::Fill),
                            if active {
                                icon("check", 15.)
                            } else {
                                space().into()
                            }
                        ],
                        muted(description).size(10)
                    ]
                    .spacing(13),
                )
                .padding(16)
                .width(Length::Fill)
                .style(if active { selected } else { outline })
                .on_press(Message::Appearance(appearance)),
            );
        }
        column![
            self.settings_card("Appearance", "", choices.into()),
            if self
                .settings_group
                .is_none_or(|g| g == "Reading and layout")
            {
                Element::from(container(self.reading_settings()).padding(23).style(card))
            } else {
                space().into()
            },
            self.settings_card(
                "Mail & performance",
                "Choose how often to check for new messages.",
                column![
                    row![
                        column![
                            text("Check for new mail").size(13),
                            muted("Minutes between background checks").size(11)
                        ]
                        .spacing(5),
                        space().width(Length::Fill),
                        input("5", self.field("sync_minutes"), |v| Message::Field(
                            "sync_minutes",
                            v
                        ))
                        .width(90)
                    ]
                    .align_y(Alignment::Center),
                    line(),
                    row![
                        icon("check", 16.),
                        muted("Next messages and the next page preload automatically.")
                    ]
                    .spacing(10)
                ]
                .spacing(19)
                .into()
            ),
            self.settings_card(
                "Tooltips",
                "",
                column![
                    checkbox(self.preferences.tooltips)
                        .label("Show tooltips on icons")
                        .on_toggle(Message::PrefTooltips),
                    checkbox(self.preferences.shortcut_tooltips)
                        .label("Show primary shortcut in tooltips")
                        .on_toggle(Message::PrefShortcutTooltips)
                ]
                .spacing(16)
                .into()
            ),
            self.settings_card(
                "About Shep",
                "",
                row![
                    image(if self.dark() {
                        self.dark_logo.clone()
                    } else {
                        self.light_logo.clone()
                    })
                    .width(42)
                    .height(42),
                    column![
                        text(format!("Shep {}", env!("CARGO_PKG_VERSION")))
                            .size(13)
                            .font(BOLD),
                        muted("").size(11)
                    ]
                    .spacing(5)
                ]
                .spacing(14)
                .align_y(Alignment::Center)
                .into()
            )
        ]
        .spacing(if self.settings_group.is_some() { 0 } else { 22 })
        .into()
    }
    fn account_settings(&self) -> Element<'_, Message> {
        let mut accounts = column![].spacing(14);
        for (index, account) in self.workspace.accounts.iter().enumerate() {
            accounts = accounts
                .push(
                    row![
                        avatar(&account.name, index, 38.),
                        column![
                            text(&account.name).size(13).font(BOLD),
                            muted(format!("{} · {}", account.email, account.protocol)).size(11)
                        ]
                        .spacing(4),
                        space().width(Length::Fill),
                        action("Edit account", Message::EditAccount(account.id.clone())),
                        self.icon_action(
                            "trash",
                            "Remove account",
                            Message::ReviewRemoval(crate::store::ConnectionRef {
                                kind: crate::store::ConnectionKind::Account,
                                id: account.id.clone()
                            })
                        )
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                )
                .push(line());
        }
        accounts = accounts.push(action("Add mail account", Message::Open(Dialog::Account)));
        if self.workspace.credential_cleanup > 0 {
            accounts = accounts.push(self.cleanup_preferences());
        }
        column![
            self.settings_card(
                "Your accounts",
                "Use IMAP or POP3 with an app password. Add as many accounts as you need.",
                accounts.into()
            ),
            self.google_settings()
        ]
        .spacing(if self.settings_group.is_some() { 0 } else { 22 })
        .into()
    }
    fn google_settings(&self) -> Element<'_, Message> {
        let busy = self.busy.contains("google") || self.busy.contains("google-disconnect");
        let lifecycle = self.preferences.google_lifecycle;
        let mut controls = row![
            button(
                text(if self.google_connected {
                    "Reconnect Google"
                } else {
                    "Continue with Google"
                })
                .size(12)
            )
            .padding([12, 18])
            .style(outline)
            .on_press_maybe(
                (!busy && !lifecycle.cleanup_pending).then_some(Message::GoogleLogin(true))
            )
        ]
        .spacing(10)
        .align_y(Alignment::Center);
        if self.google_connected {
            controls = controls.push(badge("CONNECTED"));
        }
        if !lifecycle.disconnected
            && (self.google_connected
                || self
                    .workspace
                    .calendars
                    .iter()
                    .any(|s| s.kind == CalendarKind::Google)
                || !self.preferences.google_connection_id.is_empty())
        {
            controls = controls.push(
                button(text("Disconnect…").size(12))
                    .padding([12, 18])
                    .style(outline)
                    .on_press_maybe(
                        (!self.busy.contains("google-disconnect"))
                            .then_some(Message::ReviewGoogleDisconnect),
                    ),
            );
        }
        let mut body = column![
            form_field(
                "Desktop OAuth client ID",
                "your-client-id.apps.googleusercontent.com",
                self.field("google_id"),
                "google_id",
                false
            ),
            form_field(
                "Desktop OAuth client secret",
                "From your Google desktop application credentials",
                self.field("google_secret"),
                "google_secret",
                true
            ),
            controls.wrap(),
        ]
        .spacing(16);
        if lifecycle.cleanup_pending {
            body = body.push(text("Google is disconnected. Unlock your credential store to finish removing its saved login.").size(12))
                .push(button(text("Retry Google cleanup").size(12)).padding(12).style(outline).on_press_maybe((!busy).then_some(Message::CleanupGoogle)));
        } else if lifecycle.disconnected {
            body = body
                .push(muted("Disconnected · cached calendars remain available to read.").size(12));
        }
        let access = self.preferences.google_grant.access;
        if access.known && !lifecycle.disconnected {
            body = body.push(
                row![
                    text(if access.calendar_write {
                        "Calendar · read & write"
                    } else if access.calendar_read {
                        "Calendar · read only"
                    } else {
                        "Calendar · not granted"
                    })
                    .size(12),
                    text(if access.drive {
                        "Drive backup · granted"
                    } else {
                        "Drive backup · not granted"
                    })
                    .size(12),
                ]
                .spacing(24)
                .wrap(),
            );
        }
        if !busy && !lifecycle.cleanup_pending {
            body = body.push(
                button(text("Start a new sign-in").size(12))
                    .style(button::text)
                    .on_press(Message::GoogleLogin(false)),
            );
        }
        body = body.push(muted("Enable the Drive and Calendar APIs in your Google Cloud project. Sign-in opens your browser; backups stay off until you enable them.").size(11));
        self.settings_card(
            "Google connection",
            "Connect Google Calendar and optionally save encrypted copies to Drive.",
            body.into(),
        )
    }
    fn calendar_settings(&self) -> Element<'_, Message> {
        let mut sources = column![].spacing(17);
        for source in &self.workspace.calendars {
            sources = sources
                .push(
                    row![
                        icon("calendar", 18.),
                        column![
                            text(&source.name).font(BOLD).size(13),
                            muted(match source.kind {
                                CalendarKind::Google =>
                                    if self.workspace.google_archived.contains(&source.id) {
                                        "Google Calendar · offline archive"
                                    } else if source.access.read_only() {
                                        "Google Calendar · read only"
                                    } else {
                                        "Google Calendar"
                                    },
                                CalendarKind::CalDav =>
                                    if source.access.read_only() {
                                        "CalDAV · read only"
                                    } else {
                                        "CalDAV · home server"
                                    },
                            })
                            .size(11)
                        ]
                        .spacing(4),
                        space().width(Length::Fill),
                        self.icon_action(
                            "trash",
                            "Remove calendar",
                            Message::ReviewRemoval(crate::store::ConnectionRef {
                                kind: crate::store::ConnectionKind::Calendar,
                                id: source.id.clone()
                            })
                        )
                    ]
                    .spacing(14)
                    .align_y(Alignment::Center),
                )
                .push(line());
        }
        sources=sources.push(action("Add CalDAV calendar",Message::Open(Dialog::Calendar))).push(muted("Enter your server address to find calendars, or use a calendar collection URL.").size(11));
        if self.workspace.removed_google_calendars > 0 {
            sources = sources.push(action(
                "Restore removed Google calendars",
                Message::RestoreGoogleCalendars,
            ));
        }
        if self.workspace.credential_cleanup > 0 {
            sources = sources.push(self.cleanup_preferences());
        }
        column![
            self.settings_card(
                "Connected calendars",
                "Choose which calendars you use in Shep.",
                sources.into()
            ),
            self.google_settings()
        ]
        .spacing(if self.settings_group.is_some() { 0 } else { 22 })
        .into()
    }
    fn backup_settings(&self) -> Element<'_, Message> {
        let target = self.configured_backup_target();
        let matches_saved = target == BackupTarget::from_preferences(&self.workspace.preferences);
        let ready = matches_saved && self.workspace.preferences.backup_ready;
        let last_backup = if matches_saved {
            self.workspace.preferences.last_backup
        } else {
            None
        };
        let mut form = column![
            row![
                text("Save to").size(13),
                space().width(Length::Fill),
                pick_list(
                    [BackupDestination::Local, BackupDestination::GoogleDrive],
                    Some(self.preferences.backup_destination),
                    Message::BackupDestination
                )
                .text_size(12)
                .padding(11)
                .style(select_input)
                .menu_style(select_menu)
            ]
            .align_y(Alignment::Center)
        ]
        .spacing(18);
        if self.preferences.backup_destination == BackupDestination::Local {
            form = form.push(
                row![
                    form_field(
                        "Local backup folder",
                        "/path/to/backups",
                        self.field("backup_folder"),
                        "backup_folder",
                        false
                    ),
                    action("Browse…", Message::BrowseBackup)
                ]
                .spacing(12)
                .align_y(Alignment::End),
            );
        } else if !self.google_connected || !self.preferences.google_grant.access.drive_allowed() {
            form = form.push(action(
                "Connect Google / approve Drive access",
                Message::SettingsTab(SettingsTab::Calendars),
            ));
        }
        form = form
            .push(
                row![
                    form_field(
                        "Copies to keep (1–100)",
                        "7",
                        self.field("copies"),
                        "copies",
                        false
                    ),
                    form_field(
                        "Backup interval (hours)",
                        "24",
                        self.field("hours"),
                        "hours",
                        false
                    )
                ]
                .spacing(18),
            )
            .push(
                checkbox(self.preferences.auto_backup)
                    .label("Back up automatically while Shep is running")
                    .on_toggle(Message::AutoBackup)
                    .text_size(12),
            );
        if self.preferences.auto_backup && !ready {
            form = form.push(container(muted("Finish setup: enter a passphrase and choose Back up now. Automatic backups start after that copy is saved and the passphrase is stored in your OS keychain.").size(12)).padding(12).style(subtle));
        }
        form = form.push(checkbox(self.preferences.backup_accounts).label("Include account passwords in the encrypted backup").on_toggle(Message::BackupAccounts).text_size(12))
        .push(form_field("Backup passphrase", "At least 12 characters", self.field("passphrase"), "passphrase", true))
        .push(muted("Keep the passphrase somewhere safe for restoring. A successful copy also stores it in your OS keychain for this destination. Google tokens are never included. Current snapshot limit: 256 MiB of mail.").size(11))
        .push(row![action("Save backup preferences", Message::SavePreferences),
            button(text(if self.busy.contains("backup") { "Backing up…" } else { "Back up now" }).size(12)).padding([11,17]).style(primary)
                .on_press_maybe((!self.busy.contains("backup") && self.pending_backup.is_none()).then_some(Message::Backup))
        ].spacing(12))
        .push(if let Some(time) = last_backup {
            muted(format!("Last backup: {}", chrono::DateTime::from_timestamp(time,0).unwrap_or_default().with_timezone(&chrono::Local).format("%d %b %Y at %H:%M")))
        } else { muted("No successful backup at this destination yet.") });
        let visible = self.visible_backups();
        let mut copies = column![
            row![
                text("Saved copies").font(BOLD).size(14),
                space().width(Length::Fill),
                action("Refresh copies", Message::ListBackups)
            ]
            .align_y(Alignment::Center)
        ]
        .spacing(15);
        if visible.is_empty() {
            copies = copies.push(muted(if self.backups_target.as_ref() == Some(&target) {
                "No saved copies found at this destination."
            } else {
                "Refresh to see copies at this destination."
            }));
        }
        for copy in visible.iter().take(100) {
            copies = copies.push(
                row![
                    column![
                        text(truncate(&copy.name, 58)).size(11),
                        muted(&copy.created_at).size(10)
                    ]
                    .spacing(5),
                    space().width(Length::Fill),
                    action("Restore", Message::Restore(copy.id.clone()))
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            );
        }
        column![
            self.settings_card(
                "Backups",
                "Choose a destination, schedule and number of copies to keep.",
                form.into()
            ),
            self.settings_card(
                "Restore a copy",
                "Restoring merges messages and accounts into this device. Existing mail is kept.",
                copies.into()
            )
        ]
        .spacing(if self.settings_group.is_some() { 0 } else { 22 })
        .into()
    }
    fn shortcut_settings(&self) -> Element<'_, Message> {
        let mut actions = column![
            row![
                space().width(Length::Fill),
                muted("Primary").width(138),
                muted("Secondary").width(168)
            ]
            .spacing(8)
        ]
        .spacing(3);
        for action in Action::ALL {
            let mut bindings = row![].spacing(8);
            for slot in [Slot::Primary, Slot::Secondary] {
                let active = self.remapping == Some((action, slot));
                let binding = self.preferences.shortcuts.binding(action, slot);
                let label = if active {
                    "Press a key…".into()
                } else if binding.is_empty() {
                    "Disabled".into()
                } else {
                    binding.replace(
                        "Mod",
                        if cfg!(target_os = "macos") {
                            "⌘"
                        } else {
                            "Ctrl"
                        },
                    )
                };
                let control = button(text(label).size(12))
                    .padding([10, 12])
                    .width(138)
                    .style(if active { selected } else { outline })
                    .on_press(Message::Remap(action, slot));
                bindings = bindings.push(control);
                if slot == Slot::Secondary {
                    bindings = bindings.push(
                        button(icon("close", 14.))
                            .padding(6)
                            .style(ghost)
                            .on_press_maybe(
                                (!binding.is_empty() || active)
                                    .then_some(Message::ClearShortcut(action, slot)),
                            ),
                    );
                }
            }
            actions = actions
                .push(
                    row![
                        text(action.label()).size(12),
                        space().width(Length::Fill),
                        if action == Action::Inbox
                            && !self.preferences.shortcuts.key(action).is_empty()
                        {
                            button(text("Disable").size(11))
                                .padding(6)
                                .style(ghost)
                                .on_press(Message::ClearShortcut(action, Slot::Primary))
                                .into()
                        } else {
                            Element::from(space())
                        },
                        bindings
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center)
                    .padding([7, 0]),
                )
                .push(line());
        }
        actions = actions
            .push(space().height(12))
            .push(action("Reset shortcuts", Message::ResetShortcuts));
        self.settings_card(
            "Keyboard shortcuts",
            "Click a binding, then press its replacement. Add a second key with Disabled; × clears it.",
            actions.into(),
        )
    }
    fn dialog_view(&self, dialog: Dialog) -> Element<'_, Message> {
        let (title, subtitle) = match dialog {
            Dialog::Removal => ("Remove connection?", ""),
            Dialog::GoogleDisconnect => ("Disconnect Google?", ""),
            Dialog::Outbox => ("Outbox", ""),
            Dialog::Account => ("Mail account", ""),
            Dialog::Sender => ("Sender details", ""),
            Dialog::Calendar => (
                "Connect a calendar",
                "Bring your home server calendar into Shep with CalDAV.",
            ),
            Dialog::Move => (
                "Move message",
                "Choose a destination folder. POP3 folders are local to this device.",
            ),
            Dialog::Compose => ("New message", ""),
            Dialog::Event => ("Calendar event", "Times use this device's timezone."),
            Dialog::Export => (
                "Save a copy",
                "Choose a full file path. Existing files will never be overwritten.",
            ),
            Dialog::Restore => (
                "Restore this backup?",
                "Messages and account details will be merged into this device.",
            ),
        };
        let mut labels = column![text(title).size(21).font(BOLD)].spacing(8);
        if !subtitle.is_empty() {
            labels = labels.push(muted(subtitle).size(11));
        }
        let header = row![
            labels,
            space().width(Length::Fill),
            self.icon_action("close", "Close dialog", Message::Close)
        ]
        .spacing(12)
        .align_y(Alignment::Center);
        let mut body = column![header, line()].spacing(20);
        match dialog {
            Dialog::Removal => body = body.push(self.removal_form()),
            Dialog::GoogleDisconnect => {
                let pending = self.google_disconnect_pending.is_some();
                body = body.push(text("Calendar sync and Drive backups will stop on this device. Cached calendars remain readable, and your mail and existing backups are kept.").size(13))
                    .push(muted("The saved Google login will be removed from this device. Access on other devices is managed separately in your Google Account.").size(12))
                    .push(row![
                        button(text(if pending { "Disconnecting…" } else { "Disconnect this device" }).size(12)).padding([12,16]).style(primary)
                            .on_press_maybe((!pending).then_some(Message::ConfirmGoogleDisconnect)),
                        action(if pending { "Close" } else { "Cancel" }, Message::Close)
                    ].spacing(10).wrap());
            },
            Dialog::Outbox => body = body.push(self.outbox_view()),
            Dialog::Sender => body = body.push(self.sender_dialog()),
            Dialog::Account => body = body.push(self.account_wizard()),
            Dialog::Calendar => body = body.push(self.calendar_connection_form()),
            Dialog::Move=>{
                if self.preferences.cross_account_moves {
                    let choices: Vec<_> = self.workspace.accounts.iter().filter(|a| a.protocol == Protocol::Imap).map(|a| Choice(a.id.clone(), a.name.clone())).collect();
                    let source = self.detail.as_ref().map(|d| d.summary.account_id.as_str()).unwrap_or("");
                    let id = if self.field("move_account").is_empty() { source } else { self.field("move_account") };
                    let chosen = choices.iter().find(|c| c.0 == id).cloned();
                    body = body.push(column![text("Destination account").size(12), pick_list(choices, chosen, |c:Choice| Message::Field("move_account", c.0)).text_size(12).padding(11).style(select_input).menu_style(select_menu).width(Length::Fill)].spacing(8));
                }
                let folders = crate::fuzzy::ranked(self.field("folder_search"), self.move_folders());
                let destination = folders.first().cloned();
                body=body.push(input("Find a folder…",self.field("folder_search"),|v|Message::Field("folder_search",v)).id("folder-search").on_submit_maybe(destination.map(Message::Move)));
                for (index, folder) in folders.iter().enumerate() {
                    let target = index == 0;
                    let trailing: Element<'_, Message> = if target { muted("Enter ↵").size(11).into() } else { icon("chevron", 14.) };
                    body = body.push(button(row![icon("folder",18.), text(if folder.eq_ignore_ascii_case("INBOX") { "Inbox".to_owned() } else { folder.clone() }).size(13), space().width(Length::Fill), trailing].spacing(12).align_y(Alignment::Center)).padding(13).width(Length::Fill).style(if target { selected } else { outline }).on_press(Message::Move(folder.clone())));
                }
            }
            Dialog::Compose => body = body.spacing(14).push(self.compose_form()),
            Dialog::Event if self.editing_event.is_some() && !self.event_access().update => body = body.push(self.read_only_event()),
            Dialog::Event=>{
                let choices:Vec<_>=self.workspace.calendars.iter().filter(|s| if let Some(event) = &self.editing_event { s.id == event.source_id } else { s.access.create }).map(|a|Choice(a.id.clone(),a.name.clone())).collect();let chosen=choices.iter().find(|a|a.0==self.field("source")).cloned();
                body=body.spacing(14).push(column![text("Title").size(12).font(BOLD),input("Event title",self.field("title"),|v|Message::Field("title",v)).id("event-title")].spacing(8)).push(column![text("Calendar").size(12).font(BOLD), if choices.is_empty() {
                        Element::from(column![text(if self.workspace.calendars.is_empty() { "No calendar connected" } else { "No writable calendar connected" }).size(12), action("Connect calendar", Message::ConnectCalendarFromEvent)].spacing(10))
                    } else {
                        Element::from(pick_list(choices,chosen,|c:Choice|Message::Field("source",c.0)).placeholder("Choose a calendar").text_size(12).padding(12).style(select_input).menu_style(select_menu).width(Length::Fill))
                    }].spacing(8))
                    .push(checkbox(self.field("all_day")=="true").label("All day").on_toggle(|v|Message::Field("all_day",v.to_string())))
                    .push(row![form_field("Start date · YYYY-MM-DD","2026-09-05",self.field("date"),"date",false),form_field("Last date · YYYY-MM-DD","2026-09-05",self.field("end_date"),"end_date",false)].spacing(15)).push(if self.field("all_day")=="true" { Element::from(space()) } else { Element::from(row![form_field("From · HH:MM","09:00",self.field("start"),"start",false),form_field("To · HH:MM","10:00",self.field("end"),"end",false)].spacing(15)) }).push(form_field("Location","Somewhere lovely",self.field("location"),"location",false))
                    .push(row![button(text("Save event").size(12)).padding([12,18]).style(primary).on_press_maybe((!self.field("source").is_empty()).then_some(Message::SaveEvent)),space().width(Length::Fill),if self.editing_event.is_some() && self.event_access().delete {Element::from(action("Delete event",Message::DeleteEvent))}else{Element::from(space())}].spacing(10));
            }
            Dialog::Export=>body=body.push(form_field("Full destination path","/home/you/Downloads/message.eml",self.field("path"),"path",false)).push(action("Browse…",Message::BrowseExport)).push(button(text("Save file").size(12)).padding([12,18]).style(primary).on_press(Message::SaveExport)),
            Dialog::Restore=>body=body.push(form_field("Backup passphrase","Enter the original passphrase",self.field("passphrase"),"passphrase",true)).push(muted("Existing mail, connection settings and passwords are kept. Missing account passwords are filled from the copy when available. Google sign-in and preferences stay unchanged.").size(11)).push(row![action("Cancel",Message::Close),button(text("Restore & merge").size(12)).padding([12,18]).style(primary).on_press(Message::ConfirmRestore)].spacing(10)),
        }
        if let Some((notice, true, _)) = &self.notice {
            body = body.push(container(text(notice).size(11)).padding(12).style(subtle));
        }
        container(scrollable(container(body).padding(27)).height(Length::Shrink))
            .max_height((self.size.height - 65.).max(400.))
            .width(if dialog == Dialog::Compose {
                680.
            } else {
                570.
            })
            .style(card)
            .into()
    }
}
impl App {
    fn settings_card<'a>(
        &self,
        title: &'a str,
        description: &'a str,
        content: Element<'a, Message>,
    ) -> Element<'a, Message> {
        if self.settings_group.is_some_and(|group| group != title) {
            return space().into();
        }
        let mut body = column![text(title).font(BOLD).size(16)].spacing(16);
        if !description.is_empty() {
            body = body.push(muted(description).size(12));
        }
        container(body.push(content))
            .padding(25)
            .width(Length::Fill)
            .style(card)
            .into()
    }
}
fn form_field<'a>(
    label: &'a str,
    placeholder: &str,
    value: &str,
    key: &'static str,
    secure: bool,
) -> Element<'a, Message> {
    column![
        text(label).size(12).font(BOLD),
        input(placeholder, value, move |v| Message::Field(key, v)).secure(secure)
    ]
    .spacing(8)
    .width(Length::Fill)
    .into()
}
pub(super) fn sender_name(from: &str) -> String {
    from.split('<')
        .next()
        .unwrap_or(from)
        .trim()
        .trim_matches('"')
        .to_string()
}
pub(super) fn truncate(text: &str, max: usize) -> String {
    let mut chars = text.chars();
    let mut output: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        output.push('…');
    }
    output
}
fn event_day(e: &CalendarEvent) -> NaiveDate {
    if e.all_day {
        e.start.date_naive()
    } else {
        e.start.with_timezone(&chrono::Local).date_naive()
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Choice(pub(super) String, pub(super) String);
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}
