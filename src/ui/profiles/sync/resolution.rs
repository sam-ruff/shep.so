use super::*;
use iced::widget::{column, row};
impl App {
    pub(super) fn sync_resolution_view(&self) -> Element<'_, super::super::super::Message> {
        let sync = &self.profiles.sync;
        let review = sync.review.review.as_ref().unwrap();
        let message = |m| super::super::super::Message::Profiles(super::super::Message::Sync(m));
        let busy = self.profiles.pending.is_some() || sync.saving.is_some();
        let choosing = review.phase == "review" && !busy;
        let mut body=column![
            button(text("Back").size(12)).padding([12,16]).style(outline).on_press(message(Message::Back)),
            text(format!("Review {}",publication::label(review.key))).size(16).font(BOLD),
            muted("Choose the value to share. Other devices receive it when sync resumes; newer edits on this device remain pending.").size(12),
            text(format!("Versions reviewed: {} of {}",review.seen,review.total)).size(12),
        ].spacing(14);
        match review.phase.as_str() {
            "review" => {
                body = body.push(
                    button(
                        text(format!(
                            "{}This device: {}",
                            if sync.choice == Some(Choice::Local) {
                                "✓ "
                            } else {
                                ""
                            },
                            publication::formatted_value(review.key, &review.local)
                        ))
                        .size(12),
                    )
                    .padding([12, 16])
                    .style(outline)
                    .on_press_maybe(choosing.then_some(message(Message::Choose(Choice::Local)))),
                );
                let pages = || {
                    row![
                        button(text("First").size(12))
                            .padding([12, 16])
                            .style(outline)
                            .on_press_maybe(
                                (!busy && sync.review.after.is_some())
                                    .then_some(message(Message::ReviewPage(false)))
                            ),
                        button(text("Next").size(12))
                            .padding([12, 16])
                            .style(outline)
                            .on_press_maybe(
                                (!busy && sync.review.more)
                                    .then_some(message(Message::ReviewPage(true)))
                            ),
                    ]
                    .spacing(8)
                };
                body = body.push(pages());
                for version in &sync.review.versions {
                    let choice = Choice::Version(version.operation);
                    let value = match &version.change.action {
                        Action::Setting { value, .. } => {
                            publication::formatted_value(review.key, value)
                        }
                        Action::SettingRemoved { .. } => "Default (reset)".into(),
                        _ => "Unsupported value".into(),
                    };
                    body = body.push(
                        button(
                            column![
                                text(format!(
                                    "{}{value}",
                                    if sync.choice.as_ref() == Some(&choice) {
                                        "✓ "
                                    } else {
                                        ""
                                    }
                                ))
                                .size(13),
                                muted(format!(
                                    "Device {} · Version {}",
                                    &version.device.to_string()[28..],
                                    &version.operation.to_string()[28..]
                                ))
                                .size(11),
                            ]
                            .spacing(4),
                        )
                        .width(Length::Fill)
                        .padding([12, 16])
                        .style(outline)
                        .on_press_maybe(choosing.then_some(message(Message::Choose(choice)))),
                    );
                }
                body = body.push(pages()).push(
                    row![
                        button(text("Save chosen value").size(12))
                            .padding([12, 16])
                            .on_press_maybe(
                                (choosing && review.seen == review.total && sync.choice.is_some())
                                    .then_some(message(Message::Resolve))
                            ),
                        button(text("Cancel review").size(12))
                            .padding([12, 16])
                            .style(outline)
                            .on_press_maybe((!busy).then_some(message(Message::CancelReview))),
                    ]
                    .spacing(8),
                );
                if review.seen < review.total {
                    body = body.push(
                        muted("Open the remaining version pages before saving your choice.")
                            .size(12),
                    );
                }
            }
            "staged" => {
                body=body.push(text("Your decision is saved. Retry to finish recording it and recover its result.").size(12))
                    .push(button(text("Retry saved decision").size(12)).padding([12,16]).on_press_maybe((!busy).then_some(message(Message::Resolve))));
            }
            "complete" => {
                body = body
                    .push(
                        text(
                            "Decision recorded. Shared uploads follow the profile's sync setting.",
                        )
                        .size(12),
                    )
                    .push(
                        button(text("Done").size(12))
                            .padding([12, 16])
                            .on_press_maybe((!busy).then_some(message(Message::CancelReview))),
                    );
            }
            _ => {
                body=body.push(text("The review could not finish or its values changed. Review the current versions before choosing again.").size(12))
                    .push(button(text("Review again").size(12)).padding([12,16]).on_press_maybe((!busy).then_some(message(Message::Review(review.key)))))
                    .push(button(text("Cancel review").size(12)).padding([12,16]).style(outline).on_press_maybe((!busy).then_some(message(Message::CancelReview))));
            }
        }
        if let Some(error) = sync.error.as_ref().or(review.error.as_ref()) {
            body = body.push(text(error).size(12));
            if review.phase == "review" {
                body = body.push(
                    button(text("Refresh review").size(12))
                        .padding([12, 16])
                        .style(outline)
                        .on_press_maybe((!busy).then_some(message(Message::Review(review.key)))),
                );
            }
        }
        if busy {
            body = body.push(muted("Saving review progress…").size(12));
        }
        self.settings_card(
            "Profiles and sync",
            "Preference conflict",
            body.width(Length::Fill).into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conflict_choice_and_page_survive_background_observations_and_save_waits_for_preferences() {
        let (mut app, _) = App::new();
        let (sender, mut input, mut saves) = engine::CommandSender::profile_save_test_channels();
        app.tx = Some(sender);
        let review = crate::profiles::sync::resolution::Review {
            id: Uuid::from_u128(10),
            profile: "fixture-key".into(),
            key: SettingKey::Appearance,
            device: Uuid::from_u128(11),
            subscription_revision: 1,
            history_revision: 3,
            local_revision: 2,
            local: serde_json::json!("Dark"),
            total: 51,
            seen: 51,
            phase: "review".into(),
            request: None,
            error: None,
        };
        app.profiles.sync.review_open = true;
        app.profiles.sync.observe(Arc::new(SyncObservation {
            profile: Some(review.profile.clone()),
            review: Some(Page {
                review: Some(review.clone()),
                after: Some(Uuid::from_u128(5)),
                ..Default::default()
            }),
            ..Default::default()
        }));
        app.sync_message(Message::Choose(Choice::Version(Uuid::from_u128(6))));
        app.profiles.sync.observe(Arc::new(SyncObservation {
            profile: Some(review.profile.clone()),
            phase: Some("Waiting for the open profile review".into()),
            ..Default::default()
        }));
        assert_eq!(app.profiles.sync.review.after, Some(Uuid::from_u128(5)));
        assert_eq!(
            app.profiles.sync.choice,
            Some(Choice::Version(Uuid::from_u128(6)))
        );
        app.sync_message(Message::Resolve);
        let Command::SaveProfilePreferences(generation, _, _) = saves.try_recv().unwrap() else {
            panic!("review must save displayed preferences first")
        };
        assert!(input.try_recv().is_err());
        app.sync_saved(generation - 1);
        assert!(input.try_recv().is_err());
        app.sync_saved(generation);
        let Command::Profiles(request) = input.try_recv().unwrap() else {
            panic!()
        };
        assert!(
            matches!(request.action,ProfileAction::Sync(SyncCommand::Resolve{id,choice:Some(Choice::Version(operation)),..}) if id==review.id && operation==Uuid::from_u128(6))
        );
        app.profile_result(
            request.panel,
            request.serial,
            Err("Saved decision needs recovery".into()),
        );
        let Command::Profiles(reload) = input.try_recv().unwrap() else {
            panic!()
        };
        assert!(
            matches!(reload.action,ProfileAction::Sync(SyncCommand::ReviewPage{after:Some(cursor),..}) if cursor==Uuid::from_u128(5))
        );
        app.sync_message(Message::Back);
        assert!(!app.profiles.sync.review_open);
        app.profile_result(
            reload.panel,
            reload.serial,
            Ok(Arc::new(Observation {
                sync: Some(SyncObservation {
                    review: Some(Page {
                        review: Some(review),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            })),
        );
        assert!(
            !app.profiles.sync.review_open,
            "A late page must not reopen a review after Back"
        );
        assert_eq!(
            app.profiles.sync.error.as_deref(),
            Some("Saved decision needs recovery")
        );
    }
}
