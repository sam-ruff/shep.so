//! Native folder review and durable-operation feedback. Server work stays in the engine.
use super::*;
use crate::engine::folders::{Destination, Event as FolderEvent, Preview, Request};
use crate::folder_actions::{Action as Change, Job, Status, Step};
use iced::{
    Alignment, Length, Point,
    widget::{
        button, checkbox, column, container, opaque, row, scrollable, space, text, text_input,
    },
};

#[derive(Debug, Clone)]
pub enum Message {
    Context(String, String, Point),
    Choose(usize),
    Destination(Option<String>),
    FirstDestination,
    Query(String),
    Submit,
    RefreshReview,
    Back,
    History(usize),
    SelectJob(String),
    Retry,
    Stop,
    Accept(bool),
}
#[derive(Debug, Clone)]
pub(super) struct Menu {
    pub account: String,
    pub source: String,
    pub position: Point,
    pub index: usize,
}
struct Pending {
    preview: Arc<Preview>,
    origin: MailQuery,
    redirect: Option<MailQuery>,
    redirect_revision: u64,
    staging: bool,
}
#[derive(Default)]
pub(super) struct State {
    pub menu: Option<Menu>,
    account: String,
    source: String,
    action: Option<Change>,
    options: Arc<Vec<Destination>>,
    query: String,
    filtered: Vec<usize>,
    preview: Option<Arc<Preview>>,
    serial: u64,
    loading: bool,
    pub error: Option<String>,
    pending: HashMap<String, Pending>,
    pub jobs: Arc<Vec<Job>>,
    history_serial: u64,
    history_loading: bool,
    history_action: Option<String>,
    history_error: Option<String>,
    history_updates: VecDeque<Arc<Job>>,
    history_offset: usize,
    selected: Option<String>,
    accepted: bool,
}
fn wrap(message: Message) -> super::Message {
    super::Message::Folders(message)
}
fn status(job: &Job) -> &'static str {
    if job.steps.iter().any(|s| s.status == Status::Uncertain) {
        "Needs review"
    } else if job.steps.iter().any(|s| s.status == Status::Rejected) {
        "Could not finish"
    } else if job.steps.iter().all(|s| s.status == Status::Done) {
        "Completed"
    } else if job.closed && job.steps.iter().any(|s| s.status == Status::Accepted) {
        "Stopped · unconfirmed"
    } else if job.closed {
        "Stopped"
    } else {
        "In progress"
    }
}
impl App {
    pub(super) fn folder_staging(&self) -> bool {
        self.folder_controls.pending.values().any(|p| p.staging)
    }
    pub(super) fn folder_busy(&self, account: &str) -> bool {
        self.folder_controls
            .pending
            .values()
            .any(|p| p.preview.review.account == account)
    }
    pub(super) fn folder_tree(&self, account: &str) -> Option<&Arc<crate::folders::Tree>> {
        self.folder_controls
            .pending
            .values()
            .find(|p| p.preview.review.account == account)
            .map(|p| &p.preview.tree)
            .or_else(|| self.workspace.folder_trees.get(account))
    }
    pub(super) fn original_folder(&self, account: &str, folder: &str) -> String {
        self.folder_controls
            .pending
            .values()
            .find(|p| p.preview.review.account == account)
            .and_then(|p| p.preview.originals.get(folder))
            .cloned()
            .unwrap_or_else(|| folder.into())
    }
    pub(super) fn handle_folders(&mut self, message: Message) {
        match message {
            Message::Context(account, source, position) => {
                if self.dialog.is_some() {
                    return;
                }
                self.context_menu = None;
                self.composer.context = None;
                self.folder_controls.menu = Some(Menu {
                    account,
                    source,
                    position,
                    index: 0,
                });
            }
            Message::Choose(index) => {
                let Some(menu) = self.folder_controls.menu.take() else {
                    return;
                };
                if index == 2 {
                    self.handle_folders(Message::History(0));
                    return;
                }
                if menu.source.eq_ignore_ascii_case("INBOX") || self.folder_busy(&menu.account) {
                    self.notice("This folder cannot be changed now. Open Folder changes to review pending work.", true);
                    return;
                }
                self.open(Dialog::FolderChange);
                let state = &mut self.folder_controls;
                state.account = menu.account;
                state.source = menu.source;
                state.serial += 1;
                state.preview = None;
                state.error = None;
                state.query.clear();
                state.options = Arc::default();
                state.action = (index == 1).then_some(Change::Delete);
                state.loading = true;
                let request = if index == 1 {
                    Request::Review(
                        state.serial,
                        state.account.clone(),
                        state.source.clone(),
                        Change::Delete,
                    )
                } else {
                    Request::Options(state.serial, state.account.clone(), state.source.clone())
                };
                if !self.try_command(Command::Folder(request)) {
                    self.folder_controls.loading = false;
                }
            }
            Message::Back => {
                self.folder_controls.action = None;
                self.handle_folders(Message::RefreshReview);
            }
            Message::Query(query) => {
                self.folder_controls.query = query;
                self.filter_folder_destinations();
            }
            Message::FirstDestination => {
                if let Some(&index) = self.folder_controls.filtered.first() {
                    let path = self.folder_controls.options[index].path.clone();
                    self.handle_folders(Message::Destination(path));
                }
            }
            Message::Destination(parent) => {
                self.folder_controls.action = Some(Change::Move { parent });
                self.handle_folders(Message::RefreshReview);
            }
            Message::RefreshReview => {
                let state = &mut self.folder_controls;
                let action = state.action.clone();
                state.serial += 1;
                state.preview = None;
                state.error = None;
                state.loading = true;
                let request = if let Some(action) = action {
                    Request::Review(
                        state.serial,
                        state.account.clone(),
                        state.source.clone(),
                        action,
                    )
                } else {
                    Request::Options(state.serial, state.account.clone(), state.source.clone())
                };
                if !self.try_command(Command::Folder(request)) {
                    self.folder_controls.loading = false;
                }
            }
            Message::Submit => {
                if self.dialog != Some(Dialog::FolderChange) || self.folder_controls.loading {
                    return;
                }
                let Some(preview) = self.folder_controls.preview.clone() else {
                    if self.folder_controls.action.is_none() {
                        self.handle_folders(Message::FirstDestination);
                    }
                    return;
                };
                let id = uuid::Uuid::new_v4().to_string();
                if !self.try_command(Command::Folder(Request::Start(
                    id.clone(),
                    preview.review.clone(),
                ))) {
                    return;
                }
                self.resume_folder_close_barrier();
                let origin = self.query.clone();
                let affected = self.query.account.as_deref() == Some(&preview.review.account)
                    && preview
                        .review
                        .plan
                        .members
                        .iter()
                        .any(|m| m.mailbox.name == self.query.folder);
                let redirect = if affected && preview.review.plan.action == Change::Delete {
                    self.query.folder = "INBOX".into();
                    self.query.folders = None;
                    self.query.offset = 0;
                    self.selected = None;
                    self.detail = None;
                    self.request_page();
                    Some(self.query.clone())
                } else {
                    None
                };
                let mut reveal = Vec::new();
                if let Change::Move {
                    parent: Some(parent),
                } = &preview.review.plan.action
                {
                    let mut node = preview.tree.node(parent);
                    while let Some(current) = node {
                        reveal.push(current.path.clone());
                        node = current.parent.map(|i| &preview.tree.nodes[i]);
                    }
                }
                let reveal_account = preview.review.account.clone();
                let deleting = preview.review.plan.action == Change::Delete;
                self.folder_controls.pending.insert(
                    id,
                    Pending {
                        preview,
                        origin,
                        redirect,
                        redirect_revision: self.list_revision,
                        staging: true,
                    },
                );
                if !reveal.is_empty() {
                    self.preferences
                        .expanded_folders
                        .entry(reveal_account)
                        .or_default()
                        .extend(reveal);
                    self.save_preferences();
                }
                self.dialog = None;
                self.notice(
                    if deleting {
                        "Deleting folder…"
                    } else {
                        "Moving folder…"
                    },
                    false,
                );
            }
            Message::History(offset) => {
                self.open(Dialog::FolderHistory);
                if self.notice.as_ref().is_some_and(|n| {
                    n.0 == "Folder change needs attention. Open Folder changes to review or retry."
                }) {
                    self.notice = None;
                }
                self.folder_controls.menu = None;
                self.folder_controls.history_error = None;
                self.folder_controls.history_serial += 1;
                self.folder_controls.history_offset = offset;
                self.folder_controls.history_loading = true;
                self.folder_controls.history_updates.clear();
                self.folder_controls.selected = None;
                self.folder_controls.accepted = false;
                self.send(Command::Folder(Request::History(
                    self.folder_controls.history_serial,
                    offset,
                )));
            }
            Message::SelectJob(id) => {
                self.folder_controls.selected = Some(id);
                self.folder_controls.accepted = false;
                self.folder_controls.history_error = None;
            }
            Message::Accept(value) => self.folder_controls.accepted = value,
            Message::Retry | Message::Stop => {
                let state = &self.folder_controls;
                let Some(job) = state
                    .jobs
                    .iter()
                    .find(|j| Some(&j.id) == state.selected.as_ref())
                else {
                    return;
                };
                if job.closed
                    || state.history_loading
                    || state.history_action.is_some()
                    || state.pending.contains_key(&job.id)
                {
                    return;
                }
                let uncertain = job.steps.iter().any(|s| s.status == Status::Uncertain);
                let running = state.pending.contains_key(&job.id)
                    || job.steps.iter().any(|s| s.status == Status::Running);
                if running
                    || (matches!(message, Message::Retry) && uncertain)
                    || (matches!(message, Message::Stop)
                        && (uncertain && !state.accepted
                            || job.steps.iter().any(|s| s.status == Status::Acknowledged)))
                {
                    return;
                }
                let id = job.id.clone();
                let action_id = id.clone();
                let request = if matches!(message, Message::Retry) {
                    Request::Retry(id)
                } else {
                    Request::Stop(id, state.accepted)
                };
                if self.try_command(Command::Folder(request)) {
                    if matches!(message, Message::Retry) {
                        self.resume_folder_close_barrier();
                    }
                    self.folder_controls.history_action = Some(action_id);
                    self.folder_controls.history_error = None;
                }
            }
        }
    }
    fn filter_folder_destinations(&mut self) {
        let state = &mut self.folder_controls;
        let mut scores: Vec<_> = state
            .options
            .iter()
            .enumerate()
            .filter_map(|(i, d)| crate::fuzzy::score(&state.query, &d.label).map(|s| (s, i)))
            .collect();
        scores.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| state.options[a.1].label.cmp(&state.options[b.1].label))
        });
        state.filtered = scores.into_iter().take(50).map(|(_, i)| i).collect();
    }
    fn observe_folder_job(&mut self, job: Arc<Job>) {
        if self
            .folder_controls
            .jobs
            .iter()
            .any(|old| old.id == job.id && old.revision > job.revision)
        {
            return;
        }
        let updates = &mut self.folder_controls.history_updates;
        updates.retain(|old| old.id != job.id);
        updates.push_front(job.clone());
        updates.truncate(20);
        let jobs = Arc::make_mut(&mut self.folder_controls.jobs);
        if let Some(old) = jobs.iter_mut().find(|old| old.id == job.id) {
            *old = (*job).clone();
        } else if self.folder_controls.history_offset == 0 {
            jobs.insert(0, (*job).clone());
            jobs.truncate(20);
        }
    }
    pub(super) fn reconcile_folder_accounts(&mut self) {
        let accounts: HashSet<_> = self
            .workspace
            .accounts
            .iter()
            .map(|a| a.id.as_str())
            .collect();
        Arc::make_mut(&mut self.folder_controls.jobs)
            .retain(|j| accounts.contains(j.review.account.as_str()));
        self.folder_controls
            .history_updates
            .retain(|j| accounts.contains(j.review.account.as_str()));
        self.folder_controls
            .pending
            .retain(|_, p| accounts.contains(p.preview.review.account.as_str()));
        if self
            .folder_controls
            .history_action
            .as_ref()
            .is_some_and(|id| !self.folder_controls.jobs.iter().any(|j| &j.id == id))
        {
            self.folder_controls.history_action = None;
        }
        if self
            .folder_controls
            .menu
            .as_ref()
            .is_some_and(|m| !accounts.contains(m.account.as_str()))
        {
            self.folder_controls.menu = None;
        }
        if self.dialog == Some(Dialog::FolderChange)
            && !accounts.contains(self.folder_controls.account.as_str())
        {
            self.dialog = None;
            self.folder_controls.serial += 1;
        }
    }
    pub(super) fn folder_event(&mut self, event: FolderEvent) {
        let incoming = match &event {
            FolderEvent::Update(job)
            | FolderEvent::Started(_, Ok(job))
            | FolderEvent::Finished(_, Ok(job)) => Some(job),
            _ => None,
        };
        if incoming.is_some_and(|job| {
            !self
                .workspace
                .accounts
                .iter()
                .any(|a| a.id == job.review.account)
        }) {
            return;
        }
        if incoming.is_some_and(|job| {
            self.folder_controls
                .jobs
                .iter()
                .any(|old| old.id == job.id && old.revision > job.revision)
        }) {
            return;
        }
        match event {
            FolderEvent::Options(serial, result)
                if serial == self.folder_controls.serial
                    && self.dialog == Some(Dialog::FolderChange) =>
            {
                self.folder_controls.loading = false;
                match result {
                    Ok(options) => {
                        self.folder_controls.options = options;
                        self.filter_folder_destinations();
                    }
                    Err(e) => self.folder_controls.error = Some(e),
                }
            }
            FolderEvent::Review(serial, result)
                if serial == self.folder_controls.serial
                    && self.dialog == Some(Dialog::FolderChange) =>
            {
                self.folder_controls.loading = false;
                match result {
                    Ok(preview) => self.folder_controls.preview = Some(preview),
                    Err(e) => self.folder_controls.error = Some(e),
                }
            }
            FolderEvent::Started(id, result) => match result {
                Ok(job) => {
                    if let Some(pending) = self.folder_controls.pending.get_mut(&id) {
                        pending.staging = false;
                    }
                    if self.folder_controls.history_action.as_deref() == Some(&id) {
                        self.folder_controls.history_action = None;
                    }
                    self.observe_folder_job(job);
                    self.send(Command::BulkRun(id));
                }
                Err(error) => self.folder_failed(id, error),
            },
            FolderEvent::Update(job) => self.observe_folder_job(job),
            FolderEvent::Finished(id, result) => {
                match result {
                    Ok(job) => {
                        if self.folder_controls.history_action.as_deref() == Some(&id) {
                            self.folder_controls.history_action = None;
                        }
                        if let Some(pending) = self.folder_controls.pending.remove(&id)
                            && pending.redirect.as_ref() == Some(&self.query)
                            && pending.redirect_revision == self.list_revision
                            && !job.steps.iter().any(|step| step.status == Status::Done)
                        {
                            self.query = pending.origin;
                        }
                        let renamed = job.steps.iter().any(|s| {
                            s.status == Status::Done && matches!(s.step, Step::Rename { .. })
                        });
                        let deleted: HashSet<_> = job
                            .steps
                            .iter()
                            .filter(|s| s.status == Status::Done)
                            .filter_map(|s| match &s.step {
                                Step::Delete { source } | Step::Forget { source } => Some(source),
                                _ => None,
                            })
                            .collect();
                        let mapping: HashMap<_, _> = job
                            .review
                            .plan
                            .members
                            .iter()
                            .filter(|member| renamed || deleted.contains(&member.mailbox.name))
                            .map(|m| {
                                (
                                    m.mailbox.name.clone(),
                                    crate::folder_actions::Plan::wire_destination(m),
                                )
                            })
                            .collect();
                        if self.query.account.as_deref() == Some(&job.review.account)
                            && let Some(destination) = mapping.get(&self.query.folder)
                        {
                            self.query.folder =
                                destination.clone().unwrap_or_else(|| "INBOX".into());
                        }
                        if let Some(folders) = &mut self.query.folders {
                            for folder in folders
                                .iter_mut()
                                .filter(|f| f.account.as_deref() == Some(&job.review.account))
                            {
                                if let Some(destination) = mapping.get(&folder.folder) {
                                    folder.folder =
                                        destination.clone().unwrap_or_else(|| "INBOX".into());
                                }
                            }
                        }
                        let affected_reader = self.detail.as_ref().is_some_and(|detail| {
                            detail.summary.account_id == job.review.account
                                && mapping.contains_key(&detail.summary.folder)
                        }) || self.page.rows.iter().any(|mail| {
                            Some(&mail.id) == self.selected.as_ref()
                                && mail.account_id == job.review.account
                                && mapping.contains_key(&mail.folder)
                        });
                        if affected_reader {
                            self.selected = None;
                            self.detail = None;
                        }
                        self.detail_revision += 1;
                        self.detail_cache.clear();
                        self.pending_details.clear();
                        self.request_page();
                        let failed = job
                            .steps
                            .iter()
                            .any(|s| matches!(s.status, Status::Rejected | Status::Uncertain));
                        if failed {
                            self.pending_close = None;
                            self.resume_folder_close_barrier();
                            self.folder_controls.history_error =
                                job.steps.iter().find_map(|s| s.error.clone());
                            self.notice("Folder change needs attention. Open Folder changes to review or retry.", true);
                        } else if job.closed {
                            let all_done = job.steps.iter().all(|s| s.status == Status::Done);
                            let unconfirmed =
                                job.steps.iter().any(|s| s.status == Status::Accepted);
                            self.notice(if unconfirmed { "Remaining folder changes stopped. Unconfirmed cached mail is kept." }
                            else if !all_done { "Remaining folder changes stopped." }
                            else if job.review.plan.action==Change::Delete { "Folder deleted." }
                            else { "Folder moved." },false);
                        }
                        self.observe_folder_job(job);
                    }
                    Err(error) => self.folder_failed(id, error),
                }
            }
            FolderEvent::History(serial, result)
                if serial == self.folder_controls.history_serial =>
            {
                self.folder_controls.history_loading = false;
                match result {
                    Ok(jobs) => {
                        let mut jobs = (*jobs).clone();
                        jobs.retain(|j| {
                            self.workspace
                                .accounts
                                .iter()
                                .any(|a| a.id == j.review.account)
                        });
                        for update in &self.folder_controls.history_updates {
                            if let Some(old) = jobs.iter_mut().find(|old| old.id == update.id) {
                                if old.revision < update.revision {
                                    *old = (**update).clone();
                                }
                            } else if self.folder_controls.history_offset == 0 {
                                jobs.insert(0, (**update).clone());
                            }
                        }
                        jobs.truncate(20);
                        if self
                            .folder_controls
                            .selected
                            .as_ref()
                            .is_none_or(|id| !jobs.iter().any(|j| &j.id == id))
                        {
                            self.folder_controls.selected = jobs
                                .iter()
                                .find(|j| !j.closed)
                                .or_else(|| jobs.first())
                                .map(|j| j.id.clone());
                        }
                        self.folder_controls.jobs = Arc::new(jobs);
                    }
                    Err(e) => self.folder_controls.history_error = Some(e),
                }
            }
            _ => {}
        }
    }
    fn folder_failed(&mut self, id: String, error: String) {
        if let Some(pending) = self.folder_controls.pending.remove(&id)
            && pending.redirect.as_ref() == Some(&self.query)
            && pending.redirect_revision == self.list_revision
        {
            self.query = pending.origin;
            self.selected = None;
            self.detail = None;
            self.request_page();
        }
        self.pending_close = None;
        self.resume_folder_close_barrier();
        if self.folder_controls.history_action.as_deref() == Some(&id) {
            self.folder_controls.history_action = None;
        }
        self.folder_controls.history_error = Some(error.clone());
        self.notice(error, true);
        self.send(Command::Folder(Request::History(
            self.folder_controls.history_serial,
            self.folder_controls.history_offset,
        )));
    }
    pub(super) fn folder_context_view(&self) -> Element<'_, super::Message> {
        let Some(menu) = &self.folder_controls.menu else {
            return space().into();
        };
        let mut items = column![].spacing(2);
        for (i, (label, glyph)) in [
            ("Move folder…", "move"),
            ("Delete folder…", "trash"),
            ("Folder changes", "clock"),
        ]
        .into_iter()
        .enumerate()
        {
            items = items.push(
                button(
                    row![icon(glyph, 18.), text(label).size(12)]
                        .spacing(10)
                        .align_y(Alignment::Center),
                )
                .padding([10, 12])
                .width(Length::Fill)
                .style(if i == menu.index { selected } else { ghost })
                .on_press_maybe(
                    (i == 2
                        || !menu.source.eq_ignore_ascii_case("INBOX")
                            && !self.folder_busy(&menu.account))
                    .then_some(wrap(Message::Choose(i))),
                ),
            );
        }
        let scale = self.preferences.interface_scale as f32 / 100.;
        container(opaque(container(items).width(224).padding(6).style(card)))
            .padding(iced::Padding {
                left: menu
                    .position
                    .x
                    .clamp(8., (self.size.width / scale - 240.).max(8.)),
                top: menu
                    .position
                    .y
                    .clamp(8., (self.size.height / scale - 155.).max(8.)),
                ..Default::default()
            })
            .into()
    }
    pub(super) fn folder_change_title(&self) -> &'static str {
        if self.folder_controls.action == Some(Change::Delete) {
            "Delete folder?"
        } else {
            "Move folder"
        }
    }
    pub(super) fn folder_change_form(&self) -> Element<'_, super::Message> {
        let state = &self.folder_controls;
        let label = self
            .workspace
            .folder_label(Some(&state.account), &state.source);
        let mut body = column![text(label.to_string()).size(16).font(BOLD)].spacing(14);
        if let Some(error) = &state.error {
            body = body
                .push(text(error).size(13))
                .push(action("Refresh review", wrap(Message::RefreshReview)));
        }
        if state.action.is_none() {
            body = body.push(
                text_input("Find a parent folder…", &state.query)
                    .on_input(|q| wrap(Message::Query(q)))
                    .on_submit(wrap(Message::FirstDestination))
                    .id("folder-parent-search")
                    .style(field)
                    .padding(12)
                    .size(13),
            );
            let mut choices = column![].spacing(4);
            for (position, index) in state.filtered.iter().enumerate() {
                let target = &state.options[*index];
                choices = choices.push(
                    button(row![icon("folder", 18.), text(&target.label).size(13)].spacing(10))
                        .width(Length::Fill)
                        .padding(12)
                        .style(if position == 0 { selected } else { ghost })
                        .on_press(wrap(Message::Destination(target.path.clone()))),
                );
            }
            if state.filtered.is_empty() && !state.loading {
                choices=choices.push(text(if state.options.is_empty() {
                    "No valid destination. This folder may already be at the account root or use a flat namespace."
                } else { "No matching folders. Try another name." }).size(13));
            }
            body = body.push(scrollable(choices).height(240));
        }
        if state.loading {
            body = body.push(muted("Loading folder details…"));
        }
        if let Some(preview) = &state.preview {
            let review = &preview.review;
            body = body.push(
                text(format!(
                    "{} folder{} · {} cached message{}",
                    review.plan.members.len(),
                    if review.plan.members.len() == 1 {
                        ""
                    } else {
                        "s"
                    },
                    review.cached_messages,
                    if review.cached_messages == 1 { "" } else { "s" }
                ))
                .size(13),
            );
            if review.plan.action == Change::Delete {
                body=body.push(text(if review.imap { "Permanently delete this folder, its subfolders and their messages from this account's server and cache?" } else { "Permanently delete these local folders and their cached messages? POP3 server originals are kept." }).size(13));
            } else if let Change::Move { parent } = &review.plan.action {
                body = body.push(action("Choose another folder", wrap(Message::Back)));
                let target = parent
                    .as_deref()
                    .map(|p| {
                        self.workspace
                            .folder_label(Some(&state.account), p)
                            .to_string()
                    })
                    .unwrap_or_else(|| "Account root".into());
                body = body.push(
                    text(format!(
                        "Move this folder and its subfolders into {target}."
                    ))
                    .size(13),
                );
            }
            if review.affected_history > 0 {
                body = body.push(
                    muted(format!(
                        "{} related mail history items will be updated.",
                        review.affected_history
                    ))
                    .size(12),
                );
            }
            body = body.push(
                row![
                    action("Cancel", super::Message::Close),
                    space().width(Length::Fill),
                    button(
                        text(if review.plan.action == Change::Delete {
                            "Delete folder"
                        } else {
                            "Move folder"
                        })
                        .size(13)
                    )
                    .padding([12, 16])
                    .style(if review.plan.action == Change::Delete {
                        destructive
                    } else {
                        primary
                    })
                    .on_press(wrap(Message::Submit))
                ]
                .spacing(12),
            );
        }
        body.into()
    }
    pub(super) fn folder_history_form(&self) -> Element<'_, super::Message> {
        let state = &self.folder_controls;
        let mut body = column![].spacing(10);
        if let Some(error) = &state.history_error {
            body = body.push(text(error).size(13));
        }
        if state.history_loading || state.history_action.is_some() {
            body = body.push(muted("Updating folder changes…"));
        }
        if state.jobs.is_empty() && !state.history_loading {
            body = body.push(text("No folder changes yet.").size(13));
        }
        let mut rows = column![].spacing(4);
        for job in state.jobs.iter() {
            let label = &job.label;
            rows = rows.push(
                button(
                    row![
                        text(label.to_string()).size(13),
                        space().width(Length::Fill),
                        muted(status(job)).size(12)
                    ]
                    .spacing(10),
                )
                .padding(10)
                .width(Length::Fill)
                .style(if state.selected.as_ref() == Some(&job.id) {
                    selected
                } else {
                    ghost
                })
                .on_press(wrap(Message::SelectJob(job.id.clone()))),
            );
        }
        body = body.push(scrollable(rows).height((state.jobs.len().clamp(1, 4) * 42) as f32));
        if let Some(job) = state
            .jobs
            .iter()
            .find(|j| Some(&j.id) == state.selected.as_ref())
        {
            let count = job.review.plan.members.len();
            let progress = if job.review.plan.action == Change::Delete {
                format!(
                    "{} of {count} folders deleted",
                    job.steps
                        .iter()
                        .filter(|s| s.status == Status::Done)
                        .count()
                )
            } else {
                format!(
                    "{count} folder{} → {} · {}",
                    if count == 1 { "" } else { "s" },
                    job.destination_label.as_deref().unwrap_or("Account root"),
                    status(job)
                )
            };
            body = body.push(text(progress).size(13));
            if let Some(account) = self
                .workspace
                .accounts
                .iter()
                .find(|a| a.id == job.review.account)
            {
                body = body.push(muted(&account.email).size(12));
            }
            for step in job.steps.iter().filter(|s| s.error.is_some()).take(3) {
                body = body.push(text(step.error.as_ref().unwrap()).size(12));
            }
            let uncertain = job.steps.iter().any(|s| s.status == Status::Uncertain);
            let running = state.pending.contains_key(&job.id)
                || job.steps.iter().any(|s| s.status == Status::Running);
            if uncertain {
                body = body.push(
                    checkbox(state.accepted)
                        .label("I checked the server folders and accept the unconfirmed result")
                        .on_toggle(|v| wrap(Message::Accept(v)))
                        .size(16),
                );
            }
            if !job.closed {
                body = body.push(
                    row![
                        button(text("Retry").size(12))
                            .padding([10, 14])
                            .style(primary)
                            .on_press_maybe(
                                (!state.history_loading
                                    && state.history_action.is_none()
                                    && !uncertain
                                    && !running)
                                    .then_some(wrap(Message::Retry))
                            ),
                        button(text("Stop remaining changes").size(12))
                            .padding([10, 14])
                            .style(destructive)
                            .on_press_maybe(
                                (!state.history_loading
                                    && state.history_action.is_none()
                                    && !running
                                    && (!uncertain || state.accepted)
                                    && !job.steps.iter().any(|s| s.status == Status::Acknowledged))
                                .then_some(wrap(Message::Stop))
                            )
                    ]
                    .spacing(10)
                    .wrap(),
                );
            }
        }
        body = body.push(
            row![
                button("Previous").style(ghost).on_press_maybe(
                    (state.history_offset > 0).then_some(wrap(Message::History(
                        state.history_offset.saturating_sub(20)
                    )))
                ),
                action("Refresh", wrap(Message::History(state.history_offset))),
                button("Next").style(ghost).on_press_maybe(
                    (state.jobs.len() == 20)
                        .then_some(wrap(Message::History(state.history_offset + 20)))
                )
            ]
            .spacing(10),
        );
        body.into()
    }
}

#[cfg(feature = "test-support")]
impl App {
    pub(super) fn folder_test_state(&self) -> serde_json::Value {
        let s = &self.folder_controls;
        serde_json::json!({
            "menu":s.menu.as_ref().map(|m|serde_json::json!({"account":m.account,"source":m.source,"index":m.index})),
            "loading":if self.dialog==Some(Dialog::FolderHistory) {s.history_loading || s.history_action.is_some()} else {s.loading},"error":if self.dialog==Some(Dialog::FolderChange) {s.error.as_ref()} else {s.history_error.as_ref()},"account":s.account,"source":s.source,
            "options":s.options.iter().map(|d|serde_json::json!({"path":d.path,"label":d.label})).collect::<Vec<_>>(),
            "review":s.preview.as_ref().map(|p|serde_json::json!({"folders":p.review.plan.members.len(),"messages":p.review.cached_messages,"action":p.review.plan.action})),
            "pending":s.pending.len(),"staging":self.folder_staging(),"selected":s.selected,"accepted":s.accepted,
            "jobs":s.jobs.iter().map(|j|serde_json::json!({"id":j.id,"source":j.review.plan.source,"account":j.review.account,"closed":j.closed,"status":status(j),"steps":j.steps.iter().map(|s|serde_json::json!({"status":s.status,"error":s.error})).collect::<Vec<_>>()})).collect::<Vec<_>>()
        })
    }
}

#[cfg(test)]
mod tests;
