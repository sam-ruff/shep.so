pub mod enrollment;
pub mod publication;
use super::*;
use crate::profiles::discovery::{Action as ProfileAction, Grant, Observation, Request};
use iced::{
    Alignment, Length,
    widget::{button, column, row, text, text_input},
};
use shep_profile_core::drive::catalog::Phase;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub enum Message {
    Publication(publication::Message),
    Enrollment(enrollment::Message),
    Namespace(String),
    Open,
    Pause,
    Continue,
    Retry,
    Refresh(bool),
    Page(bool),
    Close,
}

pub(super) struct Profiles {
    publication: publication::Publication,
    enrollment: enrollment::Enrollment,
    panel: Uuid,
    serial: u64,
    pending: Option<(u64, ProfileAction)>,
    grant: Option<Grant>,
    namespace: String,
    edited: bool,
    loaded: bool,
    running: bool,
    observation: Arc<Observation>,
    error: Option<String>,
}
impl Default for Profiles {
    fn default() -> Self {
        // A process-local correlation ID, not an authorization token. Keep OS
        // randomness and other credential/crypto work out of iced updates.
        static NEXT_PANEL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            publication: Default::default(),
            enrollment: Default::default(),
            panel: Uuid::from_u128(
                NEXT_PANEL.fetch_add(1, std::sync::atomic::Ordering::Relaxed) as u128,
            ),
            serial: 0,
            pending: None,
            grant: None,
            namespace: String::new(),
            edited: false,
            loaded: false,
            running: false,
            observation: Arc::new(Observation::default()),
            error: None,
        }
    }
}
impl App {
    pub(super) fn load_profiles(&mut self) {
        self.profile_grant_changed();
        if !self.profiles.loaded && self.profiles.pending.is_none() {
            self.request_profile(ProfileAction::Load);
        }
    }
    pub(super) fn pause_profiles(&mut self) {
        self.profiles.running = false;
        self.publication_pause();
        self.enrollment_pause();
    }
    pub(super) fn profile_grant_changed(&mut self) {
        if self
            .profiles
            .grant
            .as_ref()
            .is_some_and(|g| *g != Grant::from_preferences(&self.preferences))
        {
            let old = self.profiles.panel;
            let grant = Grant::from_preferences(&self.preferences);
            let namespace = self.profiles.namespace.clone();
            self.profiles = Profiles {
                namespace,
                loaded: self.profiles.loaded,
                edited: self.profiles.edited,
                error: Some("Google setup changed. Open discovery for the current account.".into()),
                ..Default::default()
            };
            self.send(Command::Profiles(Request {
                panel: old,
                serial: 0,
                grant,
                action: ProfileAction::Close,
            }));
        }
    }
    fn request_profile(&mut self, action: ProfileAction) {
        if self.profiles.pending.is_some() {
            return;
        }
        let grant = Grant::from_preferences(&self.preferences);
        self.profiles.serial += 1;
        let serial = self.profiles.serial;
        let command = Command::Profiles(Request {
            panel: self.profiles.panel,
            serial,
            grant: grant.clone(),
            action: action.clone(),
        });
        if self.try_command(command) {
            self.profiles.pending = Some((serial, action));
            self.profiles.grant = Some(grant);
            self.profiles.error = None;
        } else {
            self.pause_profiles();
            self.profiles.error = Some("Profile work could not be queued. Retry shortly.".into());
        }
    }
    pub(super) fn profile_message(&mut self, message: Message) {
        self.profile_grant_changed();
        match message {
            Message::Enrollment(message) => self.enrollment_message(message),
            Message::Publication(message) => self.publication_message(message),
            Message::Namespace(value) => {
                self.profiles.namespace = value;
                self.profiles.edited = true;
            }
            Message::Pause => self.pause_profiles(),
            Message::Open => {
                self.profiles.running = true;
                self.request_profile(ProfileAction::Open {
                    namespace: self.profiles.namespace.trim().into(),
                });
            }
            Message::Continue => {
                self.profiles.running = true;
                self.profile_pump();
            }
            Message::Retry => {
                if !self.profiles.loaded {
                    self.request_profile(ProfileAction::Load);
                    return;
                }
                self.profiles.running = true;
                if let Some(state) = &self.profiles.observation.state {
                    self.request_profile(ProfileAction::Retry {
                        revision: state.revision,
                    });
                } else {
                    self.profile_message(Message::Open);
                }
            }
            Message::Refresh(full) => {
                if let Some(state) = &self.profiles.observation.state {
                    let revision = state.revision;
                    self.profiles.running = true;
                    self.request_profile(ProfileAction::Refresh { revision, full });
                }
            }
            Message::Page(first) => {
                self.profiles.running = false;
                let after = if first {
                    None
                } else {
                    self.profiles.observation.rows.last().map(|p| p.cursor())
                };
                self.request_profile(ProfileAction::Page { after });
            }
            Message::Close => {
                self.pause_profiles();
                self.request_profile(ProfileAction::Close);
            }
        }
    }
    fn profile_pump(&mut self) {
        if self.profiles.running
            && self.profiles.pending.is_none()
            && self
                .profiles
                .observation
                .state
                .as_ref()
                .is_some_and(|s| s.phase != Phase::Complete && s.error.is_none())
        {
            self.request_profile(ProfileAction::Advance);
        }
    }
    pub(super) fn profile_result(
        &mut self,
        panel: Uuid,
        serial: u64,
        result: Result<Arc<Observation>, String>,
    ) {
        self.profile_grant_changed();
        if self.profiles.panel != panel
            || self
                .profiles
                .pending
                .as_ref()
                .is_none_or(|(n, _)| *n != serial)
        {
            return;
        }
        let (_, action) = self.profiles.pending.take().unwrap();
        match result {
            Ok(observation) => {
                if matches!(action, ProfileAction::Load) {
                    if !self.profiles.edited {
                        self.profiles.namespace = observation.namespace.clone();
                    }
                    self.profiles.loaded = true;
                } else {
                    self.profiles.error = observation
                        .error
                        .clone()
                        .or_else(|| observation.state.as_ref().and_then(|s| s.error.clone()));
                    if self.profiles.error.is_some()
                        || observation
                            .state
                            .as_ref()
                            .is_none_or(|s| s.phase == Phase::Complete)
                    {
                        self.profiles.running = false;
                    }
                    if matches!(
                        action,
                        ProfileAction::Publication(
                            crate::profiles::publication::Command::Prepare { .. }
                        )
                    ) && self.profiles.error.is_none()
                    {
                        self.profiles.publication.new_form = false;
                    }
                    if self.profiles.error.is_some() {
                        self.publication_pause();
                        self.enrollment_pause();
                    }
                    self.enrollment_observe_local(&observation);
                    self.profiles.observation = observation;
                }
                self.profile_pump();
                self.publication_pump();
                self.enrollment_pump();
            }
            Err(error) => {
                self.profiles.error = Some(error);
                self.pause_profiles();
            }
        }
    }
    pub(super) fn profile_settings(&self) -> Element<'_, super::Message> {
        if self.profiles.enrollment.visible {
            return self.enrollment_view();
        }
        if self.publication_visible() {
            return self.publication_view();
        }
        let p = &self.profiles;
        let busy = p.pending.is_some();
        let open = p.observation.state.is_some();
        let control = |label: &'static str, action, enabled: bool| {
            button(text(label).size(12))
                .padding([12, 16])
                .style(outline)
                .on_press_maybe(enabled.then_some(super::Message::Profiles(action)))
        };
        let mut body = column![
            muted("Discover accounts and preferences saved in Google's private app data. Use the same Google Cloud project on every device.").size(12),
            text("Application profile namespace").size(12).font(BOLD),
            text_input("Configured by your Google application project", &p.namespace)
                .id("profile-namespace").style(field).padding(12).size(12)
                .on_input_maybe((!busy && !open).then_some(|value| super::Message::Profiles(Message::Namespace(value)))),
        ].spacing(12);
        if !open {
            body = body.push(
                row![
                    control(
                        if busy {
                            "Opening…"
                        } else {
                            "Discover profiles"
                        },
                        Message::Open,
                        !busy && !p.namespace.trim().is_empty()
                    ),
                    button(text("Google connection").size(12))
                        .padding(12)
                        .style(ghost)
                        .on_press(super::Message::FindSetting(
                            SettingsTab::Accounts,
                            "Google connection"
                        )),
                ]
                .spacing(8)
                .wrap(),
            );
        } else {
            let state = p.observation.state.as_ref().unwrap();
            let finished = state.phase == Phase::Complete && state.error.is_none();
            body = body.push(
                row![
                    control(
                        if p.running {
                            "Pause discovery"
                        } else {
                            "Continue discovery"
                        },
                        if p.running {
                            Message::Pause
                        } else {
                            Message::Continue
                        },
                        p.running || (!busy && !finished && p.error.is_none())
                    ),
                    control(
                        "Check for changes",
                        Message::Refresh(false),
                        !busy && finished
                    ),
                    control("Restart scan", Message::Refresh(true), !busy),
                    control("Close discovery", Message::Close, !busy),
                ]
                .spacing(8)
                .wrap(),
            );
            body = body.push(
                text(if p.running {
                    format!(
                        "Discovering · {} profiles · {} files",
                        state.profiles, state.files
                    )
                } else if finished {
                    format!("Discovery complete · {} profiles", state.profiles)
                } else if busy {
                    "Pausing after the current request…".into()
                } else {
                    format!("Discovery paused · {} profiles", state.profiles)
                })
                .size(13),
            );
            if finished && state.profiles == 0 {
                body = body.push(muted("No profiles were found in this application's private Drive space. Check that your devices use the same Google Cloud project before setting up a new profile.").size(12));
            }
            for profile in &p.observation.rows {
                let name = profile.name.as_deref().unwrap_or("Unnamed profile");
                let status = if profile.removed {
                    "Removed"
                } else if profile.conflicts > 0 || profile.name_conflict {
                    "Needs conflict review"
                } else if !finished
                    || !profile.initialized
                    || profile.waiting > 0
                    || profile.ready > 0
                {
                    "Setup incomplete"
                } else {
                    "Available to review"
                };
                body = body.push(
                    button(
                        column![
                            row![
                                text(name).size(14).font(BOLD),
                                muted(status).size(12),
                                muted("Review").size(12)
                            ]
                            .spacing(14)
                            .align_y(Alignment::Center)
                            .wrap(),
                            muted(format!(
                                "{} account{} · {} preference{}",
                                profile.accounts,
                                if profile.accounts == 1 { "" } else { "s" },
                                profile.settings,
                                if profile.settings == 1 { "" } else { "s" }
                            ))
                            .size(12),
                        ]
                        .spacing(6),
                    )
                    .padding([12, 0])
                    .width(Length::Fill)
                    .style(ghost)
                    .on_press_maybe(
                        (!busy && finished && profile.initialized && !profile.removed).then_some(
                            super::Message::Profiles(Message::Enrollment(
                                enrollment::Message::Review(profile.clone()),
                            )),
                        ),
                    ),
                );
            }
            body = body.push(
                row![
                    control(
                        "First page",
                        Message::Page(true),
                        !busy && p.observation.after.is_some()
                    ),
                    control(
                        "Next page",
                        Message::Page(false),
                        !busy && p.observation.rows.len() == 50
                    ),
                ]
                .spacing(8),
            );
            if finished {
                body = body.push(control(
                    "Publish this device's setup",
                    Message::Publication(publication::Message::Open),
                    !busy,
                ));
            }
            if p.observation.enrollment.review.is_some() {
                body = body.push(control(
                    "Open saved enrollment",
                    Message::Enrollment(enrollment::Message::Open),
                    !busy,
                ));
            }
        }
        if let Some(error) = &p.error {
            body = body.push(text(error).size(12)).push(control(
                "Retry discovery",
                Message::Retry,
                !busy,
            ));
        }
        self.settings_card(
            "Profiles and sync",
            "Your shared setups",
            body.width(Length::Fill).into(),
        )
    }
    pub(super) fn profile_observation(&self) -> serde_json::Value {
        serde_json::json!({"pending":self.profiles.pending.is_some(), "running":self.profiles.running,
            "enrollment_running":self.profiles.enrollment.running,"enrollment_visible":self.profiles.enrollment.visible,"publication_running":self.profiles.publication.running, "publication_visible":self.publication_visible(), "loaded":self.profiles.loaded, "namespace":self.profiles.namespace,
            "error":self.profiles.error, "discovery":*self.profiles.observation})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> (App, tokio::sync::mpsc::Receiver<Command>) {
        let (mut app, _) = App::new();
        let (tx, rx) = engine::CommandSender::profile_test_channel();
        app.tx = Some(tx);
        (app, rx)
    }
    #[test]
    fn namespace_edits_survive_late_load_and_one_request_is_in_flight() {
        let (mut app, mut input) = app();
        app.load_profiles();
        let Command::Profiles(request) = input.try_recv().unwrap() else {
            panic!()
        };
        app.profile_message(Message::Namespace("so.shep.edited".into()));
        app.load_profiles();
        assert!(input.try_recv().is_err());
        app.profile_result(
            request.panel,
            request.serial,
            Ok(Arc::new(Observation {
                namespace: "so.shep.saved".into(),
                ..Default::default()
            })),
        );
        assert_eq!(app.profiles.namespace, "so.shep.edited");
        assert!(app.profiles.loaded);
    }
    #[test]
    fn pause_and_navigation_do_not_schedule_more_network_work_and_new_grants_ignore_old_errors() {
        let (mut app, mut input) = app();
        app.profiles.loaded = true;
        app.profile_message(Message::Namespace("so.shep.edited".into()));
        app.profile_message(Message::Open);
        let Command::Profiles(request) = input.try_recv().unwrap() else {
            panic!()
        };
        app.profile_message(Message::Pause);
        app.profile_result(
            request.panel,
            request.serial,
            Ok(Arc::new(Observation {
                state: Some(
                    serde_json::from_value(serde_json::json!({
                        "revision":0,"scan":0,"phase":"initial","files":0,"profiles":0,"pending":0,
                        "incomplete_profiles":0,"completed_revision":null,"error":null
                    }))
                    .unwrap(),
                ),
                ..Default::default()
            })),
        );
        assert!(input.try_recv().is_err());
        app.profile_message(Message::Continue);
        let Command::Profiles(old) = input.try_recv().unwrap() else {
            panic!()
        };
        let _ = app.handle(super::super::Message::Tab(Tab::Mail));
        assert!(!app.profiles.running);
        app.preferences.google_grant.id = "replacement".into();
        app.profile_result(old.panel, old.serial, Err("obsolete-provider-error".into()));
        assert_ne!(app.profiles.panel, old.panel);
        assert!(
            !app.profiles
                .error
                .as_ref()
                .unwrap()
                .contains("obsolete-provider")
        );
        assert!(app.profiles.observation.rows.is_empty());
        let Command::Profiles(close) = input.try_recv().unwrap() else {
            panic!()
        };
        assert!(matches!(close.action, ProfileAction::Close));
        assert!(app.profiles.loaded);
        assert_eq!(app.profiles.namespace, "so.shep.edited");
        app.profile_message(Message::Retry);
        let Command::Profiles(retry) = input.try_recv().unwrap() else {
            panic!()
        };
        assert!(
            matches!(retry.action, ProfileAction::Open { namespace } if namespace == "so.shep.edited")
        );
    }
}
