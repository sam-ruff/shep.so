use super::*;
use iced::widget::column;

impl App {
    pub(in crate::ui) fn advance_profile_cycle(&mut self) {
        let state = &mut self.profile_sync;
        // A saved password toggle runs its own pass, even with sync paused, so
        // turning it off can remove this device's entries.
        if state.passwords_due
            && self.pending_close.is_none()
            && !state.pending()
            && state.loading.is_none()
            && !self.preference_sync.dirty()
            && state.snapshot.as_ref().is_some_and(|s| {
                s.available && s.enrollment.selection.as_ref().is_some_and(|v| v.ready)
            })
        {
            self.shared_profile_action(Action::SyncPasswords);
            return;
        }
        let state = &mut self.profile_sync;
        if self.pending_close.is_some()
            || state.pending()
            || state.loading.is_some()
            || state.cycle_paused
            || self.preference_sync.dirty()
            || !state.snapshot.as_ref().is_some_and(|s| {
                s.available
                    && s.enrollment.options.enabled
                    && s.enrollment.selection.as_ref().is_some_and(|v| v.ready)
            })
        {
            return;
        }
        let next = state
            .next_sync
            .get_or_insert_with(|| Instant::now() + Duration::from_secs(2));
        if Instant::now() >= *next {
            self.shared_profile_action(Action::Sync);
        }
    }
    pub(in crate::ui) fn profile_google_status(&mut self, revision: u64, connected: bool) {
        let state = &mut self.profile_sync;
        if !connected {
            state.login_pending = None;
            state.offer = false;
        } else if state.login_seen != Some(revision) {
            state.login_seen = Some(revision);
            state.login_pending = Some(revision);
        }
    }

    /// One speculative check per verified connection/session. Ticks only inspect
    /// small UI state; persistence, credentials and discovery stay on owners.
    pub(in crate::ui) fn advance_profile_login(&mut self) {
        let Some(revision) = self.profile_sync.login_pending else {
            return;
        };
        if self.pending_close.is_some()
            || !self.google_connected
            || revision != self.preferences.google_lifecycle.revision
        {
            self.profile_sync.login_pending = None;
            return;
        }
        let Some(snapshot) = &self.profile_sync.snapshot else {
            return;
        };
        if snapshot.google_revision != revision
            || self.profile_sync.pending()
            || self.profile_sync.loading.is_some()
            || self.preference_sync.dirty()
        {
            return;
        }
        self.profile_sync.login_pending = None;
        if crate::profile_sync::onboarding::eligible(snapshot) {
            self.shared_profile_action(Action::AfterLogin);
        }
    }

    pub(super) fn can_auto_enroll(&self) -> bool {
        self.google_connected
            && self.pending_close.is_none()
            && !self.preference_sync.dirty()
            && self.profile_sync.automatic_generation == Some(self.preference_sync.generation())
            && self.workspace.accounts.is_empty()
            && self.workspace.drafts.is_empty()
            && self.composer.current.draft.id.is_empty()
            && !self.composer.pending()
            && self.dialog.is_none()
            && self.profile_sync.desired.empty()
            && self.profile_sync.saving.is_none()
            && self.profile_sync.review.as_ref().is_some_and(|r| {
                r.local().google_revision == self.preferences.google_lifecycle.revision
                    && r.local().google_identity == self.preferences.google_connection_id
                    && crate::profile_sync::onboarding::automatic_candidate(r)
            })
    }

    pub(in crate::ui) fn shared_profile_offer(&self) -> Option<Element<'_, Message>> {
        if !self.profile_sync.offer
            || self
                .profile_sync
                .snapshot
                .as_ref()
                .is_some_and(|s| s.enrollment.selection.is_some())
            || (self.tab == Tab::Preferences && self.settings_group == Some("Profiles and sync"))
        {
            return None;
        }
        let existing = self
            .profile_sync
            .review
            .as_ref()
            .is_some_and(|r| r.profile_count() > 0);
        Some(
            iced::widget::container(
                column![
                    text(if self.profile_sync.error.is_some() {
                        "Shared profiles need attention"
                    } else if existing {
                        "Shared profiles found"
                    } else {
                        "Sync accounts and settings?"
                    })
                    .size(14)
                    .font(BOLD),
                    components::muted(if self.profile_sync.error.is_some() {
                        "Open Profiles and sync to review the problem and retry."
                    } else if existing {
                        "Choose a profile to use on this device."
                    } else {
                        "Use the same setup on your other devices with Google Drive."
                    })
                    .size(12),
                    row![
                        components::action(
                            if existing {
                                "Choose profile"
                            } else {
                                "Set up sync"
                            },
                            Message::FindSetting(SettingsTab::Accounts, "Profiles and sync")
                        ),
                        components::action("Not now", Message::ProfileSync(Action::NotNow))
                    ]
                    .spacing(10)
                ]
                .spacing(10),
            )
            .padding(16)
            .max_width(500)
            .style(components::card)
            .into(),
        )
    }
}
