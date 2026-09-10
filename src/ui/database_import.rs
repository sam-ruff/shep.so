use super::*;
use crate::transfer::{
    Request, Update,
    import::{InstallPhase, Installed, Phase, Progress, Review},
};
use iced::{
    Alignment,
    widget::{checkbox, column, progress_bar, row, text, text_input},
};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub enum Action {
    Begin,
    Path(u64, Option<PathBuf>),
    Name(String),
    ConfirmReview(bool),
    Install,
    Cancel,
}
#[derive(Default)]
pub(super) struct State {
    serial: u64,
    pending: Option<Pending>,
    saved: Option<Installed>,
    error: Option<String>,
}
struct Pending {
    id: u64,
    started: bool,
    choosing: bool,
    cancelling: bool,
    cancel_sent: bool,
    progress: Progress,
    review: Option<Arc<Review>>,
    name: String,
    reviewed: bool,
    save: Option<u64>,
    save_acked: bool,
    installing: Option<InstallPhase>,
}
impl State {
    pub fn pending(&self) -> bool {
        self.pending.is_some()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observation(&self) -> serde_json::Value {
        serde_json::json!({
            "phase":self.pending.as_ref().map(|p| if p.cancelling {"Cancelling".into()} else if p.choosing {"Choosing".into()} else if let Some(phase) = p.installing {format!("{phase:?}")} else if p.save.is_some() {"Saving preferences".into()} else if p.review.is_some() {"Review".into()} else {format!("{:?}",p.progress.phase)}),
            "name":self.pending.as_ref().map(|p| &p.name),
            "reviewed":self.pending.as_ref().is_some_and(|p| p.reviewed),
            "messages":self.pending.as_ref().and_then(|p| p.review.as_ref().map(|r| r.messages)),
            "saved":self.saved.as_ref().map(|s| s.id.key()),
            "registered":self.saved.as_ref().is_some_and(|s| s.registered),
            "error":self.error,
        })
    }
}
fn pending_actions(review: &Review) -> u64 {
    review.pending_outgoing + review.pending_bulk + review.pending_folders + review.pending_moves
}
impl App {
    pub(super) fn database_import_action(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::Begin => {
                if self.database_import.pending()
                    || self.database_transfer.pending.is_some()
                    || self.tx.is_none()
                {
                    return Task::none();
                }
                self.database_import.serial += 1;
                let id = self.database_import.serial;
                self.database_import.error = None;
                self.database_import.pending = Some(Pending {
                    id,
                    started: false,
                    choosing: true,
                    cancelling: false,
                    cancel_sent: false,
                    progress: Default::default(),
                    review: None,
                    name: "Imported mail".into(),
                    reviewed: false,
                    save: None,
                    save_acked: false,
                    installing: None,
                });
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Import Shep database")
                            .add_filter("SQLite database", &["sqlite", "db"])
                            .pick_file()
                            .await
                            .map(|file| file.path().to_path_buf())
                    },
                    move |path| Message::DatabaseImport(Action::Path(id, path)),
                );
            }
            Action::Path(id, path) => {
                if !self
                    .database_import
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.id == id && p.choosing)
                {
                    return Task::none();
                }
                let Some(source) = path else {
                    self.database_import.pending = None;
                    return Task::none();
                };
                if self.try_command(Command::Database(Request::Import {
                    request: id,
                    source,
                })) {
                    let pending = self.database_import.pending.as_mut().unwrap();
                    pending.started = true;
                    pending.choosing = false;
                } else {
                    self.database_import.pending = None;
                    self.database_import.error =
                        Some("The import worker is busy. Choose the file again to retry.".into());
                }
            }
            Action::Name(name) => {
                if let Some(p) = &mut self.database_import.pending
                    && p.installing.is_none()
                    && p.save.is_none()
                {
                    p.name = name;
                }
            }
            Action::ConfirmReview(reviewed) => {
                if let Some(p) = &mut self.database_import.pending {
                    p.reviewed = reviewed;
                }
            }
            Action::Install => {
                let Some(p) = &self.database_import.pending else {
                    return Task::none();
                };
                if p.cancelling
                    || p.installing.is_some()
                    || p.save.is_some()
                    || !p
                        .review
                        .as_ref()
                        .is_some_and(|r| pending_actions(r) == 0 || p.reviewed)
                {
                    return Task::none();
                }
                if let Err(error) = crate::profiles::name_checked(p.name.clone())
                    .and_then(|_| self.read_preferences())
                {
                    self.database_import.error = Some(error.to_string());
                    return Task::none();
                }
                let generation = self.preference_sync.changed();
                if self.queue_preference_write(generation, self.preferences.clone()) {
                    self.database_import.pending.as_mut().unwrap().save = Some(generation);
                    self.database_import.error = None;
                } else {
                    self.database_import.error =
                        Some("Settings could not be saved. Retry the import.".into());
                }
            }
            Action::Cancel => {
                if let Some(p) = &mut self.database_import.pending {
                    if p.started {
                        p.cancelling = true;
                        self.advance_database_import();
                    } else {
                        self.database_import.pending = None;
                    }
                }
            }
        }
        Task::none()
    }

    pub(super) fn import_preferences_saved(&mut self, generation: u64) {
        if let Some(p) = &mut self.database_import.pending
            && p.save == Some(generation)
        {
            p.save_acked = true;
        }
        self.advance_database_import();
    }
    pub(super) fn import_preferences_failed(&mut self, generation: u64, error: &str) {
        if let Some(p) = &mut self.database_import.pending
            && p.save == Some(generation)
        {
            p.save = None;
            p.save_acked = false;
            self.database_import.error = Some(format!(
                "Settings could not be saved: {error}. Retry the import."
            ));
        }
    }
    pub(super) fn advance_database_import(&mut self) {
        let Some(p) = &self.database_import.pending else {
            return;
        };
        if p.cancelling {
            if !p.cancel_sent {
                let id = p.id;
                if self.try_command(Command::Database(Request::Cancel(id))) {
                    self.database_import.pending.as_mut().unwrap().cancel_sent = true;
                }
            }
        } else if p.save_acked && p.installing.is_none() {
            let command = Request::Install {
                request: p.id,
                name: p.name.clone(),
            };
            if self.try_command(Command::Database(command)) {
                self.database_import.pending.as_mut().unwrap().installing =
                    Some(InstallPhase::Preparing);
            } else {
                let p = self.database_import.pending.as_mut().unwrap();
                p.save = None;
                p.save_acked = false;
                self.database_import.error =
                    Some("The import worker is busy. Retry the import.".into());
            }
        }
    }
    pub(super) fn database_import_update(&mut self, id: u64, update: Update) -> Task<Message> {
        let Some(p) = self
            .database_import
            .pending
            .as_mut()
            .filter(|p| p.id == id && p.started)
        else {
            return Task::none();
        };
        match update {
            Update::ImportProgress(progress) => p.progress = progress,
            Update::Review(review) => p.review = Some(review),
            Update::Installing(phase) => p.installing = Some(phase),
            Update::ReviewError(error) => {
                p.installing = None;
                p.save = None;
                p.save_acked = false;
                self.database_import.error = Some(error);
            }
            Update::ImportFinished(result) => {
                self.database_import.pending = None;
                match result {
                    Ok(Some(saved)) => {
                        if let Some(warning) = &saved.warning {
                            self.database_import.error = Some(warning.clone());
                            self.notice(warning, true);
                            self.pending_close = None;
                        } else {
                            self.notice("Database imported. Choose it in Profiles to use it on the next launch.", false);
                        }
                        self.database_import.saved = Some(saved);
                        self.profile_action(profiles::Action::Refresh);
                    }
                    Ok(None) => self.notice("Database import cancelled", false),
                    Err(error) => {
                        self.database_import.error = Some(error.clone());
                        self.notice(format!("Database import failed: {error}"), true);
                        self.pending_close = None;
                    }
                }
                if let Some(window) = self.pending_close.take() {
                    return self.handle(Message::WindowClose(window));
                }
            }
            _ => {}
        }
        Task::none()
    }

    pub(super) fn database_import_controls(&self) -> Element<'_, Message> {
        use components::{action, muted, outline, primary};
        let state = &self.database_import;
        let mut content = column![].spacing(12);
        let message = |action| Message::DatabaseImport(action);
        if let Some(p) = &state.pending {
            if let Some(review) = &p.review
                && !p.cancelling
                && p.installing.is_none()
                && p.save.is_none()
            {
                content = content
                    .push(text("Review database import").size(16).font(BOLD))
                    .push(muted(format!(
                        "{} messages · {} accounts · {} drafts · {} calendars",
                        review.messages,
                        review.accounts.len(),
                        review.drafts,
                        review.calendars
                    )))
                    .push(
                        text_input("Profile name", &p.name)
                            .style(components::field)
                            .id("import-profile-name")
                            .on_input(move |v| message(Action::Name(v)))
                            .on_submit(message(Action::Install))
                            .padding(12)
                            .size(13),
                    );
                for account in review.accounts.iter().take(3) {
                    content = content
                        .push(muted(format!("{} · {}", account.name, account.email)).size(12));
                }
                if review.accounts.len() > 3 {
                    content = content.push(
                        muted(format!("and {} more accounts", review.accounts.len() - 3)).size(12),
                    );
                }
                content = content.push(muted("Adds a separate profile. Reconnect its accounts and Google sign-in after opening it.").size(12));
                if pending_actions(review) > 0 {
                    content = content.push(muted(format!("Review needed: {} outgoing messages, {} mail groups, {} folder changes and {} moves.", review.pending_outgoing, review.pending_bulk, review.pending_folders, review.pending_moves)).size(12))
                        .push(checkbox(p.reviewed).label("Keep unfinished actions for review before retrying").on_toggle(move |v| message(Action::ConfirmReview(v))).text_size(12));
                }
                let enabled = crate::profiles::name_checked(p.name.clone()).is_ok()
                    && (pending_actions(review) == 0 || p.reviewed);
                content = content.push(
                    row![
                        iced::widget::button(text("Import profile").size(12))
                            .padding([11, 14])
                            .style(primary)
                            .on_press_maybe(enabled.then(|| message(Action::Install))),
                        action("Cancel", message(Action::Cancel))
                    ]
                    .spacing(10),
                );
            } else {
                let label = if p.cancelling {
                    "Cancelling import…"
                } else if p.choosing {
                    "Choose a Shep database…"
                } else if let Some(phase) = p.installing {
                    match phase {
                        InstallPhase::Preparing => "Preparing imported profile…",
                        InstallPhase::Publishing => "Saving imported profile…",
                        InstallPhase::Registering => "Registering imported profile…",
                    }
                } else if p.save.is_some() {
                    "Saving your settings…"
                } else {
                    match p.progress.phase {
                        Phase::CheckingSource => "Checking selected database…",
                        Phase::Copying => "Copying database…",
                        Phase::CheckingCopy => "Checking database integrity…",
                        Phase::Reviewing => "Preparing import review…",
                    }
                };
                content = content.push(
                    row![muted(label), action("Cancel", message(Action::Cancel))]
                        .spacing(16)
                        .align_y(Alignment::Center),
                );
                if p.installing.is_none() && p.progress.total_pages > 0 {
                    content = content.push(
                        progress_bar(
                            0.0..=1.0,
                            p.progress.copied_pages as f32 / p.progress.total_pages as f32,
                        )
                        .girth(4),
                    );
                }
            }
        } else {
            content = content.push(
                iced::widget::button(text("Import database…").size(12))
                    .padding([11, 14])
                    .style(outline)
                    .on_press_maybe(
                        self.database_transfer
                            .pending
                            .is_none()
                            .then(|| message(Action::Begin)),
                    ),
            );
        }
        if let Some(saved) = &state.saved {
            content = content.push(
                row![
                    muted(format!("Imported {}", saved.name)).size(12),
                    action(
                        "Profiles",
                        Message::FindSetting(SettingsTab::Accounts, "Profiles")
                    )
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            );
        }
        if let Some(error) = &state.error {
            content = content.push(text(error).size(12));
        }
        content.into()
    }
}

#[cfg(test)]
mod tests;
