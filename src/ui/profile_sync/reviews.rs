use super::*;
use crate::profile_sync::reviews::Choice;
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
fn display(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Bool(value) => if *value { "On" } else { "Off" }.into(),
        serde_json::Value::String(value) => value.clone(),
        value => value.to_string(),
    }
}
impl App {
    pub(super) fn shared_setting_reviews(&self, idle: bool) -> Element<'_, Message> {
        let state = &self.profile_sync;
        let mut body = column![
            text("Choose shared preferences").font(BOLD).size(16),
            muted("Your saved choice will sync to other devices.").size(12)
        ]
        .spacing(12);
        if let Some(reviews) = &state.setting_reviews {
            if reviews.is_empty() {
                body = body.push(text("No preferences need a choice.").size(13));
            }
            for review in reviews {
                let choices: Vec<_> = review
                    .versions()
                    .iter()
                    .enumerate()
                    .map(|(index, version)| Candidate {
                        operation: version.operation,
                        label: format!(
                            "{}{}{}",
                            display(&version.value),
                            if version.reset { " (default)" } else { "" },
                            if review.versions().len() > 1 {
                                format!(" · version {}", index + 1)
                            } else {
                                String::new()
                            }
                        ),
                    })
                    .collect();
                let choice = state
                    .setting_choices
                    .get(&review.key())
                    .cloned()
                    .or_else(|| choices.first().cloned());
                let shared = choice.as_ref().map(|c| c.operation);
                let candidate_review = review.clone();
                body = body.push(
                    column![
                        text(review.label()).font(BOLD).size(14),
                        text(format!("This device: {}", display(review.local()))).size(13),
                        button(text("Use this device's choice").size(13))
                            .padding([10, 14])
                            .style(outline)
                            .on_press_maybe(idle.then(|| Message::ProfileSync(
                                Action::ResolveSetting(review.clone(), Choice::Local)
                            ))),
                        pick_list(choices, choice, move |value| Message::ProfileSync(
                            Action::SettingCandidate(candidate_review.clone(), value)
                        ))
                        .width(Length::Fill)
                        .text_size(13)
                        .padding(10),
                        button(text("Use shared choice").size(13))
                            .padding([10, 14])
                            .style(outline)
                            .on_press_maybe(idle.then_some(shared).flatten().map(|id| {
                                Message::ProfileSync(Action::ResolveSetting(
                                    review.clone(),
                                    Choice::Shared(id),
                                ))
                            })),
                    ]
                    .spacing(8),
                );
            }
        }
        body.push(
            button(text("Done").size(13))
                .padding([10, 14])
                .style(outline)
                .on_press_maybe(idle.then_some(Message::ProfileSync(Action::CloseSettingReviews))),
        )
        .into()
    }
}

#[cfg(test)]
impl Candidate {
    pub(super) fn fixture(operation: Uuid) -> Self {
        Self {
            operation,
            label: "Fixture choice".into(),
        }
    }
}
