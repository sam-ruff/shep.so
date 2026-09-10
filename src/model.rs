use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const PAGE_SIZE: usize = 50;
pub const CHANNEL_CAPACITY: usize = 32;
pub use shep_mail_core::model::*;
pub type MailDetail = shep_mail_core::model::MailDetail<crate::email_content::HtmlBody>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MailSort {
    #[default]
    Newest,
    Oldest,
    Sender,
    Subject,
    Relevance,
}
impl MailSort {
    pub const BROWSE: [Self; 4] = [Self::Newest, Self::Oldest, Self::Sender, Self::Subject];
    pub const SEARCH: [Self; 5] = [
        Self::Relevance,
        Self::Newest,
        Self::Oldest,
        Self::Sender,
        Self::Subject,
    ];
}
impl fmt::Display for MailSort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Newest => "Newest first",
            Self::Oldest => "Oldest first",
            Self::Sender => "Sender A–Z",
            Self::Subject => "Subject A–Z",
            Self::Relevance => "Best match",
        })
    }
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MailFilter {
    #[default]
    All,
    Unread,
    Read,
    Flagged,
    Attachments,
}
impl MailFilter {
    pub const ALL: [Self; 5] = [
        Self::All,
        Self::Unread,
        Self::Read,
        Self::Flagged,
        Self::Attachments,
    ];
}
impl fmt::Display for MailFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::All => "All mail",
            Self::Unread => "Unread",
            Self::Read => "Read",
            Self::Flagged => "Flagged",
            Self::Attachments => "Attachments",
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailQuery {
    /// UI-only folder projection for cached reads, never provider identities or
    /// a captured selection scope. Missing on older serialized queries.
    #[serde(default)]
    pub project_moves: Vec<MailMoveProjection>,
    /// Small pending-action identities observed in the same snapshot as counts.
    /// This does not alter the folder/search result scope.
    pub observe: Vec<String>,
    pub observe_bulk: Vec<String>,
    pub folders: Option<Vec<FolderSelection>>,
    /// Concrete folders temporarily excluded by a reviewed pending deletion.
    #[serde(default)]
    pub exclude_folders: Vec<FolderSelection>,
    pub sent_only: bool,
    pub account: Option<String>,
    pub folder: String,
    pub search: String,
    /// Interactive search spans the folders of the selected accounts. Keep the
    /// browsing scope so clearing the text returns to the previous folder.
    #[serde(default)]
    pub search_all_folders: bool,
    pub unread_only: bool,
    pub read_only: bool,
    pub attachments_only: bool,
    pub sort: MailSort,
    pub starred_only: bool,
    pub offset: usize,
}

impl MailQuery {
    pub fn searches_all_folders(&self) -> bool {
        self.search_all_folders && !self.search.trim().is_empty()
    }
    /// Shared by SQLite results, frozen selections and optimistic UI membership.
    /// Explicit folder groups keep their account set, including an empty set.
    pub fn search_scope(&self) -> std::borrow::Cow<'_, Self> {
        if !self.searches_all_folders() {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut scope = self.clone();
        scope.search_all_folders = false;
        scope.folder.clear();
        scope.sent_only = false;
        if let Some(folders) = &self.folders {
            scope.account = None;
            if folders.iter().any(|f| f.account.is_none()) {
                scope.folders = None;
            } else {
                let mut folders: Vec<_> = folders
                    .iter()
                    .map(|f| FolderSelection {
                        account: f.account.clone(),
                        folder: String::new(),
                        sent_only: false,
                    })
                    .collect();
                folders.sort_by(|a, b| a.account.cmp(&b.account));
                folders.dedup();
                scope.folders = Some(folders);
            }
        }
        std::borrow::Cow::Owned(scope)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMoveProjection {
    pub id: String,
    pub source_account: String,
    pub source_folder: String,
    pub account: String,
    pub folder: String,
    pub unread: bool,
    pub starred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderSelection {
    pub account: Option<String>,
    pub folder: String,
    pub sent_only: bool,
}

#[derive(Debug, Clone, Default)]
pub struct MailPage {
    pub move_pending_total: usize,
    pub relocated: std::collections::HashMap<String, Mail>,
    pub move_recovery: std::collections::HashMap<String, crate::mail_actions::journal::MoveRecord>,
    pub move_placeholders: std::collections::HashSet<String>,
    pub bulk_observed: std::collections::HashMap<String, bool>,
    pub bulk_placeholders: std::collections::HashSet<String>,
    pub bulk_revision: u64,
    pub bulk_pending: std::collections::HashSet<String>,
    pub rows: Vec<Mail>,
    pub total: usize,
    pub unread: usize,
    /// One reviewed deletion scope; ordinary pages carry no folder summary.
    pub folder_count: Option<(usize, usize)>,
    pub inbox_unread: std::collections::BTreeMap<String, usize>,
    pub observed: std::collections::HashMap<String, Option<MailMembership>>,
}

impl MailPage {
    /// Durable recovery IDs address protected cached MIME and can be read cold.
    pub fn is_transient_placeholder(&self, id: &str) -> bool {
        self.is_placeholder(id) && !self.move_recovery.contains_key(id)
    }
    pub fn is_placeholder(&self, id: &str) -> bool {
        self.bulk_placeholders.contains(id) || self.move_placeholders.contains(id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailMembership {
    pub account: String,
    pub folder: String,
    pub unread: bool,
}
impl From<&Mail> for MailMembership {
    fn from(mail: &Mail) -> Self {
        Self {
            account: mail.account_id.clone(),
            folder: mail.folder.clone(),
            unread: mail.unread,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalendarKind {
    Google,
    CalDav,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarSource {
    pub id: String,
    pub name: String,
    pub kind: CalendarKind,
    pub url: String,
    pub username: String,
    #[serde(default)]
    pub access: CalendarAccess,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarAccess {
    pub create: bool,
    pub update: bool,
    pub delete: bool,
}
impl CalendarAccess {
    pub const READ_ONLY: Self = Self {
        create: false,
        update: false,
        delete: false,
    };
    pub fn read_only(self) -> bool {
        !self.create && !self.update && !self.delete
    }
}
impl Default for CalendarAccess {
    fn default() -> Self {
        Self {
            create: true,
            update: true,
            delete: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub id: String,
    pub source_id: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub location: String,
    pub description: String,
    pub all_day: bool,
    pub etag: Option<String>,
    pub remote_url: Option<String>,
}

impl CalendarEvent {
    /// Remote identifiers are unique within a calendar, not across calendars.
    pub fn key(&self) -> String {
        Self::scoped_key(&self.source_id, &self.id)
    }

    pub fn scoped_key(source: &str, id: &str) -> String {
        format!("{}:{source}{id}", source.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BackupDestination {
    #[default]
    Local,
    GoogleDrive,
    S3,
    Sftp,
    Ftp,
}
impl fmt::Display for BackupDestination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Local => "Local folder",
            Self::GoogleDrive => "Google Drive",
            Self::S3 => "S3-compatible storage",
            Self::Sftp => "SFTP",
            Self::Ftp => "FTP / FTPS",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub appearance: Appearance,
    pub palettes: crate::appearance::Palettes,
    pub reader_split: f32,
    pub sidebar_width: Option<f32>,
    pub window_size: Option<WindowSize>,
    pub mail_sort: MailSort,
    pub unified_inbox: bool,
    pub collapsed_accounts: Vec<String>,
    pub expanded_folders: std::collections::HashMap<String, std::collections::HashSet<String>>,
    pub collapsed_drafts: bool,
    pub cross_account_moves: bool,
    pub reader_font_size: u16,
    pub interface_scale: u16,
    pub tooltips: bool,
    pub shortcut_tooltips: bool,
    pub unread_badge: bool,
    pub close_to_tray: bool,
    pub notifications: crate::notifications::Settings,
    pub image_policy: ImagePolicy,
    pub reply_display: ReplyDisplay,
    pub group_conversations: bool,
    pub contacts: Vec<String>,
    pub image_senders: Vec<String>,
    pub image_domains: Vec<String>,
    pub image_messages: Vec<String>,
    pub backup_destinations: Vec<crate::backup::config::Destination>,
    pub backup_selected: Option<String>,
    pub backup_destination: BackupDestination,
    pub backup_s3: crate::backup::s3::Settings,
    pub backup_sftp: crate::backup::sftp::Settings,
    pub backup_ftp: crate::backup::ftp::Settings,
    pub backup_folder: String,
    pub backup_format: crate::backup::format::Options,
    pub backup_copies: usize,
    pub backup_hours: u64,
    pub backup_accounts: bool,
    pub auto_backup: bool,
    pub last_backup: Option<i64>,
    pub backup_ready: bool,
    pub google_connection_id: String,
    pub google_grant: GoogleGrant,
    pub google_lifecycle: GoogleLifecycle,
    pub sync_minutes: u64,
    pub mail_check_seconds: u64,
    pub google_client_id: String,
    pub google_client_secret: String,
    /// Desired permissions for the next sign-in, distinct from the active grant.
    pub google_services: Option<GoogleServices>,
    pub shortcuts: crate::shortcuts::Keymap,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            appearance: Appearance::System,
            palettes: Default::default(),
            reader_split: 0.315,
            sidebar_width: None,
            window_size: None,
            mail_sort: MailSort::Newest,
            unified_inbox: true,
            collapsed_accounts: Vec::new(),
            expanded_folders: Default::default(),
            collapsed_drafts: false,
            cross_account_moves: false,
            reader_font_size: 14,
            interface_scale: 100,
            tooltips: true,
            shortcut_tooltips: true,
            unread_badge: true,
            close_to_tray: false,
            notifications: Default::default(),
            image_policy: ImagePolicy::BlockAll,
            reply_display: ReplyDisplay::Collapsed,
            group_conversations: true,
            contacts: Vec::new(),
            image_senders: Vec::new(),
            image_domains: Vec::new(),
            image_messages: Vec::new(),
            backup_destinations: Vec::new(),
            backup_selected: None,
            backup_destination: BackupDestination::Local,
            backup_s3: Default::default(),
            backup_sftp: Default::default(),
            backup_ftp: Default::default(),
            backup_folder: String::new(),
            backup_format: Default::default(),
            backup_copies: 7,
            backup_hours: 24,
            backup_accounts: false,
            auto_backup: false,
            last_backup: None,
            backup_ready: false,
            google_connection_id: String::new(),
            google_grant: Default::default(),
            google_lifecycle: Default::default(),
            sync_minutes: 5,
            mail_check_seconds: 15,
            google_client_id: std::env::var("SHEP_GOOGLE_CLIENT_ID").unwrap_or_default(),
            google_client_secret: std::env::var("SHEP_GOOGLE_CLIENT_SECRET").unwrap_or_default(),
            google_services: None,
            shortcuts: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleLifecycle {
    pub revision: u64,
    pub disconnected: bool,
    pub cleanup_pending: bool,
}

/// Non-secret pointer to the credential selected by the connection transaction.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleGrant {
    pub id: String,
    pub client_id: String,
    pub access: GoogleAccess,
}

impl Preferences {
    pub fn requested_google_services(&self) -> GoogleServices {
        self.google_services.unwrap_or_else(|| {
            let access = self.google_grant.access;
            GoogleServices {
                drive: access.known && access.drive,
                calendar: if access.known && access.calendar_write {
                    GoogleCalendarRequest::ReadWrite
                } else if access.known && access.calendar_read {
                    GoogleCalendarRequest::ReadOnly
                } else {
                    GoogleCalendarRequest::Off
                },
            }
        })
    }
    pub fn active_google_client(&self) -> &str {
        if self.google_grant.client_id.is_empty() {
            &self.google_client_id
        } else {
            &self.google_grant.client_id
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleServices {
    pub drive: bool,
    pub calendar: GoogleCalendarRequest,
}
impl GoogleServices {
    pub fn any(self) -> bool {
        self.drive || self.calendar != GoogleCalendarRequest::Off
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoogleCalendarRequest {
    #[default]
    Off,
    ReadOnly,
    ReadWrite,
}
impl fmt::Display for GoogleCalendarRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Off => "No Calendar access",
            Self::ReadOnly => "Read calendars",
            Self::ReadWrite => "Read and edit calendars",
        })
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GoogleAccess {
    pub known: bool,
    pub drive: bool,
    pub calendar_read: bool,
    pub calendar_write: bool,
}
impl GoogleAccess {
    pub fn drive_allowed(self) -> bool {
        !self.known || self.drive
    }
    pub fn calendar_allowed(self) -> bool {
        !self.known || self.calendar_read
    }
    pub fn calendar_write_allowed(self) -> bool {
        !self.known || self.calendar_write
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Appearance {
    Light,
    Dark,
    #[default]
    System,
}
impl fmt::Display for Appearance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::System => "System",
        })
    }
}
impl Preferences {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.sidebar_width
                .is_none_or(|w| w.is_finite() && (160.0..=480.0).contains(&w)),
            "Sidebar width must be between 160 and 480."
        );
        anyhow::ensure!(
            self.window_size.is_none_or(|s| s.width.is_finite()
                && s.height.is_finite()
                && s.width > 0.
                && s.height > 0.),
            "Window dimensions must be positive and finite."
        );
        anyhow::ensure!(
            self.reader_split.is_finite() && (0.2..=0.7).contains(&self.reader_split),
            "Pane split must be between 20% and 70%."
        );
        anyhow::ensure!(
            (1..=100).contains(&self.backup_copies),
            "Keep between 1 and 100 backup copies."
        );
        anyhow::ensure!(
            (1..=8760).contains(&self.backup_hours),
            "Backup interval must be 1–8760 hours."
        );
        anyhow::ensure!(
            (1..=60).contains(&self.sync_minutes),
            "Calendar sync interval must be 1–60 minutes."
        );
        anyhow::ensure!(
            (5..=3600).contains(&self.mail_check_seconds),
            "Mail check interval must be 5–3600 seconds."
        );
        anyhow::ensure!(
            (11..=26).contains(&self.reader_font_size),
            "Message text size must be 11–26."
        );
        anyhow::ensure!(
            (80..=140).contains(&self.interface_scale),
            "Interface size must be 80–140%."
        );
        crate::backup::config::validate(self)?;
        self.shortcuts.validate()?;
        Ok(())
    }
}
