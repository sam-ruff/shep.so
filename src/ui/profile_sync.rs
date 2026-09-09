mod onboarding;
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
    AfterLogin,
    AutoJoin,
    Automatic(bool),
    NotNow,
    Discover,
    Create,
    Page(Option<String>),
    Choose(String),
    AcceptJoin,
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
    login_pending: Option<u64>,
    login_seen: Option<u64>,
    automatic_generation: Option<u64>,
    automatic_job: Option<u64>,
    offer: bool,
    desired: Changes,
    sent: Option<Changes>,
    saving: Option<u64>,
    loading: Option<u64>,
    job: Option<u64>,
    stopping: Option<u64>,
    serial: u64,
    review: Option<Arc<Discovery>>,
    join_review: Option<Arc<crate::profile_sync::join::Review>>,
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
    fn accepts_snapshot(&self, snapshot: &Snapshot) -> bool {
        self.snapshot.as_ref().is_none_or(|s| {
            snapshot.enrollment.revision >= s.enrollment.revision
                && snapshot.preferences_revision >= s.preferences_revision
                && snapshot.connections_revision >= s.connections_revision
                && snapshot.google_revision >= s.google_revision
        })
    }
    pub fn pending(&self) -> bool {
        self.saving.is_some()
            || (!self.desired.empty() && self.error.is_none())
            || self.job.is_some()
            || self.stopping.is_some()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observation(&self) -> serde_json::Value {
        serde_json::json!({"loaded":self.snapshot.is_some(),"available":self.snapshot.as_ref().is_some_and(|s|s.available),"empty_workspace":self.snapshot.as_ref().is_some_and(|s|s.empty_workspace),
            "options":self.options(),"offer":self.offer,"login_pending":self.login_pending.is_some(),"saving":self.saving.is_some(),"working":self.job.is_some(),"stopping":self.stopping.is_some(),
            "review":self.review.as_ref().map(|r|r.records()),
            "profiles":self.review.as_ref().map(|r|r.profiles()),
            "join_review":self.join_review.as_ref().map(|r|serde_json::json!({"name":r.name(),"accounts":r.accounts,"settings":r.settings})),"name":self.name,"error":self.error,
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
            Action::NotNow => {
                self.profile_sync.offer = false;
                self.shared_profile_action(Action::Automatic(false));
                return;
            }
            Action::CancelReview => {
                self.profile_sync.review = None;
                self.profile_sync.join_review = None;
                return;
            }
            Action::Enabled(value)
            | Action::Accounts(value)
            | Action::Settings(value)
            | Action::Automatic(value) => {
                if self.profile_sync.snapshot.is_none() {
                    self.profile_sync.error = Some(
                        "Load the saved profile choices before changing them. Try Refresh status."
                            .into(),
                    );
                    return;
                }
                let mut changes = self.profile_sync.desired;
                match action {
                    Action::Enabled(_) => changes.enabled = Some(value),
                    Action::Accounts(_) => changes.accounts = Some(value),
                    Action::Automatic(_) => changes.discover_on_login = Some(value),
                    _ => changes.settings = Some(value),
                }
                let options = changes.apply(self.profile_sync.options());
                if let Err(error) = options.validate() {
                    self.profile_sync.error = Some(error.to_string());
                    return;
                }
                if matches!(action, Action::Automatic(false)) {
                    self.profile_sync.offer = false;
                    self.profile_sync.login_pending = None;
                }
                if matches!(action, Action::Automatic(true)) && self.google_connected {
                    self.profile_sync.login_pending =
                        Some(self.preferences.google_lifecycle.revision);
                }
                self.profile_sync.desired = changes;
                self.profile_sync.review = None;
                self.profile_sync.join_review = None;
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
            Action::Discover
            | Action::AfterLogin
            | Action::AutoJoin
            | Action::Create
            | Action::Resume
            | Action::Page(_)
            | Action::Choose(_)
            | Action::AcceptJoin => {
                if self.profile_sync.job.is_some()
                    || self.profile_sync.saving.is_some()
                    || !self.profile_sync.desired.empty()
                    || self.preference_sync.dirty()
                {
                    return;
                }
                match action {
                    Action::Discover => {
                        self.profile_sync.login_pending = None;
                        self.profile_sync.offer = false;
                        Request::Discover(id)
                    }
                    Action::AfterLogin => Request::AfterLogin(id),
                    Action::AutoJoin => {
                        let Some(review) = self.profile_sync.review.clone() else {
                            return;
                        };
                        Request::AutoJoin {
                            request: id,
                            review,
                        }
                    }
                    Action::Resume => Request::Resume(id),
                    Action::Page(after) => {
                        let Some(review) = self.profile_sync.review.clone() else {
                            return;
                        };
                        Request::Page {
                            request: id,
                            review,
                            after,
                        }
                    }
                    Action::Choose(cursor) => {
                        let Some(review) = self.profile_sync.review.clone() else {
                            return;
                        };
                        Request::JoinReview {
                            request: id,
                            review,
                            cursor,
                        }
                    }
                    Action::AcceptJoin => {
                        let Some(review) = self.profile_sync.join_review.clone() else {
                            return;
                        };
                        Request::JoinAccept {
                            request: id,
                            review,
                        }
                    }
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
            if matches!(request, Request::AfterLogin(_) | Request::AutoJoin { .. }) {
                state.automatic_job = Some(id);
            }
            if matches!(request, Request::AfterLogin(_)) {
                state.automatic_generation = Some(self.preference_sync.generation());
            }
            if !matches!(request, Request::Status(_)) {
                state.error = None;
            }
            match request {
                Request::Status(_) => state.loading = Some(id),
                Request::Stop(_) => state.stopping = Some(id),
                _ => {
                    state.job = Some(id);
                    if !matches!(
                        request,
                        Request::AutoJoin { .. }
                            | Request::Page { .. }
                            | Request::JoinReview { .. }
                            | Request::JoinAccept { .. }
                    ) {
                        state.review = None;
                        state.join_review = None;
                    }
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
        let automatic = state.automatic_job == Some(id);
        let pending = matches!(update, Update::Pending(_));
        if automatic && !pending {
            state.automatic_job = None;
        }
        let published = matches!(update, Update::Published(_));
        let joined = matches!(update, Update::Joined(_) | Update::AutoJoined { .. });
        let login_review = matches!(update, Update::LoginReview(_));
        let auto_message = if let Update::AutoJoined {
            name,
            accounts,
            settings,
            ..
        } = &update
        {
            Some(format!(
                "{name} imported · {} · {}{}",
                counted(*accounts as u64, "account"),
                counted(*settings as u64, "preference"),
                if *accounts > 0 {
                    " · reconnect accounts in Preferences"
                } else {
                    ""
                }
            ))
        } else {
            None
        };
        let failed = matches!(update, Update::Failed(_));
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
            Update::Status(snapshot)
            | Update::Published(snapshot)
            | Update::Pending(snapshot)
            | Update::Joined(snapshot)
            | Update::AutoJoined { snapshot, .. } => {
                if state.accepts_snapshot(&snapshot) {
                    if state
                        .review
                        .as_ref()
                        .is_some_and(|r| r.local() != &*snapshot)
                    {
                        state.review = None;
                    }
                    if state
                        .join_review
                        .as_ref()
                        .is_some_and(|r| r.local() != &*snapshot)
                    {
                        state.join_review = None;
                    }
                    state.snapshot = Some(snapshot);
                }
                if job && joined {
                    state.review = None;
                    state.join_review = None;
                    state.offer = false;
                    self.notice(
                        auto_message.unwrap_or_else(|| "Shared profile imported".into()),
                        false,
                    );
                }
                if job && published {
                    self.notice("Profile created on Google Drive", false);
                }
            }
            Update::Review(review) | Update::LoginReview(review) => {
                if (!login_review
                    || (self.google_connected
                        && review.local().google_revision
                            == self.preferences.google_lifecycle.revision
                        && review.local().google_identity == self.preferences.google_connection_id))
                    && state.desired.empty()
                    && state.saving.is_none()
                    && state.accepts_snapshot(review.local())
                {
                    state.snapshot = Some(Arc::new(review.local().clone()));
                    state.offer = login_review;
                    state.review = Some(review);
                    if state.name.is_empty() {
                        state.name = "Personal".into();
                    }
                } else {
                    refresh = true;
                }
            }
            Update::JoinReview(review) => {
                if state.desired.empty()
                    && state.saving.is_none()
                    && state.accepts_snapshot(review.local())
                {
                    state.snapshot = Some(Arc::new(review.local().clone()));
                    state.join_review = Some(review);
                } else {
                    refresh = true;
                }
            }
            Update::Failed(error) => {
                state.error = Some(error);
                if automatic {
                    state.offer = true;
                }
                self.pending_close = None;
                refresh = !loading;
            }
            Update::Stopped => {
                refresh = true;
            }
        }
        if login_review && self.can_auto_enroll() {
            self.profile_sync.offer = false;
            self.shared_profile_action(Action::AutoJoin);
        }
        if refresh {
            self.shared_profile_action(Action::Refresh);
        } else if !failed {
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
                .on_toggle_maybe(
                    state
                        .snapshot
                        .is_some()
                        .then_some(move |v| msg(Action::Accounts(v)))
                )
                .text_size(13),
            checkbox(options.settings)
                .label("Appearance and mail preferences")
                .on_toggle_maybe(
                    state
                        .snapshot
                        .is_some()
                        .then_some(move |v| msg(Action::Settings(v)))
                )
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
                    muted(if selected.origin == crate::profile_sync::enrollment::Origin::Join { "Initial profile imported. Continuous updates are still being implemented." } else { "Initial profile saved. Continuous updates are still being implemented." })
                        .size(12),
                );
                if !self.workspace.account_reconnect.is_empty() {
                    body = body.push(action(
                        "Reconnect accounts",
                        Message::FindSetting(SettingsTab::Accounts, "Your accounts"),
                    ));
                }
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
            if state.snapshot.is_none() {
                body = body.push(muted(if state.error.is_some() {
                    "Saved profile choices are unavailable. Update Shep or restore a working database."
                } else {
                    "Loading profile choices…"
                }).size(12));
            } else if !available {
                body = body
                    .push(
                        muted("Connect Google with Drive permission to share a profile.").size(12),
                    )
                    .push(action(
                        "Google connection",
                        Message::FindSetting(SettingsTab::Accounts, "Google connection"),
                    ));
            } else if let Some(review) = &state.join_review {
                body = body
                    .push(
                        text(format!("Use {} on this device?", review.name()))
                            .size(16)
                            .font(BOLD),
                    )
                    .push(
                        muted(format!(
                            "Add {} · apply {}",
                            counted(review.accounts as u64, "account"),
                            counted(review.settings as u64, "preference")
                        ))
                        .size(13),
                    );
                for (name, email) in &review.account_preview {
                    body =
                        body.push(column![text(name).size(13), muted(email).size(12)].spacing(4));
                }
                if review.accounts > review.account_preview.len() {
                    body = body.push(
                        muted(format!(
                            "And {} more accounts",
                            review.accounts - review.account_preview.len()
                        ))
                        .size(12),
                    );
                }
                body = body.push(muted(if review.accounts == 0 { "Shared preferences replace matching local choices. Your accounts and mail are kept." } else if review.settings == 0 { "Your existing accounts, mail and preferences are kept. Reconnect imported accounts with their passwords here." } else { "Your existing accounts and mail are kept. Reconnect imported accounts here; shared preferences replace matching local choices." }).size(12))
                    .push(row![button(text("Import profile").size(13)).padding([12,16]).style(components::primary).on_press_maybe(idle.then(||msg(Action::AcceptJoin))),
                        action("Cancel",msg(Action::CancelReview))].spacing(10));
            } else if let Some(review) = &state.review {
                if review.profile_count() > 0 {
                    body = body.push(text("Choose a shared profile").size(16).font(BOLD));
                    for profile in review.profiles() {
                        let usable = profile.initialized
                            && !profile.removed
                            && profile.waiting == 0
                            && profile.conflicts == 0
                            && !profile.name_conflict
                            && profile.name.is_some();
                        body = body.push(
                            row![
                                column![
                                    text(profile.name.as_deref().unwrap_or("Unnamed profile"))
                                        .size(14)
                                        .font(BOLD),
                                    muted(if usable {
                                        format!(
                                            "{} · {}",
                                            counted(profile.accounts, "account"),
                                            counted(profile.settings, "preference")
                                        )
                                    } else if !profile.initialized
                                        && !profile.removed
                                        && profile.conflicts == 0
                                    {
                                        "Setup incomplete · finish on the original device".into()
                                    } else {
                                        "Needs review on its original device".into()
                                    })
                                    .size(12)
                                ]
                                .spacing(4),
                                space().width(Length::Fill),
                                button(text("Review").size(13))
                                    .padding([12, 16])
                                    .style(outline)
                                    .on_press_maybe(
                                        (idle && usable)
                                            .then(|| msg(Action::Choose(profile.cursor())))
                                    )
                            ]
                            .spacing(12)
                            .align_y(Alignment::Center),
                        );
                    }
                    let mut pages = row![].spacing(10);
                    if review.after().is_some() {
                        pages = pages.push(action("First page", msg(Action::Page(None))));
                    }
                    if review.has_more() {
                        pages = pages.push(action(
                            "Next page",
                            msg(Action::Page(review.profiles().last().map(|p| p.cursor()))),
                        ));
                    }
                    body = body.push(pages);
                }
                body = body
                    .push(
                        muted(if review.records() == 0 {
                            "No shared profiles found. Create one from this workspace?"
                        } else {
                            "Or create a separate profile from this workspace."
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
                            action("Not now", msg(Action::NotNow))
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
        if selected.is_none() && state.snapshot.is_some() {
            body = body.push(
                checkbox(options.discover_on_login)
                    .label("Check for shared profiles after Google sign-in")
                    .on_toggle(move |v| msg(Action::Automatic(v)))
                    .text_size(12),
            );
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

fn counted(count: u64, noun: &str) -> String {
    format!("{count} {noun}{}", if count == 1 { "" } else { "s" })
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
    async fn profile_login_coalesces_status_waits_for_saves_and_respects_decline_and_close() {
        let (mut app, mut queue, original) = app().await;
        let mut saved = (*original).clone();
        saved.available = true;
        app.profile_sync.snapshot = Some(Arc::new(saved.clone()));
        app.google_connected = true;
        app.profile_google_status(0, true);
        app.preference_sync.changed();
        app.advance_profile_login();
        assert!(queue.try_recv().is_err());
        app.preference_sync = Default::default();
        app.advance_profile_login();
        let Some(Command::ProfileSync(Request::AfterLogin(id))) = queue.recv().await else {
            panic!("expected automatic discovery");
        };
        app.profile_google_status(0, true);
        app.advance_profile_login();
        assert!(queue.try_recv().is_err());
        let _ =
            app.shared_profile_update(id, Update::Failed("Offline; try discovery again.".into()));
        assert!(app.profile_sync.offer);
        let Some(Command::ProfileSync(Request::Status(status))) = queue.recv().await else {
            panic!("expected status refresh");
        };
        let _ = app.shared_profile_update(status, Update::Status(Arc::new(saved.clone())));
        app.advance_profile_login();
        assert!(queue.try_recv().is_err());
        app.shared_profile_action(Action::NotNow);
        assert!(!app.profile_sync.offer && !app.profile_sync.options().discover_on_login);
        let Some(Command::ProfileSync(Request::Change { request, .. })) = queue.recv().await else {
            panic!("expected durable opt-out");
        };
        saved.enrollment.revision += 1;
        saved.enrollment.options.discover_on_login = false;
        let _ = app.shared_profile_update(request, Update::Status(Arc::new(saved)));
        app.profile_google_status(1, true);
        app.profile_google_status(1, false);
        app.advance_profile_login();
        assert!(queue.try_recv().is_err());
        app.profile_sync.login_pending = Some(0);
        app.pending_close = Some(iced::window::Id::unique());
        app.advance_profile_login();
        assert!(app.profile_sync.login_pending.is_none() && queue.try_recv().is_err());
    }

    #[tokio::test]
    async fn profile_join_review_cannot_restore_older_category_choices_after_a_save() {
        let (mut app, mut queue, original) = app().await;
        let mut newer = (*original).clone();
        newer.enrollment.revision += 1;
        newer.enrollment.options.accounts = false;
        app.profile_sync.snapshot = Some(Arc::new(newer));
        app.profile_sync.job = Some(55);
        let review = crate::profile_sync::join::Review {
            automatic: false,
            id: uuid::Uuid::new_v4(),
            local: (*original).clone(),
            selection: crate::profile_sync::enrollment::Selection {
                binding: shep_profile_core::history::Binding {
                    namespace: "so.shep".into(),
                    principal: "drive:fixture".into(),
                    profile: uuid::Uuid::new_v4(),
                    generation: uuid::Uuid::new_v4(),
                },
                name: "Old review".into(),
                origin: crate::profile_sync::enrollment::Origin::Join,
                ready: true,
            },
            revision: 1,
            device: uuid::Uuid::new_v4(),
            accounts: 1,
            settings: 1,
            account_preview: vec![],
        };
        let _ = app.shared_profile_update(55, Update::JoinReview(Arc::new(review)));
        assert!(app.profile_sync.join_review.is_none());
        assert!(!app.profile_sync.options().accounts);
        assert!(matches!(
            queue.try_recv().unwrap(),
            Command::ProfileSync(Request::Status(_))
        ));
    }

    #[tokio::test]
    async fn profile_load_failure_cannot_trap_close_or_drop_newer_choices_on_retry() {
        let (mut app, mut queue, original) = app().await;
        app.profile_sync.snapshot = None;
        app.shared_profile_action(Action::Accounts(false));
        assert!(app.profile_sync.desired.empty());
        assert!(!app.profile_sync.pending());
        assert!(app.profile_sync.error.is_some());
        app.profile_sync.snapshot = Some(original.clone());
        app.shared_profile_action(Action::Accounts(false));
        let Command::ProfileSync(Request::Change { request, .. }) = queue.try_recv().unwrap()
        else {
            panic!("change")
        };
        app.shared_profile_action(Action::Settings(false));
        let _ = app.shared_profile_update(request, Update::Failed("Could not save".into()));
        let Command::ProfileSync(Request::Status(refresh)) = queue.try_recv().unwrap() else {
            panic!("refresh")
        };
        let _ = app.shared_profile_update(refresh, Update::Failed("Could not read".into()));
        assert!(
            !app.profile_sync.pending(),
            "unsent failed choices must not trap a subsequent close"
        );
        assert_eq!(app.profile_sync.desired.settings, Some(false));
        app.shared_profile_action(Action::Refresh);
        let Command::ProfileSync(Request::Status(refresh)) = queue.try_recv().unwrap() else {
            panic!("retry")
        };
        let _ = app.shared_profile_update(refresh, Update::Status(original));
        let Command::ProfileSync(Request::Change { changes, .. }) = queue.try_recv().unwrap()
        else {
            panic!("retained intent")
        };
        assert_eq!(changes.settings, Some(false));
        assert_eq!(changes.accounts, None);
        assert!(app.profile_sync.pending());
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
