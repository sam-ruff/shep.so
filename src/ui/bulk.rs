//! UI holds one optimistic metadata page and compact group receipts. Exact
//! membership, execution state and all per-message Undo receipts live in SQLite.
use super::*;
use crate::{
    bulk::{Action as BulkAction, Item, Job},
    mail_actions::Flags,
    store::SelectionSnapshot,
};
use iced::{
    Alignment, Length,
    widget::{button, column, container, row, scrollable, space, text, tooltip},
};
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum Message {
    Confirm,
    History,
    SelectJob(String),
    JobsPage(usize),
    ItemsPage(Option<u64>),
    Undo(String),
    Resume(String),
    Resolve(String),
    ConfirmResolution,
    CancelResolution,
}
#[derive(Clone)]
pub(super) enum Intent {
    Move {
        account: Option<String>,
        folder: String,
    },
    Read,
    Star,
}
#[derive(Default)]
pub(super) struct State {
    intent: Option<Intent>,
    freeze_pending: Option<u64>,
    serial: u64,
    review: Option<Arc<SelectionSnapshot>>,
    action: Option<BulkAction>,
    retiring: Option<crate::store::MailSelectionId>,
    pub staging: Option<String>,
    stop_requested: bool,
    pub stopped: bool,
    pub waiting_reader: Option<String>,
    staged_review: Option<crate::store::MailSelectionId>,
    resolving: Option<String>,
    prediction: Option<Prediction>,
    pub jobs: VecDeque<Arc<Job>>,
    tokens: HashMap<u64, String>,
    originals: VecDeque<(String, Mail)>,
    current_ids: HashMap<String, String>,
    hints: HashMap<String, Hint>,
    undo: HashMap<String, Hint>,
    pub selected_job: Option<String>,
    items: Arc<Vec<Item>>,
    jobs_offset: usize,
    items_after: Option<u64>,
    history_serial: u64,
    history_refreshed: Option<Instant>,
    error: Option<String>,
}
#[derive(Clone)]
struct Hint {
    action: BulkAction,
    origin: MailQuery,
    groups: Vec<crate::store::SelectionGroup>,
}
struct Prediction {
    id: String,
    action: BulkAction,
    origin: MailQuery,
    selected: HashSet<String>,
    threshold: Option<u64>,
    groups: Vec<crate::store::SelectionGroup>,
}
impl App {
    pub(super) fn bulk_owns_mail(&self, id: &str) -> bool {
        self.page.bulk_pending.contains(id)
            || self.page.bulk_placeholders.contains(id)
            || self
                .bulk
                .prediction
                .as_ref()
                .is_some_and(|p| p.selected.contains(id))
    }
    pub(super) fn begin_bulk(&mut self, intent: Intent) {
        if self.bulk.stopped {
            if !self.try_command(Command::BulkResume(String::new())) {
                return;
            }
            self.bulk.stopped = false;
            self.bulk.stop_requested = false;
        }
        if self.mail_selection.count == 0
            || self.bulk.freeze_pending.is_some()
            || self.bulk.staging.is_some()
        {
            return;
        }
        if self.bulk.hints.len() >= CHANNEL_CAPACITY {
            self.notice(
                "Finish or dismiss a previous group before starting another.",
                true,
            );
            return;
        }
        if !matches!(intent, Intent::Read)
            || self
                .mail_actions
                .read_candidate
                .as_ref()
                .is_some_and(|m| !self.mail_selection.visible.contains(&m.id))
        {
            self.finish_read();
        } else {
            self.mail_actions.read_candidate = None;
        }
        self.bulk.intent = Some(intent);
        self.bulk.review = None;
        self.bulk.action = None;
        self.bulk.error = None;
        self.dialog = Some(Dialog::BulkReview);
        self.focused_input = None;
        self.pending_focus = None;
    }
    pub(super) fn cancel_bulk_review(&mut self) {
        if let Some(review) = self.bulk.review.take() {
            self.bulk.retiring = Some(review.id);
        }
        self.bulk.intent = None;
        self.bulk.action = None;
        self.dialog = None;
    }
    pub(super) fn pump_bulk(&mut self) {
        if self.pending_close.is_some()
            && self.bulk.staging.is_none()
            && !self.bulk.stop_requested
            && !self.bulk.stopped
            && self.try_command(Command::BulkStop)
        {
            self.bulk.stop_requested = true;
        }
        if let Some(id) = self.bulk.retiring {
            if self
                .tx
                .as_ref()
                .is_some_and(|tx| tx.try_send(Command::ReleaseSelection(id)).is_ok())
            {
                self.bulk.retiring = None;
            } else {
                return;
            }
        }
        if self.bulk.intent.is_some()
            && self.bulk.freeze_pending.is_none()
            && self.bulk.review.is_none()
            && self.mail_actions.pending() == 0
            && !self.mail_selection.busy()
        {
            let Some(snapshot) = self
                .mail_selection
                .snapshot
                .clone()
                .filter(|s| s.selected > 0)
            else {
                self.cancel_bulk_review();
                return;
            };
            let serial = self.bulk.serial + 1;
            let visible = self.page.rows.iter().map(|m| m.id.clone()).collect();
            if self.try_command(Command::ReviewSelection(
                serial,
                snapshot.id,
                snapshot.revision,
                visible,
            )) {
                self.bulk.serial = serial;
                self.bulk.freeze_pending = Some(serial);
            }
        }
        self.bulk.tokens.retain(|token, _| {
            self.action_toasts
                .current
                .as_ref()
                .is_some_and(|t| t.contains(*token))
        });
        self.bulk.hints.retain(|id, _| {
            self.bulk.tokens.values().any(|job| job == id)
                || self.bulk.undo.contains_key(id)
                || self.bulk.prediction.as_ref().is_some_and(|p| &p.id == id)
        });
        self.bulk
            .originals
            .retain(|(id, _)| self.bulk.hints.contains_key(id));
        self.bulk
            .current_ids
            .retain(|id, _| self.bulk.originals.iter().any(|(_, m)| &m.id == id));
    }
    pub(super) fn bulk_observed_ids(&self) -> Vec<String> {
        self.bulk
            .hints
            .keys()
            .chain(self.bulk.undo.keys())
            .chain(self.bulk.prediction.iter().map(|p| &p.id))
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .take(CHANNEL_CAPACITY)
            .collect()
    }
    pub(super) fn handle_bulk(&mut self, message: Message) -> Task<super::Message> {
        match message {
            Message::Confirm => {
                let Some(review) = self.bulk.review.clone().filter(|s| s.available > 0) else {
                    return Task::none();
                };
                let Some(action) = self.bulk.action.clone() else {
                    return Task::none();
                };
                let id = uuid::Uuid::new_v4().to_string();
                if !self.try_command(Command::BulkStart(id.clone(), review.id, action.clone())) {
                    return Task::none();
                }
                if let BulkAction::Move { account, folder } = &action {
                    let token = self.action_toasts.add_group(
                        account.as_deref().unwrap_or(""),
                        folder,
                        review.available,
                        Instant::now(),
                    );
                    self.bulk.tokens.insert(token, id.clone());
                }
                for mail in &self.page.rows {
                    if self.mail_selection.visible.contains(&mail.id) {
                        self.bulk.originals.push_back((id.clone(), mail.clone()));
                    }
                }
                while self.bulk.originals.len() > PAGE_SIZE {
                    self.bulk.originals.pop_front();
                }
                self.bulk.prediction = Some(Prediction {
                    id: id.clone(),
                    action,
                    origin: self.query.clone(),
                    selected: self.mail_selection.visible.clone(),
                    threshold: None,
                    groups: review.groups.clone(),
                });
                self.bulk.hints.insert(
                    id.clone(),
                    Hint {
                        action: self.bulk.prediction.as_ref().unwrap().action.clone(),
                        origin: self.query.clone(),
                        groups: review.groups.clone(),
                    },
                );
                self.bulk.staging = Some(id);
                self.bulk.staged_review = Some(review.id);
                self.bulk.review = None;
                self.bulk.intent = None;
                self.bulk.action = None;
                self.dialog = None;
                self.focused_input = None;
                self.pending_focus = None;
                self.clear_mail_selection();
                self.invalidate_action_snapshot();
                self.project_mail_flags();
                self.selected = None;
                self.detail = None;
                if let Some(mail) = self.page.rows.first() {
                    self.select(mail.id.clone());
                }
                self.request_page();
            }
            Message::History => {
                self.dialog = Some(Dialog::BulkHistory);
                self.bulk.error = None;
                self.bulk.history_serial += 1;
                self.send(Command::BulkJobs(self.bulk.history_serial, 0));
                self.bulk.jobs_offset = 0;
            }
            Message::SelectJob(id) => {
                self.bulk.history_refreshed = Some(Instant::now());
                self.bulk.selected_job = Some(id.clone());
                self.bulk.items = Arc::new(vec![]);
                self.bulk.items_after = None;
                self.bulk.history_serial += 1;
                self.send(Command::BulkItems(self.bulk.history_serial, id, None));
            }
            Message::JobsPage(offset) => {
                self.bulk.jobs_offset = offset;
                self.bulk.selected_job = None;
                self.bulk.history_serial += 1;
                self.send(Command::BulkJobs(self.bulk.history_serial, offset));
            }
            Message::ItemsPage(after) => {
                self.bulk.history_refreshed = Some(Instant::now());
                if let Some(id) = self.bulk.selected_job.clone() {
                    self.bulk.items_after = after;
                    self.bulk.history_serial += 1;
                    self.send(Command::BulkItems(self.bulk.history_serial, id, after));
                }
            }
            Message::Undo(id) => {
                self.request_group_undo(id);
            }
            Message::Resume(id) => {
                self.pending_close = None;
                self.bulk.stop_requested = false;
                self.bulk.stopped = false;
                self.send(Command::BulkResume(id));
            }
            Message::Resolve(id) => self.bulk.resolving = Some(id),
            Message::ConfirmResolution => {
                if let Some(id) = self.bulk.resolving.clone()
                    && self.try_command(Command::BulkResolve(id))
                {
                    self.bulk.resolving = None;
                }
            }
            Message::CancelResolution => self.bulk.resolving = None,
        }
        Task::none()
    }
    fn remember_job(&mut self, job: Arc<Job>) {
        if self
            .bulk
            .jobs
            .iter()
            .any(|old| old.id == job.id && old.revision > job.revision)
        {
            return;
        }
        self.bulk.jobs.retain(|old| old.id != job.id);
        self.bulk.jobs.push_front(job);
        while self.bulk.jobs.len() > 20 {
            self.bulk.jobs.pop_back();
        }
    }
    pub(super) fn bulk_event(&mut self, event: Event) {
        match event {
            Event::BulkIdentity(job, source, current) => {
                if self
                    .bulk
                    .originals
                    .iter()
                    .any(|(id, m)| id == &job && m.id == source)
                {
                    if let Some(current) = current {
                        self.bulk.current_ids.insert(source, current);
                    } else {
                        self.bulk.current_ids.remove(&source);
                    }
                }
            }
            Event::BulkReview(serial, result) if self.bulk.freeze_pending == Some(serial) => {
                self.bulk.freeze_pending = None;
                match result {
                    Ok(review) => {
                        let Some(intent) = self.bulk.intent.clone() else {
                            self.bulk.retiring = Some(review.id);
                            return;
                        };
                        self.bulk.action = Some(match intent {
                            Intent::Move { account, folder } => {
                                BulkAction::Move { account, folder }
                            }
                            Intent::Read => BulkAction::Flags(Flags {
                                unread: Some(review.unread == 0),
                                starred: None,
                            }),
                            Intent::Star => BulkAction::Flags(Flags {
                                unread: None,
                                starred: Some(review.starred < review.available),
                            }),
                        });
                        self.bulk.review = Some(review.clone());
                        if review.selected == 1 {
                            let _ = self.handle_bulk(Message::Confirm);
                        }
                    }
                    Err(error) => {
                        self.cancel_bulk_review();
                        self.notice(format!("Could not prepare this selection. {error}"), true);
                    }
                }
            }
            Event::BulkStarted(id, result) => {
                if self.bulk.staging.as_ref() == Some(&id) {
                    self.bulk.staging = None;
                }
                match result {
                    Ok(job) => {
                        self.bulk.staged_review = None;
                        if let Some(prediction) =
                            self.bulk.prediction.as_mut().filter(|p| p.id == id)
                        {
                            prediction.threshold = Some(job.revision);
                        }
                        self.remember_job(job);
                        self.send(Command::BulkRun(id));
                    }
                    Err(error) => {
                        self.bulk.retiring = self.bulk.staged_review.take();
                        self.pending_close = None;
                        if self.bulk.prediction.as_ref().is_some_and(|p| p.id == id) {
                            self.bulk.prediction = None;
                        }
                        let tokens: Vec<_> = self
                            .bulk
                            .tokens
                            .iter()
                            .filter(|(_, job)| *job == &id)
                            .map(|(token, _)| *token)
                            .collect();
                        for token in tokens {
                            self.action_toasts.failed(token);
                        }
                        self.project_mail_flags();
                        self.notice(format!("The group was not changed. {error}"), true);
                    }
                }
                self.request_page();
            }
            Event::BulkUpdate(job) => {
                if self.dialog == Some(Dialog::BulkHistory)
                    && self.bulk.selected_job.as_ref() == Some(&job.id)
                    && self
                        .bulk
                        .history_refreshed
                        .is_none_or(|t| t.elapsed() >= Duration::from_millis(500))
                {
                    self.bulk.history_serial += 1;
                    self.send(Command::BulkItems(
                        self.bulk.history_serial,
                        job.id.clone(),
                        self.bulk.items_after,
                    ));
                    self.bulk.history_refreshed = Some(Instant::now());
                }
                if job.remaining > 0 {
                    self.send(Command::BulkRun(job.id.clone()));
                }
                for (token, id) in &self.bulk.tokens {
                    if id == &job.id {
                        let cancelled = if job.undo_requested { 0 } else { job.cancelled };
                        self.action_toasts.set_count(
                            *token,
                            job.total
                                .saturating_sub(job.failed + job.uncertain + cancelled),
                        );
                    }
                }
                self.remember_job(job);
                self.request_page();
            }
            Event::BulkFinished(id, result) => {
                match result {
                    Ok(job) => {
                        if job.failed + job.uncertain > 0 {
                            self.notice(format!("{} messages could not be confirmed. Open History to review their results.",job.failed+job.uncertain),true);
                        }
                        self.remember_job(job);
                    }
                    Err(error) => {
                        self.pending_close = None;
                        self.bulk.stop_requested = false;
                        if self.bulk.undo.remove(&id).is_some() {
                            for (token, group) in &self.bulk.tokens {
                                if group == &id {
                                    self.action_toasts.failed(*token);
                                }
                            }
                            self.project_mail_flags();
                        }
                        self.bulk.error = Some(error.clone());
                        self.notice(
                            format!("Mail changes need attention. Open History. {error}"),
                            true,
                        );
                    }
                }
                if self.bulk.selected_job.as_ref() == Some(&id) {
                    self.bulk.history_serial += 1;
                    self.send(Command::BulkItems(
                        self.bulk.history_serial,
                        id,
                        self.bulk.items_after,
                    ));
                }
                self.request_page();
            }
            Event::BulkJobs(serial, result) if serial == self.bulk.history_serial => match result {
                Ok(jobs) => {
                    self.bulk.jobs = jobs.iter().cloned().map(Arc::new).collect();
                }
                Err(error) => self.bulk.error = Some(error),
            },
            Event::BulkItems(serial, id, result)
                if serial == self.bulk.history_serial
                    && self.bulk.selected_job.as_ref() == Some(&id) =>
            {
                match result {
                    Ok(items) => self.bulk.items = items,
                    Err(error) => self.bulk.error = Some(error),
                }
            }
            _ => {}
        }
    }
    pub(super) fn undo_combined_actions(&mut self, tokens: Vec<u64>) {
        let counts: Vec<_> = tokens
            .iter()
            .map(|token| (*token, self.action_toasts.weight(*token)))
            .collect();
        let mut accepted = HashSet::new();
        let mut singles = Vec::new();
        for token in &tokens {
            if let Some(id) = self.bulk.tokens.get(token).cloned() {
                if self.request_group_undo(id) {
                    accepted.insert(*token);
                }
            } else if self.mail_actions.undo_available(*token) {
                accepted.insert(*token);
                singles.push(*token);
            }
        }
        if accepted.is_empty() {
            return;
        }
        if !singles.is_empty() {
            self.undo_actions(singles);
        }
        self.action_toasts.restored_counts(
            counts
                .into_iter()
                .filter(|(token, _)| accepted.contains(token))
                .collect(),
            Instant::now(),
        );
    }
    fn request_group_undo(&mut self, id: String) -> bool {
        if self.bulk.stopped {
            if !self.try_command(Command::BulkResume(id.clone())) {
                return false;
            }
            self.bulk.stopped = false;
            self.bulk.stop_requested = false;
        }
        if self.bulk.undo.contains_key(&id) {
            return false;
        }
        if self.try_command(Command::BulkUndo(id.clone())) {
            if let Some(hint) = self.bulk.hints.get(&id).cloned() {
                // Detailed partial failures have their own authoritative effect
                // snapshot. Do not guess which unread accounts they belonged to.
                if self
                    .bulk
                    .jobs
                    .iter()
                    .find(|j| j.id == id)
                    .is_none_or(|j| j.failed + j.uncertain == 0)
                {
                    self.bulk.undo.insert(id.clone(), hint);
                }
            }
            if self.bulk.prediction.as_ref().is_some_and(|p| p.id == id) {
                self.bulk.prediction = None;
            }
            self.invalidate_action_snapshot();
            self.project_mail_flags();
            self.request_page();
            return true;
        }
        false
    }
    pub(super) fn project_bulk(&mut self) {
        let mut page = (*self.page).clone();
        if self.bulk.prediction.as_ref().is_some_and(|p| {
            page.bulk_observed.contains_key(&p.id)
                || p.threshold.is_some_and(|r| page.bulk_revision >= r)
        }) {
            self.bulk.prediction = None;
        }
        if let Some(prediction) = &self.bulk.prediction {
            let hint = Hint {
                action: prediction.action.clone(),
                origin: prediction.origin.clone(),
                groups: prediction.groups.clone(),
            };
            self.apply_group_counts(&mut page, &hint, false);
            page.rows.retain_mut(|mail| {
                if !prediction.selected.contains(&mail.id) {
                    return true;
                }
                prediction.action.apply(mail);
                page.bulk_pending.insert(mail.id.clone());
                self.bulk_mail_visible(mail)
            });
        }
        self.bulk
            .undo
            .retain(|id, _| page.bulk_observed.get(id) != Some(&true));
        for (id, hint) in &self.bulk.undo {
            // No job in this snapshot means the forward action has not been
            // projected yet: its original rows/counts already reflect Undo.
            if page.bulk_observed.get(id) != Some(&false) {
                continue;
            }
            self.apply_group_counts(&mut page, hint, true);
            let originals: Vec<_> = self
                .bulk
                .originals
                .iter()
                .filter(|(job, _)| job == id)
                .map(|(_, m)| m)
                .collect();
            // Identity can change during MOVE. Hide destination rows using
            // logical metadata only for UI projection, never for provider writes.
            page.rows.retain(|m| {
                !originals
                    .iter()
                    .any(|o| o.id == m.id || self.bulk.current_ids.get(&o.id) == Some(&m.id))
            });
            if same_scope(&self.query, &hint.origin) {
                for mail in originals {
                    if self.bulk_mail_visible(mail) {
                        page.bulk_pending.insert(mail.id.clone());
                        page.bulk_placeholders.insert(mail.id.clone());
                        page.rows.push(mail.clone());
                    }
                }
                page.rows.sort_by(|a, b| {
                    match self.query.sort {
                        MailSort::Oldest => a.timestamp.cmp(&b.timestamp),
                        MailSort::Subject => {
                            a.subject.to_lowercase().cmp(&b.subject.to_lowercase())
                        }
                        MailSort::Sender => a.sender.to_lowercase().cmp(&b.sender.to_lowercase()),
                        _ => b.timestamp.cmp(&a.timestamp),
                    }
                    .then_with(|| a.id.cmp(&b.id))
                });
                page.rows.truncate(PAGE_SIZE);
            }
        }
        self.page = Arc::new(page);
    }
    fn bulk_mail_visible(&self, mail: &Mail) -> bool {
        self.bulk_scope_contains(&self.query, &mail.account_id, &mail.folder)
            && !(self.query.unread_only && !mail.unread
                || self.query.read_only && mail.unread
                || self.query.starred_only && !mail.starred)
    }
    fn bulk_scope_contains(&self, query: &MailQuery, account: &str, folder: &str) -> bool {
        let contains = |selected: &Option<String>, target: &str, sent: bool| {
            selected.as_ref().is_none_or(|a| a == account)
                && if sent {
                    // Explicit server Sent mapping when configured; automatic
                    // discoveries will be reconciled by the database snapshot.
                    self.workspace
                        .accounts
                        .iter()
                        .find(|a| a.id == account)
                        .filter(|a| !a.sent_folder.is_empty())
                        .map_or(folder == "Sent", |a| a.sent_folder == folder)
                } else {
                    target.is_empty() || target == folder
                }
        };
        query.folders.as_ref().map_or_else(
            || contains(&query.account, &query.folder, query.sent_only),
            |folders| {
                folders
                    .iter()
                    .any(|f| contains(&f.account, &f.folder, f.sent_only))
            },
        )
    }
    fn apply_group_counts(&self, page: &mut MailPage, hint: &Hint, undo: bool) {
        let sign = if undo { -1_isize } else { 1 };
        let mut total_delta = 0_isize;
        let mut unread_delta = 0_isize;
        for group in &hint.groups {
            let (account, folder, unread) = match &hint.action {
                BulkAction::Move { account, folder } => (
                    account.as_deref().unwrap_or(&group.account),
                    folder.as_str(),
                    group.unread,
                ),
                BulkAction::Flags(flags) => (
                    &*group.account,
                    &*group.folder,
                    flags
                        .unread
                        .map_or(group.unread, |v| if v { group.total } else { 0 }),
                ),
            };
            let before_inbox = group.folder.eq_ignore_ascii_case("INBOX");
            let after_inbox = folder.eq_ignore_ascii_case("INBOX");
            if before_inbox {
                add_count(
                    page.inbox_unread.entry(group.account.clone()).or_default(),
                    -sign * (group.unread as isize),
                );
            }
            if after_inbox {
                add_count(
                    page.inbox_unread.entry(account.into()).or_default(),
                    sign * (unread as isize),
                );
            }
            // Captured search membership is known only in the original query.
            // Other folder-only scopes can be projected exactly from aggregates.
            if !same_scope(&self.query, &hint.origin) {
                continue;
            }
            let before = self.bulk_scope_contains(&self.query, &group.account, &group.folder);
            let mut after = self.bulk_scope_contains(&self.query, account, folder);
            if let BulkAction::Flags(flags) = hint.action
                && (self.query.unread_only && flags.unread == Some(false)
                    || self.query.read_only && flags.unread == Some(true)
                    || self.query.starred_only && flags.starred == Some(false))
            {
                after = false;
            }
            total_delta += sign
                * ((if after { group.total } else { 0 }) as isize
                    - (if before { group.total } else { 0 }) as isize);
            unread_delta += sign
                * ((if after { unread } else { 0 }) as isize
                    - (if before { group.unread } else { 0 }) as isize);
        }
        add_count(&mut page.total, total_delta);
        add_count(&mut page.unread, unread_delta);
    }
    pub(super) fn bulk_test_state(&self, data: &mut serde_json::Value) {
        data["bulk"] = serde_json::json!({"preparing":self.bulk.freeze_pending.is_some(),"review_count":self.bulk.review.as_ref().map(|s|s.selected),"available":self.bulk.review.as_ref().map(|s|s.available),"action":self.bulk.action.as_ref().map(|a|a.label()),"staging":self.bulk.staging,"jobs":self.bulk.jobs.iter().map(|j|serde_json::json!({"id":j.id,"total":j.total,"remaining":j.remaining,"running":j.running,"undo_requested":j.undo_requested,"completed":j.completed,"restored":j.restored,"failed":j.failed,"uncertain":j.uncertain,"cancelled":j.cancelled})).collect::<Vec<_>>(),"items":self.bulk.items.iter().map(|i|serde_json::json!({"subject":i.original.as_ref().map(|m|&m.subject),"status":i.status,"undo":i.undo,"error":i.error})).collect::<Vec<_>>()});
    }
    fn group_icon(
        &self,
        name: &str,
        label: String,
        message: super::Message,
    ) -> Element<'_, super::Message> {
        let control = button(icon(name, 20.))
            .padding(10)
            .style(ghost)
            .on_press_maybe((self.mail_selection.count > 0).then_some(message));
        if !self.preferences.tooltips {
            return control.into();
        }
        tooltip(
            control,
            container(text(label).size(12)).padding(8).style(card),
            tooltip::Position::Bottom,
        )
        .into()
    }
    pub(super) fn group_reader(&self) -> Element<'_, super::Message> {
        let mut rows = column![].spacing(12);
        for mail in self
            .page
            .rows
            .iter()
            .filter(|m| self.mail_selection.visible.contains(&m.id))
            .take(5)
        {
            rows = rows.push(
                column![
                    text(super::views::truncate(&mail.subject, 65))
                        .size(13)
                        .font(BOLD),
                    muted(super::views::sender_name(&mail.sender)).size(12)
                ]
                .spacing(4),
            );
        }
        column![
            container(
                row![
                    self.group_icon(
                        "archive",
                        self.shortcut_hint("Archive selected messages", Action::Archive),
                        super::Message::Move("Archive".into())
                    ),
                    self.group_icon(
                        "trash",
                        self.shortcut_hint("Move selected messages to Trash", Action::Delete),
                        super::Message::Move("Trash".into())
                    ),
                    self.group_icon(
                        "mail",
                        "Change read status of selected messages".into(),
                        super::Message::ToggleRead
                    ),
                    self.group_icon(
                        "flag",
                        self.shortcut_hint("Change flags of selected messages", Action::Star),
                        super::Message::ToggleStar
                    ),
                    space().width(Length::Fill),
                    self.group_icon(
                        "move",
                        self.shortcut_hint("Move selected messages", Action::Move),
                        super::Message::Open(Dialog::Move)
                    )
                ]
                .align_y(Alignment::Center)
            )
            .padding([10, 18]),
            line(),
            scrollable(
                container(
                    column![
                        text(if self.mail_selection.count == 0 {
                            "Select messages".into()
                        } else {
                            format!("{} messages selected", self.mail_selection.count)
                        })
                        .size(24)
                        .font(BOLD),
                        muted(if self.mail_selection.count > PAGE_SIZE {
                            "Selection includes other pages."
                        } else {
                            "Choose an action above, or double-click a message to open it."
                        })
                        .size(12),
                        rows,
                    ]
                    .spacing(24)
                )
                .padding(28)
            )
            .height(Length::Fill)
        ]
        .height(Length::Fill)
        .into()
    }
    pub(super) fn bulk_review_form(&self) -> Element<'_, super::Message> {
        let Some(review) = &self.bulk.review else {
            return column![
                text("Preparing your selection…").size(14),
                muted("You can cancel while this finishes.").size(12)
            ]
            .spacing(12)
            .into();
        };
        let label = self
            .bulk
            .action
            .as_ref()
            .map(|a| a.label())
            .unwrap_or_default();
        let destructive = matches!(&self.bulk.action,Some(BulkAction::Move{folder,..}) if folder.eq_ignore_ascii_case("Trash"));
        let mut body = column![
            text(
                self.bulk
                    .action
                    .as_ref()
                    .map(|a| a.review_label(review.available))
                    .unwrap_or_default()
            )
            .size(20)
            .font(BOLD),
            muted(format!(
                "{} selected across {} {}.",
                review.selected,
                review.accounts.len(),
                if review.accounts.len() == 1 {
                    "account"
                } else {
                    "accounts"
                }
            ))
            .size(12)
        ]
        .spacing(16);
        if review.selected > review.available {
            body = body.push(
                text(format!(
                    "{} messages are no longer available and will be skipped.",
                    review.selected - review.available
                ))
                .size(12),
            );
        }
        body.push(
            row![
                action("Cancel", super::Message::Close),
                space().width(Length::Fill),
                button(text(label).size(13))
                    .padding([12, 18])
                    .style(if destructive {
                        super::components::destructive
                    } else {
                        primary
                    })
                    .on_press_maybe(
                        (review.available > 0).then_some(super::Message::Bulk(Message::Confirm))
                    )
            ]
            .spacing(12)
            .align_y(Alignment::Center),
        )
        .into()
    }
    pub(super) fn bulk_history_form(&self) -> Element<'_, super::Message> {
        if self.bulk.resolving.is_some() {
            return column![text("Accept the current folders and read status?").size(18).font(BOLD),
                text("Check the affected messages after refreshing. This clears unconfirmed changes from this group without retrying them. They will no longer be available for group Undo.").size(13),
                row![action("Back",super::Message::Bulk(Message::CancelResolution)), space().width(Length::Fill), action("Accept current state",super::Message::Bulk(Message::ConfirmResolution))].spacing(12)
            ].spacing(20).into();
        }
        let mut body = column![].spacing(12);
        if let Some(error) = &self.bulk.error {
            body = body.push(text(error).size(12));
        }
        if let Some(id) = &self.bulk.selected_job {
            if let Some(job) = self.bulk.jobs.iter().find(|j| &j.id == id) {
                body = body.push(
                    text(format!("{} · {} messages", job.action.label(), job.total))
                        .size(16)
                        .font(BOLD),
                );
                body = body.push(
                    muted(format!(
                        "{} completed · {} remaining · {} need attention",
                        job.completed + job.restored,
                        job.remaining,
                        job.failed + job.uncertain
                    ))
                    .size(12),
                );
                let mut controls = row![action(
                    "Back",
                    super::Message::Bulk(Message::JobsPage(self.bulk.jobs_offset))
                )]
                .spacing(8);
                if job.completed
                    + if job.undo_requested {
                        job.failed
                    } else {
                        job.remaining
                    }
                    > 0
                {
                    controls = controls.push(action(
                        if job.undo_requested {
                            "Retry Undo"
                        } else {
                            "Undo"
                        },
                        super::Message::Bulk(Message::Undo(id.clone())),
                    ));
                }
                if job.remaining > 0 && job.running == 0 {
                    controls = controls.push(action(
                        "Continue",
                        super::Message::Bulk(Message::Resume(id.clone())),
                    ));
                }
                body = body.push(controls);
                if job.uncertain > 0 {
                    body=body.push(text("Some changes were not acknowledged. Refresh and check the source and destination folders before accepting the current state.").size(12));
                    body = body.push(action(
                        "Accept current mail state",
                        super::Message::Bulk(Message::Resolve(id.clone())),
                    ));
                }
            }
            for item in self.bulk.items.iter() {
                body = body.push(
                    container(
                        column![
                            text(
                                item.original
                                    .as_ref()
                                    .map(|m| super::views::truncate(&m.subject, 70))
                                    .unwrap_or_else(|| "Unavailable message".into())
                            )
                            .size(13),
                            muted(format!(
                                "{}{}",
                                if item.undo { "Undo · " } else { "" },
                                match item.status.as_str() {
                                    "queued" => "Waiting",
                                    "running" => "Saving…",
                                    "done" =>
                                        if item.undo {
                                            "Restored"
                                        } else {
                                            "Completed"
                                        },
                                    "failed" => "Needs attention",
                                    "uncertain" => "Not confirmed",
                                    "cancelled" => "Cancelled",
                                    _ => "Unknown result",
                                }
                            ))
                            .size(11),
                            text(item.error.as_deref().unwrap_or("")).size(11)
                        ]
                        .spacing(4),
                    )
                    .padding(12)
                    .width(Length::Fill)
                    .style(card),
                );
            }
            if self.bulk.items_after.is_some() || self.bulk.items.len() == PAGE_SIZE {
                body = body.push(
                    row![
                        action("First page", super::Message::Bulk(Message::ItemsPage(None))),
                        space().width(Length::Fill),
                        button("Next").style(outline).padding(10).on_press_maybe(
                            (self.bulk.items.len() == PAGE_SIZE).then(|| super::Message::Bulk(
                                Message::ItemsPage(self.bulk.items.last().map(|i| i.position))
                            ))
                        )
                    ]
                    .spacing(8),
                );
            }
        } else {
            if self.bulk.jobs.is_empty() {
                body = body.push(muted("No recent group changes."));
            }
            for job in &self.bulk.jobs {
                body = body.push(
                    button(
                        column![
                            text(format!("{} · {} messages", job.action.label(), job.total))
                                .size(14),
                            muted(format!(
                                "{} completed · {} remaining · {} need attention",
                                job.completed + job.restored,
                                job.remaining,
                                job.failed + job.uncertain
                            ))
                            .size(11)
                        ]
                        .spacing(6),
                    )
                    .padding(14)
                    .width(Length::Fill)
                    .style(outline)
                    .on_press(super::Message::Bulk(Message::SelectJob(job.id.clone()))),
                );
            }
            if self.bulk.jobs_offset > 0 || self.bulk.jobs.len() == 20 {
                body = body.push(row![
                    button("Newer").style(outline).padding(10).on_press_maybe(
                        (self.bulk.jobs_offset > 0).then(|| super::Message::Bulk(
                            Message::JobsPage(self.bulk.jobs_offset.saturating_sub(20))
                        ))
                    ),
                    space().width(Length::Fill),
                    button("Older").style(outline).padding(10).on_press_maybe(
                        (self.bulk.jobs.len() == 20).then(|| super::Message::Bulk(
                            Message::JobsPage(self.bulk.jobs_offset + 20)
                        ))
                    )
                ]);
            }
        }
        body.into()
    }
}

fn add_count(value: &mut usize, delta: isize) {
    *value = value.saturating_add_signed(delta);
}
fn same_scope(a: &MailQuery, b: &MailQuery) -> bool {
    let normalize = |q: &MailQuery| {
        let mut q = q.clone();
        q.offset = 0;
        q.observe.clear();
        q.observe_bulk.clear();
        q
    };
    normalize(a) == normalize(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{MailSelectionId, Store};

    #[tokio::test]
    async fn page_observations_prevent_double_projection_before_stage_acknowledgments() {
        let store = Store::memory().unwrap();
        let mails = (0..75)
            .map(|i| {
                parse_mail(
                    "work",
                    &i.to_string(),
                    "INBOX",
                    format!("From: fixture@example.test\r\nSubject: Fixture {i}\r\n\r\nBody")
                        .into_bytes(),
                    true,
                    false,
                )
                .unwrap()
            })
            .collect();
        store.upsert(mails).await.unwrap();
        let source = MailSelectionId::default();
        let query = MailQuery {
            folder: "INBOX".into(),
            ..Default::default()
        };
        let source = store
            .capture_selection(source, 0, query.clone(), true, vec![])
            .await
            .unwrap();
        let frozen = store
            .freeze_selection(source.id, source.revision)
            .await
            .unwrap();
        let original = store.query(query.clone()).await.unwrap();
        let action = BulkAction::Move {
            account: None,
            folder: "Archive".into(),
        };
        let hint = Hint {
            action: action.clone(),
            origin: query.clone(),
            groups: frozen.groups.clone(),
        };
        let (mut app, _) = App::new();
        app.query = query.clone();
        app.set_mail_page(Arc::new(original.clone()));
        app.bulk.hints.insert("test".into(), hint.clone());
        app.bulk.originals = original
            .rows
            .iter()
            .map(|m| ("test".into(), m.clone()))
            .collect();
        app.bulk.prediction = Some(Prediction {
            id: "test".into(),
            action: action.clone(),
            origin: query.clone(),
            groups: frozen.groups.clone(),
            selected: original.rows.iter().map(|m| m.id.clone()).collect(),
            threshold: None,
        });
        app.project_mail_flags();
        assert_eq!(
            (app.page.total, app.page.unread, app.page.rows.len()),
            (0, 0, 0)
        );
        assert_eq!(
            app.page.inbox_unread.get("work"),
            Some(&0),
            "All 75 selected unread messages, not only the 50 visible rows"
        );
        store
            .start_bulk("test".into(), frozen.id, action)
            .await
            .unwrap();
        let observed = MailQuery {
            observe_bulk: vec!["test".into()],
            ..query.clone()
        };
        // A read worker can return this after SQL commit but before BulkStarted.
        app.set_mail_page(Arc::new(store.query(observed.clone()).await.unwrap()));
        assert!(app.bulk.prediction.is_none());
        assert_eq!(app.page.total, 0);
        app.bulk.undo.insert("test".into(), hint);
        app.project_mail_flags();
        assert_eq!(
            (app.page.total, app.page.unread, app.page.rows.len()),
            (75, 75, 50)
        );
        assert_eq!(app.page.bulk_placeholders.len(), 50);
        let id = app.page.rows[0].id.clone();
        app.select(id.clone());
        assert_eq!(app.bulk.waiting_reader.as_ref(), Some(&id));
        assert!(
            app.pending_details.is_empty(),
            "Never fetch using a speculative restored identity"
        );
        store.request_bulk_undo("test".into()).await.unwrap();
        app.set_mail_page(Arc::new(store.query(observed).await.unwrap()));
        assert!(app.bulk.undo.is_empty());
        assert_eq!((app.page.total, app.page.unread), (75, 75));
        assert_eq!(app.page.inbox_unread.get("work"), Some(&75));
    }
    #[test]
    fn a_rejected_or_expired_undo_does_not_claim_success_or_erase_new_feedback() {
        let (mut app, _) = App::new();
        let (sender, _held_receiver) = engine::CommandSender::selection_test_channel();
        for _ in 0..CHANNEL_CAPACITY {
            sender
                .try_send(Command::ReleaseSelection(MailSelectionId::default()))
                .unwrap();
        }
        app.tx = Some(sender);
        let token = app
            .action_toasts
            .add_group("work", "Archive", 75, Instant::now());
        app.bulk.tokens.insert(token, "queued-group".into());
        app.undo_combined_actions(vec![token]);
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Archived 75 messages"
        );
        assert!(app.notice.as_ref().is_some_and(|(_, error, _)| *error));
        app.bulk.tokens.clear();
        app.action_toasts
            .add_group("work", "Trash", 2, Instant::now());
        app.undo_combined_actions(vec![token]);
        assert_eq!(
            app.action_toasts.current.as_ref().unwrap().label(),
            "Deleted 2 messages"
        );
    }
}
