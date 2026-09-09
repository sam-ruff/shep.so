use super::*;
use iced::{
    Alignment, Length,
    widget::{button, checkbox, column, container, pick_list, row, space, text},
};

impl App {
    pub(super) fn account_wizard(&self) -> Element<'_, Message> {
        let step = self.field("setup_step").parse::<u8>().unwrap_or(0);
        let mut tabs = row![].spacing(8);
        for (index, label) in [(0, "1  Account"), (1, "2  Incoming"), (2, "3  SMTP")] {
            tabs = tabs.push(
                button(text(label).size(12))
                    .padding([10, 12])
                    .style(if step == index { selected } else { ghost })
                    .on_press(Message::Field("setup_step", index.to_string())),
            );
        }
        let mut body = column![tabs].spacing(18);
        match step {
            0 => {
                body = body
                    .push(self.account_field("Account name", "Personal or Work", "name", false))
                    .push(self.account_field("Email address", "you@example.com", "email", false))
                    .push(action(
                        "Use Fastmail server settings",
                        Message::FastmailPreset,
                    ));
            }
            1 => {
                body = body
                    .push(
                        row![
                            text("Protocol").size(12),
                            space().width(Length::Fill),
                            pick_list(
                                [Protocol::Imap, Protocol::Pop3],
                                Some(self.protocol),
                                Message::Protocol
                            )
                            .text_size(12)
                            .padding(10)
                            .style(select_input)
                            .menu_style(select_menu)
                        ]
                        .align_y(Alignment::Center),
                    )
                    .push(
                        row![
                            self.account_field(
                                "Incoming server",
                                "imap.example.com",
                                "host",
                                false
                            ),
                            container(self.account_field("Port", "993", "port", false)).width(95)
                        ]
                        .spacing(12),
                    )
                    .push(
                        column![
                            text("Connection security").size(12),
                            pick_list(
                                [ConnectionSecurity::Tls, ConnectionSecurity::StartTls],
                                Some(self.connection_security("incoming_security")),
                                |v| Message::Field("incoming_security", format!("{v:?}"))
                            )
                            .text_size(12)
                            .padding(10)
                            .width(Length::Fill)
                            .style(select_input)
                            .menu_style(select_menu)
                        ]
                        .spacing(8),
                    )
                    .push(
                        column![
                            text("Authentication").size(12),
                            pick_list(
                                [IncomingAuth::Password, IncomingAuth::Plain],
                                Some(self.incoming_auth()),
                                |v| Message::Field("incoming_auth", format!("{v:?}"))
                            )
                            .text_size(12)
                            .padding(10)
                            .width(Length::Fill)
                            .style(select_input)
                            .menu_style(select_menu)
                        ]
                        .spacing(8),
                    )
                    .push(self.account_field(
                        "Username",
                        "Defaults to your email address",
                        "username",
                        false,
                    ))
                    .push(self.account_field(
                        "Password / app password",
                        if self.field("id").is_empty()
                            || self.workspace.profile_reconnect.contains(self.field("id"))
                        {
                            "App password"
                        } else {
                            "Leave blank to keep the saved password"
                        },
                        "password",
                        true,
                    ))
                    .push(
                        button(
                            text(if self.busy.contains("test:Incoming") {
                                "Testing…"
                            } else {
                                "Test incoming connection"
                            })
                            .size(12),
                        )
                        .padding(11)
                        .style(outline)
                        .on_press_maybe(
                            (!self.busy.contains("test:Incoming"))
                                .then_some(Message::TestConnection(ConnectionTarget::Incoming)),
                        ),
                    );
                if !self.field("test_incoming").is_empty() {
                    body = body.push(text(self.field("test_incoming")).size(12));
                }
                if self.field("host").ends_with("fastmail.com") {
                    body = body.push(muted("Fastmail requires an app password, your full login address, and SSL/TLS on 993 (IMAP) or 995 (POP3).").size(11));
                }
            }
            _ => {
                body = body
                    .push(
                        row![
                            self.account_field(
                                "SMTP server",
                                "smtp.example.com",
                                "smtp_host",
                                false
                            ),
                            container(self.account_field("Port", "587", "smtp_port", false))
                                .width(95)
                        ]
                        .spacing(12),
                    )
                    .push(
                        column![
                            text("Connection security").size(12),
                            pick_list(
                                [ConnectionSecurity::Tls, ConnectionSecurity::StartTls],
                                Some(self.connection_security("smtp_security")),
                                |v| Message::Field("smtp_security", format!("{v:?}"))
                            )
                            .text_size(12)
                            .padding(10)
                            .width(Length::Fill)
                            .style(select_input)
                            .menu_style(select_menu)
                        ]
                        .spacing(8),
                    )
                    .push(
                        column![
                            text("Authentication").size(12),
                            pick_list(
                                [
                                    SmtpAuth::Automatic,
                                    SmtpAuth::Plain,
                                    SmtpAuth::Login,
                                    SmtpAuth::None
                                ],
                                Some(self.smtp_auth()),
                                |v| Message::Field("smtp_auth", format!("{v:?}"))
                            )
                            .text_size(12)
                            .padding(10)
                            .width(Length::Fill)
                            .style(select_input)
                            .menu_style(select_menu)
                        ]
                        .spacing(8),
                    )
                    .push(self.account_field(
                        "SMTP username",
                        "Defaults to the incoming username",
                        "smtp_username",
                        false,
                    ))
                    .push(
                        checkbox(self.field("smtp_separate") == "true")
                            .label("Use a different SMTP password")
                            .on_toggle(|v| Message::Field("smtp_separate", v.to_string())),
                    );
                if self.field("smtp_separate") == "true" {
                    body = body.push(self.account_field(
                        "SMTP password",
                        if self.workspace.profile_reconnect.contains(self.field("id")) {
                            "Enter this device's SMTP password"
                        } else {
                            "Leave blank to keep a saved SMTP password"
                        },
                        "smtp_password",
                        true,
                    ));
                }
                body = body.push(
                    button(
                        text(if self.busy.contains("test:Smtp") {
                            "Testing…"
                        } else {
                            "Test SMTP connection"
                        })
                        .size(12),
                    )
                    .padding(11)
                    .style(outline)
                    .on_press_maybe(
                        (!self.busy.contains("test:Smtp"))
                            .then_some(Message::TestConnection(ConnectionTarget::Smtp)),
                    ),
                );
                if !self.field("test_smtp").is_empty() {
                    body = body.push(text(self.field("test_smtp")).size(12));
                }
                if self.protocol == Protocol::Imap {
                    let policy = match self.field("sent_copy") {
                        "ServerManaged" => SentCopyPolicy::ServerManaged,
                        "LocalOnly" => SentCopyPolicy::LocalOnly,
                        _ => SentCopyPolicy::Automatic,
                    };
                    body = body.push(
                        column![
                            text("Sent copies").size(12),
                            pick_list(
                                [
                                    SentCopyPolicy::Automatic,
                                    SentCopyPolicy::ServerManaged,
                                    SentCopyPolicy::LocalOnly
                                ],
                                Some(policy),
                                |p| Message::Field("sent_copy", format!("{p:?}"))
                            )
                            .text_size(12)
                            .padding(10)
                            .width(Length::Fill)
                            .style(select_input)
                            .menu_style(select_menu)
                        ]
                        .spacing(8),
                    );
                    if policy != SentCopyPolicy::LocalOnly {
                        body = body.push(self.account_field(
                            "Sent folder (optional)",
                            "Detect the server's Sent folder",
                            "sent_folder",
                            false,
                        ));
                    }
                } else {
                    body = body.push(muted("POP3 keeps Sent copies locally.").size(11));
                }
            }
        }
        body.push(
            row![
                button(text("Back").size(12))
                    .padding([11, 18])
                    .style(ghost)
                    .on_press_maybe(
                        (step > 0).then(|| Message::Field(
                            "setup_step",
                            step.saturating_sub(1).to_string()
                        ))
                    ),
                space().width(Length::Fill),
                button(
                    text(if step == 2 {
                        "Save account"
                    } else {
                        "Continue"
                    })
                    .size(12)
                )
                .padding([11, 18])
                .style(primary)
                .on_press(if step == 2 {
                    Message::SaveAccount
                } else {
                    Message::Field("setup_step", (step + 1).to_string())
                }),
            ]
            .spacing(8),
        )
        .into()
    }
    fn account_field<'a>(
        &'a self,
        label: &'a str,
        hint: &'a str,
        key: &'static str,
        secure: bool,
    ) -> Element<'a, Message> {
        column![
            text(label).size(12).font(BOLD),
            input(hint, self.field(key), move |v| Message::Field(key, v)).secure(secure)
        ]
        .spacing(8)
        .width(Length::Fill)
        .into()
    }
}
