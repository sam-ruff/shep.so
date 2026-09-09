mod account_reviews;
mod join;
mod onboarding;
mod reviews;
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
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum Action {
    Refresh,
    Sync,
    AccountReviews(Option<String>),
    ResolveAccount(
        Arc<crate::profile_sync::account_reviews::Review>,
        crate::profile_sync::account_reviews::Choice,
    ),
    AccountCandidate(
        Arc<crate::profile_sync::account_reviews::Review>,
        account_reviews::Candidate,
    ),
    ReviewRemovedAccount(Arc<crate::profile_sync::account_reviews::Review>),
    CloseAccountReviews,
    SettingReviews,
    ResolveSetting(
        Arc<crate::profile_sync::reviews::Review>,
        crate::profile_sync::reviews::Choice,
    ),
    SettingCandidate(
        Arc<crate::profile_sync::reviews::Review>,
        reviews::Candidate,
    ),
    CloseSettingReviews,
    AfterLogin,
    AutoJoin,
    Automatic(bool),
    NotNow,
    Discover,
    Create,
    Page(Option<String>),
    Choose(String),
    AcceptJoin(
        Arc<crate::profile_sync::join::Review>,
        crate::profile_sync::join::links::Links,
    ),
    JoinLink(
        Arc<crate::profile_sync::join::Review>,
        uuid::Uuid,
        Option<String>,
    ),
    JoinPage(Arc<crate::profile_sync::join::Review>, usize),
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
    join_links: crate::profile_sync::join::links::Links,
    join_offset: usize,
    account_reviews: Option<Vec<Arc<crate::profile_sync::account_reviews::Review>>>,
    account_after: Option<String>,
    account_choices: std::collections::BTreeMap<String, account_reviews::Candidate>,
    setting_reviews: Option<Vec<Arc<crate::profile_sync::reviews::Review>>>,
    setting_choices: std::collections::BTreeMap<shep_profile_core::SettingKey, reviews::Candidate>,
    name: String,
    error: Option<String>,
    next_sync: Option<Instant>,
    cycle_paused: bool,
    cycle: Option<crate::profile_sync::continuous::Report>,
}
impl State {
    pub(super) fn connection_removed(&mut self) {
        self.account_reviews = None;
        self.account_choices.clear();
        self.account_after = None;
        self.cycle = None;
        self.next_sync = Some(Instant::now());
    }
    fn accepts_account_review(
        &self,
        review: &Arc<crate::profile_sync::account_reviews::Review>,
    ) -> bool {
        self.account_reviews
            .as_ref()
            .is_some_and(|current| current.iter().any(|r| Arc::ptr_eq(r, review)))
    }
    fn accepts_setting_review(&self, review: &Arc<crate::profile_sync::reviews::Review>) -> bool {
        self.setting_reviews
            .as_ref()
            .is_some_and(|current| current.iter().any(|r| Arc::ptr_eq(r, review)))
    }
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
            "account_reviews": self.account_reviews.as_ref().map(|r| r.iter().map(|r|serde_json::json!({"id":r.local().id,"name":r.local().name,"host":r.local().host,"removed":r.removed(),"versions":r.versions().iter().map(|v|&v.account.host).collect::<Vec<_>>()})).collect::<Vec<_>>()),
            "account_after": self.account_after,
            "setting_reviews": self.setting_reviews.as_ref().map(|r|r.iter().map(|r|serde_json::json!({"label":r.label(),"local":r.local(),"versions":r.versions().iter().map(|v|&v.value).collect::<Vec<_>>()})).collect::<Vec<_>>()),
            "review":self.review.as_ref().map(|r|r.records()),
            "profiles":self.review.as_ref().map(|r|r.profiles()),
            "join_review":self.join_review.as_ref().map(|r|serde_json::json!({"name":r.name(),"accounts":r.accounts,"settings":r.settings,"offset":self.join_offset,"links":self.join_links,"page":r.account_page(self.join_offset).iter().map(|o|serde_json::json!({"shared":o.shared,"name":o.name,"email":o.email,"matches":o.matches.iter().map(|m|serde_json::json!({"id":m.id,"name":m.name})).collect::<Vec<_>>()})).collect::<Vec<_>>()})),"name":self.name,"error":self.error,
            "cycle":self.cycle.as_ref().map(|r|serde_json::json!({"applied":r.applied,"review":r.review,"published":r.published,"remaining":r.remaining})),
            "enrollment":self.snapshot.as_ref().map(|s|&s.enrollment)})
    }
}
impl App {
    pub(super) fn shared_profile_action(&mut self, action: Action) {
        match action {
            Action::JoinLink(review, shared, local) => {
                if self.profile_sync.job.is_none()
                    && self
                        .profile_sync
                        .join_review
                        .as_ref()
                        .is_some_and(|r| Arc::ptr_eq(r, &review))
                {
                    let mut links = self.profile_sync.join_links.clone();
                    if let Some(local) = local {
                        links.insert(shared, local);
                    } else {
                        links.remove(&shared);
                    }
                    match review.validate_links(&links) {
                        Ok(()) => self.profile_sync.join_links = links,
                        Err(error) => self.notice(error.to_string(), true),
                    }
                }
                return;
            }
            Action::JoinPage(review, offset) => {
                if self.profile_sync.job.is_none()
                    && self
                        .profile_sync
                        .join_review
                        .as_ref()
                        .is_some_and(|r| Arc::ptr_eq(r, &review))
                    && offset < review.accounts
                {
                    self.profile_sync.join_offset = offset;
                }
                return;
            }
            Action::AccountCandidate(review, candidate) => {
                if self.profile_sync.job.is_none()
                    && self.profile_sync.accepts_account_review(&review)
                {
                    self.profile_sync
                        .account_choices
                        .insert(review.local().id.clone(), candidate);
                }
                return;
            }
            Action::ReviewRemovedAccount(review) => {
                if self.profile_sync.job.is_none()
                    && review.removed()
                    && self.profile_sync.accepts_account_review(&review)
                {
                    // This opens the ordinary local-data review. Its final
                    // confirmation rechecks current mail/drafts/pending changes;
                    // opening it alone never removes or suppresses anything.
                    self.review_removal(crate::store::ConnectionRef {
                        kind: crate::store::ConnectionKind::Account,
                        id: review.local().id.clone(),
                    });
                }
                return;
            }
            Action::CloseAccountReviews => {
                self.profile_sync.account_reviews = None;
                self.profile_sync.account_choices.clear();
                self.profile_sync.account_after = None;
                return;
            }
            Action::SettingCandidate(review, candidate) => {
                if self.profile_sync.accepts_setting_review(&review) {
                    self.profile_sync
                        .setting_choices
                        .insert(review.key(), candidate);
                }
                return;
            }
            Action::CloseSettingReviews => {
                self.profile_sync.setting_reviews = None;
                self.profile_sync.setting_choices.clear();
                return;
            }
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
                self.profile_sync.cycle_paused = false;
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
                self.profile_sync.account_reviews = None;
                self.profile_sync.account_choices.clear();
                self.profile_sync.account_after = None;
                self.profile_sync.setting_reviews = None;
                self.profile_sync.setting_choices.clear();
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
                self.profile_sync.cycle_paused = true;
                Request::Stop(id)
            }
            Action::Discover
            | Action::Sync
            | Action::AccountReviews(_)
            | Action::ResolveAccount(..)
            | Action::SettingReviews
            | Action::ResolveSetting(..)
            | Action::AfterLogin
            | Action::AutoJoin
            | Action::Create
            | Action::Resume
            | Action::Page(_)
            | Action::Choose(_)
            | Action::AcceptJoin(..) => {
                if self.profile_sync.job.is_some()
                    || self.profile_sync.saving.is_some()
                    || !self.profile_sync.desired.empty()
                    || self.preference_sync.dirty()
                {
                    return;
                }
                match action {
                    Action::AccountReviews(after) => Request::AccountReviews { request: id, after },
                    Action::ResolveAccount(review, choice) => {
                        if !self.profile_sync.accepts_account_review(&review) {
                            return;
                        }
                        Request::ResolveAccount {
                            request: id,
                            review,
                            choice,
                        }
                    }
                    Action::SettingReviews => Request::SettingReviews(id),
                    Action::ResolveSetting(review, choice) => {
                        if !self.profile_sync.accepts_setting_review(&review) {
                            return;
                        }
                        Request::ResolveSetting {
                            request: id,
                            review,
                            choice,
                        }
                    }
                    Action::Sync => {
                        self.profile_sync.cycle_paused = false;
                        self.profile_sync.next_sync =
                            Some(Instant::now() + Duration::from_secs(30));
                        Request::Sync(id)
                    }
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
                    Action::AcceptJoin(review, links) => {
                        if !self
                            .profile_sync
                            .join_review
                            .as_ref()
                            .is_some_and(|r| Arc::ptr_eq(r, &review))
                            || links != self.profile_sync.join_links
                            || review.validate_links(&links).is_err()
                        {
                            return;
                        }
                        Request::JoinAccept {
                            request: id,
                            review,
                            links,
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
            Update::AccountReviews {
                snapshot,
                reviews,
                after,
                saved,
            } => {
                if state.accepts_snapshot(&snapshot)
                    && state.desired.empty()
                    && state.saving.is_none()
                {
                    state.snapshot = Some(snapshot);
                    state.account_reviews = Some(reviews);
                    state.account_after = after;
                    state.account_choices.clear();
                    if saved {
                        state.next_sync = Some(Instant::now() + Duration::from_secs(2));
                        self.notice("Account choice saved", false);
                    }
                } else {
                    refresh = true;
                }
            }
            Update::SettingReviews {
                snapshot,
                reviews,
                saved,
            } => {
                if state.accepts_snapshot(&snapshot)
                    && state.desired.empty()
                    && state.saving.is_none()
                {
                    state.snapshot = Some(snapshot);
                    state.setting_reviews = Some(reviews);
                    state.setting_choices.clear();
                    if saved {
                        state.next_sync = Some(Instant::now() + Duration::from_secs(2));
                        self.notice("Preference choice saved · waiting to sync", false);
                    }
                } else {
                    refresh = true;
                }
            }
            Update::Synced { snapshot, report } => {
                if state.accepts_snapshot(&snapshot) {
                    state.snapshot = Some(snapshot);
                }
                state.next_sync = Some(
                    Instant::now()
                        + Duration::from_secs(if report.remaining && report.review == 0 {
                            2
                        } else {
                            30
                        }),
                );
                state.cycle = Some(report);
            }
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
                    state.join_links.clear();
                    state.join_offset = 0;
                } else {
                    refresh = true;
                }
            }
            Update::Failed(error) => {
                state.next_sync = Some(Instant::now() + Duration::from_secs(60));
                state.error = Some(error.clone());
                if automatic {
                    state.offer = true;
                }
                // A review may be scrolled below the inline error. A rejected
                // choice must remain visible at the current viewport as well.
                if job && (state.setting_reviews.is_some() || state.account_reviews.is_some()) {
                    self.notice(error, true);
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
                    muted(if state.cycle.as_ref().is_some_and(|r|r.review>0) {
                        "Some shared changes need review. Existing accounts and local changes have been kept."
                    } else if state.cycle.as_ref().is_some_and(|r|r.remaining) {
                        "Changes are saved on this device and waiting to upload."
                    } else if !options.enabled || state.cycle_paused {
                        "Profile sync is paused on this device."
                    } else if state.cycle.is_some() { "Profile checked. Changes sync automatically in the background." }
                    else { "Ready to check for account and preference changes." })
                        .size(12),
                );
                body = body.push(
                    button(text("Sync now").size(13))
                        .padding([12, 16])
                        .style(outline)
                        .on_press_maybe(
                            (idle && available && options.enabled).then(|| msg(Action::Sync)),
                        ),
                );
                if options.settings {
                    body = body.push(
                        button(text("Review shared preferences").size(13))
                            .padding([12, 16])
                            .style(outline)
                            .on_press_maybe(
                                (idle && available && options.enabled)
                                    .then(|| msg(Action::SettingReviews)),
                            ),
                    );
                    if state.setting_reviews.is_some() {
                        body = body.push(self.shared_setting_reviews(idle));
                    }
                }
                if options.accounts {
                    body = body.push(
                        button(text("Review shared accounts").size(13))
                            .padding([12, 16])
                            .style(outline)
                            .on_press_maybe(
                                (idle && available && options.enabled)
                                    .then(|| msg(Action::AccountReviews(None))),
                            ),
                    );
                    if state.account_reviews.is_some() {
                        body = body.push(self.shared_account_reviews(idle));
                    }
                }
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
                            "{} · apply {}",
                            if state.join_links.is_empty() {
                                format!("Add {}", counted(review.accounts as u64, "account"))
                            } else {
                                format!(
                                    "Link {} · add {}",
                                    counted(state.join_links.len() as u64, "account"),
                                    counted(
                                        review.accounts.saturating_sub(state.join_links.len())
                                            as u64,
                                        "account"
                                    )
                                )
                            },
                            counted(review.settings as u64, "preference")
                        ))
                        .size(13),
                    );
                if review.accounts > 0 {
                    body = body.push(self.shared_join_accounts(review, idle));
                }
                body = body.push(muted(if review.accounts > 0 && state.join_links.len() == review.accounts { "Existing accounts keep their mail and sign-in. Shared preferences replace matching local choices." } else if review.accounts == 0 { "Shared preferences replace matching local choices. Your accounts and mail are kept." } else if review.settings == 0 { "Your existing accounts, mail and preferences are kept. Reconnect imported accounts with their passwords here." } else { "Your existing accounts and mail are kept. Reconnect imported accounts here; shared preferences replace matching local choices." }).size(12))
                    .push(row![button(text("Import profile").size(13)).padding([12,16]).style(components::primary).on_press_maybe(idle.then(||msg(Action::AcceptJoin(review.clone(), state.join_links.clone())))),
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

    #[test]
    fn profile_account_review_local_removal_clears_stale_choices() {
        let review = Arc::new(crate::profile_sync::account_reviews::Review::fixture(
            "Removed account",
        ));
        let mut state = State {
            account_reviews: Some(vec![review.clone()]),
            account_after: Some("older cursor".into()),
            ..Default::default()
        };
        assert!(state.accepts_account_review(&review));
        state.connection_removed();
        assert!(!state.accepts_account_review(&review));
        assert!(state.account_after.is_none());
        assert!(state.next_sync.is_some());
    }

    #[tokio::test]
    async fn profile_account_review_controls_reject_replaced_rows_and_option_changes() {
        use crate::profile_sync::account_reviews::{Choice, Review};
        let (mut app, mut queue, _) = app().await;
        let earlier = Arc::new(Review::fixture("Earlier"));
        let later = Arc::new(Review::fixture("Later"));
        app.profile_sync.account_reviews = Some(vec![earlier.clone(), later.clone()]);
        let stale = Action::ResolveAccount(earlier.clone(), Choice::Local);
        app.profile_sync.account_reviews = Some(vec![later.clone()]);
        app.shared_profile_action(Action::AccountCandidate(
            earlier.clone(),
            account_reviews::Candidate::fixture(earlier.versions()[0].operation),
        ));
        app.shared_profile_action(stale);
        assert!(queue.try_recv().is_err());
        assert!(app.profile_sync.account_choices.is_empty());
        app.shared_profile_action(Action::AccountCandidate(
            later.clone(),
            account_reviews::Candidate::fixture(later.versions()[0].operation),
        ));
        assert!(
            app.profile_sync
                .account_choices
                .contains_key(&later.local().id)
        );
        app.shared_profile_action(Action::ResolveAccount(later.clone(), Choice::Local));
        let Command::ProfileSync(Request::ResolveAccount {
            review, request, ..
        }) = queue.try_recv().unwrap()
        else {
            panic!("exact displayed review")
        };
        assert!(Arc::ptr_eq(&review, &later));
        // A stale result cannot re-open a dismissed/replaced review.
        let _ = app.shared_profile_update(
            request + 1,
            Update::AccountReviews {
                snapshot: app.profile_sync.snapshot.clone().unwrap(),
                reviews: vec![earlier],
                after: None,
                saved: true,
            },
        );
        assert!(app.profile_sync.accepts_account_review(&later));
        app.profile_sync.job = None;
        app.shared_profile_action(Action::Accounts(false));
        assert!(app.profile_sync.account_reviews.is_none());
        app.shared_profile_action(Action::ResolveAccount(later, Choice::Local));
        assert!(!matches!(
            queue.try_recv(),
            Ok(Command::ProfileSync(Request::ResolveAccount { .. }))
        ));
    }

    #[tokio::test]
    async fn profile_setting_review_events_keep_identity_when_an_earlier_row_disappears() {
        use crate::profile_sync::reviews::{Choice, Review};
        use shep_profile_core::SettingKey;
        let (mut app, mut queue, _) = app().await;
        let earlier = Arc::new(Review::fixture(
            SettingKey::Appearance,
            serde_json::json!("Dark"),
        ));
        let later = Arc::new(Review::fixture(
            SettingKey::Tooltips,
            serde_json::json!(true),
        ));
        app.profile_sync.setting_reviews = Some(vec![earlier.clone(), later.clone()]);
        let old_click = Action::ResolveSetting(earlier.clone(), Choice::Local);
        let old_menu = Action::SettingCandidate(
            earlier.clone(),
            reviews::Candidate::fixture(earlier.versions()[0].operation),
        );
        // A completed earlier choice removes row zero before an old native
        // event is delivered. That event must not be retargeted to Tooltips.
        app.profile_sync.setting_reviews = Some(vec![later.clone()]);
        app.shared_profile_action(old_menu);
        app.shared_profile_action(old_click);
        assert!(queue.try_recv().is_err());
        assert!(app.profile_sync.setting_choices.is_empty());
        app.shared_profile_action(Action::SettingCandidate(
            later.clone(),
            reviews::Candidate::fixture(later.versions()[0].operation),
        ));
        assert!(
            app.profile_sync
                .setting_choices
                .contains_key(&SettingKey::Tooltips)
        );
        app.shared_profile_action(Action::ResolveSetting(later.clone(), Choice::Local));
        let Command::ProfileSync(Request::ResolveSetting { review, .. }) =
            queue.try_recv().unwrap()
        else {
            panic!("expected the exact displayed review")
        };
        assert!(Arc::ptr_eq(&review, &later));
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
    async fn profile_join_link_controls_reject_stale_reviews_and_older_import_choices() {
        use crate::profile_sync::join::{
            Review,
            links::{AccountOffer, Links, LocalAccount},
        };
        let (mut app, mut queue, original) = app().await;
        let shared = uuid::Uuid::new_v4();
        let review = Arc::new(Review {
            id: uuid::Uuid::new_v4(),
            automatic: false,
            local: (*original).clone(),
            selection: crate::profile_sync::enrollment::Selection {
                binding: shep_profile_core::history::Binding {
                    namespace: "so.shep".into(),
                    principal: "drive:fixture".into(),
                    profile: uuid::Uuid::new_v4(),
                    generation: uuid::Uuid::new_v4(),
                },
                name: "Home".into(),
                origin: crate::profile_sync::enrollment::Origin::Join,
                ready: true,
            },
            revision: 1,
            device: uuid::Uuid::new_v4(),
            accounts: 1,
            settings: 0,
            account_offers: vec![AccountOffer {
                shared,
                name: "Shared".into(),
                email: "a@example.test".into(),
                matches: vec![LocalAccount {
                    id: "native".into(),
                    name: "Existing".into(),
                }],
            }],
        });
        app.profile_sync.join_review = Some(review.clone());
        app.shared_profile_action(Action::JoinLink(
            review.clone(),
            shared,
            Some("native".into()),
        ));
        let chosen = Links::from([(shared, "native".into())]);
        assert_eq!(app.profile_sync.join_links, chosen);
        // The import button from the earlier frame still means Add new. It
        // must not override the newer explicit reuse choice.
        app.shared_profile_action(Action::AcceptJoin(review.clone(), Links::new()));
        assert!(queue.try_recv().is_err());
        assert!(app.profile_sync.job.is_none());
        let replacement = Arc::new((*review).clone());
        app.profile_sync.join_review = Some(replacement.clone());
        app.shared_profile_action(Action::JoinLink(review.clone(), shared, None));
        app.shared_profile_action(Action::AcceptJoin(review, chosen.clone()));
        assert_eq!(app.profile_sync.join_links, chosen);
        assert!(queue.try_recv().is_err());
        app.shared_profile_action(Action::AcceptJoin(replacement.clone(), chosen.clone()));
        let Command::ProfileSync(Request::JoinAccept {
            review: actual,
            links,
            ..
        }) = queue.try_recv().unwrap()
        else {
            panic!("expected exact current import");
        };
        assert!(Arc::ptr_eq(&actual, &replacement));
        assert_eq!(links, chosen);
        // Further edits cannot mutate the choices while that import is pending.
        app.shared_profile_action(Action::JoinLink(replacement, shared, None));
        assert_eq!(app.profile_sync.join_links, chosen);
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
            account_offers: vec![],
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
