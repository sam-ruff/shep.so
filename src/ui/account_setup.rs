use super::*;
use crate::store::account_setup::{Attempt, Stage};
use iced::widget::{column, row, text};
use secrecy::{ExposeSecret, SecretString};

pub(super) struct Submission {
    account: Account,
    password: SecretString,
    smtp: SecretString,
}

#[derive(Default)]
pub(super) struct State {
    pub(super) pending: std::collections::HashMap<String, Submission>,
    rows: Vec<Attempt>,
    serial: u64,
    next: Option<String>,
    error: Option<String>,
    admission_error: Option<(String, String)>,
    active: std::collections::HashMap<String, String>,
    removed: std::collections::HashSet<String>,
    retired: std::collections::HashSet<String>,
    stopping: Option<u64>,
}

impl App {
    pub(super) fn cancel_account_setup_stop(&mut self) {
        self.account_setup.stopping = None;
    }

    pub(super) fn account_setup_removed(&mut self, account: &str) {
        self.account_setup.removed.insert(account.into());
        self.account_setup.serial += 1;
        let mut retired = Vec::new();
        self.account_setup.pending.retain(|id, submission| {
            if submission.account.id != account {
                return true;
            }
            retired.push(id.clone());
            false
        });
        self.account_setup.active.retain(|id, owner| {
            if owner != account {
                return true;
            }
            retired.push(id.clone());
            false
        });
        self.account_setup.rows.retain(|row| {
            if row.account.id != account {
                return true;
            }
            retired.push(row.id.clone());
            false
        });
        for id in retired {
            self.busy.remove(&format!("account:{id}"));
            if self
                .account_setup
                .admission_error
                .as_ref()
                .is_some_and(|(request, _)| request == &id)
            {
                self.account_setup.admission_error = None;
            }
            self.account_setup.retired.insert(id);
        }
    }

    pub(super) fn account_setup_needs_flush(&self) -> bool {
        !self.account_setup.pending.is_empty() || self.account_setup.stopping.is_some()
    }

    pub(super) fn account_admission_pending(&self) -> bool {
        self.account_setup
            .pending
            .values()
            .any(|p| p.account.id == self.field("id"))
    }

    pub(super) fn admit_account_setup(&mut self) {
        if self.account_admission_pending() {
            return;
        }
        if self.account_setup.pending.len() >= 32 {
            self.notice(
                "Wait for your saved account requests before adding another.",
                true,
            );
            return;
        }
        let account = match self.account_form() {
            Ok(account) => account,
            Err(error) => {
                self.notice(error.to_string(), true);
                return;
            }
        };
        let id = uuid::Uuid::new_v4().to_string();
        self.pending_close = None;
        self.account_setup.stopping = None;
        if self.bulk.stopped || self.bulk.stop_requested {
            if !self.try_command(Command::BulkResume(String::new())) {
                return;
            }
            self.bulk.stopped = false;
            self.bulk.stop_requested = false;
        }
        let previous = self
            .workspace
            .accounts
            .iter()
            .find(|a| a.id == account.id)
            .cloned();
        let pending = Submission {
            account: account.clone(),
            password: self.field("password").to_string().into(),
            smtp: self.field("smtp_password").to_string().into(),
        };
        self.fields.insert("id", account.id.clone());
        if !self.try_command(Command::AdmitAccount(id.clone(), account, previous)) {
            return;
        }
        self.account_setup.pending.insert(id, pending);
    }

    pub(super) fn account_setup_admitted(&mut self, id: String, result: Result<Attempt, String>) {
        let Some(pending) = self.account_setup.pending.remove(&id) else {
            return;
        };
        let attempt = match result {
            Ok(attempt) if attempt.id == id && attempt.account == pending.account => attempt,
            Ok(_) => {
                self.notice("The saved account request changed. Refresh Accounts.", true);
                return;
            }
            Err(error) => {
                self.pending_close = None;
                self.account_setup.admission_error = Some((id, error.clone()));
                self.notice(error, true);
                return;
            }
        };
        let same_form = self.dialog == Some(Dialog::Account)
            && self.account_form().ok().as_ref() == Some(&pending.account)
            && self.field("password") == pending.password.expose_secret()
            && self.field("smtp_password") == pending.smtp.expose_secret();
        self.account_setup
            .active
            .insert(id.clone(), attempt.account.id.clone());
        for older in &mut self.account_setup.rows {
            if older.account.id == attempt.account.id
                && older.id != id
                && older.stage != Stage::Activated
            {
                older.stage = Stage::Cancelled;
            }
        }
        self.account_setup_changed(attempt);
        if same_form {
            self.dialog = None;
            self.fields.clear();
            if self.tab == Tab::Preferences {
                self.settings_fields();
            }
        }
        if !self.try_command(Command::ConnectAccount(
            id.clone(),
            pending.password,
            pending.smtp,
        )) {
            self.account_setup.admission_error = Some((id, "Account setup is saved. Open Connection activity and re-enter credentials to continue.".into()));
        } else {
            self.notice(
                "Account setup saved. Checking the connection in the background.",
                false,
            );
        }
    }

    pub(super) fn account_setup_changed(&mut self, attempt: Attempt) {
        if self.account_setup.removed.contains(&attempt.account.id) {
            self.account_setup.retired.insert(attempt.id);
            return;
        }
        self.account_setup.serial += 1;
        if let Some(saved) = self
            .account_setup
            .rows
            .iter()
            .find(|row| row.id == attempt.id)
            && terminal(saved.stage)
            && !terminal(attempt.stage)
        {
            return;
        }
        let succeeded = attempt.stage == Stage::Activated;
        if terminal(attempt.stage) {
            self.account_setup.active.remove(&attempt.id);
        }
        if matches!(attempt.stage, Stage::Failed | Stage::Interrupted) {
            self.notice(
                "Account setup needs attention. Open Preferences, Accounts, Connection activity.",
                true,
            );
        }
        self.account_setup.rows.retain(|row| row.id != attempt.id);
        self.account_setup.rows.insert(0, attempt);
        self.account_setup.rows.truncate(20);
        if self
            .account_setup
            .rows
            .first()
            .is_some_and(|a| terminal(a.stage))
            && self.pending_close.is_none()
            && self.tx.is_some()
            && !self.busy.contains("credential-cleanup")
        {
            self.send(Command::CleanupCredentials);
        }
        if succeeded {
            self.send(Command::Sync);
        }
    }

    pub(super) fn load_account_setups(&mut self, after: Option<String>) {
        self.account_setup.serial += 1;
        self.send(Command::AccountSetups(self.account_setup.serial, after));
    }

    pub(super) fn pump_account_setup_close(&mut self) {
        if self.pending_close.is_none()
            || self.account_setup.active.is_empty()
            || !self.account_setup.pending.is_empty()
            || !self.bulk.stop_requested
            || self.account_setup.stopping.is_some()
        {
            return;
        }
        let generation = self.bulk.stop_generation;
        if self.try_command(Command::InterruptAccountSetups(generation)) {
            self.account_setup.stopping = Some(generation);
        }
    }

    pub(super) fn account_setups_stopped(&mut self, generation: u64, result: Result<(), String>) {
        if self.account_setup.stopping != Some(generation) || self.pending_close.is_none() {
            return;
        }
        self.account_setup.stopping = None;
        if let Err(error) = result {
            self.account_setup.admission_error =
                Some((format!("close:{generation}"), error.clone()));
            self.pending_close = None;
            self.notice(error, true);
            return;
        }
        for (id, _) in self.account_setup.active.drain() {
            self.busy.remove(&format!("account:{id}"));
            if let Some(attempt) = self.account_setup.rows.iter_mut().find(|a| a.id == id) {
                attempt.stage = Stage::Interrupted;
            }
        }
    }

    pub(super) fn stale_account_busy(&self, key: &str) -> bool {
        key.strip_prefix("account:").is_some_and(|id| {
            self.account_setup.retired.contains(id)
                || self
                    .account_setup
                    .rows
                    .iter()
                    .any(|a| a.id == id && terminal(a.stage))
        })
    }

    pub(super) fn dismiss_account_setup_error(&mut self, id: String) {
        if self
            .account_setup
            .admission_error
            .as_ref()
            .is_some_and(|(saved, _)| saved == &id)
        {
            self.account_setup.admission_error = None;
        }
    }

    pub(super) fn account_setups_loaded(
        &mut self,
        request: u64,
        result: Result<Vec<Attempt>, String>,
    ) {
        if request != self.account_setup.serial {
            return;
        }
        match result {
            Ok(mut rows) => {
                self.account_setup.next = (rows.len() > 20).then(|| rows[19].id.clone());
                rows.truncate(20);
                rows.retain(|row| !self.account_setup.removed.contains(&row.account.id));
                self.account_setup.rows = rows;
                self.account_setup.error = None;
            }
            Err(error) => self.account_setup.error = Some(error),
        }
    }

    pub(super) fn retry_account_setup(&mut self, id: String) {
        let Some(attempt) = self.account_setup.rows.iter().find(|a| a.id == id).cloned() else {
            return;
        };
        if matches!(attempt.stage, Stage::Activated | Stage::Cancelled) {
            return;
        }
        self.open_setup_account(attempt.account);
    }

    fn open_setup_account(&mut self, a: Account) {
        self.open(Dialog::Account);
        self.protocol = a.protocol;
        self.fields
            .insert("incoming_security", format!("{:?}", a.incoming_security));
        self.fields
            .insert("incoming_auth", format!("{:?}", a.incoming_auth));
        self.fields
            .insert("smtp_security", format!("{:?}", a.smtp_security()));
        self.fields
            .insert("smtp_auth", format!("{:?}", a.smtp_auth));
        for (key, value) in [
            ("id", a.id),
            ("name", a.name),
            ("email", a.email),
            ("host", a.host),
            ("port", a.port.to_string()),
            ("username", a.username),
            ("smtp_host", a.smtp_host),
            ("smtp_port", a.smtp_port.to_string()),
            ("smtp_username", a.smtp_username),
            ("smtp_separate", a.smtp_separate_password.to_string()),
            ("sent_copy", format!("{:?}", a.sent_copy)),
            ("sent_folder", a.sent_folder),
        ] {
            self.fields.insert(key, value);
        }
    }

    pub(super) fn account_setup_activity(&self) -> Element<'_, Message> {
        let mut body = column![
            text("Connection activity").size(14),
            action("Refresh connections", Message::AccountSetupPage(None))
        ]
        .spacing(10);
        if let Some(error) = &self.account_setup.error {
            body = body.push(text(error).size(12));
        }
        if let Some((id, error)) = &self.account_setup.admission_error {
            body = body.push(text(error).size(12)).push(action(
                "Dismiss setup message",
                Message::DismissAccountSetupError(id.clone()),
            ));
        }
        for attempt in &self.account_setup.rows {
            let status = match attempt.stage {
                Stage::Admitted => "Saved, waiting to connect",
                Stage::Staged => "Checking connection",
                Stage::Checked => "Activating checked credentials",
                Stage::Activated => "Connected",
                Stage::Failed => "Could not connect. Previous settings were kept.",
                Stage::Interrupted => "Interrupted. Re-enter credentials to continue.",
                Stage::Cancelled => "Replaced by a newer request",
            };
            let mut item =
                column![text(&attempt.account.name).size(13), text(status).size(12)].spacing(4);
            if let Some(error) = &attempt.error {
                item = item.push(text(error).size(12));
            }
            if matches!(
                attempt.stage,
                Stage::Failed | Stage::Interrupted | Stage::Admitted
            ) {
                item = item.push(action(
                    "Review connection",
                    Message::RetryAccountSetup(attempt.id.clone()),
                ));
            }
            body = body.push(item);
        }
        if let Some(next) = &self.account_setup.next {
            body = body.push(row![action(
                "Next connections",
                Message::AccountSetupPage(Some(next.clone()))
            )]);
        }
        body.into()
    }
}

fn terminal(stage: Stage) -> bool {
    matches!(
        stage,
        Stage::Activated | Stage::Failed | Stage::Interrupted | Stage::Cancelled
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    fn account() -> Account {
        serde_json::from_value(serde_json::json!({"id":"ui-account","name":"Personal","email":"sam@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"sam","smtp_host":"smtp.example.test","smtp_port":465})).expect("account")
    }

    #[tokio::test]
    async fn removal_retires_only_owned_requests_and_fences_late_progress_and_history() {
        let (mut app, _) = App::new();
        let store = Store::memory().expect("store");
        let removed = store
            .admit_account_setup(uuid::Uuid::new_v4().to_string(), account(), None)
            .await
            .expect("removed request");
        let mut other_account = account();
        other_account.id = "other-account".into();
        let other = store
            .admit_account_setup(uuid::Uuid::new_v4().to_string(), other_account, None)
            .await
            .expect("other request");
        app.account_setup.rows = vec![removed.clone(), other.clone()];
        for attempt in [&removed, &other] {
            app.account_setup
                .active
                .insert(attempt.id.clone(), attempt.account.id.clone());
            app.busy.insert(format!("account:{}", attempt.id));
        }
        app.account_setup.pending.insert(
            "pending-removed".into(),
            Submission {
                account: removed.account.clone(),
                password: "secret".into(),
                smtp: "".into(),
            },
        );
        app.account_setup.pending.insert(
            "pending-other".into(),
            Submission {
                account: other.account.clone(),
                password: "secret".into(),
                smtp: "".into(),
            },
        );
        app.account_setup.admission_error = Some(("other-error".into(), "Keep this error".into()));
        let history = app.account_setup.serial;
        app.removal.target = Some(crate::store::ConnectionRef {
            kind: crate::store::ConnectionKind::Account,
            id: removed.account.id.clone(),
        });
        app.removal.removing = Some(7);
        app.connection_removed(7, Ok(0));
        assert_eq!(app.account_setup.rows.len(), 1);
        assert_eq!(app.account_setup.rows[0].id, other.id);
        assert!(!app.account_setup.active.contains_key(&removed.id));
        assert!(app.account_setup.active.contains_key(&other.id));
        assert!(!app.account_setup.pending.contains_key("pending-removed"));
        assert!(app.account_setup.pending.contains_key("pending-other"));
        assert!(!app.busy.contains(&format!("account:{}", removed.id)));
        assert!(app.busy.contains(&format!("account:{}", other.id)));
        assert!(app.stale_account_busy(&format!("account:{}", removed.id)));
        assert!(app.stale_account_busy("account:pending-removed"));
        app.account_setup_admitted("pending-removed".into(), Err("Late admission".into()));
        let mut late = removed.clone();
        late.stage = Stage::Staged;
        app.account_setup_changed(late);
        app.account_setups_loaded(history, Ok(vec![removed.clone()]));
        assert_eq!(app.account_setup.rows[0].id, other.id);
        app.account_setups_loaded(app.account_setup.serial, Ok(vec![removed, other.clone()]));
        assert_eq!(app.account_setup.rows.len(), 1);
        assert_eq!(app.account_setup.rows[0].id, other.id);
        assert_eq!(
            app.account_setup
                .admission_error
                .as_ref()
                .map(|(_, e)| e.as_str()),
            Some("Keep this error")
        );
    }

    #[tokio::test]
    async fn admission_closes_matching_form_before_provider_work_and_keeps_old_account_active() {
        let (mut app, _) = App::new();
        let (sender, mut local, mut network) = engine::CommandSender::close_test_channels();
        app.tx = Some(sender);
        let original = account();
        Arc::make_mut(&mut app.workspace).accounts = vec![original.clone()];
        app.open_setup_account(original.clone());
        app.fields.insert("password", "fixture-secret".into());
        app.admit_account_setup();
        assert!(app.has_required_close_work());
        assert!(
            network.try_recv().is_err(),
            "no provider before local admission"
        );
        let Command::AdmitAccount(id, desired, previous) = local.try_recv().expect("input saved")
        else {
            panic!("admission");
        };
        let store = Store::memory().expect("store");
        store
            .save_account(original.clone())
            .await
            .expect("original");
        let attempt = store
            .admit_account_setup(id.clone(), desired, previous)
            .await
            .expect("admit");
        app.account_setup_admitted(id.clone(), Ok(attempt));
        assert!(app.dialog.is_none());
        assert!(!app.fields.contains_key("password"));
        assert_eq!(app.workspace.accounts, vec![original]);
        assert!(matches!(network.try_recv(), Ok(Command::ConnectAccount(key, ..)) if key == id));
        assert_eq!(app.account_setup.rows[0].stage, Stage::Admitted);
        assert!(
            app.has_required_close_work(),
            "queued staging owns a close dependency"
        );
    }

    #[tokio::test]
    async fn late_admission_does_not_close_newer_form_and_history_refresh_keeps_admission_failure()
    {
        let (mut app, _) = App::new();
        let (sender, mut local, _network) = engine::CommandSender::close_test_channels();
        app.tx = Some(sender);
        app.open_setup_account(account());
        app.fields.insert("password", "fixture-secret".into());
        app.admit_account_setup();
        let Command::AdmitAccount(id, account, previous) = local.try_recv().expect("admission")
        else {
            panic!("admission");
        };
        app.fields.insert("name", "Newer typed name".into());
        let store = Store::memory().expect("store");
        let attempt = store
            .admit_account_setup(id.clone(), account, previous)
            .await
            .expect("admit");
        app.account_setup_admitted(id, Ok(attempt));
        assert_eq!(app.dialog, Some(Dialog::Account));
        assert_eq!(app.field("name"), "Newer typed name");
        app.admit_account_setup();
        let Command::AdmitAccount(id, ..) = local.try_recv().expect("second admission") else {
            panic!("admission");
        };
        app.account_setup_admitted(id, Err("Local admission failed".into()));
        assert_eq!(app.field("password"), "fixture-secret");
        app.account_setups_loaded(app.account_setup.serial, Ok(vec![]));
        assert_eq!(
            app.account_setup
                .admission_error
                .as_ref()
                .map(|(_, error)| error.as_str()),
            Some("Local admission failed")
        );
    }

    #[tokio::test]
    async fn interrupted_attempt_reopens_configuration_without_passwords_and_stale_progress_cannot_reactivate_it()
     {
        let (mut app, _) = App::new();
        let store = Store::memory().expect("store");
        let staged = store
            .admit_account_setup(uuid::Uuid::new_v4().to_string(), account(), None)
            .await
            .expect("attempt");
        store
            .interrupt_account_setup(staged.id.clone())
            .await
            .expect("interrupt");
        let interrupted = store.account_setup(staged.id.clone()).await.expect("saved");
        app.account_setup_changed(interrupted);
        app.account_setup_changed(staged.clone());
        assert_eq!(app.account_setup.rows[0].stage, Stage::Interrupted);
        app.retry_account_setup(staged.id);
        assert_eq!(app.field("host"), "imap.example.test");
        assert!(app.field("password").is_empty());
        assert!(app.field("smtp_password").is_empty());
    }

    #[tokio::test]
    async fn new_input_cancels_close_and_old_stop_ack_cannot_release_its_connection() {
        let (mut app, _) = App::new();
        let (sender, mut local, mut network) = engine::CommandSender::close_test_channels();
        app.tx = Some(sender);
        let store = Store::memory().expect("store");
        app.open_setup_account(account());
        app.fields.insert("password", "first-secret".into());
        app.admit_account_setup();
        let Command::AdmitAccount(first, account, previous) = local.try_recv().expect("first")
        else {
            panic!("admit");
        };
        app.account_setup_admitted(
            first.clone(),
            Ok(store
                .admit_account_setup(first.clone(), account.clone(), previous)
                .await
                .expect("first saved")),
        );
        assert!(matches!(
            network.try_recv(),
            Ok(Command::ConnectAccount(..))
        ));
        let window = iced::window::Id::unique();
        let _ = app.update(Message::WindowClose(window));
        let Command::BulkStop(old) = local.try_recv().expect("stop") else {
            panic!("stop");
        };
        assert!(matches!(local.try_recv(), Ok(Command::InterruptAccountSetups(g)) if g == old));
        app.open_setup_account(account.clone());
        app.fields.insert("password", "second-secret".into());
        app.admit_account_setup();
        assert!(app.pending_close.is_none());
        assert!(matches!(local.try_recv(), Ok(Command::BulkResume(_))));
        let Command::AdmitAccount(second, desired, previous) = local.try_recv().expect("second")
        else {
            panic!("admit");
        };
        app.account_setup_admitted(
            second.clone(),
            Ok(store
                .admit_account_setup(second.clone(), desired, previous)
                .await
                .expect("second saved")),
        );
        let _ = app.update(Message::WindowClose(window));
        let Command::BulkStop(new) = local.try_recv().expect("new stop") else {
            panic!("stop");
        };
        assert!(new > old);
        assert!(matches!(local.try_recv(), Ok(Command::InterruptAccountSetups(g)) if g == new));
        app.account_setups_stopped(old, Ok(()));
        assert!(app.busy.contains(&format!("account:{second}")));
        assert!(app.account_setup_needs_flush());
        app.account_setups_stopped(new, Ok(()));
        assert!(!app.busy.contains(&format!("account:{second}")));
        assert!(!app.account_setup_needs_flush());
    }
}
