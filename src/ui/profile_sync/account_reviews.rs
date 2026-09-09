use super::*;
use crate::{model::Account, profile_sync::account_reviews::Choice};
use iced::widget::{column, pick_list};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    operation: Uuid,
    label: String,
}
impl std::fmt::Display for Candidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}
fn connection(account: &Account) -> Element<'_, Message> {
    column![
        text(format!("{} · {}", account.email, account.protocol)).size(13),
        text(format!(
            "Incoming: {}:{} · {} · {}",
            account.host, account.port, account.incoming_security, account.incoming_auth
        ))
        .size(12),
        text(format!(
            "SMTP: {}:{} · {} · {}",
            account.smtp_host,
            account.smtp_port,
            account.smtp_security(),
            account.smtp_auth
        ))
        .size(12),
        text(format!("Incoming login: {}", account.username)).size(12),
        text(format!(
            "SMTP login: {} · {}",
            account.smtp_username(),
            if account.smtp_auth == crate::model::SmtpAuth::None {
                "No password required"
            } else if account.smtp_separate_password {
                "Separate password"
            } else {
                "Incoming password"
            }
        ))
        .size(12),
        text(format!(
            "Sent: {}{}",
            account.sent_copy,
            if account.sent_folder.is_empty() {
                String::new()
            } else {
                format!(" · {}", account.sent_folder)
            }
        ))
        .size(12),
    ]
    .spacing(4)
    .into()
}
impl App {
    pub(super) fn shared_account_reviews(&self, idle: bool) -> Element<'_, Message> {
        let state = &self.profile_sync;
        let mut body = column![
            text("Choose account connections").font(BOLD).size(16),
            muted("Keep your current connection, or add the shared setup and reconnect it. Your previous account and mail remain available.").size(12)
        ].spacing(14);
        if let Some(reviews) = &state.account_reviews {
            if reviews.is_empty() {
                body = body.push(text("No connections on this page need a choice.").size(13));
            }
            for review in reviews {
                let choices: Vec<_> = review
                    .versions()
                    .iter()
                    .enumerate()
                    .map(|(i, v)| Candidate {
                        operation: v.operation,
                        label: format!("{} · version {}", v.account.host, i + 1),
                    })
                    .collect();
                let chosen = state
                    .account_choices
                    .get(&review.local().id)
                    .cloned()
                    .or_else(|| choices.first().cloned());
                let account = chosen.as_ref().and_then(|choice| {
                    review
                        .versions()
                        .iter()
                        .find(|v| v.operation == choice.operation)
                });
                let candidate_review = review.clone();
                let mut card = column![
                    text(&review.local().name).font(BOLD).size(14),
                    text("This device").font(BOLD).size(12),
                    connection(review.local()),
                    button(text("Keep this device's connection").size(13))
                        .padding([10, 14])
                        .style(outline)
                        .on_press_maybe(idle.then(|| Message::ProfileSync(
                            Action::ResolveAccount(review.clone(), Choice::Local)
                        ))),
                    pick_list(choices, chosen, move |v| Message::ProfileSync(
                        Action::AccountCandidate(candidate_review.clone(), v)
                    ))
                    .width(Length::Fill)
                    .text_size(13)
                    .padding(10),
                ]
                .spacing(8);
                if let Some(version) = account {
                    card = card.push(connection(&version.account)).push(
                        button(text("Add shared connection").size(13))
                            .padding([10, 14])
                            .style(outline)
                            .on_press_maybe(idle.then(|| {
                                Message::ProfileSync(Action::ResolveAccount(
                                    review.clone(),
                                    Choice::AddShared(version.operation),
                                ))
                            })),
                    );
                }
                body = body.push(card);
            }
        }
        if let Some(after) = &state.account_after {
            body = body.push(
                button(text("Next accounts").size(13))
                    .padding([10, 14])
                    .style(outline)
                    .on_press_maybe(idle.then(|| {
                        Message::ProfileSync(Action::AccountReviews(Some(after.clone())))
                    })),
            );
        }
        body.push(
            button(text("Done").size(13))
                .padding([10, 14])
                .style(outline)
                .on_press_maybe(idle.then_some(Message::ProfileSync(Action::CloseAccountReviews))),
        )
        .into()
    }
}

#[cfg(test)]
impl Candidate {
    pub(super) fn fixture(operation: Uuid) -> Self {
        Self {
            operation,
            label: "Fixture connection".into(),
        }
    }
}
