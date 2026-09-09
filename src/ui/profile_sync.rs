use super::*;
use crate::profile_sync::{
    commands::{Request, Update},
    enrollment::{Changes, Options, Snapshot},
    setup::Discovery,
};
use iced::{
    Alignment, Length,
    widget::{button, checkbox, column, row, space, text, text_input},
};

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    Discover,
    Create,
    Resume,
    Stop,
    Enabled(bool),
    Accounts(bool),
    Settings(bool),
    Name(String),
    CancelReview,
}
#[derive(Default)]
pub(super) struct State {
    snapshot: Option<Arc<Snapshot>>,
    desired: Changes,
    sent: Option<Changes>,
    saving: Option<u64>,
    loading: Option<u64>,
    job: Option<u64>,
    stopping: Option<u64>,
    serial: u64,
    review: Option<Arc<Discovery>>,
    name: String,
    error: Option<String>,
}
impl State {
    fn next(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }
    fn saved_options(&self) -> Options {
        self.snapshot
            .as_ref()
            .map(|s| s.enrollment.options)
            .unwrap_or_default()
    }
    fn options(&self) -> Options {
        self.desired
            .apply(self.sent.unwrap_or_default().apply(self.saved_options()))
    }
    pub fn pending(&self) -> bool {
        self.saving.is_some()
            || !self.desired.empty()
            || self.job.is_some()
            || self.stopping.is_some()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observation(&self) -> serde_json::Value {
        serde_json::json!({"loaded":self.snapshot.is_some(),"available":self.snapshot.as_ref().is_some_and(|s|s.available),
            "options":self.options(),"saving":self.saving.is_some(),"working":self.job.is_some(),"stopping":self.stopping.is_some(),
            "review":self.review.as_ref().map(|r|r.records()),"name":self.name,"error":self.error,
            "enrollment":self.snapshot.as_ref().map(|s|&s.enrollment)})
    }
}
impl App {
    pub(super) fn shared_profile_action(&mut self, action: Action) {
        match action {
            Action::Name(name) => {
                self.profile_sync.name = name;
                return;
            }
            Action::CancelReview => {
                self.profile_sync.review = None;
                return;
            }
            Action::Enabled(value) | Action::Accounts(value) | Action::Settings(value) => {
                let mut changes = self.profile_sync.desired;
                match action {
                    Action::Enabled(_) => changes.enabled = Some(value),
                    Action::Accounts(_) => changes.accounts = Some(value),
                    _ => changes.settings = Some(value),
                }
                let options = changes.apply(self.profile_sync.options());
                if let Err(error) = options.validate() {
                    self.profile_sync.error = Some(error.to_string());
                    return;
                }
                self.profile_sync.desired = changes;
                self.profile_sync.review = None;
                self.save_profile_options();
                return;
            }
            _ => {}
        }
        let id = self.profile_sync.next();
        let request = match action {
            Action::Refresh => {
                if self.profile_sync.loading.is_some() {
                    return;
                }
                Request::Status(id)
            }
            Action::Stop => {
                if self.profile_sync.stopping.is_some() {
                    return;
                }
                Request::Stop(id)
            }
            Action::Discover | Action::Create | Action::Resume => {
                if self.profile_sync.job.is_some()
                    || self.profile_sync.saving.is_some()
                    || !self.profile_sync.desired.empty()
                    || self.preference_sync.dirty()
                {
                    return;
                }
                match action {
                    Action::Discover => Request::Discover(id),
                    Action::Resume => Request::Resume(id),
                    _ => {
                        if self.profile_sync.name.trim().is_empty()
                            || !(self.profile_sync.options().accounts
                                || self.profile_sync.options().settings)
                        {
                            return;
                        }
                        let Some(review) = self.profile_sync.review.clone() else {
                            return;
                        };
                        Request::Create {
                            request: id,
                            review,
                            name: self.profile_sync.name.trim().into(),
                            options: Options {
                                enabled: true,
                                ..self.profile_sync.options()
                            },
                        }
                    }
                }
            }
            _ => return,
        };
        if self.try_command(Command::ProfileSync(request.clone())) {
            let state = &mut self.profile_sync;
            if !matches!(request, Request::Status(_)) {
                state.error = None;
            }
            match request {
                Request::Status(_) => state.loading = Some(id),
                Request::Stop(_) => state.stopping = Some(id),
                _ => {
                    state.job = Some(id);
                    state.review = None;
                }
            }
        } else {
            self.profile_sync.error = Some("Profile work could not be queued. Try again.".into());
            self.pending_close = None;
        }
    }

    fn save_profile_options(&mut self) {
        let state = &mut self.profile_sync;
        if state.saving.is_some() {
            return;
        }
        let Some(snapshot) = &state.snapshot else {
            return;
        };
        let changes = state.desired;
        if changes.empty() {
            return;
        }
        if changes.apply(snapshot.enrollment.options) == snapshot.enrollment.options {
            state.desired = Changes::default();
            return;
        }
        let request = state.next();
        if self.try_command(Command::ProfileSync(Request::Change { request, changes })) {
            self.profile_sync.saving = Some(request);
            self.profile_sync.sent = Some(changes);
            self.profile_sync.desired = Changes::default();
            self.profile_sync.error = None;
        } else {
            self.profile_sync.desired = Changes::default();
            self.profile_sync.error = Some("Profile choices could not be saved. Try again.".into());
            self.pending_close = None;
        }
    }

    pub(super) fn shared_profile_update(&mut self, id: u64, update: Update) -> Task<Message> {
        let state = &mut self.profile_sync;
        let saving = state.saving == Some(id);
        let loading = state.loading == Some(id);
        let job = state.job == Some(id);
        let stopping = state.stopping == Some(id);
        if !saving && !loading && !job && !stopping {
            return Task::none();
        }
        let pending = matches!(update, Update::Pending(_));
        let published = matches!(update, Update::Published(_));
        if saving {
            state.saving = None;
            state.sent = None;
        }
        if loading {
            state.loading = None;
        }
        if job && !pending {
            state.job = None;
        }
        if stopping {
            state.stopping = None;
        }
        let mut refresh = false;
        match update {
            Update::Status(snapshot) | Update::Published(snapshot) | Update::Pending(snapshot) => {
                if state.snapshot.as_ref().is_none_or(|s| {
                    snapshot.enrollment.revision >= s.enrollment.revision
                        && snapshot.preferences_revision >= s.preferences_revision
                        && snapshot.connections_revision >= s.connections_revision
                        && snapshot.google_revision >= s.google_revision
                }) {
                    state.snapshot = Some(snapshot);
                }
                if job && published {
                    self.notice("Profile created on Google Drive", false);
                }
            }
            Update::Review(review) => {
                if state.desired.empty() && state.saving.is_none() {
                    state.snapshot = Some(Arc::new(review.local().clone()));
                    state.review = Some(review);
                    if state.name.is_empty() {
                        state.name = "Personal".into();
                    }
                } else {
                    refresh = true;
                }
            }
            Update::Failed(error) => {
                state.error = Some(error);
                self.pending_close = None;
                refresh = !loading;
            }
            Update::Stopped => {
                refresh = true;
            }
        }
        if refresh {
            self.shared_profile_action(Action::Refresh);
        } else {
            self.save_profile_options();
        }
        if !self.profile_sync.pending()
            && let Some(window) = self.pending_close.take()
        {
            return self.handle(Message::WindowClose(window));
        }
        Task::none()
    }

    pub(super) fn shared_profile_card(&self) -> Element<'_, Message> {
        use components::{action, muted, outline};
        let state = &self.profile_sync;
        let msg = |a| Message::ProfileSync(a);
        let options = state.options();
        let available = state.snapshot.as_ref().is_some_and(|s| s.available);
        let selected = state
            .snapshot
            .as_ref()
            .and_then(|s| s.enrollment.selection.as_ref());
        let idle = state.job.is_none() && state.saving.is_none() && !self.preference_sync.dirty();
        let controls = column![
            checkbox(options.accounts)
                .label("Account definitions")
                .on_toggle(move |v| msg(Action::Accounts(v)))
                .text_size(13),
            checkbox(options.settings)
                .label("Appearance and mail preferences")
                .on_toggle(move |v| msg(Action::Settings(v)))
                .text_size(13),
        ]
        .spacing(16);
        let mut body = column![].spacing(16);
        if let Some(selected) = selected {
            body = body.push(text(&selected.name).size(16).font(BOLD));
            body = body.push(
                checkbox(options.enabled)
                    .label("Enable profile sync on this device")
                    .on_toggle(move |v| msg(Action::Enabled(v)))
                    .text_size(13),
            );
            body = body.push(controls);
            if selected.ready {
                body = body.push(
                    muted("Initial profile saved. Continuous updates are still being implemented.")
                        .size(12),
                );
            } else {
                body = body.push(
                    muted("Setup is pending. Resume to finish the original saved copy.").size(12),
                );
                body = body.push(
                    button(text("Resume setup").size(13))
                        .padding([12, 16])
                        .style(outline)
                        .on_press_maybe(
                            (idle && available && options.enabled).then(|| msg(Action::Resume)),
                        ),
                );
            }
        } else {
            body = body.push(controls);
            if !available {
                body = body
                    .push(
                        muted("Connect Google with Drive permission to share a profile.").size(12),
                    )
                    .push(action(
                        "Google connection",
                        Message::FindSetting(SettingsTab::Accounts, "Google connection"),
                    ));
            } else if let Some(review) = &state.review {
                body = body
                    .push(
                        muted(if review.records() == 0 {
                            "No shared profiles found. Create one from this workspace?"
                        } else {
                            "Shared profile data exists. A separate profile keeps it intact."
                        })
                        .size(12),
                    )
                    .push(
                        text_input("Profile name", &state.name)
                            .id("shared-profile-name")
                            .style(components::field)
                            .padding(12)
                            .size(13)
                            .on_input(move |v| msg(Action::Name(v)))
                            .on_submit(msg(Action::Create)),
                    )
                    .push(
                        muted(format!(
                            "{} account definitions · reconnect accounts on other devices",
                            if options.accounts {
                                review.local().accounts
                            } else {
                                0
                            }
                        ))
                        .size(12),
                    )
                    .push(
                        row![
                            button(text("Create shared profile").size(13))
                                .padding([12, 16])
                                .style(components::primary)
                                .on_press_maybe(
                                    (idle
                                        && !state.name.trim().is_empty()
                                        && (options.accounts || options.settings))
                                        .then(|| msg(Action::Create))
                                ),
                            action("Not now", msg(Action::CancelReview))
                        ]
                        .spacing(10),
                    );
            } else {
                body = body.push(
                    button(text("Discover profiles").size(13))
                        .padding([12, 16])
                        .style(outline)
                        .on_press_maybe(idle.then(|| msg(Action::Discover))),
                );
            }
        }
        if state.job.is_some() {
            body = body.push(
                row![
                    muted("Checking shared profiles…").size(12),
                    space().width(Length::Fill),
                    button(
                        text(if state.stopping.is_some() {
                            "Stopping…"
                        } else {
                            "Stop"
                        })
                        .size(12)
                    )
                    .padding([11, 14])
                    .style(outline)
                    .on_press_maybe(state.stopping.is_none().then(|| msg(Action::Stop)))
                ]
                .align_y(Alignment::Center),
            );
        }
        if let Some(error) = &state.error {
            body = body.push(
                text(error)
                    .size(12)
                    .color(iced::Color::from_rgb8(205, 72, 82)),
            );
        }
        if state.snapshot.is_none() || state.error.is_some() {
            body = body.push(action("Refresh status", msg(Action::Refresh)));
        }
        self.settings_card(
            "Profiles and sync",
            "Share account definitions and portable preferences through Google Drive.",
            body.into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{engine::CommandSender, store::Store};

    async fn app() -> (App, tokio::sync::mpsc::Receiver<Command>, Arc<Snapshot>) {
        let original = Arc::new(Store::memory().unwrap().profile_enrollment().await.unwrap());
        let (sender, queue) = CommandSender::profile_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.profile_sync.snapshot = Some(original.clone());
        (app, queue, original)
    }

    #[tokio::test]
    async fn profile_controls_keep_newer_repeated_choices_after_an_older_failure() {
        let (mut app, mut queue, original) = app().await;
        app.shared_profile_action(Action::Accounts(false));
        let Command::ProfileSync(Request::Change { request, changes }) = queue.try_recv().unwrap()
        else {
            panic!("field change")
        };
        assert_eq!(changes.accounts, Some(false));
        assert_eq!(changes.enabled, None);
        app.shared_profile_action(Action::Accounts(true));
        app.shared_profile_action(Action::Accounts(false));
        app.shared_profile_action(Action::Settings(false));
        assert!(!app.profile_sync.options().accounts);
        assert!(!app.profile_sync.options().settings);
        assert!(queue.try_recv().is_err());
        let _ = app.shared_profile_update(request, Update::Failed("Disk unavailable".into()));
        let Command::ProfileSync(Request::Status(refresh)) = queue.try_recv().unwrap() else {
            panic!("refresh")
        };
        assert!(!app.profile_sync.options().accounts);
        let _ = app.shared_profile_update(refresh, Update::Status(original.clone()));
        let Command::ProfileSync(Request::Change {
            request: retry,
            changes,
        }) = queue.try_recv().unwrap()
        else {
            panic!("newest choices")
        };
        assert_eq!(changes.accounts, Some(false));
        assert_eq!(changes.settings, Some(false));
        let mut saved = (*original).clone();
        saved.enrollment.options = changes.apply(saved.enrollment.options);
        saved.enrollment.revision += 1;
        let _ = app.shared_profile_update(retry, Update::Status(Arc::new(saved)));
        assert!(!app.profile_sync.pending());
        assert!(queue.try_recv().is_err());
    }

    #[tokio::test]
    async fn profile_pending_receipt_keeps_job_owned_and_newer_options_visible() {
        let (mut app, mut queue, original) = app().await;
        app.profile_sync.job = Some(900);
        app.shared_profile_action(Action::Accounts(false));
        let Command::ProfileSync(Request::Change { request, .. }) = queue.try_recv().unwrap()
        else {
            panic!("change")
        };
        let mut pending = (*original).clone();
        pending.enrollment.revision = 1;
        pending.enrollment.options.enabled = true;
        let _ = app.shared_profile_update(900, Update::Pending(Arc::new(pending.clone())));
        assert_eq!(app.profile_sync.job, Some(900));
        assert!(app.profile_sync.options().enabled);
        assert!(!app.profile_sync.options().accounts);
        assert!(app.notice.is_none());
        pending.enrollment.options.accounts = false;
        pending.enrollment.revision = 2;
        let _ = app.shared_profile_update(request, Update::Status(Arc::new(pending)));
        let _ = app.shared_profile_update(900, Update::Stopped);
        let Command::ProfileSync(Request::Status(refresh)) = queue.try_recv().unwrap() else {
            panic!("refresh")
        };
        let _ = app.shared_profile_update(refresh, Update::Status(original));
        assert_eq!(
            app.profile_sync
                .snapshot
                .as_ref()
                .unwrap()
                .enrollment
                .revision,
            2
        );
        assert!(!app.profile_sync.options().accounts);
        assert!(!app.profile_sync.pending());
    }
}
