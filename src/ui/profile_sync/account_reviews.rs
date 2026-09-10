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
/// A native account offered for linking to a new shared definition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalCandidate {
    id: String,
    label: String,
}
impl LocalCandidate {
    pub(super) fn id(&self) -> &str {
        &self.id
    }
}
impl std::fmt::Display for LocalCandidate {
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
            text("Review shared accounts").font(BOLD).size(16),
            muted("Review shared changes before changing connections or removing local mail.")
                .size(12)
        ]
        .spacing(14);
        if let Some(reviews) = &state.account_reviews {
            if reviews.is_empty() {
                body = body.push(text("No connections on this page need a choice.").size(13));
            }
            for review in reviews {
                if let Some(link) = review.link() {
                    body = body.push(self.shared_account_link(review, link, idle));
                    continue;
                }
                if review.removed() {
                    body = body.push(column![
                        text(&review.local().name).font(BOLD).size(14),
                        text(&review.local().email).size(13),
                        muted("Removed from the shared profile. Keep this account on this device, or review its local data before removing it here.").size(12),
                        row![
                            button(text("Keep on this device").size(13)).padding([10, 14]).style(outline)
                                .on_press_maybe(idle.then(|| Message::ProfileSync(Action::ResolveAccount(review.clone(), Choice::KeepRemovedLocal)))),
                            button(text("Review removal…").size(13)).padding([10, 14]).style(outline)
                                .on_press_maybe(idle.then(|| Message::ProfileSync(Action::ReviewRemovedAccount(review.clone())))),
                        ].spacing(8),
                    ].spacing(10));
                    continue;
                }
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

impl App {
    fn shared_account_link<'a>(
        &'a self,
        review: &'a Arc<crate::profile_sync::account_reviews::Review>,
        link: &'a crate::profile_sync::account_reviews::Link,
        idle: bool,
    ) -> Element<'a, Message> {
        let state = &self.profile_sync;
        let exact: Vec<_> = link.matches.iter().filter(|m| m.exact).collect();
        let chosen = state
            .account_links
            .get(&review.shared())
            .map(|c| c.id.clone())
            .filter(|id| exact.iter().any(|m| m.account.id == *id))
            .or_else(|| exact.first().map(|m| m.account.id.clone()));
        let local = link
            .matches
            .iter()
            .find(|m| Some(&m.account.id) == chosen.as_ref())
            .or_else(|| link.matches.first());
        let mut card = column![
            text(&link.account.name).font(BOLD).size(14),
            text(&link.account.email).size(13),
            muted(if link.linkable() {
                "New in the shared profile. This device already has an account with the same connection."
            } else {
                "New in the shared profile. This device has an account with the same address but different server settings, so it cannot be linked."
            })
            .size(12),
            text("Shared").font(BOLD).size(12),
            connection(&link.account),
            text("This device").font(BOLD).size(12),
        ]
        .spacing(8);
        if exact.len() > 1 {
            let choices: Vec<_> = exact
                .iter()
                .map(|m| LocalCandidate {
                    id: m.account.id.clone(),
                    label: m.account.name.clone(),
                })
                .collect();
            let selected = choices
                .iter()
                .find(|c| Some(&c.id) == chosen.as_ref())
                .cloned();
            let candidate_review = review.clone();
            card = card.push(
                pick_list(choices, selected, move |c| {
                    Message::ProfileSync(Action::LinkCandidate(candidate_review.clone(), c))
                })
                .width(Length::Fill)
                .text_size(13)
                .padding(10),
            );
        }
        if let Some(local) = local {
            if exact.len() <= 1 {
                card = card.push(text(&local.account.name).size(13));
            }
            card = card.push(connection(&local.account));
        }
        if let Some(id) = chosen {
            card = card.push(
                button(text("Link to existing account").size(13))
                    .padding([10, 14])
                    .style(outline)
                    .on_press_maybe(idle.then(|| {
                        Message::ProfileSync(Action::ResolveAccount(
                            review.clone(),
                            Choice::LinkExisting(id.clone()),
                        ))
                    })),
            );
        }
        card.push(
            button(text("Add as a new account").size(13))
                .padding([10, 14])
                .style(outline)
                .on_press_maybe(idle.then(|| {
                    Message::ProfileSync(Action::ResolveAccount(review.clone(), Choice::AddNew))
                })),
        )
        .push(
            button(text("Keep this device's account local").size(13))
                .padding([10, 14])
                .style(outline)
                .on_press_maybe(idle.then(|| {
                    Message::ProfileSync(Action::ResolveAccount(review.clone(), Choice::KeepLocal))
                })),
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
#[cfg(test)]
impl LocalCandidate {
    pub(super) fn fixture(id: &str) -> Self {
        Self {
            id: id.into(),
            label: "Fixture account".into(),
        }
    }
}
