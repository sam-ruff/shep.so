use super::*;
use crate::profiles::enrollment::{Command as EnrollmentCommand, Row};
use iced::widget::{checkbox, column};
use shep_profile_core::{SettingKey, drive::catalog::Profile};
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub enum Message {
    Review(Profile),
    Open,
    Back,
    Pause,
    Continue,
    Cancel,
    Approve,
    Accounts(bool),
    Settings(bool),
    Choose(u64, bool),
    Page(bool),
    Details(Option<u64>),
}
#[derive(Default)]
pub(super) struct Enrollment {
    pub visible: bool,
    pub running: bool,
    saving: Option<(u64, EnrollmentCommand, Grant)>,
    accounts: bool,
    settings: bool,
    detail: Option<u64>,
    versions: BTreeMap<SettingKey, u64>,
}
impl App {
    pub(in crate::ui) fn enrollment_pause(&mut self) {
        self.profiles.enrollment.running = false;
        self.profiles.enrollment.saving = None;
    }
    pub(in crate::ui) fn enrollment_saved(&mut self, request: u64) {
        if self
            .profiles
            .enrollment
            .saving
            .as_ref()
            .is_none_or(|(r, _, _)| *r != request)
        {
            return;
        }
        let (_, command, grant) = self.profiles.enrollment.saving.take().unwrap();
        if grant != Grant::from_preferences(&self.preferences) {
            self.profile_grant_changed();
            return;
        }
        self.profiles.enrollment.running = true;
        self.request_profile(ProfileAction::Enrollment(command));
    }
    pub(in crate::ui) fn enrollment_save_failed(&mut self, request: u64) {
        if self
            .profiles
            .enrollment
            .saving
            .as_ref()
            .is_some_and(|(r, _, _)| *r == request)
        {
            self.enrollment_pause();
            self.profiles.error = Some("Save Preferences, then retry this profile review.".into());
        }
    }
    fn enrollment_save(&mut self, command: EnrollmentCommand) {
        let request = self.preference_sync.changed();
        self.profiles.enrollment.saving =
            Some((request, command, Grant::from_preferences(&self.preferences)));
        if !self.try_command(Command::SavePreferences(request, self.preferences.clone())) {
            self.enrollment_save_failed(request);
        }
    }
    pub(super) fn enrollment_message(&mut self, message: Message) {
        if matches!(message, Message::Pause | Message::Back) {
            self.enrollment_pause();
            if matches!(message, Message::Back) {
                self.profiles.enrollment.visible = false;
            }
            return;
        }
        if self.profiles.pending.is_some() || self.profiles.enrollment.saving.is_some() {
            return;
        }
        let review = self.profiles.observation.enrollment.review.clone();
        match message {
            Message::Review(profile) => {
                let Some(id) = self.profiles.observation.enrollment.next_id else {
                    return;
                };
                if let Err(error) = self.read_preferences() {
                    self.profiles.error = Some(error.to_string());
                    return;
                }
                self.profiles.enrollment.visible = true;
                self.profiles.enrollment.accounts = true;
                self.profiles.enrollment.settings = true;
                self.enrollment_save(EnrollmentCommand::Prepare {
                    id,
                    profile: profile.profile,
                    generation: profile.generation,
                    revision: profile.revision,
                });
                self.profiles.enrollment.versions = self.portable_preferences.versions.clone();
            }
            Message::Open => {
                self.profiles.enrollment.visible = true;
                if let Some(review) = review {
                    self.profiles.enrollment.accounts = review.include_accounts;
                    self.profiles.enrollment.settings = review.include_settings;
                }
                self.request_profile(ProfileAction::Enrollment(EnrollmentCommand::Current));
            }
            Message::Continue => {
                self.profiles.error = None;
                self.profiles.enrollment.running = true;
                self.enrollment_pump();
            }
            Message::Accounts(value) => self.profiles.enrollment.accounts = value,
            Message::Settings(value) => self.profiles.enrollment.settings = value,
            Message::Details(position) => self.profiles.enrollment.detail = position,
            Message::Approve => {
                if let Some(review) = review {
                    self.enrollment_save(EnrollmentCommand::Approve {
                        id: review.id,
                        accounts: self.profiles.enrollment.accounts,
                        settings: self.profiles.enrollment.settings,
                    });
                }
            }
            Message::Cancel => {
                if let Some(review) = review {
                    self.request_profile(ProfileAction::Enrollment(EnrollmentCommand::Cancel {
                        id: review.id,
                    }));
                }
            }
            Message::Choose(position, selected) => {
                if let Some(review) = review {
                    self.request_profile(ProfileAction::Enrollment(EnrollmentCommand::Choose {
                        id: review.id,
                        position,
                        selected,
                    }));
                }
            }
            Message::Page(first) => {
                if let Some(review) = review {
                    self.profiles.enrollment.detail = None;
                    let after = if first {
                        0
                    } else {
                        self.profiles
                            .observation
                            .enrollment
                            .rows
                            .last()
                            .map_or(0, |r| r.position)
                    };
                    self.request_profile(ProfileAction::Enrollment(EnrollmentCommand::Rows {
                        id: review.id,
                        after,
                    }));
                }
            }
            Message::Pause | Message::Back => {}
        }
    }
    pub(super) fn enrollment_pump(&mut self) {
        if !self.profiles.enrollment.running
            || self.profiles.pending.is_some()
            || self.profiles.enrollment.saving.is_some()
        {
            return;
        }
        if let Some(review) = &self.profiles.observation.enrollment.review {
            if matches!(
                review.phase.as_str(),
                "copying" | "draining" | "fields" | "planning" | "applying" | "settings"
            ) {
                self.request_profile(ProfileAction::Enrollment(EnrollmentCommand::Step {
                    id: review.id,
                }));
            } else {
                self.profiles.enrollment.running = false;
            }
        }
    }
    pub(super) fn enrollment_observe_local(&mut self, observation: &Observation) {
        let Some(local) = &observation.enrollment.local else {
            return;
        };
        if local.connections_revision >= self.workspace.connections_revision {
            let workspace = Arc::make_mut(&mut self.workspace);
            workspace.accounts = local.accounts.clone();
            workspace.connections_revision = local.connections_revision;
            workspace.profile_reconnect = local.reconnect.clone();
        }
        if local.preferences.revision <= self.preference_sync.saved.revision {
            return;
        }
        let previous = super::super::profile_preferences::Effects::from(&self.preferences);
        // Preserve newer UI intent even before its FIFO persistence acknowledgement.
        let live =
            crate::profiles::preferences::export(&self.preferences).expect("typed preferences");
        let protected: Vec<_> = self
            .portable_preferences
            .versions
            .iter()
            .filter(|(k, v)| {
                **v > self
                    .profiles
                    .enrollment
                    .versions
                    .get(k)
                    .copied()
                    .unwrap_or(0)
            })
            .map(|(k, _)| *k)
            .collect();
        self.preference_sync
            .observe(local.preferences.clone(), &mut self.preferences);
        if let Some(applied) = observation
            .enrollment
            .review
            .as_ref()
            .and_then(|r| r.settings_receipt.as_ref())
            .and_then(|r| r["applied"].as_object())
        {
            for (name, _) in applied {
                if let Ok(key) =
                    serde_json::from_value::<SettingKey>(serde_json::Value::String(name.clone()))
                    && !protected.contains(&key)
                {
                    let current = crate::profiles::preferences::export(&local.preferences.value)
                        .expect("typed preferences");
                    let _ = crate::profiles::preferences::apply(
                        &mut self.preferences,
                        key,
                        current.get(&key),
                    );
                    if let Some(value) = current.get(&key) {
                        self.portable_preferences.remote(key, value.clone());
                    }
                }
            }
        }
        for key in protected {
            let _ = crate::profiles::preferences::apply(&mut self.preferences, key, live.get(&key));
        }
        self.update_saved_preferences();
        self.apply_profile_preference_effects(previous);
    }
    pub(super) fn enrollment_view(&self) -> Element<'_, super::super::Message> {
        let p = &self.profiles;
        let e = &p.enrollment;
        let observation = &p.observation.enrollment;
        let busy = p.pending.is_some() || e.saving.is_some();
        let message = |m| super::super::Message::Profiles(super::Message::Enrollment(m));
        let control = |label: &'static str, m, enabled: bool| {
            button(text(label).size(12))
                .padding([12, 16])
                .style(outline)
                .on_press_maybe(enabled.then_some(message(m)))
        };
        let mut body=column![control("Back to profiles",Message::Back,true),muted("Review the accounts and preferences to use on this device. Imported accounts need passwords here before they can connect.").size(12)].spacing(12);
        if let Some(review) = &observation.review {
            body = body.push(
                text(review.name.as_deref().unwrap_or("Shared profile"))
                    .size(18)
                    .font(BOLD),
            );
            let reviewing = review.phase == "review";
            body = body.push(
                text(match review.phase.as_str() {
                    "copying" => format!(
                        "Preparing review · {} of {} records",
                        review.copied, review.total
                    ),
                    "draining" | "fields" | "planning" => {
                        format!("Preparing review · {} items", review.rows)
                    }
                    "review" => format!("Review {} items before applying", review.rows),
                    "complete" => format!(
                        "Profile applied · {} {} applied · {} skipped",
                        review.applied,
                        if review.applied == 1 {
                            "account"
                        } else {
                            "accounts"
                        },
                        review.kept
                    ),
                    "cancelled" => "Review cancelled".into(),
                    _ => format!(
                        "Applying · {} {} applied · {} skipped",
                        review.applied,
                        if review.applied == 1 {
                            "account"
                        } else {
                            "accounts"
                        },
                        review.kept
                    ),
                })
                .size(13),
            );
            if review.phase == "complete" && review.include_settings {
                body = body.push(self.sync_prepare_button(
                    crate::profiles::sync::control::Source::Enrollment(review.id),
                ));
            }
            if reviewing {
                body =
                    body.push(checkbox(e.accounts).label("Accounts").on_toggle_maybe(
                        (!busy).then_some(move |v| message(Message::Accounts(v))),
                    ));
                body =
                    body.push(checkbox(e.settings).label("Preferences").on_toggle_maybe(
                        (!busy).then_some(move |v| message(Message::Settings(v))),
                    ));
                body = body.push(
                    row![
                        control("Apply selected items", Message::Approve, !busy),
                        control("Cancel review", Message::Cancel, !busy)
                    ]
                    .spacing(8)
                    .wrap(),
                );
            } else if !matches!(review.phase.as_str(), "complete" | "cancelled") {
                body = body.push(
                    row![
                        control(
                            if e.running { "Pause" } else { "Continue" },
                            if e.running {
                                Message::Pause
                            } else {
                                Message::Continue
                            },
                            e.running || !busy
                        ),
                        control(
                            "Cancel review",
                            Message::Cancel,
                            !busy
                                && matches!(
                                    review.phase.as_str(),
                                    "copying" | "draining" | "fields" | "planning"
                                )
                        )
                    ]
                    .spacing(8)
                    .wrap(),
                );
            }
            if let Some(error) = p.error.as_ref().or(review.error.as_ref()) {
                body = body.push(text(error).size(12));
            }
            if review.phase == "complete" {
                body=body.push(muted("Reconnect imported accounts in Preferences → Accounts. Existing mail, drafts and passwords were preserved. Preferences edited during this review were kept.").size(12));
            }
            for r in &observation.rows {
                let mut item = column![
                    row![
                        checkbox(r.selected).label(row_label(r)).on_toggle_maybe(
                            (reviewing && !busy && r.available)
                                .then_some(move |v| message(Message::Choose(r.position, v)))
                        ),
                        control(
                            if e.detail == Some(r.position) {
                                "Hide details"
                            } else {
                                "Details"
                            },
                            Message::Details(if e.detail == Some(r.position) {
                                None
                            } else {
                                Some(r.position)
                            }),
                            !busy
                        )
                    ]
                    .spacing(8)
                    .wrap()
                ]
                .spacing(6);
                if let Some(reason) = &r.reason {
                    item = item.push(muted(reason).size(12));
                }
                if let Some(receipt) = &r.receipt {
                    item = item.push(
                        muted(if receipt == "applied" {
                            "Applied"
                        } else if r.new_account {
                            "Not imported"
                        } else {
                            "Kept on this device"
                        })
                        .size(12),
                    );
                }
                if e.detail == Some(r.position) {
                    if let Some(a) = &r.account {
                        item=item.push(text(format!("{}\n{}: {}:{} · {} · {}\nUsername: {}\nSMTP: {}:{} · {} · {}\nSMTP username: {} · {}\nSent copies: {} · {}", a.email,a.protocol,a.host,a.port,a.incoming_security,a.incoming_auth,a.username,a.smtp_host,a.smtp_port,a.smtp_security(),a.smtp_auth,a.smtp_username(),if a.smtp_separate_password {"Separate password"}else{"Account password"},a.sent_copy,if a.sent_folder.is_empty(){"Automatic folder"}else{&a.sent_folder})).size(12));
                    } else {
                        item = item.push(text(row_label(r)).size(12));
                    }
                }
                body = body.push(item.padding([8, 0]));
            }
            body = body.push(
                row![
                    control(
                        "First page",
                        Message::Page(true),
                        !busy && observation.after > 0
                    ),
                    control(
                        "Next page",
                        Message::Page(false),
                        !busy && observation.rows.len() == 50
                    )
                ]
                .spacing(8),
            );
        } else if let Some(error) = &p.error {
            body = body.push(text(error).size(12));
        }
        self.settings_card(
            "Profiles and sync",
            "Use a shared profile",
            body.width(Length::Fill).into(),
        )
    }
}
fn row_label(row: &Row) -> String {
    if let Some(account) = &row.account {
        return format!("{} · {}", account.name, account.email);
    }
    if let Some(name) = row.target.strip_prefix("setting:")
        && let Ok(key) =
            serde_json::from_value::<SettingKey>(serde_json::Value::String(name.into()))
    {
        return format!(
            "{} · {}",
            publication::label(key),
            if row.value.is_null() {
                "Default".into()
            } else {
                publication::formatted_value(key, &row.value)
            }
        );
    }
    "Unavailable profile item".into()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enrollment_waits_for_exact_saved_preferences_and_pause_failure_or_grant_change_cancels_dispatch()
     {
        for cancellation in 0..4 {
            let (mut app, _) = App::new();
            let (sender, mut profiles, mut saves) =
                engine::CommandSender::profile_save_test_channels();
            app.tx = Some(sender);
            app.enrollment_save(EnrollmentCommand::Prepare {
                id: Uuid::from_u128(500),
                profile: Uuid::from_u128(501),
                generation: Uuid::from_u128(502),
                revision: 3,
            });
            let Command::SaveProfilePreferences(request, _, _) = saves.try_recv().unwrap() else {
                panic!()
            };
            app.enrollment_saved(request + 1);
            assert!(profiles.try_recv().is_err());
            match cancellation {
                1 => app.enrollment_pause(),
                2 => app.enrollment_save_failed(request),
                3 => app.preferences.google_grant.id = "changed".into(),
                _ => {}
            }
            app.enrollment_saved(request);
            if cancellation == 0 {
                let Command::Profiles(request) = profiles.try_recv().unwrap() else {
                    panic!()
                };
                assert!(matches!(
                    request.action,
                    ProfileAction::Enrollment(EnrollmentCommand::Prepare { .. })
                ));
            } else {
                assert!(profiles.try_recv().is_err());
            }
        }
    }
    #[test]
    fn local_preference_edits_survive_an_older_enrollment_reply_and_other_selected_fields_apply() {
        use crate::{
            model::Appearance,
            profiles::enrollment::{Local, Observation as EnrollmentObservation, Review},
        };
        use serde_json::json;
        let (mut app, _) = App::new();
        let (sender, _profiles, _saves) = engine::CommandSender::profile_save_test_channels();
        app.tx = Some(sender);
        let baseline = app.preferences.clone();
        app.preferences.appearance = Appearance::Light;
        app.portable_preferences
            .prepare(&app.preferences, &baseline);
        app.portable_preferences.accepted();
        app.preference_sync.changed();
        let remote = Preferences {
            appearance: Appearance::Dark,
            unified_inbox: false,
            ..baseline
        };
        let observation = Observation {
            enrollment: EnrollmentObservation {
                review: Some(Review {
                    id: Uuid::from_u128(1),
                    binding: shep_profile_core::history::Binding {
                        namespace: "so.shep.fixture".into(),
                        principal: "drive:fixture".into(),
                        profile: Uuid::from_u128(2),
                        generation: Uuid::from_u128(3),
                    },
                    name: None,
                    phase: "complete".into(),
                    copied: 3,
                    total: 3,
                    cursor: 3,
                    history_revision: 3,
                    field_after: None,
                    rows: 2,
                    applied: 0,
                    kept: 0,
                    include_accounts: false,
                    include_settings: true,
                    baseline: Default::default(),
                    settings_receipt: Some(
                        json!({"applied":{"appearance":"Dark","unified_inbox":false},"kept":[]}),
                    ),
                    error: None,
                }),
                local: Some(Local {
                    preferences: PreferenceSnapshot {
                        revision: 2,
                        value: remote,
                    },
                    accounts: vec![],
                    connections_revision: 0,
                    reconnect: Default::default(),
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        app.enrollment_observe_local(&observation);
        assert_eq!(app.preferences.appearance, Appearance::Light);
        assert!(!app.preferences.unified_inbox);
        assert!(
            app.portable_preferences
                .prepare(&app.preferences, &app.preference_sync.saved.value)
                .is_empty()
        );
        app.enrollment_observe_local(&observation);
        assert_eq!(app.preferences.appearance, Appearance::Light);
    }
}
