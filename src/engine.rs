mod account_setup;
mod account_sync;
mod account_work;
mod backups;
#[cfg(test)]
mod backups_tests;
mod bulk;
mod calendar_connections;
mod database_transfers;
mod dispatch;
pub mod folders;
mod google_lifecycle;
mod mail_actions;
mod mail_sync;
mod move_recovery;
mod outgoing;
mod profile_sync;
mod profiles;
mod removals;
mod restore;
#[cfg(test)]
mod restore_tests;
pub mod selections;
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
    ProfileSync(crate::profile_sync::commands::Request),
    Profiles(u64, crate::profiles::Request),
    Database(crate::transfer::Request),
    Folder(folders::Request),
    MoveRecoveries(u64, Option<String>),
    RecoverMailMove(
        u64,
        Arc<crate::mail_actions::journal::MoveRecord>,
        crate::mail_actions::journal::RecoveryAction,
        bool,
    ),
    Query(u64, MailQuery, bool),
    Selection(u64, selections::Request, Vec<String>),
    ReviewSelection(u64, crate::store::MailSelectionId, u64, Vec<String>),
    ReleaseSelection(crate::store::MailSelectionId),
    BulkStart(String, crate::store::MailSelectionId, crate::bulk::Action),
    BulkRun(String),
    BulkStop,
    BulkResume(String),
    BulkUndo(String),
    BulkJobs(u64, usize),
    BulkItems(u64, String, Option<u64>),
    BulkResolve(String),
    Conversation(u64, String, Option<String>, Option<usize>),
    IndexConversations,
    LoadImages(Vec<String>),
    Detail {
        revision: u64,
        id: String,
        prefetch: bool,
    },
    SaveAccount(Account, SecretString, SecretString),
    TestConnection(Account, SecretString, SecretString, ConnectionTarget),
    SavePreferences(u64, crate::preference_edits::Write),
    Sync,
    Move(u64, Mail, String),
    Transfer(u64, Mail, String, String),
    UndoMove(u64, Mail, Arc<crate::mail_actions::MoveReceipt>),
    Flags(u64, Mail, crate::mail_actions::Flags),
    SaveDraft(Draft),
    AutoSaveDraft(Draft),
    DeleteDraft(String),
    ForwardDraft(String, String),
    Print(u64, String, crate::printing::Options),
    AddDraftFiles(Draft, Vec<std::path::PathBuf>),
    RemoveDraftFile(String, String),
    Send(Draft),
    OutgoingPage(u64, usize),
    ResolveOutgoing(String, crate::outgoing::RecoveryAction, bool),
    RepairOutgoing,
    GoogleLogin(Preferences, bool),
    DisconnectGoogle(u64),
    CleanupGoogle,
    CheckGoogleConnection,
    DiscoverCalendars(u64, String, String, SecretString),
    ConnectCalendars(u64, u64, Vec<CalendarSource>, SecretString),
    RemovalPreview(u64, crate::store::ConnectionRef),
    RemoveConnection(u64, crate::store::RemovalPreview, bool),
    CleanupCredentials,
    RestoreGoogleCalendars,
    SyncCalendar,
    SaveEvent(CalendarEvent),
    DeleteEvent(CalendarEvent),
    Backup(BackupTarget, SecretString),
    AutomaticBackup(BackupTarget),
    BackupIncluded(u64, String, BackupTarget),
    ConnectS3(u64, BackupTarget, Option<(SecretString, SecretString)>),
    ConnectSftp(u64, BackupTarget, Option<SecretString>),
    ConnectFtp(u64, BackupTarget, Option<SecretString>),
    ProbeSftp(u64, backup::sftp::Settings),
    ListBackups(u64, BackupTarget),
    BackupHistory(u64, BackupTarget),
    RetryBackupHistory(String, BackupTarget, SecretString),
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
    pub(crate) fn key(&self) -> Option<String> {
        match self {
            Self::SaveAccount(account, ..) => Some(format!("account:{}", account.id)),
            Self::RecoverMailMove(request, record, ..) => {
                Some(format!("move-recovery:{}:{request}", record.token))
            }
            Self::TestConnection(_, _, _, target) => Some(format!("test:{target:?}")),
            Self::ResolveOutgoing(id, ..) => Some(format!("outgoing:{id}")),
            Self::RepairOutgoing => Some("outgoing-repair".into()),
            Self::Sync => Some("sync".into()),
            Self::SyncCalendar => Some("calendar".into()),
            Self::CleanupCredentials => Some("credential-cleanup".into()),
            Self::RestoreGoogleCalendars => Some("restore-calendars".into()),
            Self::GoogleLogin(..) => Some("google".into()),
            Self::DisconnectGoogle(_) | Self::CleanupGoogle => Some("google-disconnect".into()),
            Self::Backup(target, _)
            | Self::RetryBackupHistory(_, target, _)
            | Self::AutomaticBackup(target)
            | Self::BackupIncluded(_, _, target)
            | Self::Restore(target, ..)
            | Self::ConnectS3(_, target, _)
            | Self::ConnectSftp(_, target, _)
            | Self::ConnectFtp(_, target, _) => Some(target.work_key()),
            Self::ProbeSftp(_, settings) => {
                Some(format!("sftp-probe:{}:{}", settings.host, settings.port))
            }
            Self::Send(d) => Some(format!("send:{}", d.id)),
            Self::SaveEvent(e) | Self::DeleteEvent(e) => Some(format!("event:{}", e.key())),
            Self::Flags(request, m, _) => Some(format!("flags:{}:{request}", m.id)),
            Self::UndoMove(request, m, _) => Some(format!("undo:{}:{request}", m.id)),
            Self::Move(request, m, _) => Some(format!("move:{}:{request}", m.id)),
            Self::Transfer(request, m, _, _) => Some(format!("transfer:{}:{request}", m.id)),
            _ => None,
        }
    }
}
#[derive(Debug, Clone)]
pub enum Event {
    ProfileSync(u64, crate::profile_sync::commands::Update),
    Profiles(u64, Result<Arc<crate::profiles::Snapshot>, String>),
    Database(u64, crate::transfer::Update),
    Folder(folders::Event),
    MoveRecoveries(
        u64,
        Result<Arc<Vec<crate::mail_actions::journal::MoveRecord>>, String>,
    ),
    MailMoveRecovery(
        u64,
        String,
        Result<Arc<crate::mail_actions::journal::MoveRecord>, String>,
    ),
    MoveRecovered(Arc<crate::mail_actions::journal::MoveRecord>),
    Ready(CommandSender, Arc<Workspace>, bool),
    Selection(
        u64,
        Result<Option<Arc<crate::store::SelectionSnapshot>>, String>,
    ),
    BulkReview(u64, Result<Arc<crate::store::SelectionSnapshot>, String>),
    BulkStarted(String, Result<Arc<crate::bulk::Job>, String>),
    BulkUpdate(Arc<crate::bulk::Job>),
    BulkIdentity(String, String, Option<String>),
    BulkStopped,
    BulkResumed(String),
    BulkFinished(String, Result<Arc<crate::bulk::Job>, String>),
    BulkJobs(u64, Result<Arc<Vec<crate::bulk::Job>>, String>),
    BulkItems(u64, String, Result<Arc<Vec<crate::bulk::Item>>, String>),
    RemoteImage(String, Result<Vec<u8>, String>),
    Workspace(Arc<Workspace>),
    PreferencesSaved(u64, Arc<crate::store::PreferenceSnapshot>),
    PreferencesSaveFailed(u64, String),
    Page(u64, Arc<MailPage>, bool),
    Conversation(
        u64,
        String,
        Result<Arc<crate::store::ConversationPage>, String>,
    ),
    ConversationsIndexed(Result<(), String>),
    Detail {
        revision: u64,
        id: String,
        result: Result<Arc<MailDetail>, String>,
        prefetch: bool,
    },
    MailSyncFinished(Result<(), String>),
    MailArrived(Arc<crate::notifications::Arrival>),
    FlagsFinished(u64, Mail, Result<(), String>),
    MoveFinished(
        u64,
        Mail,
        String,
        Result<Arc<crate::mail_actions::MoveReceipt>, String>,
    ),
    TransferFinished(
        u64,
        Mail,
        Result<Arc<crate::mail_actions::MoveReceipt>, String>,
    ),
    UndoFinished(
        u64,
        Mail,
        Result<Arc<crate::mail_actions::MoveReceipt>, String>,
    ),
    #[cfg(feature = "test-support")]
    PreviewSync(u64),
    #[cfg(feature = "test-support")]
    PreviewAccountSync(bool),
    Changed,
    Calendar(u64, Arc<Vec<CalendarEvent>>),
    Backups(u64, BackupTarget, Result<Arc<Vec<BackupCopy>>, String>),
    BackupSaved(BackupTarget, BackupCopy),
    S3Connection(u64, BackupTarget, Result<(), String>),
    SftpConnection(u64, BackupTarget, Result<(), String>),
    FtpConnection(u64, BackupTarget, Result<(), String>),
    SftpFingerprint(u64, backup::sftp::Settings, Result<String, String>),
    BackupFinished(BackupTarget),
    BackupRun(u64, BackupTarget, backup::run::Status),
    BackupHistory(
        u64,
        BackupTarget,
        Result<Arc<Vec<backup::history::Entry>>, String>,
    ),
    BackupHistoryChanged(BackupTarget),
    Busy(String, bool),
    Notice(String),
    Error(String),
    GoogleStatus(u64, bool),
    GoogleDisconnected(u64, Result<(), String>),
    AccountSaved(String),
    RemovalPreview(u64, Result<crate::store::RemovalPreview, String>),
    ConnectionRemoved(u64, Result<usize, String>),
    CalendarsDiscovered(
        u64,
        Result<Vec<providers::calendar::discovery::DiscoveredCalendar>, String>,
    ),
    CalendarsConnected(u64, Result<(), String>),
    DraftSaved(String, u64, Result<Arc<crate::store::DraftState>, String>),
    DraftDeleted(String, Result<Arc<crate::store::DraftState>, String>),
    DraftFiles(String, Result<Arc<crate::store::DraftState>, String>),
    ForwardDraft(String, Result<Arc<crate::store::DraftState>, String>),
    Print(u64, Result<Arc<crate::printing::Preview>, String>),
    Sent(String, u64),
    SubmissionQueued(String, u64),
    OutgoingPage(u64, Result<Arc<crate::outgoing::OutgoingPage>, String>),
    OutgoingChanged,
    ReviewOutgoing(String, u64),
    CalendarEventSaved(String),
    ConnectionTest(ConnectionTarget, Result<String, String>),
}
#[derive(Clone)]
struct Engine {
    profiles: Option<crate::profiles::Session>,
    credentials: crate::credentials::Credentials,
    store: Store,
    google: providers::google::Google,
    demo: bool,
    account_work: account_work::Accounts,
    calendar_work: account_work::Accounts,
    calendar_setup_lock: Arc<tokio::sync::Mutex<()>>,
    connection_lifecycle_lock: Arc<tokio::sync::Mutex<()>>,
    secret_remover: Arc<dyn removals::SecretRemover>,
    outbound: Arc<dyn providers::outgoing::Outbound>,
    google_connection_lock: Arc<tokio::sync::RwLock<()>>,
    passphrases: Arc<dyn backup::PassphraseStore>,
    restore_credentials: Arc<dyn backup::restore::CredentialRestorer>,
    backup_uploads: Arc<tokio::sync::OnceCell<backup::journal::Journal>>,
    mail_sync_settings: mail_sync::Settings,
    provider_slots: dispatch::Slots,
    printing: crate::printing::Service,
    bulk_control: Arc<bulk::Control>,
}
type Output = futures::channel::mpsc::Sender<Event>;

pub fn subscription(demo: &bool) -> impl Stream<Item = Event> + use<> {
    let demo = *demo;
    iced::stream::channel(CHANNEL_CAPACITY, move |mut output: Output| async move {
        let (tx, input) = CommandSender::channel();
        let (store, profiles) = match profiles::open_workspace(demo).await {
            Ok(opened) => opened,
            Err(error) => {
                let _ = output
                    .send(Event::Error(format!(
                        "Could not open local storage: {}",
                        error
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
        let credentials = crate::credentials::Credentials::new(
            profiles
                .as_ref()
                .map(|p| p.current.scope())
                .unwrap_or_default(),
        );
        #[cfg(feature = "test-support")]
        let credentials = if demo && crate::test_support::backups::active() {
            match crate::test_support::backups::credentials(&store).await {
                Ok(credentials) => credentials,
                Err(error) => {
                    let _ = output.send(Event::Error(error.to_string())).await;
                    return;
                }
            }
        } else {
            credentials
        };
        let engine = Engine {
            profiles,
            credentials: credentials.clone(),
            store,
            google: providers::google::Google::with_credentials(credentials.clone()),
            demo,
            account_work: Default::default(),
            calendar_work: Default::default(),
            calendar_setup_lock: Default::default(),
            connection_lifecycle_lock: Default::default(),
            secret_remover: Arc::new(removals::OsSecretRemover(credentials.clone())),
            outbound: Arc::new(providers::outgoing::Servers {
                credentials: credentials.clone(),
            }),
            google_connection_lock: Default::default(),
            passphrases: Arc::new(backup::OsPassphraseStore(credentials.clone())),
            restore_credentials: Arc::new(backup::restore::OsCredentialRestorer(credentials)),
            backup_uploads: Default::default(),
            mail_sync_settings: Default::default(),
            provider_slots: Default::default(),
            printing: Default::default(),
            bulk_control: Default::default(),
        };
        // An owned fixture can keep all network capacity occupied indefinitely.
        // It proves close/read/persistence behavior without a real provider.
        #[cfg(feature = "test-support")]
        let _held_provider_slots = {
            let mut slots = Vec::new();
            if demo && std::env::args().any(|a| a == "--held-provider-slots") {
                for _ in 0..8 {
                    slots.push(engine.provider_slots.acquire().await);
                }
            }
            slots
        };
        let workspace = match engine.store.workspace().await {
            Ok(w) => w,
            Err(e) => {
                let _ = output.send(Event::Error(e.to_string())).await;
                return;
            }
        };
        engine
            .mail_sync_settings
            .set(workspace.preferences.mail_check_seconds);
        if !demo && workspace.preferences.google_lifecycle.cleanup_pending {
            let _ = tx.try_send(Command::CleanupGoogle);
        } else if !demo && !workspace.preferences.active_google_client().is_empty() {
            // Credential stores may wait for an unlock dialog. Show the cached
            // workspace immediately and check Google from the provider worker.
            let _ = tx.try_send(Command::CheckGoogleConnection);
        }
        #[cfg(feature = "test-support")]
        if demo
            && std::env::args().any(|a| a == "--profile-login")
            && std::env::args().any(|a| a.starts_with("--profile-drive-url="))
        {
            let _ = tx.try_send(Command::CheckGoogleConnection);
        }
        let _ = tx.try_send(Command::IndexConversations);
        let held_capacity_fixture = demo
            && cfg!(feature = "test-support")
            && std::env::args().any(|a| a == "--held-provider-slots");
        if !held_capacity_fixture {
            let _ = tx.try_send(Command::CleanupCredentials);
            let _ = tx.try_send(Command::RepairOutgoing);
        }
        if engine.profiles.is_some() {
            let _ = tx.try_send(Command::Profiles(
                0,
                crate::profiles::Request::List { offset: 0 },
            ));
        }
        let preview_google = demo && workspace.preferences.google_grant.access.known;
        let _ = output
            .send(Event::Ready(tx, Arc::new(workspace), preview_google))
            .await;
        if let Ok((revision, events)) = engine.store.calendar_snapshot().await {
            let _ = output
                .send(Event::Calendar(revision, Arc::new(events)))
                .await;
        }
        engine.run(input, output).await;
    })
}

impl Engine {
    async fn send_calendar(&self, output: &mut Output) -> anyhow::Result<()> {
        let (revision, events) = self.store.calendar_snapshot().await?;
        output
            .send(Event::Calendar(revision, Arc::new(events)))
            .await?;
        Ok(())
    }

    async fn calendar_access(&self, id: &str) -> account_work::Access {
        self.calendar_work.write(id).await
    }
    async fn account_access(&self, id: &str) -> account_work::Access {
        self.account_work.write(id).await
    }

    async fn workspace(&self, output: &mut Output) -> anyhow::Result<()> {
        output
            .send(Event::Workspace(Arc::new(self.store.workspace().await?)))
            .await?;
        Ok(())
    }
    async fn account(&self, id: &str) -> anyhow::Result<Account> {
        self.store.require_account_reconnected(id.into()).await?;
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
                credentials: self.credentials.clone(),
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
        self.send_calendar(output).await?;
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
        #[cfg(feature = "test-support")]
        if self.demo && crate::test_support::backups::active() {
            return crate::test_support::backups::provider(&self.store, prefs);
        }
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
            BackupDestination::Ftp => {
                let secret = self.credentials.read(&prefs.backup_ftp.secret_id()).await.map_err(|_| anyhow::anyhow!("FTP credentials are unavailable. Open Backups and test and save this connection."))?;
                Box::new(backup::ftp::FtpBackup::new(&prefs.backup_ftp, secret)?)
            }
            BackupDestination::Sftp => {
                let secret = self.credentials.read(&prefs.backup_sftp.secret_id()).await
                    .map_err(|_| anyhow::anyhow!("SFTP credentials are unavailable for this verified server. Open Backups and test and save the connection."))?;
                Box::new(backup::sftp::SftpBackup::new(&prefs.backup_sftp, secret)?)
            }
            BackupDestination::S3 => {
                let secret = self.credentials.read(&prefs.backup_s3.identity().secret_id()).await
                    .map_err(|_| anyhow::anyhow!("S3 credentials are unavailable. Open Backups and test and save this connection."))?;
                Box::new(backup::s3::S3Backup::from_secret(
                    &prefs.backup_s3,
                    &secret,
                )?)
            }
            BackupDestination::GoogleDrive => {
                Box::new(backup::DriveBackup::new(self.google.clone(), prefs.clone()))
            }
        })
    }
    async fn execute(&self, command: Command, mut output: Output) -> anyhow::Result<()> {
        let explicit_draft = matches!(&command, Command::SaveDraft(_));
        let deleting_event = matches!(&command, Command::DeleteEvent(_));
        match command {
            Command::ProfileSync(_) => anyhow::bail!("Profile sync reached the wrong worker"),
            Command::Database(_) => anyhow::bail!("Database transfer reached the wrong worker"),
            Command::Profiles(request, action) => {
                return self.profiles_command(request, action, output).await;
            }
            Command::ReviewSelection(serial, id, revision, visible) => {
                let result = async {
                    let frozen = self.store.freeze_selection(id, revision).await?;
                    match self.store.selection_snapshot(frozen.id, visible).await {
                        Ok(snapshot) => Ok(snapshot),
                        Err(error) => {
                            let _ = self.store.release_selection(frozen.id).await;
                            Err(error)
                        }
                    }
                }
                .await
                .map(Arc::new)
                .map_err(|e: anyhow::Error| format!("{e:#}"));
                output.send(Event::BulkReview(serial, result)).await?;
            }
            Command::ReleaseSelection(id) => self.store.release_selection(id).await?,
            Command::Folder(request) => self.folder_command(request, output).await?,
            Command::BulkStart(id, selection, action) => {
                let result = self
                    .store
                    .start_bulk(id.clone(), selection, action)
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"));
                output.send(Event::BulkStarted(id, result)).await?;
            }
            Command::BulkResume(id) => {
                if !id.is_empty() {
                    self.store.continue_bulk(id.clone()).await?;
                }
                self.bulk_control.stopping.set(false);
                output.send(Event::BulkResumed(id)).await?;
            }
            Command::BulkStop => {
                self.bulk_control.stopping.set(true);
                if !self.bulk_control.active.get() {
                    output.send(Event::BulkStopped).await?;
                }
            }
            Command::BulkRun(_) => anyhow::bail!("Mail groups use their dedicated execution queue"),
            Command::BulkUndo(id) => match self.store.request_bulk_undo(id.clone()).await {
                Ok(job) => output.send(Event::BulkUpdate(Arc::new(job))).await?,
                Err(error) => {
                    output
                        .send(Event::BulkFinished(id, Err(format!("{error:#}"))))
                        .await?
                }
            },
            Command::BulkJobs(serial, offset) => {
                let result = self
                    .store
                    .bulk_jobs(offset)
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"));
                output.send(Event::BulkJobs(serial, result)).await?;
            }
            Command::BulkItems(serial, id, after) => {
                let result = self
                    .store
                    .bulk_items(id.clone(), after)
                    .await
                    .map(Arc::new)
                    .map_err(|e| format!("{e:#}"));
                output.send(Event::BulkItems(serial, id, result)).await?;
            }
            Command::BulkResolve(id) => {
                let result = async {
                    let lease = self.store.bulk_lease(id.clone()).await?;
                    self.store.accept_bulk_uncertainty(&lease).await
                }
                .await
                .map(Arc::new)
                .map_err(|e: anyhow::Error| format!("{e:#}"));
                output.send(Event::BulkFinished(id, result)).await?;
            }
            Command::Selection(serial, request, visible) => {
                self.selection(serial, request, visible, output).await?;
            }
            Command::CheckGoogleConnection => {
                let _guard = self.google_connection_lock.read().await;
                let prefs: Preferences = self.store.get("preferences").await?;
                let connected = !self.demo && self.google.connected(&prefs).await?;
                #[cfg(feature = "test-support")]
                let connected = connected
                    || (self.demo
                        && std::env::args().any(|a| a == "--profile-login")
                        && std::env::args().any(|a| a.starts_with("--profile-drive-url=")));
                output
                    .send(Event::GoogleStatus(
                        prefs.google_lifecycle.revision,
                        connected,
                    ))
                    .await?;
            }
            Command::DisconnectGoogle(revision) => {
                let result = self
                    .disconnect_google(revision, &mut output)
                    .await
                    .map_err(|e| format!("{e:#}"));
                output
                    .send(Event::GoogleDisconnected(revision, result))
                    .await?;
            }
            Command::CleanupGoogle => {
                let _guard = self.google_connection_lock.write().await;
                let result = self.cleanup_google_locked().await;
                self.workspace(&mut output).await?;
                result?;
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
                            #[cfg(feature = "test-support")]
                            crate::test_support::image_delay().await;
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
            Command::Conversation(generation, anchor, focus, offset) => {
                let result = self
                    .store
                    .conversation_around(anchor.clone(), focus, offset)
                    .await
                    .map(Arc::new)
                    .map_err(|error| format!("{error:#}"));
                output
                    .send(Event::Conversation(generation, anchor, result))
                    .await?;
            }
            Command::IndexConversations => {
                let result = async {
                    while self.store.index_conversation_batch().await? {
                        tokio::task::yield_now().await;
                    }
                    Ok::<_, anyhow::Error>(())
                }
                .await
                .map_err(|error| format!("{error:#}"));
                output.send(Event::ConversationsIndexed(result)).await?;
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
                    let _guard = self.account_access(&account.id).await;
                    let secret = self.setup_password(&account, &password, &smtp_password, target).await?;
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
                let _lifecycle = self.connection_lifecycle_lock.lock().await;
                let _guard = self.account_access(&account.id).await;
                self.store.ensure_folder_idle(account.id.clone()).await?;
                self.store
                    .check_connection(crate::store::ConnectionRef {
                        kind: crate::store::ConnectionKind::Account,
                        id: account.id.clone(),
                    })
                    .await?;
                let saved_id = account.id.clone();
                // Resolve all required credentials before writing any of them.
                // Downloaded endpoint changes cannot reuse a saved old secret.
                let password = self
                    .setup_password(
                        &account,
                        &password,
                        &smtp_password,
                        ConnectionTarget::Incoming,
                    )
                    .await?;
                let separate =
                    if account.smtp_separate_password && account.smtp_auth != SmtpAuth::None {
                        Some(
                            self.setup_password(
                                &account,
                                &password,
                                &smtp_password,
                                ConnectionTarget::Smtp,
                            )
                            .await?,
                        )
                    } else {
                        None
                    };
                if let Some(smtp_password) = separate {
                    self.credentials
                        .write(&format!("{}:smtp", account.id), smtp_password)
                        .await?;
                }
                self.credentials
                    .write(&account.id, password)
                    .await
                    .context("Could not save the account credential")?;
                self.store.save_account(account).await?;
                self.workspace(&mut output).await?;
                output.send(Event::AccountSaved(saved_id)).await?;
                output
                    .send(Event::Notice(
                        "Account saved. Use Sync to receive your mail.".into(),
                    ))
                    .await?;
            }
            Command::SavePreferences(request, prefs) => {
                let event = match self.store.save_preferences(prefs).await {
                    Ok(snapshot) => {
                        self.mail_sync_settings
                            .set(snapshot.value.mail_check_seconds);
                        Event::PreferencesSaved(request, Arc::new(snapshot))
                    }
                    Err(error) => Event::PreferencesSaveFailed(request, error.to_string()),
                };
                output.send(event).await?;
            }
            Command::Sync => {
                if self.demo {
                    #[cfg(feature = "test-support")]
                    {
                        if std::env::args().any(|arg| arg == "--held-account-sync") {
                            return self.preview_held_account_sync(output).await;
                        }
                        let (round, arrival) = crate::test_support::sync_mail(&self.store).await?;
                        if let Some(arrival) = arrival {
                            output.send(Event::MailArrived(Arc::new(arrival))).await?;
                        }
                        output.send(Event::PreviewSync(round)).await?;
                    }
                    #[cfg(not(feature = "test-support"))]
                    tokio::time::sleep(Duration::from_millis(1500)).await;
                    output.send(Event::Changed).await?;
                    return Ok(());
                }
                let accounts = self.store.accounts_ready_to_sync().await?;
                let results: Vec<_> = futures::stream::iter(accounts)
                    .map(|a| {
                        let engine = self.clone();
                        let mut output = output.clone();
                        async move {
                            let name = a.name.clone();
                            let result = engine
                                .sync_account(a, output.clone())
                                .await
                                .with_context(|| format!("{name} sync failed"));
                            // Flush even a short account check's final cache
                            // changes without waiting for a slower account.
                            output.send(Event::Changed).await?;
                            result
                        }
                    })
                    .buffer_unordered(3)
                    .collect()
                    .await;
                let failures: Vec<_> = results
                    .into_iter()
                    .filter_map(|result| result.err().map(|error| format!("{error:#}")))
                    .collect();
                self.recover_completed_moves(output.clone()).await?;
                self.workspace(&mut output).await?;
                output.send(Event::Changed).await?;
                anyhow::ensure!(failures.is_empty(), "{}", failures.join("\n"));
            }
            Command::MoveRecoveries(request, after) => {
                let result = self
                    .store
                    .pending_mail_moves(None, after)
                    .await
                    .map(Arc::new)
                    .map_err(|error| format!("{error:#}"));
                output.send(Event::MoveRecoveries(request, result)).await?;
            }
            Command::RecoverMailMove(request, record, action, confirmed) => {
                let token = record.token.clone();
                let result = self
                    .recover_mail_move((*record).clone(), action, confirmed)
                    .await;
                let recovered = result.as_ref().ok().cloned();
                output
                    .send(Event::MailMoveRecovery(
                        request,
                        token,
                        result.map(Arc::new).map_err(|error| format!("{error:#}")),
                    ))
                    .await?;
                if let Some(record) = recovered {
                    output.send(Event::MoveRecovered(Arc::new(record))).await?;
                }
                self.workspace(&mut output).await?;
                output.send(Event::Changed).await?;
            }
            Command::Transfer(request, mail, destination, folder) => {
                let result = self
                    .transfer_message(&mail, destination, folder, output.clone(), None)
                    .await;
                let refresh = result.as_ref().ok().map(|(account, _)| account.clone());
                output
                    .send(Event::TransferFinished(
                        request,
                        mail,
                        result
                            .map(|(_, receipt)| Arc::new(receipt))
                            .map_err(|e| format!("{e:#}")),
                    ))
                    .await?;
                if !self.demo {
                    self.recover_completed_moves(output.clone()).await?;
                }
                if !self.demo
                    && let Some(account) = refresh
                    && let Err(error) = self.sync_account(account, output.clone()).await
                {
                    output.send(Event::Error(format!("The message was moved, but refreshing folders failed. Try Refresh. {error:#}"))).await?;
                }
            }
            Command::Move(request, mail, folder) => {
                let result = self
                    .change_folder(&mail, &folder, output.clone(), None)
                    .await;
                let refresh = result
                    .as_ref()
                    .ok()
                    .and_then(|(account, _)| account.clone());
                output
                    .send(Event::MoveFinished(
                        request,
                        mail,
                        folder,
                        result
                            .map(|(_, receipt)| Arc::new(receipt))
                            .map_err(|e| format!("{e:#}")),
                    ))
                    .await?;
                if !self.demo {
                    self.recover_completed_moves(output.clone()).await?;
                }
                if let Some(account) = refresh
                    && let Err(error) = self.sync_account(account, output.clone()).await
                {
                    output.send(Event::Error(format!("The message was moved, but refreshing folders failed. Try Refresh. {error:#}"))).await?;
                }
            }
            Command::UndoMove(request, original, receipt) => {
                let result = self
                    .undo_move(original.clone(), &receipt, output.clone(), None)
                    .await;
                let refresh = result
                    .as_ref()
                    .ok()
                    .and_then(|(account, _)| account.clone());
                output
                    .send(Event::UndoFinished(
                        request,
                        original,
                        result
                            .map(|(_, receipt)| Arc::new(receipt))
                            .map_err(|e| format!("{e:#}")),
                    ))
                    .await?;
                if !self.demo {
                    self.recover_completed_moves(output.clone()).await?;
                }
                if !self.demo
                    && let Some(account) = refresh
                    && let Err(error) = self.sync_account(account, output.clone()).await
                {
                    output.send(Event::Error(format!("The message was restored, but refreshing folders failed. Try Refresh. {error:#}"))).await?;
                }
            }
            Command::Flags(request, mail, changes) => {
                let result = self
                    .change_flags(&mail, changes)
                    .await
                    .map_err(|e| format!("{e:#}"));
                output
                    .send(Event::FlagsFinished(request, mail, result))
                    .await?;
            }
            Command::SaveDraft(draft) | Command::AutoSaveDraft(draft) => {
                let id = draft.id.clone();
                let revision = draft.revision;
                let result = async {
                    self.store.save_draft(draft).await?;
                    self.store.draft_state().await
                }
                .await
                .map(Arc::new)
                .map_err(|e| format!("Could not save the draft: {e:#}"));
                let saved = result.is_ok();
                output
                    .send(Event::DraftSaved(id.clone(), revision, result))
                    .await?;
                if saved && explicit_draft {
                    output.send(Event::Notice("Draft saved.".into())).await?;
                }
            }
            Command::DeleteDraft(id) => {
                let result = async {
                    #[cfg(feature = "test-support")]
                    if self.demo
                        && std::env::args().any(|arg| arg == "--discard-failure-once")
                        && !self.store.get::<bool>("preview_discard_failed").await?
                    {
                        self.store.put("preview_discard_failed", true).await?;
                        tokio::time::sleep(std::time::Duration::from_millis(1800)).await;
                        anyhow::bail!("Preview storage failure. Your draft is intact; try again.");
                    }
                    self.store.delete_draft(id.clone()).await
                }
                .await;
                let deleted = result.is_ok();
                output
                    .send(Event::DraftDeleted(
                        id,
                        result
                            .map(Arc::new)
                            .map_err(|e| format!("Could not discard the draft: {e:#}")),
                    ))
                    .await?;
                if deleted {
                    self.workspace(&mut output).await?;
                    output.send(Event::OutgoingChanged).await?;
                }
            }
            Command::AddDraftFiles(draft, paths) => {
                let id = draft.id.clone();
                let result = async {
                    #[cfg(feature = "test-support")]
                    if self.demo {
                        crate::test_support::attachment_delay(&self.store).await?;
                    }
                    self.store.add_draft_files(draft, paths).await
                }
                .await;
                output
                    .send(Event::DraftFiles(
                        id,
                        result.map(Arc::new).map_err(|e| format!("{e:#}")),
                    ))
                    .await?;
            }
            Command::Print(request, source, options) => {
                let result = async {
                    #[cfg(feature = "test-support")]
                    if self.demo {
                        crate::test_support::print_delay(&self.store).await?;
                    }
                    self.printing.prepare(&self.store, &source, options).await
                }
                .await;
                output
                    .send(Event::Print(request, result.map_err(|e| e.to_string())))
                    .await?;
            }
            Command::ForwardDraft(source, id) => {
                let result = async {
                    #[cfg(feature = "test-support")]
                    if self.demo {
                        crate::test_support::forward_delay(&self.store).await?;
                    }
                    self.store.forward_draft(source, id.clone()).await
                }
                .await;
                output
                    .send(Event::ForwardDraft(
                        id,
                        result
                            .map(Arc::new)
                            .map_err(|e| format!("Could not prepare the forward: {e:#}")),
                    ))
                    .await?;
            }
            Command::RemoveDraftFile(draft, file) => {
                let result = self.store.remove_draft_file(draft.clone(), file).await;
                output
                    .send(Event::DraftFiles(
                        draft,
                        result.map(Arc::new).map_err(|e| format!("{e:#}")),
                    ))
                    .await?;
            }
            Command::Send(draft) => self.send_draft(draft, &mut output).await?,
            Command::OutgoingPage(request, offset) => {
                let result = self
                    .store
                    .outgoing_page(offset)
                    .await
                    .map(Arc::new)
                    .map_err(|e| e.to_string());
                output.send(Event::OutgoingPage(request, result)).await?;
            }
            Command::ResolveOutgoing(attempt, action, confirmed) => {
                let result = self
                    .resolve_outgoing(attempt, action, confirmed, &mut output)
                    .await;
                self.workspace(&mut output).await?;
                output.send(Event::OutgoingChanged).await?;
                output.send(Event::Changed).await?;
                result?;
            }
            Command::RepairOutgoing => self.repair_outgoing(&mut output).await?,
            Command::GoogleLogin(prefs, retry) => {
                anyhow::ensure!(!self.demo, "Google sign-in is disabled in preview.");
                prefs.validate()?;
                // The UI starts OAuth only after the corresponding preferences save
                // is acknowledged. A delayed provider job must not overwrite settings.
                let _guard = self.google_connection_lock.write().await;
                let current: Preferences = self.store.get("preferences").await?;
                anyhow::ensure!(
                    prefs.google_lifecycle.revision == current.google_lifecycle.revision
                        && prefs.google_client_id == current.google_client_id
                        && prefs.google_client_secret == current.google_client_secret
                        && prefs.google_services == current.google_services,
                    "Google changed before sign-in started. Choose Connect Google again."
                );
                self.cleanup_google_locked().await?;
                let grant = self.google.login_with_retry(&prefs, retry).await?;
                let (grant, identity, sources) = self.google.prepare_grant(&prefs, grant).await?;
                let _lifecycle = self.connection_lifecycle_lock.lock().await;
                let saved = self
                    .store
                    .activate_google(prefs, grant, identity, sources)
                    .await?;
                // Activation is committed even if redundant old-secret cleanup fails.
                // The keychain vault is bounded and every lookup follows the DB ID.
                let cleanup_ok = self.google.finish_activation(&saved.value).await.is_ok();
                self.workspace(&mut output).await?;
                output
                    .send(Event::GoogleStatus(
                        saved.value.google_lifecycle.revision,
                        true,
                    ))
                    .await?;
                output
                    .send(if cleanup_ok { Event::Notice("Google connected. Granted permissions are shown in Preferences.".into()) }
                    else { Event::Error("Google connected, but its previous saved grant could not be removed. Unlock the keychain and reconnect to retry cleanup.".into()) })
                    .await?;
            }
            Command::DiscoverCalendars(request, url, username, password) => {
                let result = self
                    .discover_calendars(url, username, password)
                    .await
                    .map_err(|e| format!("{e:#}"));
                output
                    .send(Event::CalendarsDiscovered(request, result))
                    .await?;
            }
            Command::ConnectCalendars(request, revision, sources, password) => {
                let result = self
                    .connect_calendars(sources, password, revision)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let saved = result.is_ok();
                output
                    .send(Event::CalendarsConnected(request, result))
                    .await?;
                if saved {
                    self.workspace(&mut output).await?;
                }
            }
            Command::RemovalPreview(request, target) => {
                let result = self
                    .store
                    .removal_preview(target)
                    .await
                    .map_err(|e| format!("{e:#}"));
                output.send(Event::RemovalPreview(request, result)).await?;
            }
            Command::RemoveConnection(request, preview, cancel) => {
                let result = self
                    .remove_connection(preview, cancel)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let removed = result.is_ok();
                output
                    .send(Event::ConnectionRemoved(request, result))
                    .await?;
                if removed {
                    self.workspace(&mut output).await?;
                    output.send(Event::Changed).await?;
                    self.send_calendar(&mut output).await?;
                }
            }
            Command::CleanupCredentials => {
                let failed = self.cleanup_credentials().await?;
                if failed > 0 {
                    output.send(Event::Error("A removed connection still has saved credentials. Unlock your credential store and choose Retry credential cleanup in Preferences.".into())).await?;
                }
                self.workspace(&mut output).await?;
            }
            Command::RestoreGoogleCalendars => {
                let count = self.restore_google_calendars().await?;
                self.workspace(&mut output).await?;
                output
                    .send(Event::Notice(format!(
                        "{count} Google calendars restored. Use Sync calendar to load their events."
                    )))
                    .await?;
            }
            Command::SyncCalendar => {
                let _google = self.google_connection_lock.read().await;
                if !self.demo {
                    let sources: Vec<CalendarSource> = self.store.get("calendars").await?;
                    let prefs: Preferences = self.store.get("preferences").await?;
                    if !prefs.google_lifecycle.disconnected
                        && prefs.google_grant.access.calendar_allowed()
                        && sources.iter().any(|s| s.kind == CalendarKind::Google)
                    {
                        match self.google.calendars(&prefs).await {
                            Ok(updated) => {
                                self.store.refresh_google_sources(updated).await?;
                                self.workspace(&mut output).await?;
                            }
                            Err(error) => {
                                output
                                    .send(Event::Error(format!(
                                        "Could not refresh Google calendar access: {error:#}"
                                    )))
                                    .await?;
                            }
                        }
                    }
                    let sources: Vec<CalendarSource> = self.store.get("calendars").await?;
                    let archived: HashSet<String> = self.store.get("google_archived").await?;
                    let now = chrono::Utc::now();
                    for source in sources {
                        if archived.contains(&source.id)
                            || (source.kind == CalendarKind::Google
                                && (prefs.google_lifecycle.disconnected
                                    || !prefs.google_grant.access.calendar_allowed()))
                        {
                            continue;
                        }
                        let _guard = self.calendar_access(&source.id).await;
                        let Some(source) = self
                            .store
                            .get::<Vec<CalendarSource>>("calendars")
                            .await?
                            .into_iter()
                            .find(|s| s.id == source.id)
                        else {
                            continue;
                        };
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
                self.send_calendar(&mut output).await?;
            }
            Command::SaveEvent(event) | Command::DeleteEvent(event) => {
                let _google = self.google_connection_lock.read().await;
                anyhow::ensure!(
                    deleting_event || event.end > event.start,
                    "The event must end after it starts."
                );
                let _guard = self.calendar_access(&event.source_id).await;
                let source = self
                    .store
                    .get::<Vec<CalendarSource>>("calendars")
                    .await?
                    .into_iter()
                    .find(|s| s.id == event.source_id)
                    .context("Choose a connected calendar")?;
                providers::calendar::ensure_event_access(&source, &event, deleting_event)?;
                let saved = if self.demo {
                    event.clone()
                } else {
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
            Command::ProbeSftp(request, settings) => {
                let result = if self.demo {
                    #[cfg(feature = "test-support")]
                    {
                        crate::test_support::sftp_fingerprint(&settings.host)
                    }
                    #[cfg(not(feature = "test-support"))]
                    {
                        Err(anyhow::anyhow!(
                            "SFTP fingerprint probes are disabled in preview."
                        ))
                    }
                } else {
                    backup::sftp::probe_fingerprint(&settings).await
                };
                output
                    .send(Event::SftpFingerprint(
                        request,
                        settings,
                        result.map_err(|error| format!("{error:#}")),
                    ))
                    .await?;
            }
            Command::ConnectFtp(request, target, supplied) => {
                let result = self.connect_ftp(&target, supplied).await;
                output
                    .send(Event::FtpConnection(
                        request,
                        target,
                        result.map_err(|error| format!("{error:#}")),
                    ))
                    .await?;
            }
            Command::ConnectSftp(request, target, supplied) => {
                let result = self.connect_sftp(&target, supplied).await;
                output
                    .send(Event::SftpConnection(
                        request,
                        target,
                        result.map_err(|error| format!("{error:#}")),
                    ))
                    .await?;
            }
            Command::ConnectS3(request, target, supplied) => {
                let result = self
                    .connect_s3(&target, supplied, backup::s3::S3Backup::from_secret)
                    .await;
                output
                    .send(Event::S3Connection(
                        request,
                        target,
                        result.map_err(|error| format!("{error:#}")),
                    ))
                    .await?;
            }
            Command::Backup(target, passphrase) => {
                self.run_backup(target, Some(passphrase), &mut output)
                    .await?;
            }
            Command::BackupIncluded(request, id, target) => {
                self.backup_included(request, id, target, &mut output)
                    .await?;
            }
            Command::AutomaticBackup(target) => {
                self.run_backup(target, None, &mut output).await?;
            }
            Command::RetryBackupHistory(id, target, secret) => {
                let history = self.store.backup_history(target.clone()).await?;
                anyhow::ensure!(
                    history
                        .first()
                        .is_some_and(|entry| entry.id == id && entry.outcome.attention()),
                    "Backup activity changed. Refresh it before retrying."
                );
                if let Some(copy) = history.first().and_then(|entry| entry.copy.as_ref()) {
                    let pending = self.backup_journal().await?.pending(&target).await?;
                    anyhow::ensure!(
                        pending
                            .as_ref()
                            .is_some_and(|entry| &entry.upload.id == copy),
                        "This activity has no matching pending upload on this device. Refresh copies to check it, or choose Back up now to create a new copy."
                    );
                }
                self.run_backup(target, Some(secret), &mut output).await?;
            }
            Command::BackupHistory(request, target) => {
                let result = self.store.backup_history(target.clone()).await;
                output
                    .send(Event::BackupHistory(
                        request,
                        target,
                        result.map(Arc::new).map_err(|error| format!("{error:#}")),
                    ))
                    .await?;
            }
            Command::ListBackups(request, target) => {
                let result: anyhow::Result<Vec<BackupCopy>> = async {
                    self.allow_backup()?;
                    let _guard = self.backup_connection_guard(&target).await;
                    let prefs = self.store.get("preferences").await?;
                    Self::check_backup_target(&target, &prefs)?;
                    let prefs = backup::config::resolve(&prefs, &target)?;
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
                self.run_restore(target, id, passphrase, &mut output)
                    .await?;
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

    pub(super) fn engine() -> Engine {
        Engine {
            profiles: None,
            credentials: Default::default(),
            store: Store::memory().unwrap(),
            google: Default::default(),
            demo: true,
            account_work: Default::default(),
            calendar_work: Default::default(),
            calendar_setup_lock: Default::default(),
            connection_lifecycle_lock: Default::default(),
            secret_remover: Arc::new(removals::OsSecretRemover::default()),
            outbound: Arc::new(providers::outgoing::Servers::default()),
            google_connection_lock: Default::default(),
            passphrases: Arc::new(backup::OsPassphraseStore::default()),
            restore_credentials: Arc::new(backup::restore::OsCredentialRestorer::default()),
            backup_uploads: Default::default(),
            mail_sync_settings: Default::default(),
            provider_slots: Default::default(),
            printing: Default::default(),
            bulk_control: Default::default(),
        }
    }
    pub(super) fn event(source: &str) -> CalendarEvent {
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
        for id in ["home", "work"] {
            engine
                .store
                .save_source(CalendarSource {
                    id: id.into(),
                    name: id.into(),
                    kind: CalendarKind::CalDav,
                    url: "https://calendar.example.test/".into(),
                    username: "test".into(),
                    access: Default::default(),
                })
                .await
                .unwrap();
        }
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
