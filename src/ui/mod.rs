mod account;
mod action_toasts;
mod backups;
mod bulk;
mod calendar_setup;
mod closing;
mod components;
mod composing;
mod context_menu;
mod conversations;
mod database_import;
mod database_transfers;
mod drag_mail;
mod ellipsis;
mod find_message;
mod folder_controls;
#[cfg(test)]
mod google_lifecycle_tests;
mod html_reader;
mod layout;
mod mail_actions;
mod mail_selection;
mod move_recovery;
mod native_input;
mod notifications;
mod outgoing;
mod pointer;
mod preference_sync;
mod printing;
mod profile_sync;
mod profiles;
mod read_tracking;
mod reading;
#[cfg(test)]
mod reading_tests;
mod refresh;
mod removals;
mod selectable;
mod settings_search;
mod sidebar;
mod tray;
mod views;

use crate::{
    backup::{BackupCopy, BackupTarget},
    engine::{self, Command, Event},
    model::*,
    shortcuts::{Action, Slot},
    store::{PreferenceSnapshot, Workspace},
};
use chrono::{Datelike, NaiveDate, TimeZone};
use components::*;
use iced::{
    Element, Size, Subscription, Task, Theme,
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
    Contacts,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    FolderChange,
    FolderHistory,
    MoveRecovery,
    BulkReview,
    BulkHistory,
    Removal,
    GoogleDisconnect,
    Outbox,
    Account,
    Calendar,
    Move,
    DiscardDraft,
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
    Folders(folder_controls::Message),
    MoveRecovery(move_recovery::Message),
    WindowUnfocused,
    Html(html_reader::Message),
    Find(find_message::Message),
    HtmlScaleRequest(iced::window::Id),
    Backend(Event),
    Bulk(bulk::Message),
    Tick,
    RefreshFrame(Instant),
    Noop,
    Tab(Tab),
    SettingsTab(SettingsTab),
    Open(Dialog),
    Close,
    Query(String),
    SearchReady(u64),
    Folder(String),
    SentFolder,
    Account(Option<String>),
    Filter(MailFilter),
    Sort(MailSort),
    Starred,
    Select(String),
    Hover(String),
    NextPage(bool),
    PreviousMessage(bool),
    Draft(String),
    ToggleDrafts,
    DraftContext(String, iced::Point),
    DraftContextAction(bool),
    ReviewDiscardDraft(String),
    ConfirmDiscardDraft,
    Sync,
    SyncCalendar,
    Move(String),
    MoveFirst,
    Focus(&'static str, u8),
    FocusChecked(&'static str, bool),
    RevealSidebar(String, u8),
    ToggleStar,
    ToggleRead,
    Reply,
    ReplyAll,
    Forward,
    Print(printing::Message),
    ChooseAttachments,
    ChosenAttachments(Draft, Vec<std::path::PathBuf>),
    RemoveDraftAttachment(String),
    NewMessage,
    ShowRecipients,
    ComposeField(&'static str, String),
    CloseComposer,
    ToggleComposer,
    IncludeOriginal(bool),
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
    ReviewRemoval(crate::store::ConnectionRef),
    ConfirmRemoval,
    OpenOutbox,
    OutboxPage(usize),
    SelectOutgoing(String),
    ConfirmOutgoing(bool),
    ResolveOutgoing(crate::outgoing::RecoveryAction),
    CancelPendingTransfers(bool),
    CleanupCredentials,
    RestoreGoogleCalendars,
    DiscoverCalendars,
    ChooseCalendar(String),
    CalendarBack,
    SavePreferences,
    Appearance(Appearance),
    BackupDestination(BackupDestination),
    BackupAccounts(bool),
    AutoBackup(bool),
    GoogleLogin(bool),
    ReviewGoogleDisconnect,
    ConfirmGoogleDisconnect,
    CleanupGoogle,
    Backup,
    ListBackups,
    Restore(String),
    ConfirmRestore,
    Key(Key, keyboard::Modifiers, bool, native_input::Focus),
    Remap(Action, Slot),
    ClearShortcut(Action, Slot),
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
    SidebarResize(f32),
    DismissToast,
    DismissActionToast,
    UndoActions(Vec<u64>),
    DismissUndoErrors(Vec<u64>),
    MailContext(String, iced::Point),
    MailContextAction(context_menu::MailAction),
    DismissContext,
    ReaderSelectionReady(u64, Option<Box<selectable::Content>>),
    ReaderSelection(String, usize, text_editor::Action),
    Dismiss,
    BrowseBackup,
    BrowseExport,
    DatabaseExport,
    DatabaseImport(database_import::Action),
    Profiles(profiles::Action),
    ProfileSync(profile_sync::Action),
    DatabaseExportPath(u64, Option<std::path::PathBuf>),
    CancelDatabaseTransfer,
    ChosenPath(&'static str, Option<String>),
    PaneResize(widget::pane_grid::ResizeEvent),
    SaveLayout(u64),
    InboxScroll(f32),
    DragChanged,
    DragReveal(drag_mail::Reveal),
    DropMail(Arc<drag_mail::Payload>, Option<drag_mail::Target>),
    WindowClose(iced::window::Id),
    SidebarAction(usize),
    SidebarClick(usize, keyboard::Modifiers),
    SelectClick(String, keyboard::Modifiers),
    OpenMessageClick(String, keyboard::Modifiers),
    ToggleSelection,
    CheckMail(String),
    SelectAllMail,
    ClearSelection,
    MailPaneClicked(widget::pane_grid::Pane),
    ToggleInboxExpanded,
    ToggleAccountFolders(String),
    ToggleFolderGroup(String, String),
    Modifiers(keyboard::Modifiers),
    AccountFolder(String, String),
    AccountFolderUnified,
    PrefUnified(bool),
    PrefTooltips(bool),
    PrefUnreadBadge(bool),
    PrefCloseToTray(bool),
    Tray(crate::desktop_tray::Event),
    MainWindowOpened(iced::window::Id),
    WindowCloseRequested(iced::window::Id),
    DesktopBadge(crate::desktop_badge::Event),
    Notification(notifications::Message),
    PrefShortcutTooltips(bool),
    SettingsSearch(String),
    FindSetting(SettingsTab, &'static str),
    ShowAllSettings,
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
    PrefConversations(bool),
    ConversationMessage(String),
    ConversationFlag(String),
    ConversationPage(bool),
    ConversationScroll(u64),
    ConversationViewport(f32),
    RetryConversation,
    PrefImages(ImagePolicy),
    AllowImages(u8),
    ClearImageTrust,
}

pub struct App {
    panes: widget::pane_grid::State<MailPane>,
    reader_split: widget::pane_grid::Split,
    layout_generation: u64,
    pending_preference_save: Option<(u64, crate::preference_edits::Write)>,
    pending_close: Option<iced::window::Id>,
    tray: tray::State,
    database_transfer: database_transfers::State,
    database_import: database_import::State,
    profiles: profiles::State,
    profile_sync: profile_sync::State,
    confirm_save: Option<u64>,
    saved_toast: Option<Instant>,
    action_toasts: action_toasts::ActionToasts,
    printing: printing::State,
    desktop_badge: Option<tokio::sync::watch::Sender<u64>>,
    notifications: notifications::State,
    context_menu: Option<context_menu::Menu>,
    pending_mail_action: Option<(String, context_menu::MailAction)>,
    mail_actions: mail_actions::Actions,
    mail_selection: mail_selection::State,
    bulk: bulk::State,
    list_focus: bool,
    reader_selection: Option<Box<selectable::Content>>,
    folder_controls: folder_controls::State,
    html_reader: html_reader::State,
    find_message: find_message::State,
    remote_bytes: VecDeque<(String, Arc<[u8]>)>,
    pending_reader_selection: Option<Arc<MailDetail>>,
    reader_selection_generation: u64,
    reader_preparation: Option<iced::task::Handle>,
    inbox_scroll: f32,
    mail_drag: drag_mail::Handle,
    last_click: Option<(String, Instant)>,
    pending_focus: Option<&'static str>,
    focused_input: Option<&'static str>,
    #[cfg(feature = "test-support")]
    test_keys: VecDeque<String>,
    #[cfg(feature = "test-support")]
    test_sync_round: u64,
    #[cfg(feature = "test-support")]
    test_account_sync_waiting: bool,
    list_revision: u64,
    last_list_query: MailQuery,
    composer: composing::Composer,
    conversation: conversations::Conversation,
    inbox_expanded: bool,
    sidebar_focus: bool,
    sidebar_index: usize,
    modifiers: keyboard::Modifiers,
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
    pending_google_login: Option<(u64, Preferences, bool)>,
    pending_backup: Option<backups::PendingBackup>,
    tab: Tab,
    settings_tab: SettingsTab,
    settings_search: String,
    settings_group: Option<&'static str>,
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
    events_revision: u64,
    month: NaiveDate,
    day: NaiveDate,
    editing_event: Option<CalendarEvent>,
    calendar_setup: calendar_setup::CalendarSetup,
    removal: removals::Removal,
    outbox: outgoing::Outbox,
    move_recovery: move_recovery::Recovery,
    remapping: Option<(Action, Slot)>,
    busy: HashSet<String>,
    refresh: refresh::Animation,
    notice: Option<(String, bool, Instant)>,
    sync_notice: Option<Instant>,
    preference_notice: Option<Instant>,
    google_connected: bool,
    google_disconnect_pending: Option<u64>,
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
    #[cfg(feature = "test-support")]
    initial_page_loaded: bool,
    #[cfg(feature = "test-support")]
    idle_navigation: bool,
    test_revision: u64,
}

pub fn run() -> iced::Result {
    iced::daemon(App::boot, App::update, App::window_view)
        .title("Shep — Mail & Calendar")
        .theme(|app: &App, _| app.theme())
        .scale_factor(|app: &App, _| app.preferences.interface_scale as f32 / 100.)
        .subscription(App::subscription)
        .default_font(iced::Font::with_name("Noto Sans"))
        .font(include_bytes!("../../assets/NotoSans-Regular.ttf").as_slice())
        .font(include_bytes!("../../assets/NotoSans-SemiBold.ttf").as_slice())
        .run()
}
impl App {
    fn window_view(&self, _window: iced::window::Id) -> Element<'_, Message> {
        self.view()
    }

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
                pending_preference_save: None,
                pending_close: None,
                tray: Default::default(),
                database_transfer: Default::default(),
                database_import: Default::default(),
                profiles: Default::default(),
                profile_sync: Default::default(),
                confirm_save: None,
                saved_toast: None,
                action_toasts: Default::default(),
                printing: Default::default(),
                desktop_badge: None,
                notifications: Default::default(),
                context_menu: None,
                pending_mail_action: None,
                mail_actions: Default::default(),
                mail_selection: Default::default(),
                bulk: Default::default(),
                list_focus: true,
                reader_selection: None,
                folder_controls: Default::default(),
                html_reader: Default::default(),
                find_message: Default::default(),
                remote_bytes: VecDeque::new(),
                pending_reader_selection: None,
                reader_selection_generation: 0,
                reader_preparation: None,
                inbox_scroll: 0.,
                mail_drag: Default::default(),
                last_click: None,
                pending_focus: None,
                focused_input: None,
                #[cfg(feature = "test-support")]
                test_keys: VecDeque::new(),
                #[cfg(feature = "test-support")]
                test_sync_round: 0,
                #[cfg(feature = "test-support")]
                test_account_sync_waiting: false,
                list_revision: 0,
                last_list_query: MailQuery::default(),
                composer: Default::default(),
                conversation: Default::default(),
                inbox_expanded: false,
                sidebar_focus: false,
                sidebar_index: 0,
                modifiers: keyboard::Modifiers::default(),
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
                settings_search: String::new(),
                settings_group: None,
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
                events_revision: 0,
                month: today.with_day(1).unwrap(),
                day: today,
                editing_event: None,
                calendar_setup: Default::default(),
                removal: Default::default(),
                outbox: Default::default(),
                move_recovery: Default::default(),
                remapping: None,
                busy: HashSet::new(),
                refresh: Default::default(),
                notice: None,
                sync_notice: None,
                preference_notice: None,
                google_connected: false,
                google_disconnect_pending: None,
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
                #[cfg(feature = "test-support")]
                initial_page_loaded: false,
                #[cfg(feature = "test-support")]
                idle_navigation: demo && args.iter().any(|a| a == "--idle-navigation"),
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
        let tick = iced::time::every(std::time::Duration::from_secs(1)).map(|_| Message::Tick);
        #[cfg(feature = "test-support")]
        let tick = if self.idle_navigation {
            Subscription::none()
        } else {
            tick
        };
        Subscription::batch([
            Subscription::run_with(self.demo, engine::subscription).map(Message::Backend),
            Subscription::run(crate::desktop_badge::subscription).map(Message::DesktopBadge),
            Subscription::run_with(self.demo, crate::desktop_tray::subscription).map(Message::Tray),
            Subscription::run_with(self.demo, crate::notifications::subscription)
                .map(|event| Message::Notification(notifications::Message::Backend(event))),
            Subscription::run(crate::html_render::subscription)
                .map(|e| Message::Html(html_reader::Message::Backend(e))),
            Subscription::run(crate::html_render::preparation::subscription)
                .map(|e| Message::Html(html_reader::Message::Prepared(e))),
            tick,
            if self.refresh_animating() {
                iced::time::every(std::time::Duration::from_millis(16)).map(Message::RefreshFrame)
            } else {
                Subscription::none()
            },
            iced::system::theme_changes().map(Message::SystemTheme),
            iced::event::listen_with(|e, _status, id| match e {
                iced::Event::Window(iced::window::Event::CloseRequested) => {
                    Some(Message::WindowCloseRequested(id))
                }
                iced::Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                    Some(Message::Modifiers(modifiers))
                }
                iced::Event::Window(iced::window::Event::Unfocused) => {
                    Some(Message::WindowUnfocused)
                }
                iced::Event::Window(
                    iced::window::Event::Opened { .. } | iced::window::Event::Moved(_),
                ) => Some(Message::HtmlScaleRequest(id)),
                iced::Event::Window(iced::window::Event::Resized(size)) => {
                    Some(Message::Resize(size))
                }
                _ => None,
            }),
        ])
    }
    fn save_preferences(&mut self) {
        let request = self.preference_sync.changed();
        self.persist_preferences(request, self.preferences.clone());
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
        // Admission already owns a durable write, even before a provider slot
        // starts it. Close must not race the worker's later Busy notification.
        let close_key = match &command {
            Command::SaveAccount(..)
            | Command::SaveEvent(..)
            | Command::DeleteEvent(..)
            | Command::GoogleLogin(..) => command.key(),
            _ => None,
        };
        if close_key
            .as_ref()
            .is_some_and(|key| self.busy.contains(key))
        {
            return false;
        }
        let preferences_request = if let Command::SavePreferences(request, _) = &command {
            Some(*request)
        } else {
            None
        };
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
        if let Some(key) = close_key {
            self.busy.insert(key);
        }
        if preferences_request.is_some_and(|request| {
            self.pending_preference_save
                .as_ref()
                .is_some_and(|(pending, _)| request >= *pending)
        }) {
            self.pending_preference_save = None;
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
        self.reconcile_selection_scope();
        if self.last_list_query != self.query {
            self.mail_actions.follow = None;
            self.inbox_scroll = 0.;
            self.list_revision += 1;
            self.last_list_query = self.query.clone();
        }
        self.generation += 1;
        self.prefetch_page = None;
        self.prefetch_query = None;
        let mut query = self.query.clone();
        query.project_moves = self.mail_actions.projected_moves();
        query.observe = self.mail_actions.observed_ids();
        if let Some(id) = &self.selected
            && self.page.move_recovery.contains_key(id)
            && !query.observe.contains(id)
        {
            query.observe.push(id.clone());
        }
        query.observe_bulk = self.bulk_observed_ids();
        self.send(Command::Query(self.generation, query, false));
    }
    fn cache_detail(&mut self, detail: Arc<MailDetail>) {
        self.mail_actions.observe_detail(&detail.summary);
        self.detail_cache
            .retain(|d| d.summary.id != detail.summary.id);
        self.detail_cache.push_front(detail);
        while self.detail_cache.len() > 8
            || self
                .detail_cache
                .iter()
                .map(|d| {
                    d.body.len()
                        + d.html.as_ref().map_or(0, |html| html.bytes)
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
    fn cached_detail(&mut self, id: &str) -> Option<Arc<MailDetail>> {
        let index = self
            .detail_cache
            .iter()
            .position(|detail| detail.summary.id == id)?;
        let detail = self.detail_cache.remove(index)?;
        self.detail_cache.push_front(detail.clone());
        Some(detail)
    }
    fn preload(&mut self, id: String) {
        if self.page.is_transient_placeholder(&id) {
            return;
        }
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
        self.mail_actions.follow = None;
        self.pending_mail_action = None;
        if self.selected.as_deref() != Some(&id) {
            self.park_composer();
            self.composer.dismissed_for = None;
            self.expanded_replies.clear();
            self.conversation.page = Default::default();
        }
        self.selected = Some(id.clone());
        if self.page.is_transient_placeholder(&id) {
            self.bulk.waiting_reader = Some(id.clone());
            self.conversation = Default::default();
            self.detail = self.cached_placeholder_detail(&id);
            return;
        }
        self.bulk.waiting_reader = None;
        if self.mail_actions.restoring(&id) {
            self.conversation = Default::default();
            self.detail = self
                .detail_cache
                .iter()
                .find(|d| d.summary.id == id)
                .cloned();
            return;
        }
        self.focus_conversation_message(id.clone());
        self.request_conversation(None);
        if let Some(i) = self.page.rows.iter().position(|m| m.id == id) {
            if let Some(next) = self.page.rows.get(i + 1) {
                self.preload(next.id.clone());
            }
            if i > 0 {
                self.preload(self.page.rows[i - 1].id.clone());
            }
        }
        self.load_remote_images();
        self.restore_reply();
    }
    fn open(&mut self, dialog: Dialog) {
        self.pending_focus = None;
        self.focused_input = None;
        self.dialog = Some(dialog);
        self.fields.clear();
        self.remapping = None;
        match dialog {
            Dialog::Calendar => self.calendar_setup.invalidate(),
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
            Dialog::Event => {
                self.editing_event = None;
                self.fields.insert("date", self.day.to_string());
                self.fields.insert("end_date", self.day.to_string());
                self.fields
                    .insert("event_id", uuid::Uuid::new_v4().to_string());
                self.fields.insert("all_day", "true".into());
                self.fields.insert("start", "09:00".into());
                self.fields.insert("end", "10:00".into());
                if let Some(source) = self.workspace.calendars.iter().find(|s| s.access.create) {
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
        if matches!(message, Message::RefreshFrame(_)) {
            // Animation only changes one SVG. Do not pump mail work, prepare
            // bodies, or mix frame ticks into interaction-handler measurements.
            let task = self.handle(message);
            return if self.test_state.is_some() {
                Task::batch([task, self.write_test_state()])
            } else {
                task
            };
        }
        let start = Instant::now();
        let hidden_closing =
            self.tray.hidden && (self.pending_close.is_some() || self.composer.close.is_some());
        let hidden_write = self.tray.hidden && !hidden_closing && self.has_required_close_work();
        let previous_notice = self.notice.as_ref().map(|(_, _, at)| *at);
        let task = self.handle(message);
        self.pump_bulk();
        let close = self.continue_pending_close();
        let reopen = self.reopen_after_failed_close(
            hidden_write && self.new_error_since(previous_notice),
            hidden_closing,
        );
        let tray_close = self.continue_tray_close();
        self.update_desktop_badge();
        self.update_notification_settings();
        let task = Task::batch([
            task,
            close,
            reopen,
            tray_close,
            self.prepare_reader_selection(),
            self.prepare_html(),
            self.prepare_find(),
        ]);
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
    fn unread_badge_count(&self) -> u64 {
        if !self.preferences.unread_badge {
            return 0;
        }
        self.workspace.accounts.iter().fold(0u64, |count, account| {
            count.saturating_add(
                self.page
                    .inbox_unread
                    .get(&account.id)
                    .copied()
                    .unwrap_or(0) as u64,
            )
        })
    }
    fn update_desktop_badge(&self) {
        if let Some(sender) = &self.desktop_badge {
            let count = self.unread_badge_count();
            sender.send_if_modified(|current| {
                if *current == count {
                    false
                } else {
                    *current = count;
                    true
                }
            });
        }
    }
    fn handle(&mut self, message: Message) -> Task<Message> {
        if !self.read_navigation(&message) {
            return Task::none();
        }
        match message {
            Message::Folders(message) => {
                let focus = matches!(
                    message,
                    folder_controls::Message::Choose(0) | folder_controls::Message::Back
                );
                self.handle_folders(message);
                if focus && self.dialog == Some(Dialog::FolderChange) {
                    return focus_after_layout("folder-parent-search");
                }
            }
            Message::Notification(message) => self.handle_notification(message),
            Message::DesktopBadge(crate::desktop_badge::Event::Ready(sender)) => {
                self.desktop_badge = Some(sender);
            }
            Message::Tray(event) => return self.tray_event(event),
            Message::MainWindowOpened(window) => return self.main_window_opened(window),
            Message::WindowCloseRequested(window) => return self.request_main_close(window),
            Message::PrefCloseToTray(value) => {
                self.preferences.close_to_tray = value;
                self.save_preferences();
            }
            Message::PrefUnreadBadge(value) => {
                self.preferences.unread_badge = value;
                self.save_preferences();
            }
            Message::HtmlScaleRequest(id) => {
                return iced::window::scale_factor(id)
                    .map(|s| Message::Html(html_reader::Message::Scale(s)));
            }
            Message::Bulk(message) => return self.handle_bulk(message),
            Message::DropMail(payload, target) => self.drop_mail(payload, target),
            Message::DragReveal(reveal) => self.reveal_drag_folders(reveal),
            Message::DragChanged => {
                self.last_click = None;
            }
            Message::Html(message) => return self.handle_html(message),
            Message::Find(message) => return self.handle_find(message),
            Message::WindowUnfocused => {
                self.modifiers = keyboard::Modifiers::default();
                self.mail_drag.clear();
            }
            Message::Noop => return Task::none(),
            Message::DismissContext => {
                self.folder_controls.menu = None;
                self.context_menu = None;
                self.composer.context = None;
            }
            Message::ToggleDrafts => {
                self.preferences.collapsed_drafts = !self.preferences.collapsed_drafts;
                self.save_preferences();
            }
            Message::DraftContext(id, position) => {
                if self.dialog.is_none()
                    && self.draft_labels().iter().any(|(draft, _)| *draft == id)
                    && !self.workspace.outgoing_drafts.contains(&id)
                {
                    self.context_menu = None;
                    self.composer.context = Some(composing::DraftMenu {
                        id,
                        position,
                        discard: false,
                    });
                }
            }
            Message::DraftContextAction(discard) => {
                if let Some(menu) = self.composer.context.take() {
                    return self.handle(if discard {
                        Message::ReviewDiscardDraft(menu.id)
                    } else {
                        Message::Draft(menu.id)
                    });
                }
            }
            Message::ReviewDiscardDraft(id) => self.review_discard_draft(id),
            Message::ConfirmDiscardDraft => self.confirm_discard_draft(),
            Message::ReaderSelectionReady(generation, content) => {
                if generation == self.reader_selection_generation {
                    self.reader_preparation = None;
                    self.pending_reader_selection = None;
                    if let Some(content) = content
                        && self.detail.as_ref().is_some_and(|d| content.same_text(d))
                    {
                        self.reader_selection = Some(content);
                    }
                }
            }
            Message::ReaderSelection(id, index, action) => {
                if self.reader_id() == Some(&id)
                    && let Some(content) = &mut self.reader_selection
                {
                    content.apply(&id, index, action);
                }
            }
            Message::MailContext(id, position) => {
                if self.dialog.is_none()
                    && let Some(mail) = self.page.rows.iter().find(|m| m.id == id).cloned()
                {
                    self.context_menu = Some(context_menu::Menu {
                        mail,
                        position,
                        index: 0,
                    });
                    self.sidebar_focus = false;
                    self.select(id);
                }
            }
            Message::MailContextAction(action) => return self.choose_mail_context(action),
            Message::Backend(event) => match event {
                Event::Folder(event) => self.folder_event(event),
                Event::Ready(tx, workspace, google) => {
                    self.tx = Some(tx);
                    self.send(Command::BulkJobs(0, 0));
                    self.send(Command::Folder(engine::folders::Request::History(0, 0)));
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
                    if let Some(size) = self.preferences.window_size {
                        return iced::window::oldest().then(move |window| {
                            window.map_or_else(Task::none, |id| {
                                iced::window::resize(id, Size::new(size.width, size.height))
                            })
                        });
                    }
                }
                Event::Workspace(workspace) => {
                    self.preference_sync.observe(
                        PreferenceSnapshot {
                            revision: workspace.preferences_revision,
                            value: workspace.preferences.clone(),
                        },
                        &mut self.preferences,
                    );
                    if self.preferences.google_lifecycle.disconnected {
                        self.google_connected = false;
                        self.pending_google_login = None;
                    }
                    let mut workspace = (*workspace).clone();
                    if workspace.connections_revision < self.workspace.connections_revision {
                        workspace.accounts = self.workspace.accounts.clone();
                        workspace.account_reconnect = self.workspace.account_reconnect.clone();
                        workspace.calendars = self.workspace.calendars.clone();
                        workspace.account_folders = self.workspace.account_folders.clone();
                        workspace.folder_trees = self.workspace.folder_trees.clone();
                        workspace.folders = self.workspace.folders.clone();
                        workspace.connections_revision = self.workspace.connections_revision;
                        workspace.credential_cleanup = self.workspace.credential_cleanup;
                        workspace.google_archived = self.workspace.google_archived.clone();
                        workspace.removed_google_calendars =
                            self.workspace.removed_google_calendars;
                    }

                    if workspace.outgoing_revision < self.workspace.outgoing_revision {
                        workspace.outgoing_revision = self.workspace.outgoing_revision;
                        workspace.outgoing_pending = self.workspace.outgoing_pending;
                        workspace.outgoing_drafts = self.workspace.outgoing_drafts.clone();
                    }
                    if workspace.drafts_revision < self.workspace.drafts_revision {
                        workspace.drafts = self.workspace.drafts.clone();
                        workspace.drafts_revision = self.workspace.drafts_revision;
                    }
                    self.workspace = Arc::new(workspace);
                    self.reconcile_folder_accounts();
                    if let Some(folders) = &mut self.query.folders {
                        let before = folders.len();
                        folders.retain(|s| {
                            s.account.as_ref().is_none_or(|id| {
                                self.workspace.accounts.iter().any(|a| &a.id == id)
                            })
                        });
                        if folders.len() != before {
                            self.query.offset = 0;
                            self.selected = None;
                            self.detail = None;
                            self.request_page();
                        }
                    }
                    if self
                        .query
                        .account
                        .as_ref()
                        .is_some_and(|id| !self.workspace.accounts.iter().any(|a| &a.id == id))
                    {
                        self.query.account = None;
                        self.selected = None;
                        self.detail = None;
                        self.query.offset = 0;
                        self.request_page();
                    }
                    if !self.preferences.unified_inbox
                        && self.query.account.is_none()
                        && self.query.folders.is_none()
                    {
                        self.query.account = self.workspace.accounts.first().map(|a| a.id.clone());
                        self.request_page();
                    }
                    self.observe_draft_files();
                    self.update_saved_preferences();
                }
                Event::Conversation(generation, anchor, result) => {
                    return self.conversation_result(generation, anchor, result);
                }
                Event::ConversationsIndexed(result) => match result {
                    Ok(()) => self.request_conversation(None),
                    Err(error) => self.notice(
                        format!(
                            "Related mail could not be indexed: {error}. Reopen Shep to retry."
                        ),
                        true,
                    ),
                },
                Event::PreferencesSaveFailed(request, error) => {
                    self.database_preferences_failed(request, &error);
                    if self
                        .pending_google_login
                        .as_ref()
                        .is_some_and(|(id, _, _)| *id == request)
                    {
                        self.pending_google_login = None;
                    }
                    self.cancel_backup_save(request);
                    if self.confirm_save == Some(request) {
                        self.confirm_save = None;
                    }
                    self.pending_close = None;
                    self.notice(
                        format!("Changes could not be saved: {error}. Try Save changes again."),
                        true,
                    );
                    self.preference_notice = self.notice.as_ref().map(|notice| notice.2);
                }
                Event::PreferencesSaved(request, snapshot) => {
                    self.database_preferences_saved(request);
                    self.preference_sync.acknowledge(
                        request,
                        (*snapshot).clone(),
                        &mut self.preferences,
                    );
                    self.update_saved_preferences();
                    if !self.preference_sync.dirty() {
                        if self.confirm_save.is_some_and(|id| request >= id) {
                            self.confirm_save = None;
                            self.saved_toast = Some(Instant::now());
                            if self.preference_notice.take().is_some_and(|at| {
                                self.notice.as_ref().is_some_and(|notice| notice.2 == at)
                            }) {
                                self.notice = None;
                            }
                        }
                        if let Some(window) = self.pending_close.take() {
                            return self.handle(Message::WindowClose(window));
                        }
                    }
                    self.continue_backup_request(request);
                    if self
                        .pending_google_login
                        .as_ref()
                        .is_some_and(|(id, _, _)| *id == request)
                        && let Some((_, prefs, retry)) = self.pending_google_login.take()
                    {
                        if prefs.google_client_id == self.preferences.google_client_id
                            && prefs.google_client_secret == self.preferences.google_client_secret
                        {
                            self.send(Command::GoogleLogin(prefs, retry));
                        } else {
                            self.notice("Google client details changed. Connect Google again with the updated details.", true);
                        }
                    }
                }
                Event::BulkResumed(id) => self.send(Command::BulkRun(id)),
                Event::BulkStopped => {
                    self.bulk.stopped = true;
                    if let Some(window) = self.pending_close.take() {
                        return self.handle(Message::WindowClose(window));
                    }
                }
                event @ (Event::BulkReview(..)
                | Event::BulkStarted(..)
                | Event::BulkUpdate(..)
                | Event::BulkIdentity(..)
                | Event::BulkFinished(..)
                | Event::BulkJobs(..)
                | Event::BulkItems(..)) => self.bulk_event(event),
                Event::Selection(serial, result) => self.selection_finished(serial, result),
                Event::Page(g, page, prefetch) if g == self.generation => {
                    if prefetch {
                        if let Some(query) = self.prefetch_query.take() {
                            self.prefetch_page = Some((query, page));
                        }
                    } else {
                        #[cfg(feature = "test-support")]
                        {
                            self.initial_page_loaded = true;
                        }
                        self.set_mail_page(page);
                        let reveal_after_removal = self.finish_removal_selection();
                        if let Some(id) = self.bulk.waiting_reader.clone()
                            && !self.page.is_transient_placeholder(&id)
                        {
                            self.bulk.waiting_reader = None;
                            if self.selected.as_ref() == Some(&id)
                                && self.page.rows.iter().any(|m| m.id == id)
                            {
                                self.select(id);
                            }
                        }
                        if let Some(menu) = &mut self.context_menu
                            && let Some(current) =
                                self.page.rows.iter().find(|mail| mail.id == menu.mail.id)
                        {
                            menu.mail = current.clone();
                        }
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
                            query.project_moves = self.mail_actions.projected_moves();
                            query.observe = self.mail_actions.observed_ids();
                            if let Some(id) = &self.selected
                                && self.page.move_recovery.contains_key(id)
                                && !query.observe.contains(id)
                            {
                                query.observe.push(id.clone());
                            }
                            query.observe_bulk = self.bulk_observed_ids();
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
                        if reveal_after_removal {
                            return self.reveal_selected_mail();
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
                            if self.reader_id() == Some(&id) {
                                let mut visible = detail.clone();
                                if self.page.is_placeholder(&id)
                                    && let Some(mail) = self.page.rows.iter().find(|m| m.id == id)
                                {
                                    Arc::make_mut(&mut visible).summary = mail.clone();
                                }
                                self.detail = Some(visible);
                            }
                            if !prefetch || self.detail_cache.len() < 8 {
                                self.cache_detail(detail);
                            }
                            self.load_remote_images();
                            self.restore_reply();
                            if self.pending_mail_action.is_some() {
                                return self.finish_mail_context();
                            }
                        }
                        Err(error) if !prefetch && self.reader_id() == Some(&id) => {
                            self.pending_mail_action = None;
                            self.notice(format!("Could not load this message: {error}"), true)
                        }
                        Err(_) => {}
                    }
                }
                Event::MailSyncFinished(result) => match result {
                    Ok(()) => {
                        if self.sync_notice.is_some_and(|at| {
                            self.notice.as_ref().is_some_and(|notice| notice.2 == at)
                        }) {
                            self.notice = None;
                        }
                        self.sync_notice = None;
                    }
                    Err(error) => {
                        self.notice(error, true);
                        self.sync_notice = self.notice.as_ref().map(|notice| notice.2);
                    }
                },
                Event::MailArrived(arrival) => self.notification_arrived(arrival),
                Event::MoveRecovered(record) => self.move_recovered(&record),
                Event::MoveRecoveries(request, result) => {
                    self.move_recoveries_loaded(request, result)
                }
                Event::MailMoveRecovery(request, token, result) => {
                    return self.move_recovery_finished(request, token, result);
                }
                Event::UndoFinished(request, mail, result) => {
                    return self.undo_finished(request, mail, result);
                }
                Event::TransferFinished(request, mail, result) => {
                    return self.transfer_receipt(request, mail, result);
                }
                Event::MoveFinished(request, mail, folder, result) => {
                    return self.move_receipt(request, mail, folder, result);
                }
                Event::FlagsFinished(request, mail, result) => {
                    return self.flags_finished(request, mail, result);
                }
                #[cfg(feature = "test-support")]
                Event::PreviewSync(round) => self.test_sync_round = round,
                #[cfg(feature = "test-support")]
                Event::PreviewAccountSync(waiting) => self.test_account_sync_waiting = waiting,
                Event::Changed => {
                    self.detail_revision += 1;
                    self.prefetch_page = None;
                    self.detail_cache.retain(|d| {
                        self.mail_actions.moving(&d.summary.id)
                            || self.page.is_placeholder(&d.summary.id)
                    });
                    self.pending_details.clear();
                    self.request_page();
                    self.request_conversation(None);
                    if let Some(id) = self
                        .reader_id()
                        .filter(|id| {
                            !self.mail_actions.restoring(id)
                                && !self.page.is_transient_placeholder(id)
                        })
                        .map(str::to_owned)
                    {
                        self.send(Command::Detail {
                            revision: self.detail_revision,
                            id,
                            prefetch: false,
                        });
                    }
                }
                Event::Calendar(revision, events) => {
                    if revision >= self.events_revision {
                        self.events_revision = revision;
                        self.events = events;
                    }
                }
                Event::Busy(key, busy) => {
                    if key == "sync" {
                        if busy {
                            self.refresh.start(Instant::now());
                        } else {
                            self.refresh.stop();
                        }
                    }
                    if busy {
                        self.busy.insert(key);
                    } else {
                        self.busy.remove(&key);
                    }
                }
                Event::Notice(text) => self.notice(text, false),
                Event::Error(text) => {
                    self.pending_close = None;
                    self.composer.close = None;
                    self.notice(text, true);
                    self.pending_details.clear();
                }
                Event::GoogleStatus(revision, connected) => {
                    if revision == self.preferences.google_lifecycle.revision {
                        self.google_connected =
                            connected && !self.preferences.google_lifecycle.disconnected;
                        self.profile_google_status(revision, self.google_connected);
                        self.shared_profile_action(profile_sync::Action::Refresh);
                    }
                }
                Event::GoogleDisconnected(revision, result) => {
                    self.shared_profile_action(profile_sync::Action::Refresh);
                    if self.google_disconnect_pending == Some(revision) {
                        self.google_disconnect_pending = None;
                        match result {
                            Ok(()) if self.dialog == Some(Dialog::GoogleDisconnect) => {
                                self.dialog = None;
                                self.settings_fields();
                            }
                            Ok(()) => {}
                            Err(error) => self.notice(error, true),
                        }
                    }
                }
                Event::AccountSaved(id) => {
                    if self.dialog == Some(Dialog::Account) && self.field("id") == id {
                        self.dialog = None;
                        self.fields.clear();
                        if self.tab == Tab::Preferences {
                            self.settings_fields();
                        }
                    }
                    self.send(Command::Sync);
                }
                Event::RemovalPreview(request, result) => self.removal_preview(request, result),
                Event::ConnectionRemoved(request, result) => {
                    self.connection_removed(request, result)
                }
                Event::CalendarsDiscovered(request, result) => {
                    self.calendars_discovered(request, result)
                }
                Event::CalendarsConnected(request, result) => {
                    self.calendars_connected(request, result)
                }
                Event::DraftSaved(id, revision, result) => {
                    return self.draft_saved(id, revision, result);
                }
                Event::Database(request, update) => {
                    return match update {
                        update @ (crate::transfer::Update::Progress(_)
                        | crate::transfer::Update::Finished(_)) => {
                            self.database_update(request, update)
                        }
                        update => self.database_import_update(request, update),
                    };
                }
                Event::Profiles(request, result) => return self.profiles_update(request, result),
                Event::ProfileSync(request, update) => {
                    return self.shared_profile_update(request, update);
                }
                Event::DraftDeleted(id, result) => self.draft_deleted(id, result),
                Event::ForwardDraft(id, result) => return self.forward_ready(id, result),
                Event::Print(revision, result) => return self.print_ready(revision, result),
                Event::DraftFiles(id, result) => {
                    let current = self.composer.io.as_deref() == Some(&id);
                    if current {
                        self.composer.io = None;
                    }
                    match result {
                        Ok(state) => self.observe_drafts(&state),
                        Err(error) => {
                            if current {
                                self.pending_close = None;
                                self.composer.close = None;
                            }
                            self.fail_removal_draft_wait(&id, &error);
                            self.fail_database_preparation(&error);
                            self.notice(error, true);
                        }
                    }
                    self.continue_removal_review();
                }
                Event::OutgoingPage(request, result) => self.outgoing_page(request, result),
                Event::OutgoingChanged => {
                    if self.dialog == Some(Dialog::Outbox) {
                        self.load_outbox(self.outbox.page.offset);
                    }
                }
                Event::ReviewOutgoing(id, revision) => {
                    if self.compose_visible()
                        && self.dialog.is_none()
                        && self.composer.current.draft.id == id
                        && self.composer.current.draft.revision == revision
                    {
                        self.open_outbox();
                    } else {
                        self.notice("An outgoing message needs review in Outbox.", true);
                    }
                }
                Event::SubmissionQueued(id, revision) | Event::Sent(id, revision) => {
                    self.retire_draft(&id, Some(revision));
                }
                Event::RemoteImage(url, result) => {
                    self.requested_images.remove(&url);
                    match result {
                        Ok(bytes) => {
                            self.html_reader.cache.image_arrived(&url);
                            self.remote_bytes.retain(|(u, _)| u != &url);
                            self.remote_bytes
                                .push_back((url.clone(), Arc::from(bytes.clone())));
                            while self.remote_bytes.len() > 128
                                || self
                                    .remote_bytes
                                    .iter()
                                    .map(|(_, bytes)| bytes.len())
                                    .sum::<usize>()
                                    > 16 * 1024 * 1024
                            {
                                self.remote_bytes.pop_front();
                            }
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
                self.pending_close = Some(window);
                if self.profile_sync.pending() {
                    self.pending_close = Some(window);
                    self.shared_profile_action(profile_sync::Action::Stop);
                    return Task::none();
                }
                if self.profiles.changing() {
                    self.pending_close = Some(window);
                    return Task::none();
                }
                if self.database_import.pending() {
                    let _ = self.database_import_action(database_import::Action::Cancel);
                    if self.database_import.pending() {
                        self.pending_close = Some(window);
                        return Task::none();
                    }
                }
                if self.database_transfer.pending.is_some() {
                    self.cancel_database_transfer();
                    if self.database_transfer.pending.is_some() {
                        self.pending_close = Some(window);
                        return Task::none();
                    }
                }
                self.mail_drag.clear();
                if self.flush_pane_resize() {
                    self.save_preferences();
                }
                if self.dialog == Some(Dialog::DiscardDraft) && !self.composer.discard_pending {
                    self.cancel_discard_draft();
                }
                if self.folder_staging()
                    || self.bulk.staging.is_some()
                    || self.tx.is_some() && !self.bulk.stopped
                {
                    self.pending_close = Some(window);
                    if self.bulk.staging.is_some()
                        || self.bulk.jobs.iter().any(|j| j.remaining > 0)
                        || self.folder_controls.jobs.iter().any(|j| !j.closed)
                        || self.folder_staging()
                    {
                        self.notice(
                        "Finishing the current mail change. Queued changes will resume next time.",
                        false,
                    );
                    }
                } else if self.mail_actions.pending() > 0 || !self.move_recovery.pending.is_empty()
                {
                    self.pending_close = Some(window);
                    self.notice("Finishing your mail changes before closing…", false);
                } else if self.removal.removing.is_some()
                    || self.busy.contains("credential-cleanup")
                {
                    self.notice("Finishing credential cleanup before closing…", false);
                } else if self.busy.contains("google-disconnect") || self.busy.contains("google") {
                    self.notice(
                        "Finishing the Google connection change before closing…",
                        false,
                    );
                } else if self.busy.iter().any(|key| key.starts_with("outgoing:")) {
                    self.notice("Finishing Sent-copy recovery before closing…", false);
                } else if self.calendar_setup.saving.is_some() {
                    self.notice("Saving the calendar connection before closing…", false);
                } else if self.busy.iter().any(|key| {
                    key.starts_with("send:")
                        || key.starts_with("event:")
                        || key.starts_with("account:")
                }) {
                    self.notice("Finishing your changes before closing…", false);
                } else if self.composer.discard_pending {
                    self.notice("Finishing the draft change before closing…", false);
                } else if self.composer.forward_pending.is_some() {
                    self.notice(
                        "Wait for the forward to finish preparing before closing.",
                        false,
                    );
                } else if self.composer.io.is_some() {
                    self.notice("Saving the selected attachments before closing…", false);
                } else {
                    self.flush_pane_resize();
                    if self.preference_sync.dirty() {
                        self.pending_close = Some(window);
                        self.save_preferences();
                    } else if !self.defer_draft_exit(composing::Exit::Window(window)) {
                        self.pending_close = None;
                        return self.finish_exit();
                    }
                }
            }
            Message::Tick => {
                self.advance_profile_login();
                self.advance_profile_cycle();
                self.advance_database_import();
                self.advance_database_transfer();
                self.pump_selection();
                self.action_toasts.expire(Instant::now());
                self.prune_undos();
                self.dispatch_undos();
                if let Some((request, prefs)) = self.pending_preference_save.take()
                    && !self.try_command(Command::SavePreferences(request, prefs.clone()))
                {
                    self.pending_preference_save = Some((request, prefs));
                }
                if self.saved_toast.is_some_and(|t| t.elapsed().as_secs() >= 4) {
                    self.saved_toast = None;
                }
                self.autosave_draft();
                self.continue_removal_review();
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
                if tab == Tab::Mail && self.tab == Tab::Mail {
                    self.close_composer();
                    self.query.account = if self.preferences.unified_inbox {
                        None
                    } else {
                        self.workspace.accounts.first().map(|a| a.id.clone())
                    };
                    self.query = MailQuery {
                        account: self.query.account.take(),
                        sort: self.preferences.mail_sort,
                        ..Default::default()
                    };
                    self.full_reader = false;
                    self.open_mail_folder("INBOX".into(), false);
                    self.sidebar_index = self
                        .sidebar_items()
                        .iter()
                        .position(|item| item.active)
                        .unwrap_or(0);
                    self.sidebar_focus = false;
                    self.list_focus = true;
                }
                self.tab = tab;
                self.dialog = None;
                if tab == Tab::Mail
                    && let Some(draft) = self
                        .composer
                        .resume
                        .take()
                        .and_then(|id| self.owned_draft(&id))
                {
                    self.load_draft(draft);
                }
                if tab == Tab::Preferences {
                    self.fields.clear();
                    self.settings_fields();
                }
            }
            Message::SettingsSearch(query) => {
                self.settings_search = query;
                self.settings_group = None;
            }
            Message::FindSetting(tab, group) => {
                if group == "Profiles" {
                    self.profile_action(profiles::Action::Refresh);
                }
                let task = self.handle(Message::SettingsTab(tab));
                self.settings_group = Some(group);
                return task;
            }
            Message::ShowAllSettings => {
                self.settings_group = None;
                self.settings_search.clear();
            }
            Message::PrefTooltips(value) => {
                self.preferences.tooltips = value;
                self.save_preferences();
            }
            Message::PrefShortcutTooltips(value) => {
                self.preferences.shortcut_tooltips = value;
                self.save_preferences();
            }
            Message::SettingsTab(tab) => {
                if tab == SettingsTab::Accounts {
                    self.shared_profile_action(profile_sync::Action::Refresh);
                }
                self.defer_draft_exit(composing::Exit::Tab(Tab::Preferences));
                self.settings_search.clear();
                self.settings_group = None;
                self.tab = Tab::Preferences;
                self.settings_tab = tab;
                self.fields.clear();
                self.settings_fields();
            }
            Message::NewMessage => {
                self.new_composer();
                return focus_after_layout("to");
            }
            Message::Open(dialog) => {
                self.open(dialog);
                if dialog == Dialog::Move {
                    return focus_after_layout("folder-search");
                }
            }
            Message::Close => {
                if self.dialog == Some(Dialog::BulkHistory) {
                    let _ = self.handle_bulk(bulk::Message::CancelResolution);
                }
                if self.dialog == Some(Dialog::BulkReview) {
                    self.cancel_bulk_review();
                    return widget::operation::focus("unfocused");
                }
                if self.dialog == Some(Dialog::DiscardDraft) {
                    self.cancel_discard_draft();
                    return Task::none();
                }
                if matches!(
                    self.dialog,
                    Some(Dialog::Calendar | Dialog::Removal | Dialog::GoogleDisconnect)
                ) {
                    self.calendar_setup.invalidate();
                    self.fields.clear();
                    if self.tab == Tab::Preferences {
                        self.settings_fields();
                    }
                }
                self.pending_focus = None;
                self.focused_input = None;
                if self.dialog.is_none() && self.compose_visible() {
                    self.close_composer();
                }
                self.dialog = None;
                self.remapping = None;
                return widget::operation::focus("unfocused");
            }
            Message::Query(query) => {
                self.close_composer();
                if query.trim().is_empty() {
                    self.query.sort = self.preferences.mail_sort;
                } else if self.query.search.trim().is_empty() {
                    self.query.sort = MailSort::Relevance;
                }
                self.query.search_all_folders = !query.trim().is_empty();
                self.query.search = query;
                self.reconcile_selection_scope();
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
            Message::Folder(folder) => self.open_mail_folder(folder, false),
            Message::SentFolder => self.open_mail_folder("Sent".into(), true),
            Message::Account(account) => {
                self.close_composer();
                self.query.folders = None;
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
                self.close_composer();
                self.focused_input = None;
                self.pending_focus = None;
                self.query.sort = sort;
                if self.query.search.trim().is_empty() && sort != MailSort::Relevance {
                    self.preferences.mail_sort = sort;
                    self.save_preferences();
                }
                self.query.offset = 0;
                self.selected = None;
                self.detail = None;
                self.request_page();
            }
            Message::Filter(filter) => {
                self.close_composer();
                self.focused_input = None;
                self.pending_focus = None;
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
                self.close_composer();
                self.query.folders = None;
                self.tab = Tab::Mail;
                self.query.starred_only = true;
                self.query.unread_only = false;
                self.query.read_only = false;
                self.query.attachments_only = false;
                self.query.folder.clear();
                self.query.sent_only = false;
                self.query.offset = 0;
                self.selected = None;
                self.detail = None;
                self.request_page();
            }
            Message::Select(id) => {
                self.list_focus = true;
                self.focused_input = None;
                self.pending_focus = None;
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
                self.select_for_read(id);
            }
            Message::Hover(id) => self.preload(id),
            Message::NextPage(next) => {
                self.close_composer();
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
                    self.set_mail_page(page);
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
                        let id = m.id.clone();
                        self.select_for_read(id.clone());
                        let next = self
                            .page
                            .rows
                            .iter()
                            .position(|mail| mail.id == id)
                            .unwrap_or(next);
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
                    "find-message" => {
                        self.find_message.open && self.tab == Tab::Mail && self.dialog.is_none()
                    }
                    "folder-search" => self.dialog == Some(Dialog::Move),
                    "folder-parent-search" => self.dialog == Some(Dialog::FolderChange),
                    "event-title" => self.dialog == Some(Dialog::Event),
                    "to" | "compose-body" => {
                        self.compose_visible()
                            && !self.composer.current.minimized
                            && self.dialog.is_none()
                    }
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
            Message::Sync => {
                if self.try_command(Command::Sync) {
                    self.busy.insert("sync".into());
                    self.refresh.start(Instant::now());
                }
            }
            Message::RefreshFrame(now) => {
                if self.refresh_animating() {
                    self.refresh.advance(now);
                }
            }
            Message::SyncCalendar => self.send(Command::SyncCalendar),
            Message::MoveFirst => {
                if self.dialog == Some(Dialog::Move)
                    && let Some(folder) = self.ranked_move_folders().into_iter().next()
                {
                    return self.handle(Message::Move(folder));
                }
            }
            Message::Move(folder) => {
                if self.mail_selection.mode {
                    let account = (self.dialog == Some(Dialog::Move)
                        && self.preferences.cross_account_moves
                        && !self.field("move_account").is_empty())
                    .then(|| self.field("move_account").to_owned());
                    self.begin_bulk(bulk::Intent::Move { account, folder });
                    return Task::none();
                }
                if let Some(mail) = self.action_mail().cloned() {
                    let destination = self.field("move_account");
                    let transfer = self.dialog == Some(Dialog::Move)
                        && self.preferences.cross_account_moves
                        && !destination.is_empty()
                        && destination != mail.account_id;
                    if !transfer && mail.folder == folder {
                        self.dialog = None;
                        self.focused_input = None;
                        self.pending_focus = None;
                        return Task::none();
                    }
                    if transfer {
                        self.transfer_mail(mail, destination.to_owned(), folder);
                    } else {
                        self.move_mail(mail, folder);
                    }
                }
            }
            Message::ToggleStar | Message::ToggleRead => {
                if self.mail_selection.mode {
                    self.begin_bulk(if matches!(message, Message::ToggleRead) {
                        bulk::Intent::Read
                    } else {
                        bulk::Intent::Star
                    });
                    return Task::none();
                }
                if let Some(mail) = self.action_mail().cloned() {
                    self.toggle_mail_flag(mail, matches!(message, Message::ToggleRead));
                }
            }
            Message::Reply | Message::ReplyAll => {
                if let Some(detail) = self.detail.clone() {
                    let mut draft = detail.reply.draft(
                        &detail,
                        &self.workspace.accounts,
                        matches!(message, Message::ReplyAll),
                    );
                    self.restore_reply();
                    if self
                        .composer
                        .current
                        .draft
                        .reply_context
                        .as_ref()
                        .is_some_and(|context| context.mail_id == detail.summary.id)
                    {
                        self.composer.current.minimized = false;
                    } else {
                        draft.reply_context = Some(ReplyContext {
                            account_id: detail.summary.account_id.clone(),
                            mail_id: detail.summary.id.clone(),
                            quote: std::mem::take(&mut draft.body),
                            include_quote: true,
                        });
                        self.load_draft(draft);
                    }
                    return focus_after_layout("compose-body");
                }
            }
            Message::Forward => {
                if let Some(mail) = self.action_mail().cloned() {
                    self.begin_forward(mail.id);
                }
            }
            Message::Print(message) => return self.handle_print(message),
            Message::ChooseAttachments => return self.choose_attachments(),
            Message::ChosenAttachments(draft, paths) => self.attach_chosen(draft, paths),
            Message::RemoveDraftAttachment(id) => self.remove_draft_attachment(id),
            Message::ComposeField(key, value) => self.edit_compose_field(key, value),
            Message::CloseComposer => {
                self.close_composer();
                return widget::operation::focus("unfocused");
            }
            Message::ToggleComposer => {
                self.composer.current.minimized = !self.composer.current.minimized;
                self.pending_focus = None;
                self.focused_input = None;
                return if self.composer.current.minimized {
                    widget::operation::focus("unfocused")
                } else {
                    focus_after_layout("compose-body")
                };
            }
            Message::IncludeOriginal(value) => {
                if !self.compose_locked()
                    && let Some(context) = &mut self.composer.current.draft.reply_context
                {
                    context.include_quote = value;
                    self.draft_edited();
                }
            }
            Message::ShowRecipients => {
                self.composer.current.show_recipients = !self.composer.current.show_recipients
            }
            Message::SaveDraft => {
                self.save_current_draft();
            }
            Message::Send => {
                if self.compose_visible()
                    && self.dialog.is_none()
                    && !self.compose_locked()
                    && self.composer.io.as_deref() != Some(&self.composer.current.draft.id)
                {
                    let draft = self.current_draft();
                    if self.try_command(Command::Send(draft)) {
                        self.busy
                            .insert(format!("send:{}", self.composer.current.draft.id));
                    }
                }
            }
            Message::Draft(id) => {
                if self.workspace.outgoing_drafts.contains(&id) {
                    self.open_outbox();
                    return Task::none();
                }
                if let Some(draft) = self.owned_draft(&id) {
                    self.load_draft(draft);
                    return focus_after_layout("compose-body");
                }
            }
            Message::Field(key, value) => {
                if self.dialog == Some(Dialog::Calendar) {
                    if self.calendar_setup.saving.is_some() {
                        return Task::none();
                    }
                    self.calendar_setup.invalidate();
                }
                if self.dialog == Some(Dialog::Account)
                    && !matches!(key, "setup_step" | "sent_copy" | "sent_folder")
                {
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
                    self.fields.insert("id", account.id.clone());
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
                        ("sent_copy", format!("{:?}", a.sent_copy)),
                        ("sent_folder", a.sent_folder),
                    ] {
                        self.fields.insert(k, v);
                    }
                }
            }
            Message::OpenOutbox => self.open_outbox(),
            Message::OutboxPage(offset) => self.load_outbox(offset),
            Message::SelectOutgoing(attempt) => {
                self.outbox.selected =
                    (self.outbox.selected.as_ref() != Some(&attempt)).then_some(attempt);
                self.outbox.confirmed = false;
            }
            Message::ConfirmOutgoing(value) => self.outbox.confirmed = value,
            Message::ResolveOutgoing(action) => self.resolve_outbox(action),
            Message::MoveRecovery(message) => self.handle_move_recovery(message),
            Message::ReviewRemoval(target) => self.review_removal(target),
            Message::ConfirmRemoval => self.confirm_removal(),
            Message::CancelPendingTransfers(value) => self.removal.cancel_transfers = value,
            Message::CleanupCredentials => {
                if !self.busy.contains("credential-cleanup")
                    && self.try_command(Command::CleanupCredentials)
                {
                    self.busy.insert("credential-cleanup".into());
                }
            }
            Message::RestoreGoogleCalendars => self.send(Command::RestoreGoogleCalendars),
            Message::DiscoverCalendars => self.discover_calendars(),
            Message::SaveCalendar => self.connect_calendars(),
            Message::CalendarBack => {
                if self.calendar_setup.saving.is_none() {
                    self.calendar_setup.invalidate();
                }
            }
            Message::ChooseCalendar(url) => {
                if self.calendar_setup.saving.is_none()
                    && self.calendar_setup.choices.iter().any(|c| c.url == url)
                    && !self.calendar_setup.selected.remove(&url)
                {
                    self.calendar_setup.selected.insert(url);
                }
            }
            Message::SavePreferences => match self.read_preferences() {
                Ok(()) => {
                    self.save_preferences();
                    self.confirm_save = Some(self.preference_sync.generation());
                    self.saved_toast = None;
                }
                Err(e) => {
                    self.confirm_save = None;
                    self.notice(e.to_string(), true);
                    self.preference_notice = self.notice.as_ref().map(|notice| notice.2);
                }
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
            Message::GoogleLogin(retry) => {
                if let Err(e) = self.read_preferences() {
                    self.notice(e.to_string(), true);
                } else {
                    let request = self.preference_sync.changed();
                    self.pending_google_login = Some((request, self.preferences.clone(), retry));
                    if !self.queue_preference_write(request, self.preferences.clone()) {
                        self.pending_google_login = None;
                    }
                }
            }
            Message::ReviewGoogleDisconnect => self.open(Dialog::GoogleDisconnect),
            Message::ConfirmGoogleDisconnect => {
                if self.google_disconnect_pending.is_none() {
                    let revision = self.preferences.google_lifecycle.revision;
                    if self.try_command(Command::DisconnectGoogle(revision)) {
                        self.google_disconnect_pending = Some(revision);
                        self.busy.insert("google-disconnect".into());
                    }
                }
            }
            Message::CleanupGoogle => self.send(Command::CleanupGoogle),
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
            Message::Key(key, modifiers, captured, focus) => {
                return self.native_key(key, modifiers, captured, focus);
            }
            Message::Remap(action, slot) => {
                self.remapping = Some((action, slot));
            }
            Message::ClearShortcut(action, slot) => {
                match self
                    .preferences
                    .shortcuts
                    .remap_slot(action, slot, String::new())
                {
                    Ok(()) => {
                        self.remapping = None;
                        self.save_preferences();
                    }
                    Err(error) => self.notice(error.to_string(), true),
                }
            }
            Message::ResetShortcuts => {
                self.remapping = None;
                self.preferences.shortcuts = Default::default();
                self.save_preferences();
            }
            Message::Editor(action) => {
                if !self.compose_visible() || self.dialog.is_some() || self.compose_locked() {
                    return Task::none();
                }
                if action.is_edit() {
                    self.draft_edited();
                }
                self.composer.current.editor.perform(action);
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
                if !self.event_access().delete {
                    self.notice("This calendar does not allow deleting this event.", true);
                    return Task::none();
                }
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
            Message::Resize(size) => {
                if size != self.size {
                    self.mail_drag.clear();
                }
                self.size = size;
                if self.tx.is_some() && size.width > 0. && size.height > 0. {
                    self.preferences.window_size = Some(WindowSize {
                        width: size.width,
                        height: size.height,
                    });
                    return self.debounce_layout();
                }
            }
            Message::SidebarResize(width) => {
                self.preferences.sidebar_width = Some(width.clamp(160., self.max_sidebar_width()));
                return self.debounce_layout();
            }
            Message::DismissToast => self.saved_toast = None,
            Message::DismissActionToast => {
                self.action_toasts.current = None;
                self.prune_undos();
            }
            Message::UndoActions(tokens) => self.undo_combined_actions(tokens),
            Message::DismissUndoErrors(tokens) => self.dismiss_undo_errors(tokens),
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
            Message::DatabaseExport => return self.begin_database_export(),
            Message::DatabaseImport(action) => return self.database_import_action(action),
            Message::Profiles(action) => self.profile_action(action),
            Message::ProfileSync(action) => self.shared_profile_action(action),
            Message::DatabaseExportPath(request, path) => self.database_export_path(request, path),
            Message::CancelDatabaseTransfer => self.cancel_database_transfer(),
            Message::ChosenPath(key, path) => {
                if let Some(path) = path {
                    self.fields.insert(key, path);
                }
            }
            Message::Modifiers(modifiers) => self.modifiers = modifiers,
            Message::RevealSidebar(target, attempt) => return self.reveal_sidebar(target, attempt),
            Message::ToggleAccountFolders(account) => {
                if self.preferences.collapsed_accounts.contains(&account) {
                    self.preferences
                        .collapsed_accounts
                        .retain(|id| id != &account);
                } else {
                    self.preferences.collapsed_accounts.push(account);
                }
                self.save_preferences();
            }
            Message::ToggleFolderGroup(account, path) => {
                self.sidebar_focus = true;
                self.list_focus = false;
                let expanded = !self.folder_expanded(&account, &path);
                self.set_folder_expanded(&account, &path, expanded);
                if let Some(index) = self.sidebar_items().iter().position(|item| {
                    self.sidebar_group(&item.action).as_ref()
                        == Some(&(account.clone(), path.clone()))
                }) {
                    self.sidebar_index = index;
                }
            }
            Message::ToggleInboxExpanded => self.inbox_expanded = !self.inbox_expanded,
            Message::SidebarClick(index, modifiers) => {
                self.modifiers = modifiers;
                return self.handle(Message::SidebarAction(index));
            }
            Message::OpenMessageClick(id, modifiers) => {
                if !modifiers.control() && !modifiers.command() && !modifiers.shift() {
                    return self.handle(Message::OpenMessage(id));
                }
            }
            Message::SelectClick(id, modifiers) => {
                self.modifiers = modifiers;
                self.list_focus = true;
                return self.click_select_mail(id, modifiers);
            }
            Message::MailPaneClicked(pane) => {
                self.sidebar_focus = false;
                self.list_focus = matches!(self.panes.get(pane), Some(MailPane::Inbox));
            }
            Message::ToggleSelection => return self.toggle_selection_mode(),
            Message::CheckMail(id) => return self.checkbox_mail(id),
            Message::SelectAllMail => return self.select_all_mail(),
            Message::ClearSelection => self.clear_selected_mail(),
            Message::SidebarAction(index) => {
                self.sidebar_focus = true;
                self.list_focus = false;
                self.sidebar_index = index;
                if let Some(item) = self.sidebar_items().get(index) {
                    if (self.modifiers.control() || self.modifiers.command())
                        && let Some(folder) = self.sidebar_folder(&item.action)
                    {
                        self.toggle_folder_selection(folder);
                    } else {
                        return self.handle(item.action.clone());
                    }
                }
            }
            Message::AccountFolderUnified => {
                self.query.account = None;
                return self.handle(Message::Folder("INBOX".into()));
            }
            Message::AccountFolder(account, folder) => {
                let folder = self.original_folder(&account, &folder);
                self.query.account = Some(account);
                return self.handle(Message::Folder(folder));
            }
            Message::PrefUnified(value) => {
                self.query.folders = None;
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
                self.select_for_read(id);
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
            Message::PrefConversations(value) => {
                self.preferences.group_conversations = value;
                self.conversation.page = Default::default();
                if let Some(id) = self.selected.clone() {
                    self.focus_conversation_message(id);
                }
                self.request_conversation(None);
                self.save_preferences();
            }
            Message::ConversationMessage(id) => {
                self.focused_input = None;
                self.pending_focus = None;
                if self.conversation.page.rows.iter().any(|mail| mail.id == id) {
                    if self.reader_id() == Some(&id) {
                        self.conversation.collapsed = !self.conversation.collapsed;
                    } else {
                        self.focus_conversation_message(id);
                    }
                }
            }
            Message::ConversationFlag(id) => {
                if let Some(mail) = self
                    .conversation
                    .page
                    .rows
                    .iter()
                    .find(|mail| mail.id == id)
                    .cloned()
                {
                    self.toggle_mail_flag(mail, false);
                }
            }
            Message::ConversationPage(next) => {
                let offset = if next {
                    self.conversation.page.offset + crate::store::CONVERSATION_PAGE_SIZE
                } else {
                    self.conversation
                        .page
                        .offset
                        .saturating_sub(crate::store::CONVERSATION_PAGE_SIZE)
                };
                self.request_conversation(Some(offset));
            }
            Message::ConversationViewport(y) => self.conversation.scroll = y,
            Message::ConversationScroll(generation) => return self.conversation_scroll(generation),
            Message::RetryConversation => {
                self.request_conversation(Some(self.conversation.page.offset))
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
                if let Some(mail) = self.page.rows.iter().find(|m| m.id == id).cloned() {
                    self.toggle_mail_flag(mail, false);
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
                if self.flush_pane_resize() {
                    return self.debounce_layout();
                }
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
        if self.detail.as_ref().is_some_and(|d| d.html.is_some()) {
            return;
        }
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
        if self.mail_selection.mode && self.field("move_account").is_empty() {
            let Some(snapshot) = &self.mail_selection.snapshot else {
                return vec![];
            };
            let mut accounts = snapshot.accounts.keys();
            let Some(first) = accounts.next() else {
                return vec![];
            };
            let mut folders = self
                .workspace
                .account_folders
                .get(first)
                .cloned()
                .unwrap_or_default();
            for account in accounts {
                folders.retain(|folder| {
                    self.workspace
                        .account_folders
                        .get(account)
                        .is_some_and(|f| f.contains(folder))
                });
            }
            return folders;
        }
        let account = if self.field("move_account").is_empty() {
            self.action_mail()
                .map(|mail| mail.account_id.as_str())
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
    fn open_mail_folder(&mut self, folder: String, sent_only: bool) {
        self.close_composer();
        self.tab = Tab::Mail;
        self.query.folders = None;
        self.query.folder = folder;
        self.query.sent_only = sent_only;
        self.query.starred_only = false;
        self.query.offset = 0;
        self.selected = None;
        self.detail = None;
        self.request_page();
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
            sent_copy: match self.field("sent_copy") {
                "LocalOnly" => SentCopyPolicy::LocalOnly,
                "ServerManaged" => SentCopyPolicy::ServerManaged,
                _ => SentCopyPolicy::Automatic,
            },
            sent_folder: self.field("sent_folder").trim().into(),
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
            (
                "mail_check_seconds",
                self.preferences.mail_check_seconds.to_string(),
            ),
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
            next.mail_check_seconds = self.field("mail_check_seconds").parse()?;
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
            if self.editing_event.is_some() {
                self.event_access().update
            } else {
                self.event_access().create
            },
            "Choose a calendar that allows this event to be saved."
        );
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
        if let Some(menu) = &mut self.folder_controls.menu {
            use keyboard::key::Named;
            match key {
                Key::Named(Named::Escape) => self.folder_controls.menu = None,
                Key::Named(Named::ArrowDown) => menu.index = (menu.index + 1) % 3,
                Key::Named(Named::ArrowUp) => menu.index = (menu.index + 2) % 3,
                Key::Named(Named::Enter) => {
                    let index = menu.index;
                    return self.handle(Message::Folders(folder_controls::Message::Choose(index)));
                }
                _ => {}
            }
            return Task::none();
        }
        if self.dialog == Some(Dialog::FolderChange) && modifiers.is_empty() && !captured {
            match &key {
                Key::Named(keyboard::key::Named::Enter) => {
                    self.handle_folders(folder_controls::Message::Submit);
                    return Task::none();
                }
                Key::Character(value) if value.eq_ignore_ascii_case("y") => {
                    self.handle_folders(folder_controls::Message::Submit);
                    return Task::none();
                }
                Key::Character(value) if value.eq_ignore_ascii_case("n") => {
                    return self.handle(Message::Close);
                }
                _ => {}
            }
        }

        if key == Key::Named(keyboard::key::Named::Escape) && self.mail_drag.consume_escape()
            || self.mail_drag.holding()
        {
            return Task::none();
        }
        if self.dialog == Some(Dialog::MoveRecovery) && modifiers.is_empty() {
            match &key {
                Key::Named(keyboard::key::Named::Enter) => {
                    return self.handle(Message::MoveRecovery(move_recovery::Message::Submit));
                }
                Key::Character(value) if value.eq_ignore_ascii_case("y") => {
                    return self.handle(Message::MoveRecovery(move_recovery::Message::Submit));
                }
                Key::Character(value) if value.eq_ignore_ascii_case("n") => {
                    return self.handle(Message::Close);
                }
                _ => {}
            }
        }
        if self.bulk_confirming() && modifiers.is_empty() {
            match &key {
                Key::Named(keyboard::key::Named::Enter) => {
                    return self.handle_bulk(bulk::Message::ConfirmResolution);
                }
                Key::Character(value) if value.eq_ignore_ascii_case("y") => {
                    return self.handle_bulk(bulk::Message::ConfirmResolution);
                }
                Key::Named(keyboard::key::Named::Escape) => {
                    return self.handle_bulk(bulk::Message::CancelResolution);
                }
                Key::Character(value) if value.eq_ignore_ascii_case("n") => {
                    return self.handle_bulk(bulk::Message::CancelResolution);
                }
                _ => {}
            }
        }
        if self.dialog == Some(Dialog::BulkReview) && modifiers.is_empty() {
            match &key {
                Key::Named(keyboard::key::Named::Enter) => {
                    return self.handle_bulk(bulk::Message::Confirm);
                }
                Key::Character(value) if value.eq_ignore_ascii_case("y") => {
                    return self.handle_bulk(bulk::Message::Confirm);
                }
                Key::Character(value) if value.eq_ignore_ascii_case("n") => {
                    return self.handle(Message::Close);
                }
                _ => {}
            }
        }
        if self.dialog == Some(Dialog::DiscardDraft) && modifiers.is_empty() {
            match &key {
                Key::Named(keyboard::key::Named::Enter) => {
                    return self.handle(Message::ConfirmDiscardDraft);
                }
                Key::Character(value) if value.eq_ignore_ascii_case("y") => {
                    return self.handle(Message::ConfirmDiscardDraft);
                }
                Key::Character(value) if value.eq_ignore_ascii_case("n") => {
                    return self.handle(Message::Close);
                }
                _ => {}
            }
        }
        if let Some(menu) = &mut self.composer.context {
            match key {
                Key::Named(keyboard::key::Named::Escape) => self.composer.context = None,
                Key::Named(keyboard::key::Named::ArrowDown | keyboard::key::Named::ArrowUp) => {
                    menu.discard = !menu.discard
                }
                Key::Named(keyboard::key::Named::Enter) => {
                    let discard = menu.discard;
                    return self.handle(Message::DraftContextAction(discard));
                }
                _ => {}
            }
            return Task::none();
        }
        if self.context_menu.is_some() {
            use keyboard::key::Named;
            match key {
                Key::Named(Named::Escape) => self.context_menu = None,
                Key::Named(Named::ArrowDown) | Key::Named(Named::ArrowUp) => {
                    let count = self.mail_menu_items().len();
                    let menu = self.context_menu.as_mut().unwrap();
                    menu.index = if key == Key::Named(Named::ArrowDown) {
                        (menu.index + 1) % count
                    } else {
                        (menu.index + count - 1) % count
                    };
                }
                Key::Named(Named::Enter) => {
                    let action =
                        self.mail_menu_items()[self.context_menu.as_ref().unwrap().index].0;
                    return self.choose_mail_context(action);
                }
                _ => {}
            }
            return Task::none();
        }
        if !captured
            && self.dialog.is_none()
            && self.tab == Tab::Mail
            && self.sidebar_focus
            && modifiers.shift()
            && key == Key::Named(keyboard::key::Named::F10)
        {
            if let Some(item) = self.sidebar_items().get(self.sidebar_index)
                && let Some((account, path)) = self.sidebar_folder_context(&item.action)
            {
                self.handle_folders(folder_controls::Message::Context(
                    account.clone(),
                    path.clone(),
                    iced::Point::new(40., 220.),
                ));
            }
            return Task::none();
        }
        if !captured
            && self.dialog.is_none()
            && self.tab == Tab::Mail
            && modifiers.shift()
            && key == Key::Named(keyboard::key::Named::F10)
            && let Some(id) = self.selected.clone()
        {
            return self.handle(Message::MailContext(
                id,
                iced::Point::new(self.sidebar_width() + 50., 220.),
            ));
        }
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
        if let Some((action, slot)) = self.remapping {
            if let Some(chord) = chord {
                match self.preferences.shortcuts.remap_slot(action, slot, chord) {
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
            if self.dialog.is_none() && self.find_message.open {
                return self.handle_find(find_message::Message::Close);
            }
            if self.dialog.is_none() && self.full_reader {
                if self.preferences.shortcuts.resolve("Escape") == Some(Action::ClosePreview) {
                    return self.handle(Message::ClosePreview);
                }
                return Task::none();
            }
            if self.dialog.is_none() && self.mail_selection.mode && self.list_focus {
                self.clear_mail_selection();
                return Task::none();
            }
            return self.handle(Message::Close);
        }
        if key == Key::Named(keyboard::key::Named::Tab) {
            if self.tab == Tab::Mail && self.dialog.is_none() {
                self.sidebar_focus = !self.sidebar_focus;
                self.list_focus = !self.sidebar_focus;
                return widget::operation::focus("unfocused");
            }
            return if modifiers.shift() {
                widget::operation::focus_previous()
            } else {
                widget::operation::focus_next()
            };
        }
        if captured
            && (!modifiers.command()
                || !matches!(
                    chord
                        .as_deref()
                        .and_then(|k| self.preferences.shortcuts.resolve(k)),
                    Some(
                        Action::Search
                            | Action::Find
                            | Action::Mail
                            | Action::Calendar
                            | Action::Settings
                    )
                ))
        {
            return Task::none();
        }
        if self.dialog.is_some() {
            return Task::none();
        }
        if self.tab == Tab::Mail
            && self.sidebar_focus
            && modifiers.is_empty()
            && matches!(
                key,
                Key::Named(keyboard::key::Named::ArrowLeft | keyboard::key::Named::ArrowRight)
            )
        {
            let action = self.sidebar_tree_key(key == Key::Named(keyboard::key::Named::ArrowRight));
            return Task::batch([action, self.reveal_sidebar_focus()]);
        }
        if self.tab == Tab::Mail
            && self.sidebar_focus
            && key == Key::Named(keyboard::key::Named::Enter)
            && modifiers.is_empty()
        {
            let action = self.handle(Message::SidebarAction(self.sidebar_index));
            return Task::batch([action, self.reveal_sidebar_focus()]);
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
                if self
                    .sidebar_items()
                    .get(self.sidebar_index)
                    .is_some_and(|s| {
                        s.section || matches!(s.action, Message::ToggleFolderGroup(..))
                    })
                {
                    return self.reveal_sidebar_focus();
                }
                let action = self.handle(Message::SidebarAction(self.sidebar_index));
                return Task::batch([action, self.reveal_sidebar_focus()]);
            }
            return self.handle(Message::PreviousMessage(previous));
        }
        if let Some(action) = chord
            .as_deref()
            .and_then(|k| self.preferences.shortcuts.resolve(k))
        {
            return match action {
                Action::SelectAll => self.select_all_mail(),
                Action::Find => self.handle_find(find_message::Message::Open),
                Action::Search => {
                    self.focused_input = None;
                    self.tab = Tab::Mail;
                    self.full_reader = false;
                    focus_after_layout("search")
                }
                Action::Move => {
                    if (self.mail_selection.mode && self.mail_selection.count > 0)
                        || self.action_mail().is_some()
                    {
                        self.open(Dialog::Move);
                        return focus_after_layout("folder-search");
                    }
                    Task::none()
                }
                Action::Compose => self.handle(Message::NewMessage),
                Action::Reply => self.handle(Message::Reply),
                Action::ReplyAll => self.handle(Message::ReplyAll),
                Action::Forward => self.handle(Message::Forward),
                Action::Print => self.handle_print(printing::Message::Open),
                Action::Archive => self.handle(Message::Move("Archive".into())),
                Action::Delete => self.handle(Message::Move("Trash".into())),
                Action::Star => self.handle(Message::ToggleStar),
                Action::Sync => self.handle(Message::Sync),
                Action::Next => self.handle(Message::PreviousMessage(false)),
                Action::Previous => self.handle(Message::PreviousMessage(true)),
                Action::Mail => self.handle(Message::Tab(Tab::Mail)),
                Action::Inbox => {
                    if self.tab == Tab::Mail && self.sidebar_focus {
                        let task = self.handle(Message::Tab(Tab::Mail));
                        self.sidebar_focus = true;
                        self.list_focus = false;
                        Task::batch([task, self.reveal_sidebar_focus()])
                    } else {
                        Task::none()
                    }
                }
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
        let mut data = serde_json::json!({"revision":self.test_revision,"tab":format!("{:?}",self.tab),"settings_tab":format!("{:?}",self.settings_tab),"dialog":self.dialog.map(|d|format!("{d:?}")),"dark":self.dark(),"reader_split":self.preferences.reader_split,"saved_reader_split":self.workspace.preferences.reader_split,"sort":format!("{:?}",self.query.sort),"filter":format!("{:?}",self.mail_filter()),"offset":self.query.offset,"busy":self.busy,"query":self.query.search,"folder":self.query.folder,"total":self.page.total,"selected":self.detail.as_ref().map(|d|&d.summary.subject),"selected_id":self.selected,"starred":self.detail.as_ref().map(|d|self.mail_actions.effective(&d.summary).starred),"cache_entries":self.detail_cache.len(),"page_prefetched":self.prefetch_page.is_some(),"ready":self.tx.is_some(),"shortcuts":self.preferences.shortcuts.0,"fields":self.fields.iter().filter(|(k,_)|!k.contains("password")&&!k.contains("secret")&&!k.contains("passphrase")).collect::<HashMap<_,_>>(),"full_reader":self.full_reader,"image_policy":format!("{:?}",self.preferences.image_policy),"images_allowed":self.detail.as_ref().is_some_and(|d|crate::remote_images::allowed(&self.preferences,&d.summary)),"remote_image_count":self.detail.as_ref().map(|d|d.remote_images.len()),"reply_count":self.detail.as_ref().map(|d|d.replies.len()),"expanded_replies":self.expanded_replies,"sidebar_focus":self.sidebar_focus,"inbox_expanded":self.inbox_expanded,"unified":self.preferences.unified_inbox,"cross_account_moves":self.preferences.cross_account_moves,"reader_size":self.preferences.reader_font_size,"calendar_connected":!self.workspace.calendars.is_empty(),"draft_count":self.workspace.drafts.len(),"draft_body":self.workspace.drafts.first().map(|d|&d.body),"editor":self.composer.current.editor.text(),"notice":self.notice.as_ref().map(|n|&n.0),"update_p95_ms":samples.get(samples.len()*95/100),"uptime_ms":self.started.elapsed().as_millis(),"events":self.events.len()});
        data["compose_fields"] = serde_json::json!({
            "account": self.compose_field("account"),
            "to": self.compose_field("to"),
            "cc": self.compose_field("cc"),
            "bcc": self.compose_field("bcc"),
            "subject": self.compose_field("subject"),
        });
        data["composer"] = serde_json::json!({
            "visible": self.compose_visible(),
            "id": self.composer.current.draft.id,
            "minimized": self.composer.current.minimized,
            "parked": self.composer.parked.keys().collect::<Vec<_>>(),
            "pending": self.composer.current.pending,
            "reply": self.composer.current.draft.reply_context,
        });
        self.bulk_test_state(&mut data);
        data["database_transfer"] = self.database_transfer.observation();
        data["database_import"] = self.database_import.observation();
        data["profiles"] = self.profiles.observation();
        data["profile_sync"] = self.profile_sync.observation();
        data["mail_drag"] = self.mail_drag.observation();
        #[cfg(feature = "test-support")]
        {
            data["page_loaded"] = serde_json::json!(self.initial_page_loaded);
        }
        data["mail_selection"] = serde_json::json!({
            "mode": self.mail_selection.mode, "count": self.mail_selection.count,
            "pending": self.mail_selection.busy(), "visible": self.mail_selection.visible,
            "ready": self.mail_selection.ready(), "list_focus": self.list_focus,
            "available": self.mail_selection.snapshot.as_ref().map(|s| s.available),
        });
        #[cfg(feature = "test-support")]
        {
            data["mail_selection"]["drawn"] = serde_json::json!(
                self.mail_selection
                    .drawn_epoch
                    .load(std::sync::atomic::Ordering::Acquire)
                    == self.mail_selection.draw_epoch
            );
        }
        data["move_enter_destination"] = serde_json::json!(if self.dialog == Some(Dialog::Move) {
            self.ranked_move_folders().first().cloned()
        } else {
            None
        });
        data["tooltips"] = serde_json::json!(self.preferences.tooltips);
        data["shortcut_tooltips"] = serde_json::json!(self.preferences.shortcut_tooltips);
        data["settings_search"] = serde_json::json!(self.settings_search);
        data["settings_group"] = serde_json::json!(self.settings_group);
        data["settings_matches"] = serde_json::json!(
            self.settings_matches()
                .iter()
                .map(|s| s.title)
                .collect::<Vec<_>>()
        );
        data["inbox_unread"] = serde_json::json!(self.page.inbox_unread);
        data["tray"] = serde_json::json!({"available": self.tray.available,
            "visible": self.tray.window.is_some(), "ready": self.tray.ready, "temporary": self.tray.temporary,
            "exiting": self.tray.exiting, "enabled": self.preferences.close_to_tray,
            "saved_enabled": self.workspace.preferences.close_to_tray});
        data["close_pending"] =
            serde_json::json!(self.pending_close.is_some() || self.composer.close.is_some());
        data["unread_badge"] = serde_json::json!(self.preferences.unread_badge);
        #[cfg(feature = "test-support")]
        {
            data["notifications"] = self.notification_observation();
        }
        data["saved_unread_badge"] = serde_json::json!(self.workspace.preferences.unread_badge);
        data["count_observed_ids"] = serde_json::json!(
            self.mail_actions
                .base_page
                .observed
                .keys()
                .collect::<Vec<_>>()
        );
        data["shortcut_secondary"] = serde_json::json!(self.preferences.shortcuts.1);
        data["conversation_total"] = serde_json::json!(self.conversation.page.total);
        #[cfg(feature = "test-support")]
        {
            data["sync_round"] = serde_json::json!(self.test_sync_round);
            data["account_sync_waiting"] = serde_json::json!(self.test_account_sync_waiting);
        }
        data["refreshing"] = serde_json::json!(self.busy.contains("sync"));
        data["refresh_animation"] = serde_json::json!({
            "running": self.refresh_animating(), "angle": self.refresh.angle()
        });
        data["background_sync"] = serde_json::json!(self.busy.contains("background-sync"));
        data["mail_check_seconds"] = serde_json::json!(self.preferences.mail_check_seconds);
        data["mail_pending"] = serde_json::json!(self.mail_actions.pending());
        data["read_candidate"] = serde_json::json!(
            self.mail_actions
                .read_candidate
                .as_ref()
                .map(|mail| &mail.subject)
        );
        data["unread"] = serde_json::json!(
            self.detail
                .as_ref()
                .map(|d| self.mail_actions.effective(&d.summary).unread)
        );
        data["mail_rows"] = serde_json::json!(self.page.rows);
        #[cfg(feature = "test-support")]
        for (index, mail) in self.page.rows.iter().enumerate() {
            data["mail_rows"][index]["group_pending"] =
                serde_json::json!(self.bulk_owns_mail(&mail.id));
        }
        data["conversation_rows"] = serde_json::json!(self.conversation.page.rows);
        data["conversation_offset"] = serde_json::json!(self.conversation.page.offset);
        data["conversation_collapsed"] = serde_json::json!(self.conversation.collapsed);
        data["loaded_message_id"] =
            serde_json::json!(self.detail.as_ref().map(|detail| &detail.summary.id));
        data["reader_message_id"] = serde_json::json!(self.reader_id());
        #[cfg(feature = "test-support")]
        {
            data["folder_changes"] = self.folder_test_state();
        }
        data["move_recovery"] = serde_json::json!({"total":self.workspace.move_pending_total,"selected":self.move_recovery.selected.as_ref().map(|r|&r.token),"stage":self.move_recovery.selected.as_ref().map(|r|r.stage),"action":self.move_recovery.action,"confirmed":self.move_recovery.confirmed,"pending":self.move_recovery.pending.len(),"error":self.move_recovery.error,"rows":*self.move_recovery.rows});
        data["outgoing_pending"] = serde_json::json!(self.workspace.outgoing_pending);
        data["outgoing_rows"] = serde_json::json!(self.outbox.page.rows);
        data["outgoing_confirmed"] = serde_json::json!(self.outbox.confirmed);
        data["outgoing_error"] = serde_json::json!(self.outbox.error);
        data["outgoing_selected"] = serde_json::json!(self.outbox.selected);
        data["removal"] = serde_json::json!(self.removal.preview);
        data["removal_error"] = serde_json::json!(self.removal.error);
        data["removing"] = serde_json::json!(self.removal.removing.is_some());
        data["removal_cancel_transfers"] = serde_json::json!(self.removal.cancel_transfers);
        data["account_reconnect_count"] = serde_json::json!(self.workspace.account_reconnect.len());
        data["account_count"] = serde_json::json!(self.workspace.accounts.len());
        data["calendar_count"] = serde_json::json!(self.workspace.calendars.len());
        data["removed_google_calendars"] =
            serde_json::json!(self.workspace.removed_google_calendars);
        data["credential_cleanup"] = serde_json::json!(self.workspace.credential_cleanup);
        data["calendar_discovering"] = serde_json::json!(self.calendar_setup.discovering);
        data["calendar_saving"] = serde_json::json!(self.calendar_setup.saving.is_some());
        data["calendar_choices"] = serde_json::json!(self.calendar_setup.choices);
        data["calendar_selected"] = serde_json::json!(self.calendar_setup.selected.len());
        data["calendar_error"] = serde_json::json!(self.calendar_setup.error);
        data["calendar_sources"] = serde_json::json!(self.workspace.calendars);
        data["google_grant"] = serde_json::json!(self.preferences.google_grant);
        data["google_lifecycle"] = serde_json::json!(self.preferences.google_lifecycle);
        data["google_archived"] = serde_json::json!(self.workspace.google_archived);
        data["google_connected"] = serde_json::json!(self.google_connected);
        data["event_access"] = serde_json::json!(self.event_access());
        data["group_conversations"] = serde_json::json!(self.preferences.group_conversations);
        data["draft_attachments"] = serde_json::json!(self.composer.current.draft.attachments);
        data["drafts_collapsed"] = serde_json::json!(self.preferences.collapsed_drafts);
        data["saved_drafts_collapsed"] =
            serde_json::json!(self.workspace.preferences.collapsed_drafts);
        data["draft_context"] =
            serde_json::json!(self.composer.context.as_ref().map(|menu| &menu.id));
        data["draft_rows"] = serde_json::json!(
            self.workspace
                .drafts
                .iter()
                .map(|draft| (&draft.id, &draft.subject))
                .collect::<Vec<_>>()
        );
        data["discard_pending"] = serde_json::json!(self.composer.discard_pending);
        data["draft_io"] = serde_json::json!(self.composer.io.is_some());
        data["forward_pending"] = serde_json::json!(self.composer.forward_pending.is_some());
        data["draft_forward"] = serde_json::json!(self.composer.current.draft.forward.is_some());
        data["draft_forward_html"] = serde_json::json!(
            self.composer
                .current
                .draft
                .forward
                .as_ref()
                .is_some_and(|quote| !quote.html_body.is_empty()
                    && self.composer.current.editor.text().ends_with(&quote.text))
        );
        data["draft_in_reply_to"] = serde_json::json!(self.composer.current.draft.in_reply_to);
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
        data["selected_folders"] = serde_json::json!(self.query.folders);
        data["collapsed_accounts"] = serde_json::json!(self.preferences.collapsed_accounts);
        data["expanded_folders"] = serde_json::json!(self.preferences.expanded_folders);
        data["saved_expanded_folders"] =
            serde_json::json!(self.workspace.preferences.expanded_folders);
        data["sidebar_index"] = serde_json::json!(self.sidebar_index);
        data["sidebar_rows"] = serde_json::json!(self.sidebar_items().iter().map(|item| {
            let group=self.sidebar_group(&item.action);
            serde_json::json!({"label":item.label,"depth":item.depth,"selectable":self.sidebar_folder(&item.action).is_some(),"group":group,"expanded":group.as_ref().is_some_and(|(a,p)|self.folder_expanded(a,p))})
        }).collect::<Vec<_>>());
        data["sidebar_width"] = serde_json::json!(self.sidebar_width());
        data["saved_sidebar_width"] = serde_json::json!(self.workspace.preferences.sidebar_width);
        data["window_size"] = serde_json::json!([self.size.width, self.size.height]);
        data["saved_window_size"] = serde_json::json!(self.workspace.preferences.window_size);
        data["html_quotes_hidden"] = serde_json::json!(
            self.detail
                .as_ref()
                .is_some_and(|d| self.html_quotes_hidden(d))
        );
        data["find_open"] = serde_json::json!(self.find_message.open);
        data["find_query"] = serde_json::json!(self.find_message.query);
        data["find_match_case"] = serde_json::json!(self.find_message.match_case);
        data["find_pending"] = serde_json::json!(self.find_message.pending);
        data["find_active"] = serde_json::json!(self.find_message.active);
        data["find_count"] = serde_json::json!(
            self.find_message
                .results
                .as_ref()
                .map_or(0, |r| r.matches.len())
        );
        data["find_error"] = serde_json::json!(self.find_message.error);
        data["interface_scale"] = serde_json::json!(self.preferences.interface_scale);
        data["html_cache_ids"] = serde_json::json!(self.html_reader.cache.ids());
        data["html_cache_bytes"] = serde_json::json!(self.html_reader.cache.bytes());
        data["html_cache_hits"] = serde_json::json!(self.html_reader.cache.hits);
        data["html_view_current"] = serde_json::json!(self.html_reader.view_current());
        data["html_ready"] = serde_json::json!(self.html_reader.frame.is_some());
        data["html_formatted"] =
            serde_json::json!(self.detail.as_ref().is_some_and(|d| self.formatted(d)));
        data["html_selected_text"] = serde_json::json!(self.html_reader.selection);
        data["html_loaded_images"] = serde_json::json!(
            self.html_reader
                .supplied
                .intersection(&self.html_reader.resources)
                .count()
        );
        data["html_rendered_images"] = serde_json::json!(
            self.html_reader
                .frame
                .as_ref()
                .map_or(0, |frame| frame.loaded_images.len())
        );
        data["conversation_scroll"] = serde_json::json!(self.conversation.scroll);
        data["html_background"] = serde_json::json!(
            self.html_reader
                .frame
                .as_ref()
                .and_then(|frame| frame.background)
        );
        data["html_resources"] = serde_json::json!(self.html_reader.resources.len());
        data["html_error"] = serde_json::json!(self.html_reader.error);
        data["remote_image_pending"] = serde_json::json!(self.requested_images.len());
        data["remote_image_cached"] = serde_json::json!(self.remote_bytes.len());
        data["html_body_bounds"] = serde_json::json!(self.html_reader.body_bounds);
        data["html_body_visible"] = serde_json::json!(self.html_reader.body_visible);
        data["html_pan_target"] = serde_json::json!(self.html_reader.pan);
        data["html_pan"] = serde_json::json!(self.html_reader.frame.as_ref().map(|f| f.pan));
        data["html_width"] =
            serde_json::json!(self.html_reader.frame.as_ref().map(|f| f.content_width));
        data["html_scroll"] = serde_json::json!(self.html_reader.frame.as_ref().map(|f| f.scroll));
        data["html_height"] =
            serde_json::json!(self.html_reader.frame.as_ref().map(|f| f.content_height));
        data["html_link"] = serde_json::json!(self.html_reader.last_link);
        data["reader_text_ready"] =
            serde_json::json!(self.reader_selection.as_ref().is_some_and(|c| {
                self.detail
                    .as_ref()
                    .is_some_and(|d| Arc::ptr_eq(d, &c.source))
            }));
        data["reader_selected_text"] = serde_json::json!(
            self.reader_selection
                .as_ref()
                .and_then(|c| c.blocks.iter().find_map(|b| b.selection()))
        );
        data["context_menu"] = serde_json::json!(self.context_menu.as_ref().map(|m| &m.mail.id));
        data["context_subject"] =
            serde_json::json!(self.context_menu.as_ref().map(|m| &m.mail.subject));
        #[cfg(feature = "test-support")]
        {
            data["print_pending"] = serde_json::json!(self.printing.pending);
            data["print_source"] = serde_json::json!(self.printing.source);
            data["print_revision"] = serde_json::json!(self.printing.revision);
            data["action_toast"] = serde_json::json!(
                self.action_toasts
                    .current
                    .as_ref()
                    .map(|t| serde_json::json!({"label": t.display_label(&self.workspace), "count": t.count(), "undo": !t.undo_tokens().is_empty()}))
            );
        }
        data["undo_failures"] = serde_json::json!(self.mail_actions.undo_failures().len());
        data["saved_toast"] = serde_json::json!(self.saved_toast.is_some());
        data["contacts"] = serde_json::json!(self.preferences.contacts);
        data["sidebar_labels"] = serde_json::json!(
            self.sidebar_items()
                .iter()
                .map(|s| &s.label)
                .collect::<Vec<_>>()
        );
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
        context_menu::ContextArea::root(self.layout(), self.preferences.interface_scale)
            .with_drag(drag_mail::Region::Root(
                self.mail_drag.clone(),
                self.tab == Tab::Mail
                    && !self.full_reader
                    && self.dialog.is_none()
                    && self.context_menu.is_none()
                    && self.composer.context.is_none(),
                self.drag_rules(),
            ))
            .into()
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

#[cfg(test)]
mod folder_tests;
