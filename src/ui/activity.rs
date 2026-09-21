use super::*;
use crate::store::activity::{Domain, Recovery, Snapshot, Target};
use iced::widget::{column, row, space, text};

#[derive(Default)]
pub(super) struct State {
    pub snapshot: Arc<Snapshot>,
    displayed: Arc<Snapshot>,
    show_next: bool,
    sequence: u64,
    pub(super) loading: Option<u64>,
    again: bool,
    observed: Option<Instant>,
    pub error: Option<String>,
    pub review_error: Option<String>,
    reviewing: Option<u64>,
    backup: Option<crate::backup::history::Entry>,
}

impl App {
    pub(super) fn request_activity_recovery(&mut self, target: Target) {
        if !self.activity.displayed.targets.contains(&target) || self.activity.reviewing.is_some() {
            return;
        }
        self.activity.sequence += 1;
        let request = self.activity.sequence;
        if self.tx.as_ref().is_some_and(|tx| {
            tx.try_send(Command::ActivityRecovery(request, target))
                .is_ok()
        }) {
            self.activity.reviewing = Some(request);
            self.activity.review_error = None;
        } else {
            self.activity.review_error =
                Some("The local review queue is unavailable. Try Review again.".into());
        }
    }

    pub(super) fn activity_recovery_observed(
        &mut self,
        request: u64,
        result: Result<Recovery, String>,
    ) -> Task<Message> {
        if self.activity.reviewing != Some(request) {
            return Task::none();
        }
        self.activity.reviewing = None;
        if self.dialog != Some(Dialog::Activity) {
            return Task::none();
        }
        match result {
            Ok(Recovery::FolderCreation(job)) => self.open_creation_activity(job),
            Ok(Recovery::Backup(entry)) => {
                self.activity.backup = Some(entry);
                self.dialog = Some(Dialog::ActivityBackup);
            }
            Err(error) => {
                self.activity.review_error =
                    Some(format!("{error} Refresh Activity to review current work."))
            }
        }
        Task::none()
    }

    pub(super) fn activity_backup_form(&self) -> Element<'_, Message> {
        let Some(entry) = &self.activity.backup else {
            return text("This backup observation is no longer available. Reopen Activity.").into();
        };
        let configured = entry.target == self.configured_backup_target()
            || self
                .preferences
                .backup_destinations
                .iter()
                .any(|destination| destination.target(&self.preferences) == entry.target);
        let mut body = column![
            text(&entry.name).size(16),
            text(entry.outcome.label()).size(14),
            text(&entry.detail).size(12)
        ]
        .spacing(14);
        if let Some(error) = &self.activity.review_error {
            body = body.push(text(error).size(12));
        }
        if configured {
            body = body.push(action(
                "Open this backup destination",
                Message::ActivityBackupSettings,
            ));
        } else {
            body = body.push(text("This destination is no longer configured. The saved record is retained; restore its connection in Backups before attempting recovery.").size(12))
                .push(action("Open backup settings", Message::ActivitySettings(SettingsTab::Backups)));
        }
        body.into()
    }

    pub(super) fn activity_backup_settings(&mut self) -> Task<Message> {
        let Some(entry) = self.activity.backup.clone() else {
            return Task::none();
        };
        if self.tab == Tab::Preferences && self.settings_tab == SettingsTab::Backups {
            if let Err(error) = self.read_preferences() {
                self.activity.review_error = Some(format!(
                    "Correct the current backup settings before changing destination: {error}"
                ));
                return Task::none();
            }
            self.save_preferences();
        }
        let destination = self
            .preferences
            .backup_destinations
            .iter()
            .find(|destination| destination.target(&self.preferences) == entry.target)
            .map(|destination| destination.id.clone());
        if entry.target != self.configured_backup_target() && destination.is_none() {
            self.activity.review_error =
                Some("The backup destination changed. Reopen Activity.".into());
            return Task::none();
        }
        self.dialog = None;
        let task = self.handle(Message::SettingsTab(SettingsTab::Backups));
        if entry.target != self.configured_backup_target() {
            self.change_backup_destination(destination);
        }
        self.backup_activity.open = true;
        self.refresh_backup_history();
        task
    }

    pub(super) fn open_activity(&mut self) {
        self.activity.reviewing = None;
        self.activity.displayed = self.activity.snapshot.clone();
        self.activity.show_next = self.activity.observed.is_none();
        self.dialog = Some(Dialog::Activity);
        self.refresh_activity(true);
    }

    pub(super) fn reload_activity_view(&mut self) {
        self.activity.show_next = true;
        self.refresh_activity(true);
    }
    fn session_activity(&self) -> [(&'static str, (bool, bool), SettingsTab); 6] {
        [
            (
                "Manual backup",
                (
                    self.backup_run.iter().any(|row| row.status.pending()),
                    self.backup_run.iter().any(|row| {
                        matches!(
                            row.status,
                            crate::backup::run::Status::SavedWithWarning(_)
                                | crate::backup::run::Status::NeedsSetup(_)
                                | crate::backup::run::Status::Failed(_)
                        )
                    }),
                ),
                SettingsTab::Backups,
            ),
            (
                "Database export",
                self.database_transfer.activity(),
                SettingsTab::Backups,
            ),
            (
                "Database import",
                self.database_import.activity(),
                SettingsTab::Backups,
            ),
            (
                "Local profiles",
                self.profiles.activity(),
                SettingsTab::Accounts,
            ),
            (
                "Calendar connection",
                (
                    self.calendar_setup.discovering || self.calendar_setup.saving.is_some(),
                    self.calendar_setup.error.is_some(),
                ),
                SettingsTab::Calendars,
            ),
            (
                "Google connection",
                (
                    self.google_waiting() || self.pending_google_login.is_some(),
                    false,
                ),
                SettingsTab::Accounts,
            ),
        ]
    }
    pub(super) fn refresh_activity(&mut self, force: bool) {
        if self.activity.loading.is_some() {
            self.activity.again |= force;
            return;
        }
        if !force
            && self
                .activity
                .observed
                .is_some_and(|at| at.elapsed().as_secs() < 5)
        {
            return;
        }
        self.activity.sequence += 1;
        let request = self.activity.sequence;
        let Some(tx) = &self.tx else {
            return;
        };
        match tx.try_send(Command::Activity(request)) {
            Ok(()) => self.activity.loading = Some(request),
            Err(_) => {
                self.activity.error = Some(
                    "The local observation queue is unavailable. Retry Refresh activity.".into(),
                );
                self.activity.observed = Some(Instant::now());
            }
        }
    }

    pub(super) fn activity_observed(
        &mut self,
        request: u64,
        result: Result<Arc<Snapshot>, String>,
    ) {
        if self.activity.loading != Some(request) {
            return;
        }
        self.activity.loading = None;
        if std::mem::take(&mut self.activity.again) {
            self.refresh_activity(true);
            return;
        }
        self.activity.observed = Some(Instant::now());
        match result {
            Ok(snapshot) => {
                if self.activity.show_next {
                    self.activity.displayed = snapshot.clone();
                    self.activity.show_next = false;
                    self.activity.review_error = None;
                }
                self.activity.snapshot = snapshot;
                self.activity.error = None;
            }
            Err(error) => self.activity.error = Some(error),
        }
    }

    pub(super) fn activity_label(&self) -> &'static str {
        if self.activity.error.is_some()
            || self.activity.review_error.is_some()
            || self
                .activity
                .snapshot
                .entries
                .iter()
                .any(|entry| entry.attention)
            || self.preference_sync.error().is_some()
            || self.profile_sync.activity_error().is_some()
            || self
                .composer
                .activity_target()
                .is_some_and(|(_, failed)| failed)
            || self
                .session_activity()
                .iter()
                .any(|(_, (_, failed), _)| *failed)
        {
            "Activity · attention"
        } else if self
            .activity
            .snapshot
            .entries
            .iter()
            .any(|entry| entry.pending)
            || self.preference_sync.dirty()
            || self.composer.pending()
            || self.profile_sync.pending()
            || self
                .session_activity()
                .iter()
                .any(|(_, (pending, _), _)| *pending)
        {
            "Activity · pending"
        } else {
            "Activity"
        }
    }

    pub(super) fn activity_form(&self) -> Element<'_, Message> {
        let mut body =
            column![text("Saved work and changes that need your attention.").size(13)].spacing(14);
        body = body.push(
            text(if self.activity.loading.is_some() {
                "Checking saved activity…"
            } else {
                "Saved activity"
            })
            .size(12),
        );
        for entry in &self.activity.displayed.entries {
            if !entry.pending && !entry.attention {
                continue;
            }
            body = body.push(
                row![
                    column![
                        text(entry.domain.label()).size(14),
                        text(if entry.attention {
                            "Needs attention"
                        } else {
                            "Saved work is pending"
                        })
                        .size(12)
                    ]
                    .spacing(4),
                    space().width(iced::Length::Fill),
                    action("Review", Message::ActivityReview(entry.domain))
                ]
                .spacing(12)
                .align_y(iced::Alignment::Center),
            );
            if entry.domain == Domain::Removals {
                body = body.push(action(
                    "Calendar removal progress",
                    Message::ActivityCalendarConnections,
                ));
            }
        }
        if let Some((id, failed)) = self.composer.activity_target() {
            body = body.push(row![
                text(if failed {
                    "Draft edits are not saved"
                } else {
                    "Saving draft edits"
                })
                .size(13),
                space().width(iced::Length::Fill),
                action("Open draft", Message::ActivityDraft(id))
            ]);
        }
        if self.preference_sync.dirty() || self.preference_sync.error().is_some() {
            body = body.push(row![
                text(if self.preference_sync.error().is_some() {
                    "Preferences are not saved"
                } else {
                    "Saving preferences"
                })
                .size(13),
                space().width(iced::Length::Fill),
                action("Open Preferences", Message::ActivityPreferences)
            ]);
        }
        if self.profile_sync.activity_error().is_some() || self.profile_sync.pending() {
            body = body.push(row![
                text(
                    self.profile_sync
                        .activity_error()
                        .unwrap_or("Syncing profile choices")
                )
                .size(13),
                space().width(iced::Length::Fill),
                action("Review profile", Message::ActivityReview(Domain::Profiles))
            ]);
        }
        for (label, (pending, failed), tab) in self.session_activity() {
            if !pending && !failed {
                continue;
            }
            body = body.push(
                row![
                    column![
                        text(label).size(14),
                        text(if failed {
                            "Needs attention on this device"
                        } else {
                            "Working on this device"
                        })
                        .size(12)
                    ],
                    space().width(iced::Length::Fill),
                    action("Review", Message::ActivitySettings(tab))
                ]
                .spacing(12),
            );
        }
        if self.activity_label() == "Activity"
            && self.activity.loading.is_none()
            && self.activity.observed.is_some()
            && !self
                .activity
                .displayed
                .entries
                .iter()
                .any(|entry| entry.pending || entry.attention)
        {
            body = body.push(text("No pending work or saved failures were found.").size(13));
        }
        if let Some(error) = &self.activity.error {
            body = body.push(
                text(format!(
                    "Activity could not refresh: {error}. The last observation is retained."
                ))
                .size(12),
            );
        }
        if let Some(error) = &self.activity.review_error {
            body = body.push(text(error).size(12));
        }
        body.push(text("Saved activity is a snapshot. Refresh to see later progress.").size(12))
            .push(action("Refresh activity", Message::RefreshActivity))
            .into()
    }

    pub(super) fn review_activity(&mut self, domain: Domain) -> Task<Message> {
        if let Some(target) = self
            .activity
            .displayed
            .targets
            .iter()
            .find(|target| {
                matches!(
                    (domain, target),
                    (Domain::FolderCreations, Target::FolderCreation { .. })
                        | (Domain::Backups, Target::Backup { .. })
                )
            })
            .cloned()
        {
            self.request_activity_recovery(target);
            return Task::none();
        }
        self.dialog = None;
        match domain {
            Domain::FolderCreations => {
                self.dialog = Some(Dialog::Activity);
                self.activity.review_error =
                    Some("Folder activity changed. Refresh Activity.".into());
                Task::none()
            }
            Domain::Mail => self.handle(Message::Bulk(bulk::Message::History)),
            Domain::Moves => self.handle(Message::MoveRecovery(move_recovery::Message::Open(None))),
            Domain::Folders => self.handle(Message::Folders(folder_controls::Message::History(0))),
            Domain::Calendar => self.handle(Message::Tab(Tab::Calendar)),
            Domain::Outbox => self.handle(Message::OpenOutbox),
            Domain::Backups => self.handle(Message::SettingsTab(SettingsTab::Backups)),
            Domain::Accounts | Domain::Removals | Domain::Credentials | Domain::Profiles => {
                self.handle(Message::SettingsTab(SettingsTab::Accounts))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_backup_navigation_preserves_invalid_current_settings_for_correction() {
        let (mut app, _) = App::new();
        app.tab = Tab::Preferences;
        app.settings_tab = SettingsTab::Backups;
        app.settings_fields();
        app.fields.insert("copies", "Still editing".into());
        app.dialog = Some(Dialog::ActivityBackup);
        app.activity.backup = Some(crate::backup::history::Entry::new(
            app.configured_backup_target(),
            "Backup".into(),
            Default::default(),
        ));
        let _ = app.activity_backup_settings();
        assert_eq!(app.field("copies"), "Still editing");
        assert_eq!(app.dialog, Some(Dialog::ActivityBackup));
        assert!(app.activity.review_error.is_some());
    }

    #[test]
    fn exact_recovery_reply_cannot_replace_new_navigation_and_failure_keeps_attention() {
        let (mut app, _) = App::new();
        app.activity.reviewing = Some(7);
        app.tab = Tab::Calendar;
        app.dialog = None;
        let _ = app.activity_recovery_observed(7, Err("Old result".into()));
        assert_eq!(app.tab, Tab::Calendar);
        assert!(app.activity.review_error.is_none());
        app.dialog = Some(Dialog::Activity);
        app.activity.reviewing = Some(8);
        let _ = app.activity_recovery_observed(7, Err("Wrong request".into()));
        assert_eq!(app.activity.reviewing, Some(8));
        let _ = app.activity_recovery_observed(8, Err("The folder was removed".into()));
        assert_eq!(app.dialog, Some(Dialog::Activity));
        assert_eq!(app.activity_label(), "Activity · attention");
        assert!(
            app.activity
                .review_error
                .as_deref()
                .is_some_and(|error| error.contains("Refresh Activity"))
        );
        app.activity.loading = Some(9);
        app.activity_observed(9, Ok(Arc::default()));
        assert!(app.activity.review_error.is_some());
    }

    #[test]
    fn observations_coalesce_and_older_replies_cannot_hide_attention() {
        let (mut app, _) = App::new();
        let (sender, mut reads) = engine::CommandSender::foreground_test_channel();
        app.tx = Some(sender);
        app.refresh_activity(true);
        let Command::Activity(first) = reads.try_recv().expect("first observation") else {
            panic!("activity read")
        };
        for _ in 0..20 {
            app.refresh_activity(true);
        }
        assert!(reads.try_recv().is_err());
        let attention = Arc::new(Snapshot {
            targets: Vec::new(),
            entries: vec![crate::store::activity::Entry {
                domain: Domain::Outbox,
                pending: false,
                attention: true,
            }],
        });
        app.activity.snapshot = attention.clone();
        app.activity_observed(first, Ok(Arc::default()));
        assert!(Arc::ptr_eq(&app.activity.snapshot, &attention));
        let Command::Activity(second) = reads.try_recv().expect("replacement observation") else {
            panic!("activity read")
        };
        app.activity_observed(first, Ok(Arc::default()));
        assert_eq!(app.activity.loading, Some(second));
        app.activity_observed(second, Err("Storage unavailable".into()));
        assert!(Arc::ptr_eq(&app.activity.snapshot, &attention));
        assert_eq!(app.activity_label(), "Activity · attention");
        assert!(app.activity.error.is_some());
    }

    #[test]
    fn navigation_and_toast_expiry_do_not_dismiss_owned_attention() {
        let (mut app, _) = App::new();
        app.activity.snapshot = Arc::new(Snapshot {
            targets: Vec::new(),
            entries: vec![crate::store::activity::Entry {
                domain: Domain::Calendar,
                pending: false,
                attention: true,
            }],
        });
        let _ = app.handle(Message::OpenActivity);
        assert_eq!(app.dialog, Some(Dialog::Activity));
        let _ = app.review_activity(Domain::Calendar);
        assert_eq!(app.tab, Tab::Calendar);
        app.notice = None;
        assert_eq!(app.activity_label(), "Activity · attention");
        let _ = app.handle(Message::OpenActivity);
        assert_eq!(app.dialog, Some(Dialog::Activity));
        assert!(app.activity.snapshot.entries[0].attention);
    }

    #[test]
    fn background_progress_keeps_the_displayed_review_target_stable() {
        let (mut app, _) = App::new();
        let held = Arc::new(Snapshot {
            targets: Vec::new(),
            entries: vec![crate::store::activity::Entry {
                domain: Domain::Outbox,
                pending: true,
                attention: true,
            }],
        });
        app.activity.snapshot = held.clone();
        app.activity.observed = Some(Instant::now());
        app.open_activity();
        app.activity.loading = Some(7);
        app.activity_observed(7, Ok(Arc::default()));
        assert!(app.activity.snapshot.entries.is_empty());
        assert!(Arc::ptr_eq(&app.activity.displayed, &held));
        let _ = app.review_activity(Domain::Outbox);
        assert_eq!(app.dialog, Some(Dialog::Outbox));
    }

    #[test]
    fn sidebar_keyboard_focus_reaches_the_fixed_activity_control() {
        let (mut app, _) = App::new();
        Arc::make_mut(&mut app.workspace).outgoing_pending = 1;
        app.tab = Tab::Mail;
        app.sidebar_focus = false;
        let _ = app.key(
            Key::Named(keyboard::key::Named::Tab),
            keyboard::Modifiers::empty(),
            false,
        );
        assert!(app.sidebar_focus);
        let last = app.sidebar_items().len() - 1;
        app.sidebar_index = 0;
        for _ in 0..=last {
            let _ = app.key(
                Key::Named(keyboard::key::Named::ArrowDown),
                keyboard::Modifiers::empty(),
                false,
            );
            assert_eq!(app.dialog, None);
        }
        assert_eq!(app.sidebar_index, last);
        assert_eq!(app.dialog, None);
        let _ = app.key(
            Key::Named(keyboard::key::Named::Enter),
            keyboard::Modifiers::empty(),
            false,
        );
        assert_eq!(app.dialog, Some(Dialog::Activity));
    }
}
