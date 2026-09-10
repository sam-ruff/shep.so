use super::*;
use crate::profile_sync::join::Review;
use iced::widget::{column, pick_list};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Choice {
    id: Option<String>,
    label: String,
}
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl App {
    pub(super) fn shared_join_accounts<'a>(
        &self,
        review: &'a Arc<Review>,
        idle: bool,
    ) -> Element<'a, Message> {
        let state = &self.profile_sync;
        let mut body = column![].spacing(12);
        for offer in review.account_page(state.join_offset) {
            let mut entry =
                column![text(&offer.name).size(13), muted(&offer.email).size(12)].spacing(4);
            if !offer.matches.is_empty() {
                let mut choices = vec![Choice {
                    id: None,
                    label: "Add as a new account".into(),
                }];
                choices.extend(
                    offer
                        .matches
                        .iter()
                        .filter(|candidate| {
                            !state.join_links.iter().any(|(shared, local)| {
                                *shared != offer.shared && *local == candidate.id
                            })
                        })
                        .map(|candidate| Choice {
                            id: Some(candidate.id.clone()),
                            label: format!("Use existing · {}", candidate.name),
                        }),
                );
                let selected = choices
                    .iter()
                    .find(|candidate| candidate.id.as_ref() == state.join_links.get(&offer.shared))
                    .cloned()
                    .or_else(|| choices.first().cloned());
                let current = review.clone();
                let shared = offer.shared;
                entry = entry.push(pick_list(choices, selected, move |choice| Message::ProfileSync(Action::JoinLink(current.clone(), shared, choice.id))).width(Length::Fill).padding(10).text_size(13))
                    .push(muted("Matching connections can keep this device's mail and sign-in. Its account name will be shared.").size(12));
            }
            body = body.push(entry);
        }
        if review.accounts > 8 {
            body = body.push(
                row![
                    button(text("Previous accounts").size(13))
                        .padding([10, 14])
                        .style(outline)
                        .on_press_maybe((idle && state.join_offset > 0).then(|| {
                            Message::ProfileSync(Action::JoinPage(
                                review.clone(),
                                state.join_offset.saturating_sub(8),
                            ))
                        })),
                    text(format!(
                        "{}–{} of {}",
                        state.join_offset + 1,
                        (state.join_offset + 8).min(review.accounts),
                        review.accounts
                    ))
                    .size(13),
                    button(text("Next accounts").size(13))
                        .padding([10, 14])
                        .style(outline)
                        .on_press_maybe((idle && state.join_offset + 8 < review.accounts).then(
                            || Message::ProfileSync(Action::JoinPage(
                                review.clone(),
                                state.join_offset + 8
                            ))
                        ))
                ]
                .spacing(10)
                .align_y(iced::Alignment::Center),
            );
        }
        body.into()
    }
}
