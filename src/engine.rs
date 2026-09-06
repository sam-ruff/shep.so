mod backups;
#[cfg(test)]
mod backups_tests;
mod dispatch;
pub use dispatch::CommandSender;

use crate::{
    backup::{self, BackupCopy, BackupProvider, BackupTarget, Snapshot},
    model::*,
    providers::{self, CalendarProvider},
    store::{Store, Workspace},
};
use anyhow::Context;
use futures::{SinkExt, Stream, StreamExt};
use secrecy::{ExposeSecret, SecretString};
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum Command {
    Query(u64, MailQuery, bool),
    LoadImages(Vec<String>),
    Detail {
        revision: u64,
        id: String,
        prefetch: bool,
    },
    SaveAccount(Account, SecretString, SecretString),
    TestConnection(Account, SecretString, SecretString, ConnectionTarget),
    SavePreferences(u64, Preferences),
    Sync,
    Move(Mail, String),
    Transfer(Mail, String, String),
    Flags(Mail),
    SaveDraft(Draft),
    AutoSaveDraft(Draft),
    SaveBeforeClose(Draft),
    Send(Draft),
    GoogleLogin(Preferences),
    CheckGoogleConnection,
    SaveCalendar(CalendarSource, SecretString),
    SyncCalendar,
    SaveEvent(CalendarEvent),
    DeleteEvent(CalendarEvent),
    Backup(BackupTarget, SecretString),
    AutomaticBackup(BackupTarget),
    ListBackups(u64, BackupTarget),
    Restore(BackupTarget, String, SecretString),
    ExportAttachment(String, usize, String),
    ExportMessage(String, String),
    #[cfg(test)]
    HoldBackend {
        started: Arc<tokio::sync::Barrier>,
        release: Arc<tokio::sync::Notify>,
    },
}
impl Command {
    fn key(&self) -> Option<String> {
        match self {
            Self::TestConnection(_, _, _, target) => Some(format!("test:{target:?}")),
            Self::Sync => Some("sync".into()),
            Self::SyncCalendar => Some("calendar".into()),
            Self::GoogleLogin(_) => Some("google".into()),
            Self::Backup(..) | Self::AutomaticBackup(_) | Self::Restore(..) => {
                Some("backup".into())
            }
            Self::Send(d) => Some(format!("send:{}", d.id)),
            Self::SaveEvent(e) | Self::DeleteEvent(e) => Some(format!("event:{}", e.key())),
            Self::Flags(m) | Self::Move(m, _) | Self::Transfer(m, _, _) => {
                Some(format!("message:{}", m.id))
            }
            _ => None,
        }
    }
}
#[derive(Debug, Clone)]
pub enum Event {
    Ready(CommandSender, Arc<Workspace>, bool),
    RemoteImage(String, Result<Vec<u8>, String>),
    Workspace(Arc<Workspace>),
    PreferencesSaved(u64, Arc<crate::store::PreferenceSnapshot>),
    Page(u64, Arc<MailPage>, bool),
    Detail {
        revision: u64,
        id: String,
        result: Result<Arc<MailDetail>, String>,
        prefetch: bool,
    },
    Changed,
    Calendar(Arc<Vec<CalendarEvent>>),
    Backups(u64, BackupTarget, Result<Arc<Vec<BackupCopy>>, String>),
    BackupSaved(BackupTarget, BackupCopy),
    BackupFinished(BackupTarget),
    Busy(String, bool),
    Notice(String),
    Error(String),
    GoogleConnected,
    AccountSaved,
    CalendarSaved,
    Sent(String),
    ReadyToClose,
    CalendarEventSaved(String),
    ConnectionTest(ConnectionTarget, Result<String, String>),
}
type AccountLocks =
    Arc<std::sync::Mutex<std::collections::HashMap<String, Arc<tokio::sync::Mutex<()>>>>>;
#[derive(Clone)]
struct Engine {
    store: Store,
    google: providers::google::Google,
    demo: bool,
    account_locks: AccountLocks,
    calendar_locks: AccountLocks,
    google_connection_lock: Arc<tokio::sync::Mutex<()>>,
    passphrases: Arc<dyn backup::PassphraseStore>,
}
type Output = futures::channel::mpsc::Sender<Event>;

pub fn subscription(demo: &bool) -> impl Stream<Item = Event> + use<> {
    let demo = *demo;
    iced::stream::channel(CHANNEL_CAPACITY, move |mut output: Output| async move {
        let (tx, input) = CommandSender::channel();
        let store = tokio::task::spawn_blocking(move || {
            if demo {
                Store::memory()
            } else {
                let path = directories::ProjectDirs::from("so", "shep", "Shep")
                    .context("Could not locate the app data directory")?
                    .data_local_dir()
                    .to_path_buf();
                std::fs::create_dir_all(&path)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
                }
                Store::open(path.join("shep.sqlite"))
            }
        })
        .await;
        let store = match store {
            Ok(Ok(s)) => s,
            other => {
                let _ = output
                    .send(Event::Error(format!(
                        "Could not open local storage: {}",
                        match other {
                            Ok(Err(e)) => e.to_string(),
                            Err(e) => e.to_string(),
                            _ => String::new(),
                        }
                    )))
                    .await;
                futures::future::pending::<()>().await;
                return;
            }
        };
        #[cfg(feature = "test-support")]
        if demo && let Err(e) = crate::test_support::seed_demo(&store).await {
            let _ = output.send(Event::Error(e.to_string())).await;
        }
        let engine = Engine {
            store,
            google: Default::default(),
            demo,
            account_locks: Default::default(),
            calendar_locks: Default::default(),
            google_connection_lock: Default::default(),
            passphrases: Arc::new(backup::OsPassphraseStore),
        };
        let workspace = match engine.store.workspace().await {
            Ok(w) => w,
            Err(e) => {
                let _ = output.send(Event::Error(e.to_string())).await;
                return;
            }
        };
        if !demo && !workspace.preferences.google_client_id.is_empty() {
            // Credential stores may wait for an unlock dialog. Show the cached
            // workspace immediately and check Google from the provider worker.
            let _ = tx.try_send(Command::CheckGoogleConnection);
        }
        let _ = output
            .send(Event::Ready(tx, Arc::new(workspace), false))
            .await;
        if let Ok(events) = engine.store.events().await {
            let _ = output.send(Event::Calendar(Arc::new(events))).await;
        }
        engine.run(input, output).await;
    })
}

impl Engine {
    async fn calendar_lock(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .calendar_locks
            .lock()
            .expect("calendar lock map poisoned")
            .entry(id.into())
            .or_default()
            .clone();
        lock.lock_owned().await
    }
    async fn account_lock(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .account_locks
            .lock()
            .expect("account lock map poisoned")
            .entry(id.into())
            .or_default()
            .clone();
        lock.lock_owned().await
    }

    async fn workspace(&self, output: &mut Output) -> anyhow::Result<()> {
        output
            .send(Event::Workspace(Arc::new(self.store.workspace().await?)))
            .await?;
        Ok(())
    }
    async fn account(&self, id: &str) -> anyhow::Result<Account> {
        self.store
            .get::<Vec<Account>>("accounts")
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .context("This account is no longer available")
    }
    async fn calendar_provider(
        &self,
        source: &CalendarSource,
    ) -> anyhow::Result<Box<dyn CalendarProvider>> {
        Ok(match source.kind {
            CalendarKind::Google => Box::new(providers::calendar::GoogleCalendar {
                google: self.google.clone(),
                preferences: self.store.get("preferences").await?,
            }),
            CalendarKind::CalDav => Box::new(providers::calendar::CalDav {
                http: self.google.http.clone(),
            }),
        })
    }
    async fn complete_calendar_write(
        &self,
        event: &CalendarEvent,
        saved: CalendarEvent,
        deleting: bool,
        committed: bool,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        let needs_refresh = !deleting && committed && saved.etag.is_none();
        let cache_result = if deleting {
            self.store
                .delete_event(event.source_id.clone(), event.id.clone())
                .await
        } else {
            self.store.save_event(saved).await
        };
        // A successful remote write must never be presented as an unsaved form
        // because a subsequent cache operation or refresh failed.
        if committed || cache_result.is_ok() {
            output.send(Event::CalendarEventSaved(event.key())).await?;
        }
        cache_result.context(if committed {
            "The calendar change was saved, but the local cache could not be updated. Sync calendar to reload it."
        } else { "Could not save the calendar change locally." })?;
        output
            .send(Event::Calendar(Arc::new(self.store.events().await?)))
            .await?;
        output
            .send(Event::Notice(
                if deleting {
                    "Event deleted."
                } else if needs_refresh {
                    "Event saved. Sync calendar before editing it again."
                } else {
                    "Event saved."
                }
                .into(),
            ))
            .await?;
        Ok(())
    }

    async fn backup_provider(
        &self,
        prefs: &Preferences,
    ) -> anyhow::Result<Box<dyn BackupProvider>> {
        Ok(match prefs.backup_destination {
            BackupDestination::Local => {
                anyhow::ensure!(
                    std::path::Path::new(&prefs.backup_folder).is_absolute(),
                    "Choose an absolute backup folder path in Preferences."
                );
                Box::new(backup::LocalBackup {
                    directory: prefs.backup_folder.clone().into(),
                })
            }
            BackupDestination::GoogleDrive => Box::new(backup::DriveBackup {
                google: self.google.clone(),
                preferences: prefs.clone(),
            }),
        })
    }
    async fn execute(&self, command: Command, mut output: Output) -> anyhow::Result<()> {
        let explicit_draft = matches!(&command, Command::SaveDraft(_));
        let closing = matches!(&command, Command::SaveBeforeClose(_));
        let deleting_event = matches!(&command, Command::DeleteEvent(_));
        match command {
            Command::CheckGoogleConnection => {
                if !self.demo && self.google.connected().await {
                    output.send(Event::GoogleConnected).await?;
                }
            }
            #[cfg(test)]
            Command::HoldBackend { started, release } => {
                started.wait().await;
                release.notified().await;
            }
            Command::LoadImages(urls) => {
                let results = futures::stream::iter(urls.into_iter().take(8))
                    .map(|url| async move {
                        let result = if self.demo {
                            Ok(include_bytes!("../assets/logo-light.webp").to_vec())
                        } else {
                            crate::remote_images::fetch(&url)
                                .await
                                .map_err(|e| format!("{e:#}"))
                        };
                        (url, result)
                    })
                    .buffer_unordered(2);
                futures::pin_mut!(results);
                while let Some((url, result)) = results.next().await {
                    output.send(Event::RemoteImage(url, result)).await?;
                }
            }
            Command::Query(generation, query, prefetch) => {
                output
                    .send(Event::Page(
                        generation,
                        Arc::new(self.store.query(query).await?),
                        prefetch,
                    ))
                    .await?;
            }
            Command::Detail {
                revision,
                id,
                prefetch,
            } => {
                let result = self
                    .store
                    .detail(id.clone())
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"));
                output
                    .send(Event::Detail {
                        revision,
                        id,
                        result,
                        prefetch,
                    })
                    .await?;
            }
            Command::TestConnection(account, password, smtp_password, target) => {
                let result = async {
                    account.validate()?;
                    anyhow::ensure!(!self.demo, "Connection tests require a real account. Test workspaces do not connect to mail servers.");
                    let secret = if target == ConnectionTarget::Smtp && account.smtp_auth == SmtpAuth::None { SecretString::from("") }
                    else if target == ConnectionTarget::Smtp && account.smtp_separate_password {
                        if smtp_password.expose_secret().is_empty() { providers::read_secret(&format!("{}:smtp", account.id)).await? } else { smtp_password }
                    } else if password.expose_secret().is_empty() { providers::read_secret(&account.id).await.context("Enter a password before testing a new account")? } else { password };
                    match target { ConnectionTarget::Incoming => providers::mail::test_incoming(&account, &secret).await, ConnectionTarget::Smtp => providers::mail::test_smtp(&account, &secret).await }
                }.await;
                output
                    .send(Event::ConnectionTest(
                        target,
                        result.map_err(|e| format!("{e:#}")),
                    ))
                    .await?;
            }
            Command::SaveAccount(account, password, smtp_password) => {
                anyhow::ensure!(
                    !self.demo,
                    "Account changes are disabled in preview. Relaunch without --demo to add an account."
                );
                account.validate()?;
                let password = if password.expose_secret().is_empty() {
                    providers::read_secret(&account.id)
                        .await
                        .context("Enter an account password or app password")?
                } else {
                    password
                };
                if account.smtp_separate_password {
                    let smtp_id = format!("{}:smtp", account.id);
                    let smtp_password = if smtp_password.expose_secret().is_empty() {
                        providers::read_secret(&smtp_id)
                            .await
                            .context("Enter the separate SMTP password")?
                    } else {
                        smtp_password
                    };
                    providers::write_secret(&smtp_id, smtp_password).await?;
                }
                providers::write_secret(&account.id, password)
                    .await
                    .context("Could not save the account credential")?;
                self.store.save_account(account).await?;
                self.workspace(&mut output).await?;
                output.send(Event::AccountSaved).await?;
                output
                    .send(Event::Notice(
                        "Account saved. Use Sync to receive your mail.".into(),
                    ))
                    .await?;
            }
            Command::SavePreferences(request, prefs) => {
                let snapshot = self.store.save_preferences(prefs).await?;
                output
                    .send(Event::PreferencesSaved(request, Arc::new(snapshot)))
                    .await?;
            }
            Command::Sync => {
                if self.demo {
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    output.send(Event::Notice("Preview messages are stored locally. Add an account outside preview to sync.".into())).await?;
                    return Ok(());
                }
                let accounts: Vec<Account> = self.store.get("accounts").await?;
                let results: Vec<_> = futures::stream::iter(accounts)
                    .map(|a| {
                        let engine = self.clone();
                        let output = output.clone();
                        async move {
                            let name = a.name.clone();
                            engine
                                .sync_account(a, output)
                                .await
                                .with_context(|| format!("{name} sync failed"))
                        }
                    })
                    .buffer_unordered(3)
                    .collect()
                    .await;
                for result in results {
                    if let Err(e) = result {
                        output.send(Event::Error(format!("{e:#}"))).await?;
                    }
                }
                self.workspace(&mut output).await?;
                output.send(Event::Changed).await?;
            }
            Command::Transfer(mail, destination, folder) => {
                let preferences: Preferences = self.store.get("preferences").await?;
                anyhow::ensure!(
                    preferences.cross_account_moves,
                    "Enable moving between accounts in Preferences first."
                );
                anyhow::ensure!(
                    mail.account_id != destination,
                    "Choose a different destination account."
                );
                // Always lock in the same order to prevent opposing transfers deadlocking.
                let mut ids = [mail.account_id.clone(), destination.clone()];
                ids.sort();
                let first = self.account_lock(&ids[0]).await;
                let second = self.account_lock(&ids[1]).await;
                let source = self.account(&mail.account_id).await?;
                let destination = self.account(&destination).await?;
                anyhow::ensure!(
                    source.protocol == Protocol::Imap && destination.protocol == Protocol::Imap,
                    "Moving between accounts requires two IMAP accounts. POP3 keeps server originals."
                );
                let raw = self.store.raw_message(mail.id.clone()).await?;
                if self.demo {
                    let moved = parse_mail(
                        &destination.id,
                        &format!("local-sent-transfer-{}", uuid::Uuid::new_v4()),
                        &folder,
                        raw,
                        mail.unread,
                        mail.starred,
                    )?;
                    self.store.upsert(vec![moved]).await?;
                } else {
                    let source_secret = providers::read_secret(&source.id).await?;
                    let destination_secret = providers::read_secret(&destination.id).await?;
                    let journal_key = format!("transfer:{}", mail.id);
                    let journal: Option<(String, String, String)> =
                        self.store.get(&journal_key).await?;
                    if let Some((account, target, stage)) = &journal {
                        anyhow::ensure!(
                            account == &destination.id && target == &folder,
                            "A transfer is already pending for this message. Resume with the same destination."
                        );
                        anyhow::ensure!(
                            stage == "copied",
                            "The previous upload was interrupted. The original is safe. Check the destination in webmail before moving it there manually; Shep will not upload a possible duplicate."
                        );
                    }
                    tokio::time::timeout(
                        Duration::from_secs(35),
                        providers::mail::prepare_transfer(&source, &source_secret, &mail),
                    )
                    .await??;
                    if journal.is_none() {
                        self.store
                            .put(
                                &journal_key,
                                Some((
                                    destination.id.clone(),
                                    folder.clone(),
                                    "uploading".to_owned(),
                                )),
                            )
                            .await?;
                        tokio::time::timeout(Duration::from_secs(60), providers::mail::append_transfer(&destination, &destination_secret, &mail, &folder, raw)).await
                            .context("Upload timed out; the source is retained. Check the destination before retrying.")??;
                        self.store
                            .put(
                                &journal_key,
                                Some((destination.id.clone(), folder.clone(), "copied".to_owned())),
                            )
                            .await?;
                    }
                    tokio::time::timeout(Duration::from_secs(35), providers::mail::finish_transfer(&source, &source_secret, &mail)).await
                        .context("The destination has a copy; source removal timed out. Retry the same destination to finish without uploading again.")?
                        .context("The destination has a copy; source removal could not be confirmed. Retry the same destination to finish.")?;
                }
                let journal_key = format!("transfer:{}", mail.id);
                self.store.remove(mail.id).await?;
                self.store
                    .put(&journal_key, Option::<(String, String, String)>::None)
                    .await?;
                drop(second);
                drop(first);
                output.send(Event::Changed).await?;
                output
                    .send(Event::Notice(format!(
                        "Moved to {} / {folder}.",
                        destination.name
                    )))
                    .await?;
                if !self.demo {
                    self.sync_account(destination, output.clone()).await?;
                }
            }
            Command::Move(mail, folder) => {
                let guard = self.account_lock(&mail.account_id).await;
                if !self.demo && !mail.remote_id.starts_with("local-sent-") {
                    let account = self.account(&mail.account_id).await?;
                    let password = providers::read_secret(&account.id).await?;
                    tokio::time::timeout(
                        Duration::from_secs(45),
                        providers::mail::provider(account.protocol)
                            .move_mail(&account, &password, &mail, &folder),
                    )
                    .await??;
                    if account.protocol == Protocol::Imap
                        && !mail.remote_id.starts_with("local-sent-")
                    {
                        self.store.remove(mail.id).await?;
                        output.send(Event::Changed).await?;
                        drop(guard);
                        self.sync_account(account, output.clone()).await?;
                    } else {
                        self.store.move_local(mail.id, folder.clone()).await?;
                    }
                } else {
                    self.store.move_local(mail.id, folder.clone()).await?;
                }
                output.send(Event::Changed).await?;
                output
                    .send(Event::Notice(format!("Moved to {folder}.")))
                    .await?;
            }
            Command::Flags(mail) => {
                let _guard = self.account_lock(&mail.account_id).await;
                if !self.demo && !mail.remote_id.starts_with("local-sent-") {
                    let account = self.account(&mail.account_id).await?;
                    if account.protocol == Protocol::Imap {
                        let password = providers::read_secret(&account.id).await?;
                        tokio::time::timeout(
                            Duration::from_secs(45),
                            providers::mail::provider(account.protocol)
                                .set_flags(&account, &password, &mail),
                        )
                        .await??;
                    }
                }
                self.store.flags(mail).await?;
                output.send(Event::Changed).await?;
            }
            Command::SaveDraft(draft)
            | Command::AutoSaveDraft(draft)
            | Command::SaveBeforeClose(draft) => {
                self.store.save_draft(draft).await?;
                self.workspace(&mut output).await?;
                if explicit_draft {
                    output.send(Event::Notice("Draft saved.".into())).await?;
                }
                if closing {
                    output.send(Event::ReadyToClose).await?;
                }
            }
            Command::Send(draft) => {
                self.store.save_draft(draft.clone()).await?;
                anyhow::ensure!(
                    !self.demo,
                    "Sending is disabled in preview. Your draft is saved locally."
                );
                let account = self.account(&draft.account_id).await?;
                let password = providers::read_secret(&account.id).await?;
                let password = if account.smtp_separate_password {
                    providers::read_secret(&format!("{}:smtp", account.id)).await?
                } else {
                    password
                };
                let raw = providers::mail::send(&account, &password, &draft).await?;
                let mail = parse_mail(
                    &account.id,
                    &format!("local-sent-{}", draft.id),
                    "Sent",
                    raw,
                    false,
                    false,
                )?;
                self.store.upsert(vec![mail]).await?;
                self.store.delete_draft(draft.id.clone()).await?;
                self.workspace(&mut output).await?;
                output.send(Event::Sent(draft.id)).await?;
                output.send(Event::Changed).await?;
                output.send(Event::Notice("Message sent.".into())).await?;
            }
            Command::GoogleLogin(prefs) => {
                anyhow::ensure!(!self.demo, "Google sign-in is disabled in preview.");
                prefs.validate()?;
                // The UI starts OAuth only after the corresponding preferences save
                // is acknowledged. A delayed provider job must not overwrite settings.
                let _guard = self.google_connection_lock.lock().await;
                self.google.login(&prefs).await?;
                self.store
                    .update_preferences(|current| {
                        current.google_connection_id = uuid::Uuid::new_v4().to_string();
                        if current.backup_destination == BackupDestination::GoogleDrive {
                            current.last_backup = None;
                            current.backup_ready = false;
                        }
                    })
                    .await?;
                for source in self.google.calendars(&prefs).await? {
                    self.store.save_source(source).await?;
                }
                self.workspace(&mut output).await?;
                output.send(Event::GoogleConnected).await?;
                output
                    .send(Event::Notice(
                        "Google connected. Drive backup is optional; calendars are ready to sync."
                            .into(),
                    ))
                    .await?;
            }
            Command::SaveCalendar(source, password) => {
                anyhow::ensure!(!self.demo, "Calendar connections are disabled in preview.");
                providers::calendar::validate_caldav_url(&source.url)?;
                anyhow::ensure!(
                    !source.name.is_empty() && !source.username.is_empty(),
                    "Enter a calendar name and username."
                );
                providers::write_secret(&source.id, password).await?;
                self.store.save_source(source).await?;
                self.workspace(&mut output).await?;
                output.send(Event::CalendarSaved).await?;
                output
                    .send(Event::Notice(
                        "CalDAV calendar saved. Use Sync calendar to connect.".into(),
                    ))
                    .await?;
            }
            Command::SyncCalendar => {
                if !self.demo {
                    let sources: Vec<CalendarSource> = self.store.get("calendars").await?;
                    let now = chrono::Utc::now();
                    for source in sources {
                        let _guard = self.calendar_lock(&source.id).await;
                        let result = async {
                            let events = self
                                .calendar_provider(&source)
                                .await?
                                .events(
                                    &source,
                                    now - chrono::Duration::days(90),
                                    now + chrono::Duration::days(365),
                                )
                                .await?;
                            self.store.replace_events(source.id.clone(), events).await
                        }
                        .await;
                        if let Err(e) = result {
                            output
                                .send(Event::Error(format!("{}: {e:#}", source.name)))
                                .await?;
                        }
                    }
                }
                output
                    .send(Event::Calendar(Arc::new(self.store.events().await?)))
                    .await?;
            }
            Command::SaveEvent(event) | Command::DeleteEvent(event) => {
                anyhow::ensure!(
                    deleting_event || event.end > event.start,
                    "The event must end after it starts."
                );
                let _guard = self.calendar_lock(&event.source_id).await;
                let saved = if self.demo {
                    event.clone()
                } else {
                    let source = self
                        .store
                        .get::<Vec<CalendarSource>>("calendars")
                        .await?
                        .into_iter()
                        .find(|s| s.id == event.source_id)
                        .context("Choose a connected calendar")?;
                    let provider = self.calendar_provider(&source).await?;
                    if deleting_event {
                        provider.delete_event(&source, &event).await?;
                        event.clone()
                    } else {
                        provider.save_event(&source, &event).await?
                    }
                };
                self.complete_calendar_write(
                    &event,
                    saved,
                    deleting_event,
                    !self.demo,
                    &mut output,
                )
                .await?;
            }
            Command::Backup(target, passphrase) => {
                self.run_backup(target, Some(passphrase), &mut output)
                    .await?;
            }
            Command::AutomaticBackup(target) => {
                self.run_backup(target, None, &mut output).await?;
            }
            Command::ListBackups(request, target) => {
                let result: anyhow::Result<Vec<BackupCopy>> = async {
                    anyhow::ensure!(!self.demo, "Backup listing is disabled in preview.");
                    let _guard = self.backup_connection_guard(&target).await;
                    let prefs = self.store.get("preferences").await?;
                    Self::check_backup_target(&target, &prefs)?;
                    self.backup_provider(&prefs).await?.list().await
                }
                .await;
                output
                    .send(Event::Backups(
                        request,
                        target,
                        result.map(Arc::new).map_err(|error| format!("{error:#}")),
                    ))
                    .await?;
            }
            Command::Restore(target, id, passphrase) => {
                anyhow::ensure!(!self.demo, "Restore is disabled in preview.");
                let _guard = self.backup_connection_guard(&target).await;
                let prefs = self.store.get("preferences").await?;
                Self::check_backup_target(&target, &prefs)?;
                let bytes = self.backup_provider(&prefs).await?.download(&id).await?;
                let snapshot =
                    tokio::task::spawn_blocking(move || backup::decrypt(&bytes, &passphrase))
                        .await??;
                for (id, secret) in snapshot.credentials {
                    providers::write_secret(&id, SecretString::from(secret)).await?;
                }
                for account in snapshot.accounts {
                    self.store.save_account(account).await?;
                }
                for source in snapshot.calendars {
                    self.store.save_source(source).await?;
                }
                for chunk in snapshot.messages.chunks(50) {
                    self.store.upsert(chunk.to_vec()).await?;
                }
                self.workspace(&mut output).await?;
                output.send(Event::Changed).await?;
                output.send(Event::Notice("Backup restored. Accounts without included passwords need their passwords entered again.".into())).await?;
            }
            Command::ExportAttachment(id, index, path) => {
                let detail = self.store.detail(id).await?;
                let attachment = detail
                    .attachments
                    .get(index)
                    .context("Attachment no longer exists")?;
                write_new(&path, &attachment.bytes).await?;
                output
                    .send(Event::Notice("Attachment saved.".into()))
                    .await?;
            }
            Command::ExportMessage(id, path) => {
                let raw = self
                    .store
                    .run(move |c| {
                        Ok(
                            c.query_row("SELECT raw FROM messages WHERE id=?", [id], |r| {
                                r.get::<_, Vec<u8>>(0)
                            })?,
                        )
                    })
                    .await?;
                write_new(&path, &raw).await?;
                output
                    .send(Event::Notice("Original email saved as .eml.".into()))
                    .await?;
            }
        }
        Ok(())
    }
    async fn sync_account(&self, account: Account, mut output: Output) -> anyhow::Result<()> {
        let _guard = self.account_lock(&account.id).await;
        let password = providers::read_secret(&account.id).await?;
        let known = self.store.known(account.id.clone()).await?;
        let (tx, mut rx) = mpsc::channel(8);
        let store = self.store.clone();
        let receive = async {
            let mut last = Instant::now();
            let mut skipped = 0;
            while let Some(mail) = rx.recv().await {
                if matches!(mail, MailSyncItem::SkippedLarge) {
                    skipped += 1;
                }
                let folders_changed = matches!(&mail, MailSyncItem::Folders(..));
                store.apply_sync(mail).await?;
                if folders_changed {
                    output
                        .send(Event::Workspace(Arc::new(store.workspace().await?)))
                        .await?;
                }
                if last.elapsed() > Duration::from_millis(250) {
                    output.send(Event::Changed).await?;
                    last = Instant::now();
                }
            }
            if skipped > 0 {
                output
                    .send(Event::Notice(format!(
                        "Skipped {skipped} messages larger than the 25 MiB download limit."
                    )))
                    .await?;
            }
            Ok::<_, anyhow::Error>(())
        };
        let provider = providers::mail::provider(account.protocol);
        let sync = tokio::time::timeout(
            Duration::from_secs(480),
            provider.sync(&account, &password, &known, tx),
        );
        let (folders, ()) = tokio::try_join!(
            async { sync.await.context("Account sync timed out")? },
            receive
        )?;
        self.store
            .run(move |c| {
                use rusqlite::OptionalExtension;
                let old: Option<String> = c
                    .query_row("SELECT value FROM kv WHERE key='folders'", [], |r| r.get(0))
                    .optional()?;
                let mut all: Vec<String> = old
                    .map(|s| serde_json::from_str(&s))
                    .transpose()?
                    .unwrap_or_default();
                for f in folders {
                    if !all.contains(&f) {
                        all.push(f);
                    }
                }
                c.execute(
                    "INSERT OR REPLACE INTO kv VALUES('folders',?)",
                    [serde_json::to_string(&all)?],
                )?;
                Ok(())
            })
            .await?;
        Ok(())
    }
}
async fn write_new(path: &str, bytes: &[u8]) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .await
        .context("Could not create the file. Choose a new filename in an existing folder.")?;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    Ok(())
}

#[cfg(test)]
mod calendar_tests {
    use super::*;

    fn engine() -> Engine {
        Engine {
            store: Store::memory().unwrap(),
            google: Default::default(),
            demo: true,
            account_locks: Default::default(),
            calendar_locks: Default::default(),
            google_connection_lock: Default::default(),
            passphrases: Arc::new(backup::OsPassphraseStore),
        }
    }
    fn event(source: &str) -> CalendarEvent {
        let start = chrono::Utc::now();
        CalendarEvent {
            id: "shared-uid".into(),
            source_id: source.into(),
            title: source.into(),
            start,
            end: start + chrono::Duration::hours(1),
            location: String::new(),
            description: String::new(),
            all_day: false,
            etag: None,
            remote_url: None,
        }
    }

    #[tokio::test]
    async fn calendar_engine_save_and_delete_keep_other_calendars_with_same_uid() {
        let engine = engine();
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let home = event("home");
        let work = event("work");
        engine
            .execute(Command::SaveEvent(home.clone()), output.clone())
            .await
            .unwrap();
        engine
            .execute(Command::SaveEvent(work.clone()), output.clone())
            .await
            .unwrap();
        engine
            .execute(Command::DeleteEvent(work), output.clone())
            .await
            .unwrap();
        drop(output);
        let stored = engine.store.events().await.unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].source_id, "home");
        let mut completions = Vec::new();
        while let Some(event) = events.next().await {
            if let Event::CalendarEventSaved(key) = event {
                completions.push(key);
            }
        }
        assert_eq!(completions.len(), 3);
        assert_eq!(completions[0], home.key());
        assert_ne!(completions[0], completions[1]);
        assert_eq!(completions[1], completions[2]);
    }

    #[tokio::test]
    async fn calendar_remote_commit_closes_form_even_if_cache_update_fails() {
        let engine = engine();
        engine
            .store
            .run(|c| {
                c.execute("DROP TABLE events", [])?;
                Ok(())
            })
            .await
            .unwrap();
        let event = event("home");
        let (mut output, mut events) = futures::channel::mpsc::channel(8);
        let error = engine
            .complete_calendar_write(&event, event.clone(), false, true, &mut output)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("change was saved"));
        assert!(
            matches!(events.next().await, Some(Event::CalendarEventSaved(key)) if key == event.key())
        );
        // A failed local-only fixture write must not be acknowledged as saved.
        let error = engine
            .complete_calendar_write(&event, event.clone(), false, false, &mut output)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("Could not save"));
        drop(output);
        assert!(events.next().await.is_none());
    }
}
