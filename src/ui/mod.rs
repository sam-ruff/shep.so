mod account;
mod backups;
mod components;
mod composing;
mod preference_sync;
mod reading;
#[cfg(test)]
mod reading_tests;
mod sidebar;
mod views;

use crate::{
    backup::{BackupCopy, BackupTarget},
    engine::{self, Command, Event},
    model::*,
    shortcuts::Action,
    store::{PreferenceSnapshot, Workspace},
};
use chrono::{Datelike, NaiveDate, TimeZone};
use components::*;
use iced::{
    Element, Size, Subscription, Task, Theme, event,
    keyboard::{self, Key},
    widget::{self, text_editor},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::Arc,
    time::Instant,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Mail,
    Calendar,
    Preferences,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Accounts,
    Calendars,
    Backups,
    Shortcuts,
    Privacy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Account,
    Calendar,
    Move,
    Compose,
    Event,
    Export,
    Restore,
    Sender,
}

#[derive(Debug, Clone, Copy)]
enum MailPane {
    Inbox,
    Reader,
}

#[derive(Debug, Clone)]
pub enum Message {
    Backend(Event),
    Tick,
    Noop,
    Tab(Tab),
    SettingsTab(SettingsTab),
    Open(Dialog),
    Close,
    Query(String),
    SearchReady(u64),
    Folder(String),
    Account(Option<String>),
    Filter(MailFilter),
    Sort(MailSort),
    Starred,
    Select(String),
    Hover(String),
    NextPage(bool),
    PreviousMessage(bool),
    Draft(String),
    Sync,
    SyncCalendar,
    Move(String),
    Focus(&'static str, u8),
    FocusChecked(&'static str, bool),
    ToggleStar,
    ToggleRead,
    Reply,
    ReplyAll,
    ChooseAttachments,
    ChosenAttachments(Draft, Vec<std::path::PathBuf>),
    RemoveDraftAttachment(String),
    ShowRecipients,
    SaveDraft,
    Send,
    Field(&'static str, String),
    Protocol(Protocol),
    SaveAccount,
    FastmailPreset,
    TestConnection(ConnectionTarget),
    ConnectCalendarFromEvent,
    EditAccount(String),
    SaveCalendar,
    SavePreferences,
    Appearance(Appearance),
    BackupDestination(BackupDestination),
    BackupAccounts(bool),
    AutoBackup(bool),
    GoogleLogin,
    Backup,
    ListBackups,
    Restore(String),
    ConfirmRestore,
    Key(Key, keyboard::Modifiers, bool),
    Remap(Action),
    ResetShortcuts,
    Editor(text_editor::Action),
    Month(i32),
    Day(NaiveDate),
    NewEventOnDay(NaiveDate),
    Today,
    EditEvent(String),
    SaveEvent,
    DeleteEvent,
    SaveExport,
    ExportAttachment(usize),
    SystemTheme(iced::theme::Mode),
    Resize(Size),
    Dismiss,
    BrowseBackup,
    BrowseExport,
    ChosenPath(&'static str, Option<String>),
    PaneResize(widget::pane_grid::ResizeEvent),
    SaveLayout(u64),
    InboxScroll(f32),
    WindowClose(iced::window::Id),
    SidebarAction(usize),
    ToggleInboxExpanded,
    AccountFolder(String, String),
    AccountFolderUnified,
    PrefUnified(bool),
    PrefCrossAccount(bool),
    PrefReaderSize(u16),
    PrefScale(u16),
    ApplyPaneResize,
    FlagRow(String),
    OpenMessage(String),
    ClosePreview,
    CopyAddress(String),
    ToggleReply(usize),
    PrefReplies(ReplyDisplay),
    PrefImages(ImagePolicy),
    AllowImages(u8),
    ClearImageTrust,
}

pub struct App {
    panes: widget::pane_grid::State<MailPane>,
    reader_split: widget::pane_grid::Split,
    layout_generation: u64,
    inbox_scroll: f32,
    last_click: Option<(String, Instant)>,
    pending_focus: Option<&'static str>,
    focused_input: Option<&'static str>,
    #[cfg(feature = "test-support")]
    test_keys: VecDeque<String>,
    list_revision: u64,
    last_list_query: MailQuery,
    draft_dirty: Option<Instant>,
    composer: composing::Composer,
    inbox_expanded: bool,
    sidebar_focus: bool,
    sidebar_index: usize,
    pending_resize: Option<widget::pane_grid::ResizeEvent>,
    full_reader: bool,
    expanded_replies: HashSet<usize>,
    remote_handles: VecDeque<(String, widget::image::Handle)>,
    requested_images: HashSet<String>,
    image_errors: HashMap<String, String>,
    demo: bool,
    tx: Option<engine::CommandSender>,
    workspace: Arc<Workspace>,
    preferences: Preferences,
    preference_sync: preference_sync::PreferenceSync,
    pending_google_login: Option<(u64, Preferences)>,
    pending_backup: Option<backups::PendingBackup>,
    tab: Tab,
    settings_tab: SettingsTab,
    dialog: Option<Dialog>,
    fields: HashMap<&'static str, String>,
    protocol: Protocol,
    query: MailQuery,
    generation: u64,
    page: Arc<MailPage>,
    selected: Option<String>,
    detail: Option<Arc<MailDetail>>,
    detail_cache: VecDeque<Arc<MailDetail>>,
    detail_revision: u64,
    prefetch_page: Option<(MailQuery, Arc<MailPage>)>,
    prefetch_query: Option<MailQuery>,
    pending_details: HashSet<String>,
    events: Arc<Vec<CalendarEvent>>,
    month: NaiveDate,
    day: NaiveDate,
    editing_event: Option<CalendarEvent>,
    editor: text_editor::Content,
    draft_id: String,
    remapping: Option<Action>,
    busy: HashSet<String>,
    notice: Option<(String, bool, Instant)>,
    google_connected: bool,
    system_dark: bool,
    size: Size,
    light_logo: widget::image::Handle,
    dark_logo: widget::image::Handle,
    backups: Arc<Vec<BackupCopy>>,
    backups_target: Option<BackupTarget>,
    backups_generation: u64,
    restore_id: String,
    restore_target: Option<BackupTarget>,
    export_index: Option<usize>,
    started: Instant,
    update_samples: VecDeque<f64>,
    test_state: Option<std::path::PathBuf>,
    test_revision: u64,
}

pub fn run() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Shep — Mail & Calendar")
        .theme(App::theme)
        .scale_factor(|app: &App| app.preferences.interface_scale as f32 / 100.)
        .subscription(App::subscription)
        .window(iced::window::Settings {
            size: Size::new(1440., 920.),
            exit_on_close_request: false,
            min_size: Some(Size::new(900., 640.)),
            #[cfg(target_os = "linux")]
            platform_specific: iced::window::settings::PlatformSpecific {
                application_id: "so.shep.Shep".into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .default_font(iced::Font::with_name("Noto Sans"))
        .font(include_bytes!("../../assets/NotoSans-Regular.ttf").as_slice())
        .font(include_bytes!("../../assets/NotoSans-SemiBold.ttf").as_slice())
        .run()
}
impl App {
    fn new() -> (Self, Task<Message>) {
        let args: Vec<_> = std::env::args().collect();
        let demo = cfg!(feature = "test-support") && args.iter().any(|a| a == "--demo");
        let test_state = if demo {
            args.windows(2)
                .find(|a| a[0] == "--test-state")
                .map(|a| a[1].clone().into())
        } else {
            None
        };
        let today = chrono::Local::now().date_naive();
        let (mut panes, inbox) = widget::pane_grid::State::new(MailPane::Inbox);
        let (_, reader_split) = panes
            .split(widget::pane_grid::Axis::Vertical, inbox, MailPane::Reader)
            .expect("initial pane exists");
        panes.resize(reader_split, 0.315);
        (
            Self {
                panes,
                reader_split,
                layout_generation: 0,
                inbox_scroll: 0.,
                last_click: None,
                pending_focus: None,
                focused_input: None,
                #[cfg(feature = "test-support")]
                test_keys: VecDeque::new(),
                list_revision: 0,
                last_list_query: MailQuery::default(),
                draft_dirty: None,
                composer: Default::default(),
                inbox_expanded: false,
                sidebar_focus: false,
                sidebar_index: 0,
                pending_resize: None,
                full_reader: false,
                expanded_replies: HashSet::new(),
                remote_handles: VecDeque::new(),
                requested_images: HashSet::new(),
                image_errors: HashMap::new(),
                demo,
                tx: None,
                workspace: Arc::new(Workspace::default()),
                preferences: Preferences::default(),
                preference_sync: Default::default(),
                pending_google_login: None,
                pending_backup: None,
                tab: Tab::Mail,
                settings_tab: SettingsTab::General,
                dialog: None,
                fields: HashMap::new(),
                protocol: Protocol::Imap,
                query: MailQuery {
                    folder: "INBOX".into(),
                    ..Default::default()
                },
                generation: 0,
                page: Arc::new(MailPage::default()),
                selected: None,
                detail: None,
                detail_cache: VecDeque::new(),
                detail_revision: 0,
                prefetch_page: None,
                prefetch_query: None,
                pending_details: HashSet::new(),
                events: Arc::new(Vec::new()),
                month: today.with_day(1).unwrap(),
                day: today,
                editing_event: None,
                editor: text_editor::Content::new(),
                draft_id: String::new(),
                remapping: None,
                busy: HashSet::new(),
                notice: None,
                google_connected: false,
                system_dark: false,
                size: Size::new(1440., 920.),
                light_logo: widget::image::Handle::from_bytes(
                    include_bytes!("../../assets/logo-light.webp").as_slice(),
                ),
                dark_logo: widget::image::Handle::from_bytes(
                    include_bytes!("../../assets/logo-dark.webp").as_slice(),
                ),
                backups: Arc::new(Vec::new()),
                backups_target: None,
                backups_generation: 0,
                restore_id: String::new(),
                restore_target: None,
                export_index: None,
                started: Instant::now(),
                update_samples: VecDeque::new(),
                test_state,
                test_revision: 0,
            },
            iced::system::theme().map(Message::SystemTheme),
        )
    }
    fn theme(&self) -> Theme {
        static THEMES: std::sync::OnceLock<[Theme; 2]> = std::sync::OnceLock::new();
        let themes = THEMES.get_or_init(|| {
            let make = |dark| {
                Theme::custom(
                    if dark { "Shep Dark" } else { "Shep Light" },
                    iced::theme::Palette {
                        background: if dark { hex(0x141416) } else { hex(0xf7f7f9) },
                        text: if dark { hex(0xf4f4f5) } else { hex(0x292830) },
                        primary: if dark { hex(0xb5a0ff) } else { hex(0x7356bd) },
                        success: hex(0x398366),
                        warning: hex(0xc7954a),
                        danger: hex(0xbf5757),
                    },
                )
            };
            [make(false), make(true)]
        });
        themes[usize::from(self.dark())].clone()
    }
    fn dark(&self) -> bool {
        match self.preferences.appearance {
            Appearance::Light => false,
            Appearance::Dark => true,
            Appearance::System => self.system_dark,
        }
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            Subscription::run_with(self.demo, engine::subscription).map(Message::Backend),
            iced::time::every(std::time::Duration::from_secs(1)).map(|_| Message::Tick),
            iced::system::theme_changes().map(Message::SystemTheme),
            iced::event::listen_with(|e, status, id| match e {
                iced::Event::Window(iced::window::Event::CloseRequested) => {
                    Some(Message::WindowClose(id))
                }
                iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => Some(
                    Message::Key(key, modifiers, status == event::Status::Captured),
                ),
                iced::Event::Window(iced::window::Event::Resized(size)) => {
                    Some(Message::Resize(size))
                }
                _ => None,
            }),
        ])
    }
    fn save_preferences(&mut self) {
        let request = self.preference_sync.changed();
        self.send(Command::SavePreferences(request, self.preferences.clone()));
    }

    fn update_saved_preferences(&mut self) {
        let workspace = Arc::make_mut(&mut self.workspace);
        workspace.preferences = self.preference_sync.saved.value.clone();
        workspace.preferences_revision = self.preference_sync.saved.revision;
    }

    fn send(&mut self, command: Command) {
        self.try_command(command);
    }
    fn try_command(&mut self, command: Command) -> bool {
        if let Some(tx) = &self.tx {
            if let Err(error) = tx.try_send(command) {
                match (*error).into_inner() {
                    Command::Detail {
                        id, prefetch: true, ..
                    } => {
                        self.pending_details.remove(&id);
                    }
                    Command::Query(_, _, true) => self.prefetch_query = None,
                    _ => self.notice(
                        "The work queue is full. Please try that action again.",
                        true,
                    ),
                }
                return false;
            }
        } else {
            self.notice("Opening your local workspace…", false);
            return false;
        }
        true
    }
    fn notice(&mut self, message: impl Into<String>, error: bool) {
        self.notice = Some((message.into(), error, Instant::now()));
    }
    fn field(&self, key: &str) -> &str {
        self.fields.get(key).map(String::as_str).unwrap_or("")
    }
    fn request_page(&mut self) {
        if self.last_list_query != self.query {
            self.inbox_scroll = 0.;
            self.list_revision += 1;
            self.last_list_query = self.query.clone();
        }
        self.generation += 1;
        self.prefetch_page = None;
        self.prefetch_query = None;
        self.send(Command::Query(self.generation, self.query.clone(), false));
    }
    fn cache_detail(&mut self, detail: Arc<MailDetail>) {
        self.detail_cache
            .retain(|d| d.summary.id != detail.summary.id);
        self.detail_cache.push_front(detail);
        while self.detail_cache.len() > 8
            || self
                .detail_cache
                .iter()
                .map(|d| {
                    d.body.len()
                        + d.latest_body.len()
                        + d.replies
                            .iter()
                            .map(|r| r.heading.len() + r.body.len())
                            .sum::<usize>()
                        + d.attachments.iter().map(|a| a.bytes.len()).sum::<usize>()
                })
                .sum::<usize>()
                > 32 * 1024 * 1024
        {
            self.detail_cache.pop_back();
        }
    }
    fn preload(&mut self, id: String) {
        if self.detail_cache.iter().any(|d| d.summary.id == id)
            || self.pending_details.contains(&id)
        {
            return;
        }
        self.pending_details.insert(id.clone());
        self.send(Command::Detail {
            revision: self.detail_revision,
            id,
            prefetch: true,
        });
    }
    fn select(&mut self, id: String) {
        if self.selected.as_deref() != Some(&id) {
            self.expanded_replies.clear();
        }
        self.selected = Some(id.clone());
        self.detail = self
            .detail_cache
            .iter()
            .find(|d| d.summary.id == id)
            .cloned();
        if self.detail.is_none() {
            self.send(Command::Detail {
                revision: self.detail_revision,
                id: id.clone(),
                prefetch: false,
            });
        }
        if let Some(i) = self.page.rows.iter().position(|m| m.id == id) {
            if let Some(next) = self.page.rows.get(i + 1) {
                self.preload(next.id.clone());
            }
            if i > 0 {
                self.preload(self.page.rows[i - 1].id.clone());
            }
        }
        self.load_remote_images();
    }
    fn open(&mut self, dialog: Dialog) {
        self.pending_focus = None;
        self.focused_input = None;
        self.draft_dirty = None;
        self.dialog = Some(dialog);
        self.fields.clear();
        self.remapping = None;
        match dialog {
            Dialog::Account => {
                self.protocol = Protocol::Imap;
                self.fields.insert("port", "993".into());
                self.fields.insert("smtp_port", "587".into());
                self.fields.insert("setup_step", "0".into());
                self.fields.insert("incoming_security", "Tls".into());
                self.fields.insert("incoming_auth", "Password".into());
                self.fields.insert("smtp_security", "StartTls".into());
                self.fields.insert("smtp_auth", "Automatic".into());
            }
            Dialog::Compose => {
                self.composer.draft = Draft::default();
                self.composer.show_recipients = false;
                self.draft_id = uuid::Uuid::new_v4().to_string();
                self.editor = text_editor::Content::new();
                if let Some(account) = self
                    .query
                    .account
                    .clone()
                    .or_else(|| self.workspace.accounts.first().map(|a| a.id.clone()))
                {
                    self.fields.insert("account", account);
                }
            }
            Dialog::Event => {
                self.editing_event = None;
                self.fields.insert("date", self.day.to_string());
                self.fields.insert("end_date", self.day.to_string());
                self.fields
                    .insert("event_id", uuid::Uuid::new_v4().to_string());
                self.fields.insert("all_day", "true".into());
                self.fields.insert("start", "09:00".into());
                self.fields.insert("end", "10:00".into());
                if let Some(source) = self.workspace.calendars.first() {
                    self.fields.insert("source", source.id.clone());
                }
            }
            Dialog::Export => {
                self.export_index = None;
                self.fields.insert("path", default_export("message.eml"));
            }
            _ => {}
        }
    }
    fn update(&mut self, message: Message) -> Task<Message> {
        if matches!(message, Message::Noop) {
            return Task::none();
        }
        let start = Instant::now();
        let task = self.handle(message);
        self.update_samples
            .push_back(start.elapsed().as_secs_f64() * 1000.);
        if self.update_samples.len() > 1000 {
            self.update_samples.pop_front();
        }
        if self.test_state.is_some() {
            Task::batch([task, self.write_test_state()])
        } else {
            task
        }
    }
    fn handle(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Noop => return Task::none(),
            Message::Backend(event) => match event {
                Event::Ready(tx, workspace, google) => {
                    self.tx = Some(tx);
                    self.preferences = workspace.preferences.clone();
                    self.preference_sync =
                        preference_sync::PreferenceSync::new(PreferenceSnapshot {
                            revision: workspace.preferences_revision,
                            value: workspace.preferences.clone(),
                        });
                    self.panes
                        .resize(self.reader_split, self.preferences.reader_split);
                    self.query.sort = self.preferences.mail_sort;
                    if !self.preferences.unified_inbox {
                        self.query.account = workspace.accounts.first().map(|a| a.id.clone());
                    }
                    self.workspace = workspace;
                    self.google_connected = google;
                    self.request_page();
                }
                Event::Workspace(workspace) => {
                    self.preference_sync.observe(
                        PreferenceSnapshot {
                            revision: workspace.preferences_revision,
                            value: workspace.preferences.clone(),
                        },
                        &mut self.preferences,
                    );
                    let mut workspace = (*workspace).clone();
                    if workspace.drafts_revision < self.workspace.drafts_revision {
                        workspace.drafts = self.workspace.drafts.clone();
                        workspace.drafts_revision = self.workspace.drafts_revision;
                    }
                    self.workspace = Arc::new(workspace);
                    self.observe_draft_files();
                    self.update_saved_preferences();
                }
                Event::PreferencesSaved(request, snapshot) => {
                    self.preference_sync.acknowledge(
                        request,
                        (*snapshot).clone(),
                        &mut self.preferences,
                    );
                    self.update_saved_preferences();
                    self.continue_backup_request(request);
                    if self
                        .pending_google_login
                        .as_ref()
                        .is_some_and(|(id, _)| *id == request)
                        && let Some((_, prefs)) = self.pending_google_login.take()
                    {
                        if prefs.google_client_id == self.preferences.google_client_id
                            && prefs.google_client_secret == self.preferences.google_client_secret
                        {
                            self.send(Command::GoogleLogin(prefs));
                        } else {
                            self.notice("Google client details changed. Connect Google again with the updated details.", true);
                        }
                    }
                }
                Event::Page(g, page, prefetch) if g == self.generation => {
                    if prefetch {
                        if let Some(query) = self.prefetch_query.take() {
                            self.prefetch_page = Some((query, page));
                        }
                    } else {
                        self.page = page;
                        if self
                            .selected
                            .as_ref()
                            .is_some_and(|id| !self.page.rows.iter().any(|m| &m.id == id))
                        {
                            self.selected = None;
                            self.detail = None;
                        }
                        if self.selected.is_none()
                            && self.dialog.is_none()
                            && let Some(first) = self.page.rows.first()
                        {
                            self.select(first.id.clone());
                        }
                        if self.query.offset + PAGE_SIZE < self.page.total {
                            let mut query = self.query.clone();
                            query.offset += PAGE_SIZE;
                            self.prefetch_query = Some(query.clone());
                            self.send(Command::Query(g, query, true));
                        }
                        let ids: Vec<_> = self
                            .page
                            .rows
                            .iter()
                            .take(3)
                            .map(|m| m.id.clone())
                            .collect();
                        for id in ids {
                            self.preload(id);
                        }
                    }
                }
                Event::Detail {
                    revision,
                    id,
                    result,
                    prefetch,
                } if revision == self.detail_revision => {
                    self.pending_details.remove(&id);
                    match result {
                        Ok(detail) => {
                            if self.selected.as_deref() == Some(&id) {
                                self.detail = Some(detail.clone());
                            }
                            if !prefetch || self.detail_cache.len() < 8 {
                                self.cache_detail(detail);
                            }
                            self.load_remote_images();
                        }
                        Err(error) if !prefetch && self.selected.as_deref() == Some(&id) => {
                            self.notice(format!("Could not load this message: {error}"), true)
                        }
                        Err(_) => {}
                    }
                }
                Event::Changed => {
                    self.detail_revision += 1;
                    self.prefetch_page = None;
                    self.detail_cache.clear();
                    self.pending_details.clear();
                    self.request_page();
                    if let Some(id) = self.selected.clone() {
                        self.send(Command::Detail {
                            revision: self.detail_revision,
                            id,
                            prefetch: false,
                        });
                    }
                }
                Event::Calendar(events) => self.events = events,
                Event::Busy(key, busy) => {
                    if busy {
                        self.busy.insert(key);
                    } else {
                        self.busy.remove(&key);
                    }
                }
                Event::Notice(text) => self.notice(text, false),
                Event::Error(text) => {
                    self.notice(text, true);
                    self.pending_details.clear();
                }
                Event::GoogleConnected => self.google_connected = true,
                Event::AccountSaved => {
                    self.dialog = None;
                    self.fields.clear();
                    self.send(Command::Sync);
                }
                Event::CalendarSaved => {
                    self.dialog = None;
                    self.fields.clear();
                    self.send(Command::SyncCalendar);
                }
                Event::DraftSaved(id, revision, result) => {
                    return self.draft_saved(id, revision, result);
                }
                Event::DraftFiles(id, result) => {
                    if self.composer.io.as_deref() == Some(&id) {
                        self.composer.io = None;
                    }
                    match result {
                        Ok(state) => self.observe_drafts(&state),
                        Err(error) => self.notice(error, true),
                    }
                }
                Event::Sent(id, revision) => {
                    if self.draft_id == id
                        && self.composer.draft.revision == revision
                        && self.dialog == Some(Dialog::Compose)
                    {
                        self.dialog = None;
                        self.draft_dirty = None;
                        self.editor = text_editor::Content::new();
                        self.fields.clear();
                    }
                }
                Event::RemoteImage(url, result) => {
                    self.requested_images.remove(&url);
                    match result {
                        Ok(bytes) => {
                            self.remote_handles.retain(|(u, _)| u != &url);
                            self.remote_handles
                                .push_back((url, widget::image::Handle::from_bytes(bytes)));
                            while self.remote_handles.len() > 8 {
                                self.remote_handles.pop_front();
                            }
                        }
                        Err(error) => {
                            self.image_errors.insert(url, error);
                        }
                    }
                }
                Event::ConnectionTest(target, result) => {
                    let status = if target == ConnectionTarget::Incoming {
                        "test_incoming"
                    } else {
                        "test_smtp"
                    };
                    if self.dialog != Some(Dialog::Account)
                        || self.field(status) != "Testing connection…"
                    {
                        return Task::none();
                    }
                    self.fields.insert(
                        if target == ConnectionTarget::Incoming {
                            "test_incoming"
                        } else {
                            "test_smtp"
                        },
                        result.unwrap_or_else(|e| e),
                    );
                }
                Event::CalendarEventSaved(key) => {
                    let current = self
                        .editing_event
                        .as_ref()
                        .map(CalendarEvent::key)
                        .unwrap_or_else(|| {
                            CalendarEvent::scoped_key(self.field("source"), self.field("event_id"))
                        });
                    if self.dialog == Some(Dialog::Event) && current == key {
                        self.dialog = None;
                        self.editing_event = None;
                    }
                }
                Event::Backups(request, target, result) => {
                    if request == self.backups_generation
                        && target == self.configured_backup_target()
                    {
                        match result {
                            Ok(copies) => {
                                self.backups = copies;
                                self.backups_target = Some(target);
                            }
                            Err(error) => self
                                .notice(format!("Could not refresh saved copies: {error}"), true),
                        }
                    }
                }
                Event::BackupSaved(target, copy) => {
                    if target == self.configured_backup_target() {
                        self.backups_generation += 1;
                        if self.backups_target.as_ref() != Some(&target) {
                            self.backups = Arc::new(Vec::new());
                        }
                        let copies = Arc::make_mut(&mut self.backups);
                        copies.retain(|saved| saved.id != copy.id);
                        copies.insert(0, copy);
                        self.backups_target = Some(target);
                    }
                }
                Event::BackupFinished(target)
                    if target == self.configured_backup_target()
                        && self.tab == Tab::Preferences
                        && self.settings_tab == SettingsTab::Backups =>
                {
                    self.request_backup_copies(target);
                }
                _ => {}
            },
            Message::WindowClose(window) => {
                if self.busy.iter().any(|key| key.starts_with("send:")) {
                    self.notice(
                        "A message is being sent. Wait for delivery to finish before closing.",
                        true,
                    );
                } else if self.composer.io.is_some() {
                    self.notice(
                        "Wait for the selected files to finish attaching before closing.",
                        true,
                    );
                } else if self.dialog == Some(Dialog::Compose) {
                    if !self.defer_draft_exit(composing::Exit::Window(window)) {
                        return iced::window::close(window);
                    }
                } else {
                    return iced::window::close(window);
                }
            }
            Message::Tick => {
                if self.dialog == Some(Dialog::Compose)
                    && self.draft_dirty.is_some_and(|t| t.elapsed().as_secs() >= 1)
                    && self.try_command(Command::AutoSaveDraft(self.current_draft()))
                {
                    self.draft_dirty = None;
                }
                if self
                    .notice
                    .as_ref()
                    .is_some_and(|(_, err, time)| !*err && time.elapsed().as_secs() > 7)
                {
                    self.notice = None;
                }
            }
            Message::Tab(tab) => {
                if self.defer_draft_exit(composing::Exit::Tab(tab)) {
                    return Task::none();
                }
                self.tab = tab;
                self.dialog = None;
                if tab == Tab::Preferences {
                    self.fields.clear();
                    self.settings_fields();
                }
            }
            Message::SettingsTab(tab) => {
                self.tab = Tab::Preferences;
                self.settings_tab = tab;
                self.fields.clear();
                self.settings_fields();
            }
            Message::Open(dialog) => {
                self.open(dialog);
                if dialog == Dialog::Move {
                    return focus_after_layout("folder-search");
                }
            }
            Message::Close => {
                self.pending_focus = None;
                self.focused_input = None;
                if self.defer_draft_exit(composing::Exit::Dialog) {
                    return Task::none();
                }
                self.dialog = None;
                self.remapping = None;
                return widget::operation::focus("unfocused");
            }
            Message::Query(query) => {
                self.query.search = query;
                self.query.offset = 0;
                self.generation += 1;
                self.selected = None;
                self.detail = None;
                self.prefetch_page = None;
                let generation = self.generation;
                return Task::perform(
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                        generation
                    },
                    Message::SearchReady,
                );
            }
            Message::SearchReady(g) if g == self.generation => self.request_page(),
            Message::SearchReady(_) => {}
            Message::Folder(folder) => {
                self.tab = Tab::Mail;
                self.query.folder = folder;
                self.query.starred_only = false;
                self.query.offset = 0;
                self.selected = None;
                self.detail = None;
                self.request_page();
            }
            Message::Account(account) => {
                self.query.account = account.or_else(|| {
                    if self.preferences.unified_inbox {
                        None
                    } else {
                        self.workspace.accounts.first().map(|a| a.id.clone())
                    }
                });
                self.query.offset = 0;
                self.selected = None;
                self.detail = None;
                self.request_page();
            }
            Message::Sort(sort) => {
                self.query.sort = sort;
                self.preferences.mail_sort = sort;
                self.query.offset = 0;
                self.selected = None;
                self.detail = None;
                self.request_page();
                self.save_preferences();
            }
            Message::Filter(filter) => {
                self.query.unread_only = filter == MailFilter::Unread;
                self.query.read_only = filter == MailFilter::Read;
                self.query.starred_only = filter == MailFilter::Flagged;
                self.query.attachments_only = filter == MailFilter::Attachments;
                self.query.offset = 0;
                self.selected = None;
                self.detail = None;
                self.request_page();
            }
            Message::Starred => {
                self.tab = Tab::Mail;
                self.query.starred_only = true;
                self.query.unread_only = false;
                self.query.read_only = false;
                self.query.attachments_only = false;
                self.query.folder.clear();
                self.query.offset = 0;
                self.selected = None;
                self.detail = None;
                self.request_page();
            }
            Message::Select(id) => {
                let double = self
                    .last_click
                    .as_ref()
                    .is_some_and(|(last, time)| last == &id && time.elapsed().as_millis() < 400);
                self.last_click = if double {
                    None
                } else {
                    Some((id.clone(), Instant::now()))
                };
                if double {
                    return self.handle(Message::OpenMessage(id));
                }
                self.sidebar_focus = false;
                self.select(id);
            }
            Message::Hover(id) => self.preload(id),
            Message::NextPage(next) => {
                self.query.offset = if next {
                    (self.query.offset + PAGE_SIZE)
                        .min(self.page.total.saturating_sub(1) / PAGE_SIZE * PAGE_SIZE)
                } else {
                    self.query.offset.saturating_sub(PAGE_SIZE)
                };
                self.selected = None;
                self.detail = None;
                if let Some((query, page)) = self.prefetch_page.take()
                    && query == self.query
                {
                    self.page = page;
                    if let Some(first) = self.page.rows.first() {
                        self.select(first.id.clone());
                    }
                }
                self.request_page();
            }
            Message::PreviousMessage(previous) => {
                if let Some(i) = self
                    .selected
                    .as_ref()
                    .and_then(|id| self.page.rows.iter().position(|m| &m.id == id))
                {
                    let next = if previous {
                        i.saturating_sub(1)
                    } else {
                        (i + 1).min(self.page.rows.len().saturating_sub(1))
                    };
                    if let Some(m) = self.page.rows.get(next) {
                        self.select(m.id.clone());
                        let viewport = (self.size.height
                            / (self.preferences.interface_scale as f32 / 100.)
                            - 220.)
                            .max(104.);
                        let top = next as f32 * 104.;
                        let offset = if top < self.inbox_scroll {
                            top
                        } else if top + 104. > self.inbox_scroll + viewport {
                            top + 104. - viewport
                        } else {
                            self.inbox_scroll
                        };
                        self.inbox_scroll = offset;
                        return widget::operation::scroll_to(
                            "inbox-list",
                            widget::scrollable::AbsoluteOffset { x: 0., y: offset },
                        );
                    }
                }
            }
            Message::Focus(id, attempt) => {
                let visible = match id {
                    "folder-search" => self.dialog == Some(Dialog::Move),
                    "event-title" => self.dialog == Some(Dialog::Event),
                    "search" => self.tab == Tab::Mail && self.dialog.is_none() && !self.full_reader,
                    _ => false,
                };
                if !visible || (attempt > 0 && self.pending_focus != Some(id)) {
                    return Task::none();
                }
                self.pending_focus = Some(id);
                let focus = widget::operation::focus(id).chain(
                    widget::operation::is_focused(id)
                        .map(move |focused| Message::FocusChecked(id, focused)),
                );
                // iced operations visit the last laid-out widget tree. Retry until the newly
                // opened field exists, stopping as soon as native focus is acknowledged.
                return if attempt < 8 {
                    Task::batch([
                        focus,
                        Task::perform(
                            async move {
                                tokio::time::sleep(std::time::Duration::from_millis(32)).await;
                                (id, attempt + 1)
                            },
                            |(id, attempt)| Message::Focus(id, attempt),
                        ),
                    ])
                } else {
                    self.pending_focus = None;
                    focus
                };
            }
            Message::FocusChecked(id, focused) => {
                if focused && self.pending_focus == Some(id) {
                    self.focused_input = Some(id);
                    self.pending_focus = None;
                }
            }
            Message::Sync => self.send(Command::Sync),
            Message::SyncCalendar => self.send(Command::SyncCalendar),
            Message::Move(folder) => {
                if let Some(d) = &self.detail {
                    let destination = self.field("move_account");
                    let transfer = self.dialog == Some(Dialog::Move)
                        && self.preferences.cross_account_moves
                        && !destination.is_empty()
                        && destination != d.summary.account_id;
                    if !transfer && d.summary.folder == folder {
                        self.dialog = None;
                        return Task::none();
                    }
                    if transfer {
                        self.send(Command::Transfer(
                            d.summary.clone(),
                            destination.to_owned(),
                            folder,
                        ));
                    } else {
                        self.send(Command::Move(d.summary.clone(), folder));
                    }
                    self.dialog = None;
                    self.selected = None;
                    self.detail = None;
                }
            }
            Message::ToggleStar | Message::ToggleRead => {
                if let Some(detail) = &self.detail {
                    if self
                        .busy
                        .contains(&format!("message:{}", detail.summary.id))
                    {
                        return Task::none();
                    }
                    let mut mail = detail.summary.clone();
                    if matches!(message, Message::ToggleStar) {
                        mail.starred = !mail.starred;
                    } else {
                        mail.unread = !mail.unread;
                    }
                    self.send(Command::Flags(mail));
                }
            }
            Message::Reply | Message::ReplyAll => {
                if let Some(detail) = self.detail.clone() {
                    let draft = detail.reply.draft(
                        &detail,
                        &self.workspace.accounts,
                        matches!(message, Message::ReplyAll),
                    );
                    self.load_draft(draft);
                }
            }
            Message::ChooseAttachments => return self.choose_attachments(),
            Message::ChosenAttachments(draft, paths) => self.attach_chosen(draft, paths),
            Message::RemoveDraftAttachment(id) => self.remove_draft_attachment(id),
            Message::ShowRecipients => {
                self.composer.show_recipients = !self.composer.show_recipients
            }
            Message::SaveDraft => {
                self.save_and_exit(composing::Exit::Dialog);
            }
            Message::Send => {
                if !self.compose_locked() && self.composer.io.as_deref() != Some(&self.draft_id) {
                    let draft = self.current_draft();
                    if self.try_command(Command::Send(draft)) {
                        self.busy.insert(format!("send:{}", self.draft_id));
                    }
                }
            }
            Message::Draft(id) => {
                if let Some(draft) = self.workspace.drafts.iter().find(|d| d.id == id).cloned() {
                    self.load_draft(draft);
                }
            }
            Message::Field(key, value) => {
                if self.compose_locked() {
                    return Task::none();
                }
                if self.dialog == Some(Dialog::Account) && key != "setup_step" {
                    self.fields.remove("test_incoming");
                    self.fields.remove("test_smtp");
                    if key == "incoming_security" {
                        let port = match (self.protocol, value.as_str()) {
                            (Protocol::Imap, "StartTls") => "143",
                            (Protocol::Pop3, "StartTls") => "110",
                            (Protocol::Imap, _) => "993",
                            _ => "995",
                        };
                        self.fields.insert("port", port.into());
                    } else if key == "smtp_security" {
                        self.fields.insert(
                            "smtp_port",
                            if value == "Tls" { "465" } else { "587" }.into(),
                        );
                    }
                }
                self.fields.insert(key, value);
                if self.dialog == Some(Dialog::Compose) {
                    self.draft_edited();
                }
            }
            Message::Protocol(protocol) => {
                self.protocol = protocol;
                self.fields.remove("test_incoming");
                let security = self.field("incoming_security").to_owned();
                if self.field("host") == "imap.fastmail.com"
                    || self.field("host") == "pop.fastmail.com"
                {
                    self.fields.insert(
                        "host",
                        if protocol == Protocol::Imap {
                            "imap.fastmail.com"
                        } else {
                            "pop.fastmail.com"
                        }
                        .into(),
                    );
                }
                return self.handle(Message::Field("incoming_security", security));
            }
            Message::ConnectCalendarFromEvent => {
                self.dialog = None;
                return self.handle(Message::SettingsTab(SettingsTab::Calendars));
            }
            Message::FastmailPreset => {
                for (key, value) in [
                    (
                        "host",
                        if self.protocol == Protocol::Imap {
                            "imap.fastmail.com"
                        } else {
                            "pop.fastmail.com"
                        },
                    ),
                    ("smtp_host", "smtp.fastmail.com"),
                    ("smtp_port", "465"),
                    (
                        "port",
                        if self.protocol == Protocol::Imap {
                            "993"
                        } else {
                            "995"
                        },
                    ),
                    ("incoming_security", "Tls"),
                    ("smtp_security", "Tls"),
                    ("incoming_auth", "Password"),
                    ("smtp_auth", "Plain"),
                ] {
                    self.fields.insert(key, value.into());
                }
            }
            Message::TestConnection(target) => match self.account_form() {
                Ok(account) => {
                    self.fields.insert(
                        if target == ConnectionTarget::Incoming {
                            "test_incoming"
                        } else {
                            "test_smtp"
                        },
                        "Testing connection…".into(),
                    );
                    self.send(Command::TestConnection(
                        account,
                        self.field("password").to_string().into(),
                        self.field("smtp_password").to_string().into(),
                        target,
                    ));
                }
                Err(error) => {
                    self.notice(error.to_string(), true);
                }
            },
            Message::SaveAccount => match self.account_form() {
                Ok(account) => {
                    let password = self.field("password").to_string();
                    self.send(Command::SaveAccount(
                        account,
                        secrecy::SecretString::from(password),
                        self.field("smtp_password").to_string().into(),
                    ));
                }
                Err(e) => self.notice(e.to_string(), true),
            },
            Message::EditAccount(id) => {
                if let Some(a) = self.workspace.accounts.iter().find(|a| a.id == id).cloned() {
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
                    for (k, v) in [
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
                    ] {
                        self.fields.insert(k, v);
                    }
                }
            }
            Message::SaveCalendar => {
                let mut url = self.field("url").trim().to_string();
                if !url.ends_with('/') {
                    url.push('/');
                }
                let source = CalendarSource {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: self.field("name").into(),
                    kind: CalendarKind::CalDav,
                    url,
                    username: self.field("username").into(),
                };
                self.send(Command::SaveCalendar(
                    source,
                    secrecy::SecretString::from(self.field("password").to_string()),
                ));
            }
            Message::SavePreferences => match self.read_preferences() {
                Ok(()) => self.save_preferences(),
                Err(e) => self.notice(e.to_string(), true),
            },
            Message::Appearance(appearance) => {
                self.preferences.appearance = appearance;
                self.save_preferences();
            }
            Message::BackupDestination(destination) => {
                self.preferences.backup_destination = destination;
                self.preference_sync.changed();
            }
            Message::BackupAccounts(enabled) => {
                self.preferences.backup_accounts = enabled;
                self.preference_sync.changed();
            }
            Message::AutoBackup(enabled) => {
                self.preferences.auto_backup = enabled;
                self.preference_sync.changed();
            }
            Message::GoogleLogin => {
                if let Err(e) = self.read_preferences() {
                    self.notice(e.to_string(), true);
                } else {
                    let request = self.preference_sync.changed();
                    self.pending_google_login = Some((request, self.preferences.clone()));
                    self.send(Command::SavePreferences(request, self.preferences.clone()));
                }
            }
            Message::Backup => self.begin_backup_request(backups::BackupAction::Save(
                secrecy::SecretString::from(self.field("passphrase").to_string()),
            )),
            Message::ListBackups => self.begin_backup_request(backups::BackupAction::List),
            Message::Restore(id) => {
                if self.visible_backups().iter().any(|copy| copy.id == id) {
                    self.restore_id = id;
                    self.restore_target = self.backups_target.clone();
                    self.dialog = Some(Dialog::Restore);
                } else {
                    self.notice(
                        "Refresh the saved copies for this destination before restoring.",
                        true,
                    );
                }
            }
            Message::ConfirmRestore => {
                if self.restore_target.as_ref() == Some(&self.configured_backup_target()) {
                    self.begin_backup_request(backups::BackupAction::Restore(
                        self.restore_id.clone(),
                        secrecy::SecretString::from(self.field("passphrase").to_string()),
                    ));
                    self.dialog = None;
                } else {
                    self.notice("The backup destination changed. Close this dialog and refresh copies before restoring.", true);
                }
            }
            Message::Key(key, modifiers, captured) => return self.key(key, modifiers, captured),
            Message::Remap(action) => {
                self.remapping = Some(action);
            }
            Message::ResetShortcuts => {
                self.preferences.shortcuts = Default::default();
                self.save_preferences();
            }
            Message::Editor(action) => {
                if self.compose_locked() {
                    return Task::none();
                }
                if action.is_edit() {
                    self.draft_edited();
                }
                self.editor.perform(action);
            }
            Message::Month(delta) => {
                let months = chrono::Months::new(delta.unsigned_abs());
                self.month = if delta < 0 {
                    self.month.checked_sub_months(months)
                } else {
                    self.month.checked_add_months(months)
                }
                .unwrap_or(self.month);
            }
            Message::Today => {
                self.day = chrono::Local::now().date_naive();
                self.month = self.day.with_day(1).unwrap();
            }
            Message::Day(day) => {
                let id = format!("day:{day}");
                let double = self
                    .last_click
                    .as_ref()
                    .is_some_and(|(last, time)| last == &id && time.elapsed().as_millis() < 400);
                self.last_click = if double {
                    None
                } else {
                    Some((id, Instant::now()))
                };
                self.day = day;
                if double {
                    return self.handle(Message::NewEventOnDay(day));
                }
            }
            Message::NewEventOnDay(day) => {
                self.day = day;
                self.open(Dialog::Event);
                return focus_after_layout("event-title");
            }
            Message::EditEvent(key) => {
                if let Some(event) = self.events.iter().find(|e| e.key() == key).cloned() {
                    self.open(Dialog::Event);
                    self.fields.insert("title", event.title.clone());
                    self.fields.insert("location", event.location.clone());
                    self.fields.insert("source", event.source_id.clone());
                    self.fields.insert(
                        "date",
                        event
                            .start
                            .with_timezone(&chrono::Local)
                            .date_naive()
                            .to_string(),
                    );
                    self.fields.insert(
                        "start",
                        event
                            .start
                            .with_timezone(&chrono::Local)
                            .format("%H:%M")
                            .to_string(),
                    );
                    self.fields.insert(
                        "end",
                        event
                            .end
                            .with_timezone(&chrono::Local)
                            .format("%H:%M")
                            .to_string(),
                    );
                    self.fields.insert(
                        "end_date",
                        event
                            .end
                            .with_timezone(&chrono::Local)
                            .date_naive()
                            .to_string(),
                    );
                    self.fields.insert("all_day", event.all_day.to_string());
                    if event.all_day {
                        self.fields
                            .insert("date", event.start.date_naive().to_string());
                        self.fields.insert(
                            "end_date",
                            (event.end.date_naive() - chrono::Duration::days(1)).to_string(),
                        );
                    }
                    self.editing_event = Some(event);
                }
            }
            Message::SaveEvent => match self.event_form() {
                Ok(event) => {
                    self.send(Command::SaveEvent(event));
                }
                Err(e) => self.notice(e.to_string(), true),
            },
            Message::DeleteEvent => {
                if let Some(event) = &self.editing_event {
                    self.send(Command::DeleteEvent(event.clone()));
                }
            }
            Message::ExportAttachment(index) => {
                self.open(Dialog::Export);
                self.export_index = Some(index);
                if let Some(a) = self.detail.as_ref().and_then(|d| d.attachments.get(index)) {
                    let filename = std::path::Path::new(&a.name)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy();
                    self.fields.insert("path", default_export(&filename));
                }
            }
            Message::SaveExport => {
                if let Some(id) = &self.selected {
                    let command = if let Some(index) = self.export_index {
                        Command::ExportAttachment(id.clone(), index, self.field("path").into())
                    } else {
                        Command::ExportMessage(id.clone(), self.field("path").into())
                    };
                    self.send(command);
                    self.dialog = None;
                }
            }
            Message::SystemTheme(mode) => self.system_dark = mode == iced::theme::Mode::Dark,
            Message::Resize(size) => self.size = size,
            Message::Dismiss => self.notice = None,
            Message::BrowseBackup => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose a backup folder")
                            .pick_folder()
                            .await
                            .map(|f| f.path().to_string_lossy().into_owned())
                    },
                    |p| Message::ChosenPath("backup_folder", p),
                );
            }
            Message::BrowseExport => {
                let filename = std::path::Path::new(self.field("path"))
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                return Task::perform(
                    async move {
                        rfd::AsyncFileDialog::new()
                            .set_title("Save a copy")
                            .set_file_name(filename)
                            .save_file()
                            .await
                            .map(|f| f.path().to_string_lossy().into_owned())
                    },
                    |p| Message::ChosenPath("path", p),
                );
            }
            Message::ChosenPath(key, path) => {
                if let Some(path) = path {
                    self.fields.insert(key, path);
                }
            }
            Message::ToggleInboxExpanded => self.inbox_expanded = !self.inbox_expanded,
            Message::SidebarAction(index) => {
                self.sidebar_focus = true;
                self.sidebar_index = index;
                if let Some(item) = self.sidebar_items().get(index) {
                    return self.handle(item.action.clone());
                }
            }
            Message::AccountFolderUnified => {
                self.query.account = None;
                return self.handle(Message::Folder("INBOX".into()));
            }
            Message::AccountFolder(account, folder) => {
                self.query.account = Some(account);
                return self.handle(Message::Folder(folder));
            }
            Message::PrefUnified(value) => {
                self.preferences.unified_inbox = value;
                self.query.account = if value {
                    None
                } else {
                    self.workspace.accounts.first().map(|a| a.id.clone())
                };
                self.request_page();
                self.save_preferences();
            }
            Message::PrefCrossAccount(value) => {
                self.preferences.cross_account_moves = value;
                self.save_preferences();
            }
            Message::PrefReaderSize(value) => {
                self.preferences.reader_font_size = value;
                self.save_preferences();
            }
            Message::PrefScale(value) => {
                self.preferences.interface_scale = value;
                self.save_preferences();
            }
            Message::OpenMessage(id) => {
                self.select(id);
                self.full_reader = true;
                self.sidebar_focus = false;
            }
            Message::ClosePreview => self.full_reader = false,
            Message::CopyAddress(address) => return iced::clipboard::write(address),
            Message::ToggleReply(index) => {
                if !self.expanded_replies.remove(&index) {
                    self.expanded_replies.insert(index);
                }
            }
            Message::PrefReplies(mode) => {
                self.preferences.reply_display = mode;
                self.expanded_replies.clear();
                self.save_preferences();
            }
            Message::PrefImages(policy) => {
                self.preferences.image_policy = policy;
                self.save_preferences();
                self.load_remote_images();
            }
            Message::ClearImageTrust => {
                self.preferences.image_messages.clear();
                self.preferences.image_senders.clear();
                self.preferences.image_domains.clear();
                self.save_preferences();
            }
            Message::AllowImages(scope) => {
                if let Some(detail) = &self.detail {
                    match scope {
                        0 => self
                            .preferences
                            .image_messages
                            .push(detail.summary.id.clone()),
                        1 => {
                            if let Some(sender) =
                                crate::remote_images::sender_address(&detail.summary.sender)
                            {
                                self.preferences.image_senders.push(sender);
                            }
                        }
                        _ => {
                            if let Some(domain) =
                                crate::remote_images::sender_domain(&detail.summary.sender)
                            {
                                self.preferences.image_domains.push(domain);
                            }
                        }
                    }
                    self.save_preferences();
                    self.load_remote_images();
                }
            }
            Message::FlagRow(id) => {
                if let Some(mut mail) = self.page.rows.iter().find(|m| m.id == id).cloned()
                    && !self.busy.contains(&format!("message:{id}"))
                {
                    mail.starred = !mail.starred;
                    self.send(Command::Flags(mail));
                }
            }
            Message::InboxScroll(offset) => self.inbox_scroll = offset,
            Message::PaneResize(event) => {
                let schedule = self.pending_resize.is_none();
                self.pending_resize = Some(event);
                if schedule {
                    return Task::perform(
                        async {
                            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
                        },
                        |_| Message::ApplyPaneResize,
                    );
                }
            }
            Message::ApplyPaneResize => {
                let Some(event) = self.pending_resize.take() else {
                    return Task::none();
                };
                let ratio = event.ratio.clamp(0.2, 0.7);
                self.panes.resize(event.split, ratio);
                self.preferences.reader_split = ratio;
                self.preference_sync.changed();
                self.layout_generation += 1;
                let generation = self.layout_generation;
                return Task::perform(
                    async move {
                        tokio::time::sleep(std::time::Duration::from_millis(350)).await;
                        generation
                    },
                    Message::SaveLayout,
                );
            }
            Message::SaveLayout(generation) => {
                if generation == self.layout_generation {
                    self.save_preferences();
                }
            }
        }
        Task::none()
    }
    fn load_remote_images(&mut self) {
        if let Some(detail) = &self.detail
            && crate::remote_images::allowed(&self.preferences, &detail.summary)
        {
            let urls: Vec<_> = detail
                .remote_images
                .iter()
                .filter_map(|image| {
                    if !self.remote_handles.iter().any(|(url, _)| url == &image.url)
                        && !self.image_errors.contains_key(&image.url)
                        && self.requested_images.insert(image.url.clone())
                    {
                        Some(image.url.clone())
                    } else {
                        None
                    }
                })
                .collect();
            if !urls.is_empty() {
                self.send(Command::LoadImages(urls));
            }
        }
    }
    fn move_folders(&self) -> Vec<String> {
        let account = if self.field("move_account").is_empty() {
            self.detail
                .as_ref()
                .map(|d| d.summary.account_id.as_str())
                .unwrap_or("")
        } else {
            self.field("move_account")
        };
        self.workspace
            .account_folders
            .get(account)
            .cloned()
            .unwrap_or_else(|| self.workspace.folders.clone())
    }
    fn mail_filter(&self) -> MailFilter {
        if self.query.unread_only {
            MailFilter::Unread
        } else if self.query.read_only {
            MailFilter::Read
        } else if self.query.starred_only {
            MailFilter::Flagged
        } else if self.query.attachments_only {
            MailFilter::Attachments
        } else {
            MailFilter::All
        }
    }
    fn connection_security(&self, key: &str) -> ConnectionSecurity {
        if self.field(key) == "StartTls" {
            ConnectionSecurity::StartTls
        } else {
            ConnectionSecurity::Tls
        }
    }
    fn incoming_auth(&self) -> IncomingAuth {
        if self.field("incoming_auth") == "Plain" {
            IncomingAuth::Plain
        } else {
            IncomingAuth::Password
        }
    }
    fn smtp_auth(&self) -> SmtpAuth {
        match self.field("smtp_auth") {
            "Plain" => SmtpAuth::Plain,
            "Login" => SmtpAuth::Login,
            "None" => SmtpAuth::None,
            _ => SmtpAuth::Automatic,
        }
    }
    fn account_form(&self) -> anyhow::Result<Account> {
        let account = Account {
            id: if self.field("id").is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                self.field("id").into()
            },
            name: self.field("name").trim().into(),
            email: self.field("email").trim().into(),
            protocol: self.protocol,
            host: self.field("host").trim().into(),
            port: self
                .field("port")
                .parse()
                .map_err(|_| anyhow::anyhow!("Enter a valid incoming port."))?,
            username: if self.field("username").is_empty() {
                self.field("email")
            } else {
                self.field("username")
            }
            .trim()
            .into(),
            smtp_host: self.field("smtp_host").trim().into(),
            incoming_security: self.connection_security("incoming_security"),
            incoming_auth: self.incoming_auth(),
            smtp_security: Some(self.connection_security("smtp_security")),
            smtp_auth: self.smtp_auth(),
            smtp_username: self.field("smtp_username").trim().into(),
            smtp_separate_password: self.field("smtp_separate") == "true",
            smtp_port: self
                .field("smtp_port")
                .parse()
                .map_err(|_| anyhow::anyhow!("Enter a valid SMTP port."))?,
        };
        account.validate()?;
        Ok(account)
    }
    fn settings_fields(&mut self) {
        for (k, v) in [
            ("backup_folder", self.preferences.backup_folder.clone()),
            ("copies", self.preferences.backup_copies.to_string()),
            ("hours", self.preferences.backup_hours.to_string()),
            ("sync_minutes", self.preferences.sync_minutes.to_string()),
            ("contacts", self.preferences.contacts.join(", ")),
            ("google_id", self.preferences.google_client_id.clone()),
            (
                "google_secret",
                self.preferences.google_client_secret.clone(),
            ),
        ] {
            self.fields.insert(k, v);
        }
    }
    fn read_preferences(&mut self) -> anyhow::Result<()> {
        let mut next = self.preferences.clone();
        if self.fields.contains_key("copies") {
            next.backup_folder = self.field("backup_folder").into();
            next.backup_copies = self.field("copies").parse()?;
            next.backup_hours = self.field("hours").parse()?;
            next.sync_minutes = self.field("sync_minutes").parse()?;
            next.google_client_id = self.field("google_id").trim().into();
            next.google_client_secret = self.field("google_secret").trim().into();
        }
        if self.fields.contains_key("contacts") {
            next.contacts = self
                .field("contacts")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| {
                    crate::remote_images::sender_address(s)
                        .ok_or_else(|| anyhow::anyhow!("Enter valid contact email addresses"))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
        }
        next.validate()?;
        self.preferences = next;
        Ok(())
    }
    fn event_form(&self) -> anyhow::Result<CalendarEvent> {
        anyhow::ensure!(
            !self.field("title").trim().is_empty(),
            "Give your event a title."
        );
        anyhow::ensure!(
            !self.field("source").is_empty(),
            "Connect a calendar first."
        );
        let all_day = self.field("all_day") == "true";
        let parse = |key: &str| -> anyhow::Result<chrono::DateTime<chrono::Utc>> {
            let day = self.field(if key == "end" { "end_date" } else { "date" });
            if all_day {
                let date = NaiveDate::parse_from_str(day, "%Y-%m-%d")?
                    + chrono::Duration::days(i64::from(key == "end"));
                return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc());
            }
            let date = chrono::NaiveDateTime::parse_from_str(
                &format!("{} {}", day, self.field(key)),
                "%Y-%m-%d %H:%M",
            )?;
            Ok(chrono::Local
                .from_local_datetime(&date)
                .single()
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "This local time is ambiguous or falls in a daylight-saving gap."
                    )
                })?
                .with_timezone(&chrono::Utc))
        };
        let start = parse("start")?;
        let end = parse("end")?;
        anyhow::ensure!(end > start, "The end time must be after the start time.");
        if let Some(event) = &self.editing_event {
            anyhow::ensure!(
                event.source_id == self.field("source"),
                "Keep the original calendar when editing an existing event."
            );
            let source = self
                .workspace
                .calendars
                .iter()
                .find(|s| s.id == event.source_id);
            anyhow::ensure!(
                !source
                    .is_some_and(|s| s.kind == CalendarKind::CalDav && event.remote_url.is_none()),
                "Edit recurring events on your calendar server."
            );
        }
        Ok(CalendarEvent {
            id: self
                .editing_event
                .as_ref()
                .map(|e| e.id.clone())
                .unwrap_or_else(|| self.field("event_id").into()),
            source_id: self.field("source").into(),
            title: self.field("title").into(),
            start,
            end,
            location: self.field("location").into(),
            description: self
                .editing_event
                .as_ref()
                .map(|e| e.description.clone())
                .unwrap_or_default(),
            all_day,
            etag: self.editing_event.as_ref().and_then(|e| e.etag.clone()),
            remote_url: self
                .editing_event
                .as_ref()
                .and_then(|e| e.remote_url.clone()),
        })
    }
    fn key(&mut self, key: Key, modifiers: keyboard::Modifiers, captured: bool) -> Task<Message> {
        let chord = chord(&key, modifiers);
        #[cfg(feature = "test-support")]
        if self.demo && chord.is_some() {
            self.test_keys.push_back(format!(
                "{:?} captured={captured} dialog={:?}",
                chord, self.dialog
            ));
            while self.test_keys.len() > 16 {
                self.test_keys.pop_front();
            }
        }
        if let Some(action) = self.remapping {
            if let Some(chord) = chord {
                match self.preferences.shortcuts.remap(action, chord) {
                    Ok(()) => {
                        self.remapping = None;
                        self.save_preferences();
                    }
                    Err(e) => self.notice(e.to_string(), true),
                }
            }
            return Task::none();
        }

        if key == Key::Named(keyboard::key::Named::Escape) && modifiers.is_empty() {
            if self.dialog.is_none() && self.full_reader {
                if self.preferences.shortcuts.resolve("Escape") == Some(Action::ClosePreview) {
                    return self.handle(Message::ClosePreview);
                }
                return Task::none();
            }
            return self.handle(Message::Close);
        }
        if key == Key::Named(keyboard::key::Named::Tab) {
            if self.tab == Tab::Mail && self.dialog.is_none() {
                self.sidebar_focus = !self.sidebar_focus;
                return widget::operation::focus("unfocused");
            }
            return if modifiers.shift() {
                widget::operation::focus_previous()
            } else {
                widget::operation::focus_next()
            };
        }
        if captured && !modifiers.command() {
            return Task::none();
        }
        if self.dialog.is_some() {
            return Task::none();
        }
        if self.tab == Tab::Mail
            && matches!(
                key,
                Key::Named(keyboard::key::Named::ArrowUp | keyboard::key::Named::ArrowDown)
            )
        {
            let previous = key == Key::Named(keyboard::key::Named::ArrowUp);
            if self.sidebar_focus {
                let len = self.sidebar_items().len();
                self.sidebar_index = if previous {
                    self.sidebar_index.saturating_sub(1)
                } else {
                    (self.sidebar_index + 1).min(len.saturating_sub(1))
                };
                return self.handle(Message::SidebarAction(self.sidebar_index));
            }
            return self.handle(Message::PreviousMessage(previous));
        }
        if let Some(action) = chord
            .as_deref()
            .and_then(|k| self.preferences.shortcuts.resolve(k))
        {
            return match action {
                Action::Search => {
                    self.focused_input = None;
                    self.tab = Tab::Mail;
                    self.full_reader = false;
                    focus_after_layout("search")
                }
                Action::Move => {
                    if self.detail.is_some() {
                        self.open(Dialog::Move);
                        return focus_after_layout("folder-search");
                    }
                    Task::none()
                }
                Action::Compose => self.handle(Message::Open(Dialog::Compose)),
                Action::Reply => self.handle(Message::Reply),
                Action::ReplyAll => self.handle(Message::ReplyAll),
                Action::Archive => self.handle(Message::Move("Archive".into())),
                Action::Star => self.handle(Message::ToggleStar),
                Action::Sync => self.handle(Message::Sync),
                Action::Next => self.handle(Message::PreviousMessage(false)),
                Action::Previous => self.handle(Message::PreviousMessage(true)),
                Action::Mail => self.handle(Message::Tab(Tab::Mail)),
                Action::Calendar => self.handle(Message::Tab(Tab::Calendar)),
                Action::Settings => self.handle(Message::Tab(Tab::Preferences)),
                Action::OpenMessage => {
                    if let Some(id) = self.selected.clone() {
                        self.handle(Message::OpenMessage(id))
                    } else {
                        Task::none()
                    }
                }
                Action::ClosePreview => self.handle(Message::ClosePreview),
            };
        }
        Task::none()
    }
    fn write_test_state(&mut self) -> Task<Message> {
        let Some(path) = self.test_state.clone() else {
            return Task::none();
        };
        self.test_revision += 1;
        let mut samples: Vec<_> = self.update_samples.iter().copied().collect();
        samples.sort_by(f64::total_cmp);
        let mut data = serde_json::json!({"revision":self.test_revision,"tab":format!("{:?}",self.tab),"settings_tab":format!("{:?}",self.settings_tab),"dialog":self.dialog.map(|d|format!("{d:?}")),"dark":self.dark(),"reader_split":self.preferences.reader_split,"saved_reader_split":self.workspace.preferences.reader_split,"sort":format!("{:?}",self.query.sort),"filter":format!("{:?}",self.mail_filter()),"offset":self.query.offset,"busy":self.busy,"query":self.query.search,"folder":self.query.folder,"total":self.page.total,"selected":self.detail.as_ref().map(|d|&d.summary.subject),"selected_id":self.selected,"starred":self.detail.as_ref().map(|d|d.summary.starred),"cache_entries":self.detail_cache.len(),"page_prefetched":self.prefetch_page.is_some(),"ready":self.tx.is_some(),"shortcuts":self.preferences.shortcuts,"fields":self.fields.iter().filter(|(k,_)|!k.contains("password")&&!k.contains("secret")&&!k.contains("passphrase")).collect::<HashMap<_,_>>(),"full_reader":self.full_reader,"image_policy":format!("{:?}",self.preferences.image_policy),"images_allowed":self.detail.as_ref().is_some_and(|d|crate::remote_images::allowed(&self.preferences,&d.summary)),"remote_image_count":self.detail.as_ref().map(|d|d.remote_images.len()),"reply_count":self.detail.as_ref().map(|d|d.replies.len()),"expanded_replies":self.expanded_replies,"sidebar_focus":self.sidebar_focus,"inbox_expanded":self.inbox_expanded,"unified":self.preferences.unified_inbox,"cross_account_moves":self.preferences.cross_account_moves,"reader_size":self.preferences.reader_font_size,"calendar_connected":!self.workspace.calendars.is_empty(),"draft_count":self.workspace.drafts.len(),"draft_body":self.workspace.drafts.first().map(|d|&d.body),"editor":self.editor.text(),"notice":self.notice.as_ref().map(|n|&n.0),"update_p95_ms":samples.get(samples.len()*95/100),"uptime_ms":self.started.elapsed().as_millis(),"events":self.events.len()});
        data["draft_attachments"] = serde_json::json!(self.composer.draft.attachments);
        data["draft_io"] = serde_json::json!(self.composer.io.is_some());
        data["draft_in_reply_to"] = serde_json::json!(self.composer.draft.in_reply_to);
        data["focused_input"] = serde_json::json!(self.focused_input);
        data["auto_backup"] = serde_json::json!(self.preferences.auto_backup);
        data["backup_ready"] = serde_json::json!(self.preferences.backup_ready);
        data["saved_backup_folder"] = serde_json::json!(self.workspace.preferences.backup_folder);
        data["saved_backup_copies"] = serde_json::json!(self.workspace.preferences.backup_copies);
        data["saved_auto_backup"] = serde_json::json!(self.workspace.preferences.auto_backup);
        data["preferences_saved"] = serde_json::json!(!self.preference_sync.dirty());
        data["saved_preferences_revision"] = serde_json::json!(self.workspace.preferences_revision);
        data["saved_appearance"] =
            serde_json::json!(format!("{:?}", self.workspace.preferences.appearance));
        data["attachment_count"] =
            serde_json::json!(self.detail.as_ref().map(|d| d.attachments.len()));
        data["account"] = serde_json::json!(self.query.account);
        #[cfg(feature = "test-support")]
        {
            data["keys"] = serde_json::json!(self.test_keys);
        }
        data["inbox_scroll"] = serde_json::json!(self.inbox_scroll);
        Task::perform(
            async move {
                static SNAPSHOT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
                let _guard = SNAPSHOT_LOCK.lock().await;
                if let Ok(previous) = tokio::fs::read(&path).await
                    && let Ok(previous) = serde_json::from_slice::<serde_json::Value>(&previous)
                    && previous["revision"].as_u64() >= data["revision"].as_u64()
                {
                    return;
                }
                let temp = path.with_extension("tmp");
                if let Ok(json) = serde_json::to_vec(&data)
                    && tokio::fs::write(&temp, json).await.is_ok()
                {
                    let _ = tokio::fs::rename(temp, path).await;
                }
            },
            |_| Message::Noop,
        )
    }
    fn view(&self) -> Element<'_, Message> {
        self.layout()
    }
}
pub fn chord(key: &Key, modifiers: keyboard::Modifiers) -> Option<String> {
    let key = match key {
        Key::Character(c) => c.to_uppercase(),
        Key::Named(n) => match n {
            keyboard::key::Named::Shift
            | keyboard::key::Named::Control
            | keyboard::key::Named::Alt
            | keyboard::key::Named::Super => return None,
            _ => format!("{n:?}"),
        },
        _ => return None,
    };
    let mut parts = Vec::new();
    if modifiers.command() {
        parts.push("Mod".into());
    } else if modifiers.control() {
        parts.push("Ctrl".into());
    }
    if modifiers.alt() {
        parts.push("Alt".into());
    }
    if modifiers.shift() {
        parts.push("Shift".into());
    }
    parts.push(key);
    Some(parts.join("+"))
}
fn default_export(name: &str) -> String {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(|p| p.join(name)))
        .unwrap_or_else(|| std::path::PathBuf::from(name))
        .to_string_lossy()
        .to_string()
}

fn focus_after_layout(id: &'static str) -> Task<Message> {
    Task::perform(
        async move {
            tokio::time::sleep(std::time::Duration::from_millis(16)).await;
            id
        },
        |id| Message::Focus(id, 0),
    )
}
