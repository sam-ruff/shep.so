use super::*;
use crate::transfer::{Outcome, Phase, Progress, Request, Update};
use iced::{
    Alignment,
    widget::{column, progress_bar, row, text},
};
use std::path::PathBuf;

#[derive(Default)]
pub(super) struct State {
    serial: u64,
    pub pending: Option<Pending>,
    saved: Option<(PathBuf, u64)>,
    error: Option<String>,
}

pub(super) struct Pending {
    request: u64,
    path: Option<PathBuf>,
    preferences: Option<u64>,
    preferences_saved: bool,
    started: bool,
    cancelling: bool,
    cancel_sent: bool,
    progress: Progress,
}

impl State {
    fn begin(&mut self) -> u64 {
        self.serial += 1;
        self.pending = Some(Pending {
            request: self.serial,
            path: None,
            preferences: None,
            preferences_saved: false,
            started: false,
            cancelling: false,
            cancel_sent: false,
            progress: Progress::default(),
        });
        self.error = None;
        self.serial
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn observation(&self) -> serde_json::Value {
        serde_json::json!({
            "phase": self.pending.as_ref().map(|p| if p.cancelling { "Cancelling" }
                else if p.path.is_none() { "Choosing" }
                else if !p.started { "Saving" }
                else { match p.progress.phase {
                    Phase::Preparing => "Preparing", Phase::Copying => "Copying", Phase::Finishing => "Finishing"
                }}),
            "saved": self.saved.as_ref().map(|(path, _)| path),
            "bytes": self.saved.as_ref().map(|(_, bytes)| bytes),
            "error": self.error,
        })
    }
}

impl App {
    pub(super) fn begin_database_export(&mut self) -> Task<Message> {
        if self.database_transfer.pending.is_some()
            || self.database_import.pending()
            || self.tx.is_none()
        {
            return Task::none();
        }
        let request = self.database_transfer.begin();
        Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .set_title("Export Shep database")
                    .set_file_name(format!(
                        "shep-{}.sqlite",
                        chrono::Local::now().format("%Y-%m-%d")
                    ))
                    .add_filter("SQLite database", &["sqlite", "db"])
                    .save_file()
                    .await
                    .map(|file| file.path().to_path_buf())
            },
            move |path| Message::DatabaseExportPath(request, path),
        )
    }

    pub(super) fn database_export_path(&mut self, request: u64, path: Option<PathBuf>) {
        if !self
            .database_transfer
            .pending
            .as_ref()
            .is_some_and(|p| p.request == request && p.path.is_none())
        {
            return;
        }
        let Some(path) = path else {
            self.database_transfer.pending = None;
            return;
        };
        if let Err(error) = self.read_preferences() {
            self.fail_database_preparation(&error.to_string());
            return;
        }
        let save = self.preference_sync.changed();
        let pending = self.database_transfer.pending.as_mut().unwrap();
        pending.path = Some(path);
        pending.preferences = Some(save);
        if !self.try_command(Command::SavePreferences(save, self.preferences.clone())) {
            self.fail_database_preparation("Settings could not be queued. Retry the export.");
            return;
        }
        self.flush_draft_saves(true);
    }

    pub(super) fn database_preferences_saved(&mut self, request: u64) {
        self.import_preferences_saved(request);
        if let Some(pending) = self.database_transfer.pending.as_mut()
            && pending.preferences == Some(request)
        {
            pending.preferences_saved = true;
        }
        self.advance_database_transfer();
    }

    pub(super) fn database_preferences_failed(&mut self, request: u64, error: &str) {
        self.import_preferences_failed(request, error);
        if self
            .database_transfer
            .pending
            .as_ref()
            .is_some_and(|p| p.preferences == Some(request))
        {
            self.fail_database_preparation(error);
        }
    }

    pub(super) fn fail_database_preparation(&mut self, error: &str) {
        if self
            .database_transfer
            .pending
            .as_ref()
            .is_some_and(|p| !p.started)
        {
            self.database_transfer.pending = None;
            let error = format!("Database export could not start: {error}");
            self.database_transfer.error = Some(error.clone());
            self.notice(error, true);
        }
    }

    pub(super) fn advance_database_transfer(&mut self) {
        let Some(pending) = &self.database_transfer.pending else {
            return;
        };
        if pending.cancelling {
            if !pending.cancel_sent {
                let request = pending.request;
                if self.try_command(Command::Database(Request::Cancel(request))) {
                    self.database_transfer.pending.as_mut().unwrap().cancel_sent = true;
                }
            }
            return;
        }
        if pending.started || !pending.preferences_saved {
            return;
        }
        self.flush_draft_saves(true);
        if self.composer.pending()
            || self.composer.io.is_some()
            || self.composer.forward_pending.is_some()
            || self.composer.discard_pending
        {
            return;
        }
        let pending = self.database_transfer.pending.as_ref().unwrap();
        let Some(destination) = pending.path.clone() else {
            return;
        };
        let request = pending.request;
        if self.try_command(Command::Database(Request::Export {
            request,
            destination,
            replace: true,
        })) {
            self.database_transfer.pending.as_mut().unwrap().started = true;
        } else {
            self.fail_database_preparation("The export worker is busy. Retry the export.");
        }
    }

    pub(super) fn cancel_database_transfer(&mut self) {
        if let Some(pending) = self.database_transfer.pending.as_mut() {
            if pending.started {
                pending.cancelling = true;
                self.advance_database_transfer();
            } else {
                self.database_transfer.pending = None;
            }
        }
    }

    pub(super) fn database_update(&mut self, request: u64, update: Update) -> Task<Message> {
        let Some(pending) = self
            .database_transfer
            .pending
            .as_mut()
            .filter(|p| p.request == request && p.started)
        else {
            return Task::none();
        };
        match update {
            Update::ImportProgress(_)
            | Update::Review(_)
            | Update::ReviewError(_)
            | Update::Installing(_)
            | Update::ImportFinished(_) => {}
            Update::Progress(progress) => pending.progress = progress,
            Update::Finished(result) => {
                self.database_transfer.pending = None;
                match result {
                    Ok(Outcome::Saved {
                        path,
                        bytes,
                        warning,
                    }) => {
                        self.database_transfer.saved = Some((path, bytes));
                        if let Some(warning) = warning {
                            self.database_transfer.error = Some(warning.clone());
                            self.notice(warning, true);
                            self.pending_close = None;
                        } else {
                            self.notice("Database exported", false);
                        }
                    }
                    Ok(Outcome::Cancelled) => self.notice("Database export cancelled", false),
                    Err(error) => {
                        self.database_transfer.error = Some(error.clone());
                        self.notice(format!("Database export failed: {error}"), true);
                        self.pending_close = None;
                    }
                }
                if let Some(window) = self.pending_close.take() {
                    return self.handle(Message::WindowClose(window));
                }
            }
        }
        Task::none()
    }

    pub(super) fn database_transfer_card(&self) -> Element<'_, Message> {
        use components::{action, muted};
        let state = &self.database_transfer;
        let mut content = column![muted(
            "Includes all cached mail, attachments, drafts, accounts and settings."
        )]
        .spacing(12);
        content = content.push(muted("This file is not encrypted. Account passwords and Google sign-in are not included.").size(12));
        if let Some(pending) = &state.pending {
            let label = if pending.cancelling {
                "Cancelling export…"
            } else if pending.path.is_none() {
                "Choose where to save the database…"
            } else if !pending.started {
                "Saving your latest settings and drafts…"
            } else {
                match pending.progress.phase {
                    Phase::Preparing => "Preparing database export…",
                    Phase::Copying => "Exporting database…",
                    Phase::Finishing => "Finishing database export…",
                }
            };
            content = content.push(
                row![
                    muted(label),
                    action("Cancel", Message::CancelDatabaseTransfer)
                ]
                .spacing(16)
                .align_y(Alignment::Center),
            );
            if pending.progress.total_pages > 0 {
                content = content.push(
                    progress_bar(
                        0.0..=1.0,
                        pending.progress.copied_pages as f32 / pending.progress.total_pages as f32,
                    )
                    .girth(4),
                );
            }
        } else {
            content = content.push(
                iced::widget::button(text("Export database…").size(12))
                    .padding([11, 14])
                    .style(components::outline)
                    .on_press_maybe(
                        (!self.database_import.pending()).then_some(Message::DatabaseExport),
                    ),
            );
        }
        if let Some((path, bytes)) = &state.saved {
            content = content.push(
                muted(format!(
                    "Saved {} · {:.1} MiB",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    *bytes as f64 / 1_048_576.0
                ))
                .size(12),
            );
        }
        if let Some(error) = &state.error {
            content = content.push(text(error).size(12));
        }
        content = content
            .push(components::line())
            .push(self.database_import_controls());
        self.settings_card(
            "Database transfer",
            "Move a complete workspace between computers.",
            content.into(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_waits_for_exact_settings_ack_and_all_draft_revisions() {
        let (mut app, _) = App::new();
        let (tx, mut database, mut persistence) = engine::CommandSender::database_test_channels();
        app.tx = Some(tx);
        app.composer.current.draft.id = "unsent".into();
        app.composer.current.draft.revision = 4;
        app.composer.current.dirty = Some(Instant::now());
        let request = app.database_transfer.begin();
        app.database_export_path(request, Some(PathBuf::from("/fixture/export.sqlite")));
        let Command::SavePreferences(save, _) = persistence.try_recv().unwrap() else {
            panic!("settings must precede copy")
        };
        assert!(matches!(
            persistence.try_recv().unwrap(),
            Command::AutoSaveDraft(_)
        ));
        assert!(database.try_recv().is_err());
        app.database_preferences_saved(save + 1);
        assert!(
            !app.database_transfer
                .pending
                .as_ref()
                .unwrap()
                .preferences_saved
        );
        app.database_preferences_saved(save);
        assert!(database.try_recv().is_err());
        // The old save completed, but another edit belongs in the export too.
        app.composer.current.pending = None;
        app.composer.current.draft.revision = 5;
        app.composer.current.dirty = Some(Instant::now());
        app.advance_database_transfer();
        let Command::AutoSaveDraft(draft) = persistence.try_recv().unwrap() else {
            panic!("latest draft must be saved")
        };
        assert_eq!(draft.revision, 5);
        assert!(database.try_recv().is_err());
        app.composer.current.pending = None;
        app.composer.io = Some("unsent".into());
        app.advance_database_transfer();
        assert!(database.try_recv().is_err());
        app.composer.io = None;
        app.advance_database_transfer();
        assert!(
            matches!(database.try_recv().unwrap(), Command::Database(Request::Export { request: id, .. }) if id == request)
        );
        app.advance_database_transfer();
        assert!(database.try_recv().is_err(), "must not start another copy");
    }

    #[test]
    fn cancel_retries_backpressure_and_waits_for_receipt_then_ignores_old_results() {
        let (mut app, _) = App::new();
        let (tx, mut database, _persistence) = engine::CommandSender::database_test_channels();
        tx.try_send(Command::Database(Request::Cancel(999)))
            .unwrap();
        app.tx = Some(tx);
        let request = app.database_transfer.begin();
        app.database_transfer.pending.as_mut().unwrap().started = true;
        app.cancel_database_transfer();
        assert!(app.database_transfer.pending.as_ref().unwrap().cancelling);
        assert!(!app.database_transfer.pending.as_ref().unwrap().cancel_sent);
        database.try_recv().unwrap();
        app.advance_database_transfer();
        assert!(
            matches!(database.try_recv().unwrap(), Command::Database(Request::Cancel(id)) if id == request)
        );
        assert!(app.database_transfer.pending.is_some());
        // Cancellation races a commit: a saved file must still be reported.
        let _ = app.database_update(
            request,
            Update::Finished(Ok(Outcome::Saved {
                path: "/fixture/export.sqlite".into(),
                bytes: 500,
                warning: None,
            })),
        );
        assert!(app.database_transfer.pending.is_none());
        assert_eq!(app.database_transfer.saved.as_ref().unwrap().1, 500);
        let newer = app.database_transfer.begin();
        let _ = app.database_update(request, Update::Finished(Err("old failure".into())));
        assert_eq!(
            app.database_transfer.pending.as_ref().unwrap().request,
            newer
        );
        assert!(app.database_transfer.error.is_none());
        app.cancel_database_transfer();
        app.database_export_path(newer, Some("/stale/picker.sqlite".into()));
        assert!(app.database_transfer.pending.is_none());
    }

    #[test]
    fn save_failure_aborts_preparation_and_copy_failure_cancels_close() {
        let (mut app, _) = App::new();
        app.database_transfer.begin();
        app.database_transfer.pending.as_mut().unwrap().preferences = Some(41);
        app.database_preferences_failed(40, "unrelated");
        assert!(app.database_transfer.pending.is_some());
        app.database_preferences_failed(41, "disk full");
        assert!(app.database_transfer.pending.is_none());
        assert!(
            app.database_transfer
                .error
                .as_ref()
                .unwrap()
                .contains("disk full")
        );
        app.database_transfer.begin();
        app.fail_database_preparation("draft could not be saved");
        assert!(app.database_transfer.pending.is_none());
        let request = app.database_transfer.begin();
        app.database_transfer.pending.as_mut().unwrap().started = true;
        app.pending_close = Some(iced::window::Id::unique());
        let _ = app.database_update(request, Update::Finished(Err("destination full".into())));
        assert!(app.pending_close.is_none());
        assert!(app.database_transfer.saved.is_none());
        assert_eq!(
            app.database_transfer.error.as_deref(),
            Some("destination full")
        );
    }
}
