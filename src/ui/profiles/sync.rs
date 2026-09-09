use super::*;
use crate::profiles::sync::control::{
    Command as SyncCommand, Observation as SyncObservation, Source,
};
use crate::profiles::sync::resolution::{Choice, Page};
use iced::widget::{checkbox, column};
use shep_profile_core::{Action, SettingKey};
use std::collections::BTreeMap;
mod resolution;

#[derive(Clone, Debug)]
pub enum Message {
    Open,
    Back,
    Prepare(Source),
    Enable(bool),
    Field(SettingKey, bool),
    Check,
    Review(SettingKey),
    ReviewPage(bool),
    Choose(Choice),
    Resolve,
    CancelReview,
    ResumeReview,
}
#[derive(Default)]
pub(super) struct Sync {
    pub visible: bool,
    pub(super) observation: Arc<SyncObservation>,
    pub(super) master: Option<bool>,
    pub(super) fields: BTreeMap<SettingKey, bool>,
    saving: Option<(u64, SyncCommand, Grant)>,
    pub(super) review: Page,
    pub(super) review_open: bool,
    pub(super) choice: Option<Choice>,
    pub(super) error: Option<String>,
}
impl Sync {
    pub(super) fn observe(&mut self, observation: Arc<SyncObservation>) {
        if let (Some(old), Some(new)) = (&self.observation.subscription, &observation.subscription)
        {
            if old.binding == new.binding && old.revision > new.revision {
                return;
            }
            if old.binding != new.binding {
                self.master = None;
                self.fields.clear();
                self.review = Page::default();
                self.review_open = false;
                self.choice = None;
            }
        }
        if let Some(page) = &observation.review {
            if page.review.as_ref().map(|r| r.id) != self.review.review.as_ref().map(|r| r.id) {
                self.choice = None;
            }
            self.review = page.clone();
            if self.review.review.is_none() {
                self.review_open = false;
            }
        }
        self.observation = observation;
    }
    fn settled(&mut self, command: &SyncCommand) {
        match command {
            SyncCommand::Enable { enabled, .. } if self.master == Some(*enabled) => {
                self.master = None
            }
            SyncCommand::Field { key, enabled, .. } if self.fields.get(key) == Some(enabled) => {
                self.fields.remove(key);
            }
            _ => {}
        }
    }
}
impl App {
    pub(in crate::ui) fn sync_saved(&mut self, request: u64) {
        if self
            .profiles
            .sync
            .saving
            .as_ref()
            .is_some_and(|(id, _, _)| *id == request)
        {
            let (_, command, grant) = self.profiles.sync.saving.take().unwrap();
            if grant == Grant::from_preferences(&self.preferences) {
                self.request_profile(ProfileAction::Sync(command));
            }
        }
    }
    pub(in crate::ui) fn sync_save_failed(&mut self, request: u64) {
        if self
            .profiles
            .sync
            .saving
            .as_ref()
            .is_some_and(|(id, _, _)| *id == request)
        {
            self.profiles.sync.saving = None;
            self.profiles.sync.error =
                Some("Save Preferences, then reopen this sync review.".into());
        }
    }
    pub(super) fn sync_message(&mut self, message: Message) {
        match message {
            Message::Back => {
                if self.profiles.sync.review_open {
                    self.profiles.sync.review_open = false;
                    return;
                }
                self.profiles.sync.visible = false;
                return;
            }
            Message::Enable(enabled) => {
                self.profiles.sync.master = Some(enabled);
                self.profiles.sync.error = None;
            }
            Message::Field(key, enabled) => {
                self.profiles.sync.fields.insert(key, enabled);
                self.profiles.sync.error = None;
            }
            Message::Open => {
                self.pause_profiles();
                self.profiles.sync.error = None;
                self.profiles.sync.visible = true;
                self.profiles.sync.review_open = true;
                self.request_profile(ProfileAction::Sync(SyncCommand::Current));
            }
            Message::Prepare(source) => self.sync_save_before(SyncCommand::Prepare(source)),
            Message::Review(key) => {
                if self.profiles.pending.is_some() || self.profiles.sync.saving.is_some() {
                    return;
                }
                self.profiles.sync.review_open = true;
                if let Some(profile) = self.profiles.sync.observation.profile.clone() {
                    self.sync_save_before(SyncCommand::Review { profile, key });
                }
            }
            Message::ReviewPage(next) => {
                let sync = &self.profiles.sync;
                if let Some(review) = &sync.review.review {
                    self.request_profile(ProfileAction::Sync(SyncCommand::ReviewPage {
                        profile: review.profile.clone(),
                        id: review.id,
                        after: if next {
                            sync.review.versions.last().map(|v| v.operation)
                        } else {
                            None
                        },
                    }));
                }
            }
            Message::Choose(choice) => self.profiles.sync.choice = Some(choice),
            Message::Resolve => {
                let sync = &self.profiles.sync;
                if let Some(review) = &sync.review.review {
                    let choice = if review.phase == "staged" {
                        None
                    } else {
                        sync.choice.clone()
                    };
                    self.sync_save_before(SyncCommand::Resolve {
                        profile: review.profile.clone(),
                        id: review.id,
                        choice,
                    });
                }
            }
            Message::CancelReview => {
                if let Some(review) = &self.profiles.sync.review.review {
                    self.request_profile(ProfileAction::Sync(SyncCommand::CancelReview {
                        profile: review.profile.clone(),
                        id: review.id,
                    }));
                }
            }
            Message::ResumeReview => self.profiles.sync.review_open = true,
            Message::Check => {
                if let Some(profile) = self.profiles.sync.observation.profile.clone() {
                    self.request_profile(ProfileAction::Sync(SyncCommand::Check { profile }));
                }
            }
        }
        self.sync_pump();
    }
    fn sync_save_before(&mut self, command: SyncCommand) {
        if self.profiles.pending.is_some() || self.profiles.sync.saving.is_some() {
            return;
        }
        if let Err(error) = self.read_preferences() {
            self.profiles.sync.error = Some(error.to_string());
            return;
        }
        self.pause_profiles();
        self.profiles.sync.visible = true;
        self.profiles.sync.error = None;
        let request = self.preference_sync.changed();
        self.profiles.sync.saving =
            Some((request, command, Grant::from_preferences(&self.preferences)));
        if !self.try_command(Command::SavePreferences(request, self.preferences.clone())) {
            self.sync_save_failed(request);
        }
    }
    pub(super) fn sync_pump(&mut self) {
        if self.profiles.pending.is_some() || self.profiles.sync.saving.is_some() {
            return;
        }
        let sync = &mut self.profiles.sync;
        let Some(s) = &sync.observation.subscription else {
            return;
        };
        let Some(profile) = sync.observation.profile.clone() else {
            return;
        };
        let command = if let Some(enabled) = sync.master {
            Some(SyncCommand::Enable {
                profile,
                revision: s.revision,
                enabled,
            })
        } else if let Some((&key, &enabled)) = sync.fields.first_key_value() {
            Some(SyncCommand::Field {
                profile,
                revision: s.revision,
                key,
                enabled,
            })
        } else {
            None
        };
        if let Some(command) = command {
            self.request_profile(ProfileAction::Sync(command));
        }
    }
    pub(super) fn sync_result(
        &mut self,
        command: SyncCommand,
        result: Result<Arc<Observation>, String>,
    ) {
        self.profiles.sync.settled(&command);
        match result {
            Ok(observation) => {
                if let Some(sync) = &observation.sync {
                    self.sync_background(None, Arc::new(sync.clone()), sync.applied.clone());
                }
                if !matches!(
                    command,
                    SyncCommand::Current | SyncCommand::ReviewPage { .. }
                ) || observation.error.is_some()
                {
                    self.profiles.sync.error = observation.error.clone();
                }
                self.sync_pump();
            }
            Err(error) => {
                self.profiles.sync.error = Some(error);
                // Reload the durable revision before dispatching any newer
                // coalesced choice. Never replay a stale CAS indefinitely.
                match command {
                    SyncCommand::Resolve { profile, id, .. } => {
                        self.request_profile(ProfileAction::Sync(SyncCommand::ReviewPage {
                            profile,
                            id,
                            after: self.profiles.sync.review.after,
                        }));
                    }
                    SyncCommand::Current => {}
                    _ => self.request_profile(ProfileAction::Sync(SyncCommand::Current)),
                }
            }
        }
    }
    pub(in crate::ui) fn sync_background(
        &mut self,
        grant: Option<Grant>,
        observation: Arc<SyncObservation>,
        snapshot: Option<Arc<crate::store::PreferenceSnapshot>>,
    ) {
        if let Some(snapshot) = snapshot {
            let previous = super::super::profile_preferences::Effects::from(&self.preferences);
            self.preference_sync
                .observe((*snapshot).clone(), &mut self.preferences);
            self.portable_preferences.observed(&self.preferences);
            self.update_saved_preferences();
            self.apply_profile_preference_effects(previous);
        }
        if grant.is_none_or(|grant| grant == Grant::from_preferences(&self.preferences)) {
            self.profiles.sync.observe(observation);
        }
    }
    pub(super) fn sync_view(&self) -> Element<'_, super::super::Message> {
        let sync = &self.profiles.sync;
        if sync.review_open && sync.review.review.is_some() {
            return self.sync_resolution_view();
        }
        let message = |m| super::super::Message::Profiles(super::Message::Sync(m));
        let mut body = column![
            button(text("Back").size(12))
                .padding([12, 16])
                .style(outline)
                .on_press(message(Message::Back))
        ]
        .spacing(14);
        if let Some(s) = &sync.observation.subscription {
            let enabled = sync.master.unwrap_or(s.enabled);
            body = body
                .push(text(&s.name).size(16).font(BOLD))
                .push(
                    checkbox(enabled)
                        .label("Keep selected preferences in sync")
                        .on_toggle(move |value| message(Message::Enable(value))),
                )
                .push(
                    muted(if enabled {
                        "Changes continue syncing while you use Shep."
                    } else {
                        "Paused. Saved changes stay queued until you resume."
                    })
                    .size(12),
                );
            body = body.push(
                text(sync.observation.phase.as_deref().unwrap_or(if enabled {
                    "Ready to check shared changes"
                } else {
                    "Sync is paused"
                }))
                .size(12),
            );
            body = body.push(
                text(format!(
                    "Local changes: {} · {} · Needs review: {}",
                    s.pending,
                    sync.observation.queued.map_or_else(
                        || "Uploads not checked".into(),
                        |count| format!("Uploads queued: {count}")
                    ),
                    s.conflicts
                ))
                .size(12),
            );
            body = body.push(
                muted(if s.last_synced.is_some() {
                    "A shared change check has completed on this device."
                } else {
                    "No shared change check has completed yet."
                })
                .size(12),
            );
            if let Some(review) = &sync.review.review {
                body = body.push(
                    button(
                        text(if review.phase == "staged" {
                            "Recover saved preference decision"
                        } else {
                            "Reopen preference review"
                        })
                        .size(12),
                    )
                    .padding([12, 16])
                    .style(outline)
                    .on_press(message(Message::ResumeReview)),
                );
            }
            for field in &sync.observation.fields {
                let key = field.key;
                let selected = sync.fields.get(&key).copied().unwrap_or(field.enabled);
                let shared = match field.shared.as_ref().map(|c| &c.action) {
                    Some(Action::Setting { value, .. }) => publication::formatted_value(key, value),
                    Some(Action::SettingRemoved { .. }) => "Default".into(),
                    _ => "Not shared".into(),
                };
                body = body
                    .push(
                        checkbox(selected)
                            .label(publication::label(key))
                            .on_toggle(move |enabled| message(Message::Field(key, enabled))),
                    )
                    .push(
                        muted(format!(
                            "This device: {} · Profile: {}{}",
                            publication::formatted_value(key, &field.local),
                            shared,
                            if field.pending {
                                " · Local change pending"
                            } else {
                                ""
                            }
                        ))
                        .size(12),
                    );
                if let Some(error) = &field.error {
                    body = body.push(text(error).size(12)).push(
                        button(text("Review preference").size(12))
                            .padding([12, 16])
                            .style(outline)
                            .on_press_maybe(
                                (self.profiles.pending.is_none() && sync.saving.is_none())
                                    .then_some(message(Message::Review(key))),
                            ),
                    );
                }
            }
            body = body.push(
                button(text("Check again / restart scan").size(12))
                    .padding([12, 16])
                    .style(outline)
                    .on_press_maybe(
                        (enabled && self.profiles.pending.is_none())
                            .then_some(message(Message::Check)),
                    ),
            );
            if let Some(error) = &s.error {
                body = body.push(text(error).size(12));
            }
            body=body.push(muted("Preferences with conflicting versions stay unchanged until you review them. Account sync is still being developed.").size(12));
        } else {
            body=body.push(text(if sync.saving.is_some() || self.profiles.pending.is_some() {"Preparing sync choices…"} else {"Complete a profile publication or enrollment review, then choose Sync these preferences."}).size(12));
        }
        if let Some(error) = &sync.error {
            body = body.push(text(error).size(12)).push(
                button(text("Reload saved choices").size(12))
                    .padding([12, 16])
                    .style(outline)
                    .on_press(message(Message::Open)),
            );
        }
        self.settings_card(
            "Profiles and sync",
            "Ongoing preference sync",
            body.width(Length::Fill).into(),
        )
    }
    pub(super) fn sync_prepare_button(&self, source: Source) -> Element<'_, super::super::Message> {
        button(text("Sync these preferences").size(12))
            .padding([12, 16])
            .style(outline)
            .on_press_maybe(self.profiles.pending.is_none().then_some(
                super::super::Message::Profiles(super::Message::Sync(Message::Prepare(source))),
            ))
            .into()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::sync::Subscription;
    fn observation(revision: u64, enabled: bool) -> Arc<Observation> {
        Arc::new(Observation {
            sync: Some(SyncObservation {
                profile: Some("fixture-key".into()),
                subscription: Some(Subscription {
                    binding: shep_profile_core::history::Binding {
                        namespace: "so.shep.fixture".into(),
                        principal: "drive:fixture".into(),
                        profile: Uuid::from_u128(1),
                        generation: Uuid::from_u128(2),
                    },
                    device: Uuid::from_u128(3),
                    name: "Fixture".into(),
                    enabled,
                    revision,
                    history_revision: 1,
                    remote_cursor: 0,
                    remote_device: None,
                    last_synced: None,
                    pending: 0,
                    conflicts: 0,
                    error: None,
                }),
                ..Default::default()
            }),
            ..Default::default()
        })
    }
    #[test]
    fn rapid_master_and_field_choices_survive_late_replies_and_failure() {
        let (mut app, mut input) = super::super::tests::app();
        app.profiles
            .sync
            .observe(Arc::new(observation(1, false).sync.clone().unwrap()));
        app.sync_message(Message::Enable(true));
        let Command::Profiles(first) = input.try_recv().unwrap() else {
            panic!()
        };
        app.sync_message(Message::Enable(false));
        app.sync_message(Message::Field(SettingKey::Appearance, false));
        assert_eq!(app.profiles.sync.master, Some(false));
        assert!(input.try_recv().is_err());
        app.profile_result(first.panel, first.serial, Ok(observation(2, true)));
        let Command::Profiles(second) = input.try_recv().unwrap() else {
            panic!()
        };
        assert!(matches!(
            second.action,
            ProfileAction::Sync(SyncCommand::Enable {
                revision: 2,
                enabled: false,
                ..
            })
        ));
        app.sync_message(Message::Enable(true));
        app.profile_result(
            second.panel,
            second.serial,
            Err("Storage unavailable".into()),
        );
        assert_eq!(app.profiles.sync.master, Some(true));
        let Command::Profiles(reload) = input.try_recv().unwrap() else {
            panic!()
        };
        assert!(matches!(
            reload.action,
            ProfileAction::Sync(SyncCommand::Current)
        ));
        app.profile_result(reload.panel, reload.serial, Ok(observation(3, false)));
        assert_eq!(
            app.profiles.sync.error.as_deref(),
            Some("Storage unavailable")
        );
        let Command::Profiles(third) = input.try_recv().unwrap() else {
            panic!()
        };
        assert!(matches!(
            third.action,
            ProfileAction::Sync(SyncCommand::Enable {
                revision: 3,
                enabled: true,
                ..
            })
        ));
        app.profile_result(third.panel, third.serial, Ok(observation(4, true)));
        let Command::Profiles(field) = input.try_recv().unwrap() else {
            panic!()
        };
        assert!(matches!(
            field.action,
            ProfileAction::Sync(SyncCommand::Field {
                revision: 4,
                enabled: false,
                key: SettingKey::Appearance,
                ..
            })
        ));
        app.sync_message(Message::Back);
        app.profile_result(field.panel, field.serial, Ok(observation(5, true)));
        assert!(app.profiles.sync.fields.is_empty());
        assert!(input.try_recv().is_err());
        app.profiles
            .sync
            .observe(Arc::new(observation(2, false).sync.clone().unwrap()));
        assert!(
            app.profiles
                .sync
                .observation
                .subscription
                .as_ref()
                .unwrap()
                .enabled
        );
    }
    #[test]
    fn background_preferences_preserve_unsaved_intent_then_accept_canonical_ack_without_echo() {
        let (mut app, _) = super::super::tests::app();
        app.preferences.appearance = crate::model::Appearance::Light;
        let original = app.preferences.clone();
        app.preference_sync = super::super::super::preference_sync::PreferenceSync::new(
            crate::store::PreferenceSnapshot {
                revision: 1,
                value: original.clone(),
            },
        );
        app.preferences.appearance = crate::model::Appearance::Dark;
        let generation = app.preference_sync.changed();
        app.portable_preferences
            .prepare(&app.preferences, &original);
        let mut remote = original.clone();
        remote.appearance = crate::model::Appearance::System;
        remote.tooltips = false;
        app.sync_background(
            None,
            Arc::new(SyncObservation::default()),
            Some(Arc::new(crate::store::PreferenceSnapshot {
                revision: 2,
                value: remote.clone(),
            })),
        );
        assert_eq!(app.preferences.appearance, crate::model::Appearance::Dark);
        let mut canonical = remote;
        canonical.appearance = crate::model::Appearance::Dark;
        app.preference_sync.acknowledge(
            generation,
            crate::store::PreferenceSnapshot {
                revision: 3,
                value: canonical.clone(),
            },
            &mut app.preferences,
        );
        app.portable_preferences.observed(&app.preferences);
        assert!(!app.preferences.tooltips);
        assert_eq!(app.preferences.appearance, crate::model::Appearance::Dark);
        app.portable_preferences.accepted();
        assert!(
            app.portable_preferences
                .prepare(&app.preferences, &canonical)
                .is_empty()
        );
        app.sync_background(
            None,
            Arc::new(SyncObservation::default()),
            Some(Arc::new(crate::store::PreferenceSnapshot {
                revision: 1,
                value: original,
            })),
        );
        assert!(!app.preferences.tooltips);
    }
}
