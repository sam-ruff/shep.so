use super::*;
use crate::profiles::publication::{
    Command as PublicationCommand, Phase as PublicationPhase, Specification,
};
use anyhow::Context;
use iced::widget::{checkbox, column, container};
use shep_profile_core::SettingKey;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub enum Message {
    Open,
    Back,
    New,
    Name(String),
    Accounts(bool),
    Setting(SettingKey, bool),
    Prepare,
    Approve,
    Pause,
    Continue,
    Cancel,
    Page(bool),
    Details(Option<String>),
}
pub(super) struct Publication {
    visible: bool,
    pub(super) new_form: bool,
    name: String,
    accounts: bool,
    settings: BTreeSet<SettingKey>,
    pub(super) running: bool,
    saving: Option<(u64, PublicationCommand, Grant)>,
    detail: Option<String>,
}
impl Default for Publication {
    fn default() -> Self {
        Self {
            visible: false,
            new_form: false,
            name: "This device".into(),
            accounts: true,
            settings: crate::profiles::preferences::SUPPORTED
                .into_iter()
                .collect(),
            running: false,
            saving: None,
            detail: None,
        }
    }
}
pub(super) fn label(key: SettingKey) -> &'static str {
    match key {
        SettingKey::Appearance => "Appearance",
        SettingKey::ReplyDisplay => "Quoted history",
        SettingKey::ImagePolicy => "External images",
        SettingKey::UnifiedInbox => "Unified inbox",
        SettingKey::CrossAccountMoves => "Cross-account moves",
        SettingKey::GroupConversations => "Group conversations",
        SettingKey::DesktopBadges => "Unread launcher badge",
        SettingKey::Tooltips => "Tooltips",
        _ => "Unsupported preference",
    }
}
pub(super) fn formatted_value(key: SettingKey, value: &serde_json::Value) -> String {
    if let Some(enabled) = value.as_bool() {
        return if enabled { "On" } else { "Off" }.into();
    }
    match key {
        SettingKey::ReplyDisplay => {
            serde_json::from_value::<crate::model::ReplyDisplay>(value.clone())
                .map(|v| v.to_string())
                .unwrap_or_else(|_| "Unavailable value".into())
        }
        SettingKey::ImagePolicy => {
            serde_json::from_value::<crate::model::ImagePolicy>(value.clone())
                .map(|v| v.to_string())
                .unwrap_or_else(|_| "Unavailable value".into())
        }
        _ => value.as_str().unwrap_or("Unavailable value").into(),
    }
}
impl App {
    pub(in crate::ui) fn publication_pause(&mut self) {
        self.profiles.publication.running = false;
        self.profiles.publication.saving = None;
    }
    pub(in crate::ui) fn publication_save_failed(&mut self, request: u64) {
        if self
            .profiles
            .publication
            .saving
            .as_ref()
            .is_some_and(|(r, _, _)| *r == request)
        {
            self.publication_pause();
            self.profiles.error = Some(
                "Preferences could not be saved. Retry the review after saving your changes."
                    .into(),
            );
        }
    }
    pub(in crate::ui) fn publication_saved(&mut self, request: u64) {
        if self
            .profiles
            .publication
            .saving
            .as_ref()
            .is_none_or(|(r, _, _)| *r != request)
        {
            return;
        }
        let (_, command, grant) = self.profiles.publication.saving.take().unwrap();
        if grant != Grant::from_preferences(&self.preferences) {
            self.profile_grant_changed();
            return;
        }
        self.profiles.publication.running = true;
        self.request_profile(ProfileAction::Publication(command));
    }
    fn publication_save(&mut self, command: PublicationCommand) {
        let request = self.preference_sync.changed();
        self.profiles.publication.saving =
            Some((request, command, Grant::from_preferences(&self.preferences)));
        if !self.try_command(Command::SavePreferences(request, self.preferences.clone())) {
            self.publication_save_failed(request);
        }
    }
    pub(super) fn publication_message(&mut self, message: Message) {
        if matches!(message, Message::Pause | Message::Back) {
            self.publication_pause();
            if matches!(message, Message::Back) {
                self.profiles.publication.visible = false;
            }
            return;
        }
        if self.profiles.pending.is_some() || self.profiles.publication.saving.is_some() {
            return;
        }
        let review = self.profiles.observation.publication.review.clone();
        let result: anyhow::Result<()> = (|| {
            match message {
                Message::Open => {
                    self.profiles.running = false;
                    self.profiles.publication.visible = true;
                    self.request_profile(ProfileAction::Publication(PublicationCommand::Current));
                }
                Message::New => {
                    self.profiles.publication.new_form = true;
                    self.profiles.error = None;
                }
                Message::Name(value) => self.profiles.publication.name = value,
                Message::Accounts(value) => self.profiles.publication.accounts = value,
                Message::Setting(key, value) => {
                    if value {
                        self.profiles.publication.settings.insert(key);
                    } else {
                        self.profiles.publication.settings.remove(&key);
                    }
                }
                Message::Prepare => {
                    self.read_preferences()?;
                    let id = self
                        .profiles
                        .observation
                        .publication
                        .next_id
                        .context("Reopen discovery to prepare a profile.")?;
                    let mut settings = crate::profiles::preferences::export(&self.preferences)?;
                    settings.retain(|key, _| self.profiles.publication.settings.contains(key));
                    let specification = Specification {
                        name: self.profiles.publication.name.trim().into(),
                        include_accounts: self.profiles.publication.accounts,
                        settings,
                    };
                    self.publication_save(PublicationCommand::Prepare { id, specification });
                }
                Message::Approve => {
                    self.read_preferences()?;
                    let review = review.context("Reopen the saved review.")?;
                    let current = crate::profiles::preferences::export(&self.preferences)?;
                    anyhow::ensure!(
                        review
                            .settings
                            .iter()
                            .all(|(k, v)| current.get(k) == Some(v)),
                        "Preferences changed. Cancel this review and prepare another with the current settings."
                    );
                    self.publication_save(PublicationCommand::Approve {
                        id: review.id,
                        settings: review.settings,
                    });
                }
                Message::Continue => {
                    self.profiles.error = None;
                    self.profiles.publication.running = true;
                    self.publication_pump();
                }
                Message::Cancel => {
                    if let Some(review) = review {
                        self.request_profile(ProfileAction::Publication(
                            PublicationCommand::Cancel { id: review.id },
                        ));
                    }
                }
                Message::Page(first) => {
                    if let Some(review) = review {
                        self.profiles.publication.detail = None;
                        let after = if first {
                            0
                        } else {
                            self.profiles
                                .observation
                                .publication
                                .rows
                                .last()
                                .map_or(0, |r| r.position)
                        };
                        self.request_profile(ProfileAction::Publication(
                            PublicationCommand::Accounts {
                                id: review.id,
                                after,
                            },
                        ));
                    }
                }
                Message::Details(id) => self.profiles.publication.detail = id,
                Message::Pause | Message::Back => unreachable!(),
            }
            Ok(())
        })();
        if let Err(error) = result {
            self.profiles.error = Some(error.to_string());
            self.profiles.publication.running = false;
        }
    }
    pub(super) fn publication_pump(&mut self) {
        if !self.profiles.publication.running
            || self.profiles.pending.is_some()
            || self.profiles.publication.saving.is_some()
        {
            return;
        }
        let Some(review) = &self.profiles.observation.publication.review else {
            self.profiles.publication.running = false;
            return;
        };
        let command = match review.phase {
            PublicationPhase::Preparing => PublicationCommand::PrepareStep { id: review.id },
            PublicationPhase::Staging | PublicationPhase::Uploading => {
                PublicationCommand::Step { id: review.id }
            }
            _ => {
                self.profiles.publication.running = false;
                return;
            }
        };
        self.request_profile(ProfileAction::Publication(command));
    }
    pub(super) fn publication_visible(&self) -> bool {
        self.profiles.publication.visible
    }
    pub(super) fn publication_view(&self) -> Element<'_, super::super::Message> {
        let p = &self.profiles;
        let form = &p.publication;
        let observation = &p.observation.publication;
        let busy = p.pending.is_some() || form.saving.is_some();
        let control = |name: &'static str, action, enabled: bool| {
            button(text(name).size(12))
                .padding([12, 16])
                .style(outline)
                .on_press_maybe(enabled.then_some(super::super::Message::Profiles(
                    super::Message::Publication(action),
                )))
        };
        let mut body=column![control("Back to profiles",Message::Back,true),muted("Publish account connection details and selected preferences to your private Google Drive app data. Passwords and mail are not included. Continuous synchronization and desktop import are still in development.").size(12)].spacing(12);
        let review = observation.review.as_ref();
        if form.new_form || review.is_none_or(|r| r.phase == PublicationPhase::Cancelled) {
            body = body
                .push(text("Profile name").size(12).font(BOLD))
                .push(
                    text_input("Profile name", &form.name)
                        .padding(12)
                        .size(12)
                        .style(field)
                        .on_input_maybe((!busy).then_some(|v| {
                            super::super::Message::Profiles(super::Message::Publication(
                                Message::Name(v),
                            ))
                        })),
                )
                .push(
                    checkbox(form.accounts)
                        .label("Include this device's email accounts")
                        .spacing(12)
                        .on_toggle_maybe((!busy).then_some(|v| {
                            super::super::Message::Profiles(super::Message::Publication(
                                Message::Accounts(v),
                            ))
                        })),
                );
            let values =
                crate::profiles::preferences::export(&self.preferences).unwrap_or_default();
            for key in crate::profiles::preferences::SUPPORTED {
                let value = values
                    .get(&key)
                    .map(|value| formatted_value(key, value))
                    .unwrap_or_default();
                body = body.push(
                    container(
                        checkbox(form.settings.contains(&key))
                            .label(format!("{} · {}", label(key), value.trim_matches('"')))
                            .spacing(12)
                            .on_toggle_maybe((!busy).then_some(move |v| {
                                super::super::Message::Profiles(super::Message::Publication(
                                    Message::Setting(key, v),
                                ))
                            })),
                    )
                    .padding([8, 0]),
                );
            }
            if let Some(error) = &p.error {
                body = body.push(text(error).size(12));
            }
            body = body.push(control(
                if busy {
                    "Preparing…"
                } else {
                    "Review profile"
                },
                Message::Prepare,
                !busy && !form.name.trim().is_empty(),
            ));
        } else if let Some(review) = review {
            body = body.push(text(&review.name).size(18).font(BOLD));
            let status = match review.phase {
                PublicationPhase::Preparing => format!(
                    "Preparing accounts · {} of {}",
                    review.prepared, review.accounts
                ),
                PublicationPhase::Review => format!(
                    "Review · {} accounts · {} preferences",
                    review.accounts,
                    review.settings.len()
                ),
                PublicationPhase::Staging => format!(
                    "Saving approved records · {} of {}",
                    review.staged, review.total
                ),
                PublicationPhase::Uploading => format!(
                    "Publishing to Drive · {} of {} confirmed",
                    review.uploaded, review.total
                ),
                PublicationPhase::Complete => {
                    format!("Published · {} records confirmed in Drive", review.uploaded)
                }
                PublicationPhase::Cancelled => "Review cancelled".into(),
            };
            body = body.push(text(status).size(13));
            if review.phase == PublicationPhase::Review {
                body = body.push(
                    row![
                        control("Publish reviewed profile", Message::Approve, !busy),
                        control("Cancel review", Message::Cancel, !busy)
                    ]
                    .spacing(8)
                    .wrap(),
                );
            } else if matches!(
                review.phase,
                PublicationPhase::Preparing
                    | PublicationPhase::Staging
                    | PublicationPhase::Uploading
            ) {
                body = body.push(
                    row![
                        control(
                            if form.running {
                                "Pause publication"
                            } else {
                                "Continue publication"
                            },
                            if form.running {
                                Message::Pause
                            } else {
                                Message::Continue
                            },
                            form.running || !busy
                        ),
                        control(
                            "Cancel review",
                            Message::Cancel,
                            !busy && review.phase == PublicationPhase::Preparing
                        )
                    ]
                    .spacing(8)
                    .wrap(),
                );
                if !form.running {
                    body = body.push(
                        muted(if busy {
                            "Pausing after the current step…"
                        } else {
                            "Progress is saved. Continue when you are ready."
                        })
                        .size(12),
                    );
                }
            } else if review.phase == PublicationPhase::Complete {
                body = body.push(control("Create another profile", Message::New, !busy));
            }
            if let Some(error) = p.error.as_ref().or(review.error.as_ref()) {
                body = body.push(text(error).size(12));
            }
            for (key, value) in &review.settings {
                body = body.push(
                    text(format!(
                        "{} · {}",
                        label(*key),
                        formatted_value(*key, value)
                    ))
                    .size(12),
                );
            }
            for item in &observation.rows {
                let account = &item.account;
                let id = account.id.clone();
                body = body.push(
                    row![
                        column![
                            text(&account.name).size(13).font(BOLD),
                            muted(&account.email).size(12)
                        ]
                        .width(Length::Fill),
                        control("Connection details", Message::Details(Some(id)), !busy)
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                );
                if form.detail.as_ref() == Some(&account.id) {
                    body=body.push(text(format!("Incoming: {}:{} · {} · {}\nUsername: {}\nSMTP: {}:{} · {} · {}\nSMTP username: {}\nSent copies: {} · {}",account.host,account.port,account.protocol,account.incoming_security,account.username,account.smtp_host,account.smtp_port,account.smtp_security(),account.smtp_auth,account.smtp_username(),account.sent_copy,if account.sent_folder.is_empty() {"Automatic folder"} else {&account.sent_folder})).size(12))
                        .push(control("Hide details",Message::Details(None),!busy));
                }
            }
            if observation.after > 0 || observation.rows.len() == 50 {
                body = body.push(
                    row![
                        control(
                            "First accounts",
                            Message::Page(true),
                            !busy && observation.after > 0
                        ),
                        control(
                            "More accounts",
                            Message::Page(false),
                            !busy && observation.rows.len() == 50
                        )
                    ]
                    .spacing(8),
                );
            }
            body=body.push(muted("Accounts published here will need to reconnect on another device. Local mail, drafts and credentials stay on this device.").size(12));
        }
        self.settings_card(
            "Publish a shared profile",
            "Review before sharing",
            body.width(Length::Fill).into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn publication_waits_for_the_exact_preference_save_and_pause_or_failure_cancels_dispatch() {
        for cancellation in 0..4 {
            let (mut app, _) = App::new();
            let (sender, mut profiles, mut saves) =
                engine::CommandSender::profile_save_test_channels();
            app.tx = Some(sender);
            app.tab = Tab::Preferences;
            app.settings_fields();
            let id = Uuid::from_u128(500);
            app.profiles.observation = Arc::new(Observation {
                publication: crate::profiles::publication::Observation {
                    next_id: Some(id),
                    ..Default::default()
                },
                ..Default::default()
            });
            app.publication_message(Message::Prepare);
            let Command::SaveProfilePreferences(request, _, _) = saves.try_recv().unwrap() else {
                panic!()
            };
            assert!(profiles.try_recv().is_err());
            app.publication_saved(request + 1);
            assert!(profiles.try_recv().is_err());
            match cancellation {
                1 => app.publication_pause(),
                2 => app.publication_save_failed(request),
                3 => app.preferences.google_grant.id = "changed-fixture".into(),
                _ => (),
            }
            app.publication_saved(request);
            if cancellation == 0 {
                let Command::Profiles(request) = profiles.try_recv().unwrap() else {
                    panic!()
                };
                assert!(
                    matches!(request.action,ProfileAction::Publication(PublicationCommand::Prepare {id:actual,..}) if actual==id)
                );
            } else {
                assert!(profiles.try_recv().is_err());
            }
            assert!(app.profiles.publication.saving.is_none());
        }
    }
}
