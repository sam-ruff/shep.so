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
        /// Chosen from another account's rows, so even one message is reviewed.
        foreign: bool,
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
    pub(super) stop_requested: bool,
    pub(super) stop_generation: u64,
    pub stopped: bool,
    pub waiting_reader: Option<String>,
    staged_review: Option<crate::store::MailSelectionId>,
    resolving: Option<String>,
    prediction: Option<Prediction>,
    pub jobs: VecDeque<Arc<Job>>,
    history_jobs: VecDeque<Arc<Job>>,
    history_loading: bool,
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
    projection: shep_action_core::Projection,
    groups: Vec<crate::store::SelectionGroup>,
    observed: HashMap<String, crate::store::SelectionObservation>,
}
impl App {
    pub(super) fn reconcile_bulk_flags(&mut self, confirmed: &Mail) {
        if let Some(review) = &mut self.bulk.review {
            let review = Arc::make_mut(review);
            if let Some(before) = review.observed.get(&confirmed.id) {
                add_count(
                    &mut review.unread,
                    isize::from(confirmed.unread) - isize::from(before.unread),
                );
                add_count(
                    &mut review.starred,
                    isize::from(confirmed.starred) - isize::from(before.starred),
                );
            }
            reconcile_group_flags(&mut review.groups, &mut review.observed, confirmed);
        }
        if let Some(prediction) = &mut self.bulk.prediction {
            reconcile_group_flags(&mut prediction.groups, &mut prediction.observed, confirmed);
            if let Some(hint) = self.bulk.hints.get_mut(&prediction.id) {
                hint.groups.clone_from(&prediction.groups);
            }
        }
        for (_, original) in self
            .bulk
            .originals
            .iter_mut()
            .filter(|(_, mail)| mail.id == confirmed.id)
        {
            original.unread = confirmed.unread;
            original.starred = confirmed.starred;
        }
    }

    pub(super) fn bulk_action_label(&self, action: &BulkAction, count: Option<usize>) -> String {
        if let BulkAction::Move { account, folder } = action {
            let name = self.workspace.folder_label(account.as_deref(), folder);
            if name != *folder && !folder.eq_ignore_ascii_case("INBOX") {
                return match count {
                    Some(count) => format!("Move {} to {name}?", message_count(count)),
                    None => format!("Move to {name}"),
                };
            }
        }
        match count {
            Some(count) => action.review_label(count),
            None => action.label(),
        }
    }
    pub(super) fn bulk_owns_mail(&self, id: &str) -> bool {
        self.page.is_placeholder(id)
    }
    pub(super) fn bulk_action_owns_mail(&self, id: &str) -> bool {
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
    #[cfg(test)]
    pub(super) fn bulk_action(&self) -> Option<&BulkAction> {
        self.bulk.action.as_ref()
    }
    pub(super) fn cancel_bulk_review(&mut self) {
        if let Some(review) = self.bulk.review.take() {
            self.bulk.retiring = Some(review.id);
        }
        self.bulk.intent = None;
        self.bulk.action = None;
        self.dialog = None;
    }
    pub(super) fn resume_folder_close_barrier(&mut self) {
        self.bulk.stopped = false;
        self.bulk.stop_requested = false;
    }
    pub(super) fn pump_bulk(&mut self) {
        if self.pending_close.is_some()
            && self.bulk.staging.is_none()
            && !self.folder_staging()
            && !self.bulk.stop_requested
            && !self.bulk.stopped
            && self.try_command(Command::BulkStop(self.bulk.stop_generation + 1))
        {
            self.bulk.stop_generation += 1;
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
            && self.mail_selection.ready_for_drag()
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
            let mut visible = self.mail_actions.flag_ids();
            for mail in &self.page.rows {
                if !visible.contains(&mail.id) {
                    visible.push(mail.id.clone());
                }
            }
            visible.truncate(PAGE_SIZE);
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
            .chain(self.individual_observed_jobs())
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
                let start = Command::BulkStart(id.clone(), review.id, action.clone());
                if !self.try_command(start) {
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
                    projection: shep_action_core::Projection::Pending,
                    groups: review.groups.clone(),
                    observed: review.observed.clone(),
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
                self.bulk.resolving = None;
                self.bulk.selected_job = None;
                self.bulk.items = Arc::new(vec![]);
                self.bulk.items_after = None;
                self.bulk.history_loading = true;
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
                return history_top();
            }
            Message::JobsPage(offset) => {
                self.bulk.jobs_offset = offset;
                self.bulk.selected_job = None;
                self.bulk.history_loading = true;
                self.bulk.history_serial += 1;
                self.send(Command::BulkJobs(self.bulk.history_serial, offset));
                return history_top();
            }
            Message::ItemsPage(after) => {
                self.bulk.history_refreshed = Some(Instant::now());
                if let Some(id) = self.bulk.selected_job.clone() {
                    self.bulk.items_after = after;
                    self.bulk.history_serial += 1;
                    self.send(Command::BulkItems(self.bulk.history_serial, id, after));
                    return history_top();
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
            Message::CancelResolution => {
                self.bulk.resolving = None;
                return history_top();
            }
        }
        Task::none()
    }
    pub(super) fn remember_individual_job(&mut self, id: &str, mail: &Mail, action: &BulkAction) {
        let BulkAction::Move { account, folder } = action else {
            return;
        };
        let token = self.action_toasts.add(
            account.as_deref().unwrap_or(&mail.account_id),
            folder,
            Instant::now(),
        );
        self.bulk.tokens.insert(token, id.into());
        let mut original = mail.clone();
        (original.unread, original.starred) = self.displayed_mail_flags(mail);
        self.bulk.hints.insert(
            id.into(),
            Hint {
                action: action.clone(),
                origin: self.query.clone(),
                groups: vec![crate::store::SelectionGroup {
                    account: original.account_id.clone(),
                    folder: original.folder.clone(),
                    total: 1,
                    unread: usize::from(original.unread),
                }],
            },
        );
        self.bulk.originals.push_back((id.into(), original));
    }
    pub(super) fn individual_job_admitted(&mut self, job: Arc<Job>) {
        self.remember_job(job);
    }
    pub(super) fn individual_job_rejected(&mut self, id: &str) {
        for (token, job) in &self.bulk.tokens {
            if job == id {
                self.action_toasts.failed(*token);
            }
        }
        self.bulk.hints.remove(id);
        self.bulk.undo.remove(id);
        self.bulk.originals.retain(|(job, _)| job != id);
    }
    fn remember_job(&mut self, job: Arc<Job>) {
        if let Some(displayed) = self.bulk.history_jobs.iter_mut().find(|j| j.id == job.id)
            && displayed.revision <= job.revision
        {
            *displayed = job.clone();
        }
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
            Event::BulkIdentity(job, source, current, destination) => {
                if let Some((account, folder)) = &destination {
                    for (token, _) in self.bulk.tokens.iter().filter(|(_, id)| **id == job) {
                        self.action_toasts.acknowledged(*token, account, folder);
                    }
                }
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
                        let foreign = matches!(intent, Intent::Move { foreign: true, .. });
                        self.bulk.review = Some(review);
                        for confirmed in self.mail_actions.confirmed_flag_states() {
                            self.reconcile_bulk_flags(&confirmed);
                        }
                        let Some(review) = self.bulk.review.clone() else {
                            return;
                        };
                        let mut unread = review.unread;
                        let mut starred = review.starred;
                        for (id, before) in &review.observed {
                            let current = self.mail_actions.selection_state(id, before);
                            add_count(
                                &mut unread,
                                isize::from(current.unread) - isize::from(before.unread),
                            );
                            add_count(
                                &mut starred,
                                isize::from(current.starred) - isize::from(before.starred),
                            );
                        }
                        self.bulk.action = Some(match intent {
                            Intent::Move {
                                account, folder, ..
                            } => BulkAction::Move { account, folder },
                            Intent::Read => BulkAction::Flags(Flags {
                                unread: Some(unread == 0),
                                starred: None,
                            }),
                            Intent::Star => BulkAction::Flags(Flags {
                                unread: None,
                                starred: Some(starred < review.available),
                            }),
                        });
                        if review.selected == 1 && !foreign {
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
                            prediction.projection =
                                shep_action_core::Projection::acknowledge(Some(job.revision));
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
                if self
                    .bulk
                    .jobs
                    .iter()
                    .any(|current| current.id == job.id && current.revision > job.revision)
                {
                    return;
                }
                self.observe_individual_job(&job);
                Arc::make_mut(&mut self.mail_actions.base_page).folder_count = None;
                Arc::make_mut(&mut self.page).folder_count = None;
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
                        if self
                            .bulk
                            .jobs
                            .iter()
                            .any(|current| current.id == job.id && current.revision > job.revision)
                        {
                            return;
                        }
                        self.observe_individual_job(&job);
                        if job.remaining == 0 && job.running == 0 {
                            let _ = self.handle(super::Message::Backend(Event::Changed));
                        }
                        if job.failed + job.uncertain > 0 {
                            self.notice(format!("{} could not be confirmed. Open History to review their results.",message_count(job.failed+job.uncertain)),true);
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
            Event::BulkJobs(serial, result) if serial == self.bulk.history_serial => {
                self.bulk.history_loading = false;
                match result {
                    Ok(jobs) => {
                        // A read can finish after a more recent worker update.
                        self.bulk.history_jobs = jobs
                            .iter()
                            .map(|job| {
                                self.bulk
                                    .jobs
                                    .iter()
                                    .find(|known| {
                                        known.id == job.id && known.revision > job.revision
                                    })
                                    .cloned()
                                    .unwrap_or_else(|| Arc::new(job.clone()))
                            })
                            .collect();
                        if self.dialog != Some(Dialog::BulkHistory) {
                            for job in jobs.iter().rev() {
                                self.remember_job(Arc::new(job.clone()));
                            }
                        }
                    }
                    Err(error) => self.bulk.error = Some(error),
                }
            }
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
        if self.bulk.undo.contains_key(&id)
            || self
                .bulk
                .jobs
                .iter()
                .any(|job| job.id == id && job.undo_requested && job.failed == 0)
        {
            return false;
        }
        if self.try_command(Command::BulkUndo(id.clone())) {
            self.retire_individual_projection(&id);
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
            if self.selected.as_ref().is_some_and(|selected| {
                !self.page.rows.iter().any(|mail| &mail.id == selected)
                    && self.bulk.originals.iter().any(|(job, mail)| {
                        job == &id
                            && (&mail.id == selected
                                || self.bulk.current_ids.get(&mail.id) == Some(selected))
                    })
            }) {
                self.selected = None;
                self.detail = None;
                self.conversation = Default::default();
                if let Some(mail) = self.page.rows.first() {
                    self.select(mail.id.clone());
                }
            }
            self.request_page();
            return true;
        }
        false
    }
    pub(super) fn project_bulk(&mut self) {
        let mut page = (*self.page).clone();
        if self.bulk.prediction.as_ref().is_some_and(|p| {
            page.bulk_observed.contains_key(&p.id) || p.projection.observed_at(page.bulk_revision)
        }) {
            self.bulk.prediction = None;
        }
        if let Some(prediction) = &self.bulk.prediction {
            let hint = Hint {
                action: prediction.action.clone(),
                origin: prediction.origin.clone(),
                groups: prediction.groups.clone(),
            };
            self.apply_group_counts(&mut page, &hint, false, Some(&prediction.observed));
            page.rows.retain_mut(|mail| {
                if !prediction.selected.contains(&mail.id) {
                    return true;
                }
                prediction.action.apply(mail);
                page.bulk_pending.insert(mail.id.clone());
                if matches!(prediction.action, BulkAction::Move { .. }) {
                    page.bulk_placeholders.insert(mail.id.clone());
                }
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
            self.apply_group_counts(&mut page, hint, true, None);
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
            if same_scope(&self.query, &hint.origin) || self.query.search.trim().is_empty() {
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
    pub(super) fn bulk_scope_contains(
        &self,
        query: &MailQuery,
        account: &str,
        folder: &str,
    ) -> bool {
        if query.exclude_folders.iter().any(|excluded| {
            excluded.account.as_deref() == Some(account) && excluded.folder == folder
        }) {
            return false;
        }
        let scope = query.search_scope();
        let query = scope.as_ref();
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
    fn apply_group_counts(
        &self,
        page: &mut MailPage,
        hint: &Hint,
        undo: bool,
        observed: Option<&HashMap<String, crate::store::SelectionObservation>>,
    ) {
        let sign = if undo { -1_isize } else { 1 };
        let mut total_delta = 0_isize;
        let mut unread_delta = 0_isize;
        let mut inbox_delta = std::collections::BTreeMap::<String, isize>::new();
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
                *inbox_delta.entry(group.account.clone()).or_default() -=
                    sign * group.unread as isize;
            }
            if after_inbox {
                *inbox_delta.entry(account.into()).or_default() += sign * unread as isize;
            }
            // Captured search membership is known only in the original query.
            // Other folder-only scopes can be projected exactly from aggregates.
            if !same_scope(&self.query, &hint.origin)
                && (!self.query.search.trim().is_empty()
                    || self.query.unread_only
                    || self.query.read_only
                    || self.query.starred_only)
            {
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
        if let Some(observed) = observed {
            for (id, before) in observed {
                let current = self.mail_actions.selection_state(id, before);
                let effect = |state: &crate::store::SelectionObservation| {
                    let mut after = state.clone();
                    match &hint.action {
                        BulkAction::Move { account, folder } => {
                            if let Some(account) = account {
                                after.account.clone_from(account);
                            }
                            after.folder.clone_from(folder);
                        }
                        BulkAction::Flags(flags) => {
                            if let Some(unread) = flags.unread {
                                after.unread = unread;
                            }
                            if let Some(starred) = flags.starred {
                                after.starred = starred;
                            }
                        }
                    }
                    after
                };
                let original_after = effect(before);
                let current_after = effect(&current);
                for (state, direction) in [
                    (before, 1),
                    (&original_after, -1),
                    (&current, -1),
                    (&current_after, 1),
                ] {
                    if state.unread && state.folder.eq_ignore_ascii_case("INBOX") {
                        *inbox_delta.entry(state.account.clone()).or_default() += direction;
                    }
                }
                if same_scope(&self.query, &hint.origin) {
                    let visible = |state: &crate::store::SelectionObservation| {
                        self.bulk_scope_contains(&self.query, &state.account, &state.folder)
                            && !(self.query.unread_only && !state.unread
                                || self.query.read_only && state.unread
                                || self.query.starred_only && !state.starred)
                    };
                    let counted_before =
                        self.bulk_scope_contains(&self.query, &before.account, &before.folder);
                    let counted_after = self.bulk_scope_contains(
                        &self.query,
                        &original_after.account,
                        &original_after.folder,
                    ) && !matches!(&hint.action, BulkAction::Flags(flags) if
                            self.query.unread_only && flags.unread == Some(false)
                            || self.query.read_only && flags.unread == Some(true)
                            || self.query.starred_only && flags.starred == Some(false));
                    for (state, direction, included) in [
                        (before, 1, counted_before),
                        (&original_after, -1, counted_after),
                        (&current, -1, visible(&current)),
                        (&current_after, 1, visible(&current_after)),
                    ] {
                        if included {
                            total_delta += direction;
                            unread_delta += direction * isize::from(state.unread);
                        }
                    }
                }
            }
        }
        for (account, delta) in inbox_delta {
            add_count(page.inbox_unread.entry(account).or_default(), delta);
        }
        add_count(&mut page.total, total_delta);
        add_count(&mut page.unread, unread_delta);
    }
    pub(super) fn bulk_test_state(&self, data: &mut serde_json::Value) {
        data["bulk"] = serde_json::json!({"selected_job":self.bulk.selected_job,"resolving":self.bulk.resolving,"jobs_offset":self.bulk.jobs_offset,"items_after":self.bulk.items_after,"preparing":self.bulk.freeze_pending.is_some(),"review_count":self.bulk.review.as_ref().map(|s|s.selected),"available":self.bulk.review.as_ref().map(|s|s.available),"action":self.bulk.action.as_ref().map(|a|self.bulk_action_label(a,None)),"staging":self.bulk.staging,"jobs":self.bulk.jobs.iter().map(|j|serde_json::json!({"id":j.id,"total":j.total,"paused":j.paused,"remaining":j.remaining,"running":j.running,"undo_requested":j.undo_requested,"completed":j.completed,"restored":j.restored,"failed":j.failed,"uncertain":j.uncertain,"cancelled":j.cancelled})).collect::<Vec<_>>(),"items":self.bulk.items.iter().map(|i|serde_json::json!({"position":i.position,"subject":i.original.as_ref().map(|m|&m.subject),"status":i.status,"undo":i.undo,"error":i.error})).collect::<Vec<_>>()});
        data["bulk"]["history_jobs"] = serde_json::json!(
            self.bulk
                .history_jobs
                .iter()
                .map(|j| &j.id)
                .collect::<Vec<_>>()
        );
        data["bulk"]["history_loading"] = serde_json::json!(self.bulk.history_loading);
    }
    pub(super) fn bulk_confirming(&self) -> bool {
        self.dialog == Some(Dialog::BulkHistory) && self.bulk.resolving.is_some()
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
                            format!("{} selected", message_count(self.mail_selection.count))
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
            .map(|a| self.bulk_action_label(a, None))
            .unwrap_or_default();
        let destructive = matches!(&self.bulk.action,Some(BulkAction::Move{folder,..}) if folder.eq_ignore_ascii_case("Trash"));
        let mut body = column![
            text(
                self.bulk
                    .action
                    .as_ref()
                    .map(|a| self.bulk_action_label(a, Some(review.available)))
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
        if let Some(Intent::Move {
            account: Some(account),
            foreign: true,
            ..
        }) = &self.bulk.intent
        {
            body = body.push(
                row![muted("into").size(12), self.account_badge(account)]
                    .spacing(8)
                    .align_y(Alignment::Center),
            );
        }
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
            return column![text("Accept the current mail state?").size(18).font(BOLD),
                text("Check the affected messages after refreshing. This clears unconfirmed changes from this group without retrying them. They will no longer be available for group Undo.").size(13),
                row![action("Back",super::Message::Bulk(Message::CancelResolution)), space().width(Length::Fill), action("Accept current state",super::Message::Bulk(Message::ConfirmResolution))].spacing(12)
            ].spacing(20).into();
        }
        let mut body = column![].spacing(12);
        if let Some(error) = &self.bulk.error {
            body = body.push(text(error).size(12));
        }
        if let Some(id) = &self.bulk.selected_job {
            if let Some(job) = self
                .bulk
                .history_jobs
                .iter()
                .chain(&self.bulk.jobs)
                .find(|j| &j.id == id)
            {
                body = body.push(
                    text(format!(
                        "{} · {}",
                        self.bulk_action_label(&job.action, None),
                        message_count(job.total)
                    ))
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
                                    "repair" => "Server confirmed · updating this device",
                                    "done" =>
                                        if item.undo {
                                            "Restored"
                                        } else {
                                            "Completed"
                                        },
                                    "failed" => "Needs attention",
                                    "uncertain" => "Not confirmed",
                                    "cancelled"
                                        if item.error.as_deref()
                                            == Some(crate::bulk::ACCEPTED_STATE_NOTE) =>
                                        "Accepted current state",
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
            if self.bulk.history_loading {
                return muted("Loading changes…").into();
            }
            if self.bulk.history_jobs.is_empty() {
                body = body.push(muted("No recent group changes."));
            }
            for job in &self.bulk.history_jobs {
                body = body.push(
                    button(
                        column![
                            text(format!(
                                "{} · {}",
                                self.bulk_action_label(&job.action, None),
                                message_count(job.total)
                            ))
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
            if self.bulk.jobs_offset > 0 || self.bulk.history_jobs.len() == 20 {
                body = body.push(row![
                    button("Newer").style(outline).padding(10).on_press_maybe(
                        (self.bulk.jobs_offset > 0).then(|| super::Message::Bulk(
                            Message::JobsPage(self.bulk.jobs_offset.saturating_sub(20))
                        ))
                    ),
                    space().width(Length::Fill),
                    button("Older").style(outline).padding(10).on_press_maybe(
                        (self.bulk.history_jobs.len() == 20).then(|| super::Message::Bulk(
                            Message::JobsPage(self.bulk.jobs_offset + 20)
                        ))
                    )
                ]);
            }
        }
        body.into()
    }
}

fn history_top() -> Task<super::Message> {
    widget::operation::scroll_to(
        "dialog-scroll",
        widget::scrollable::AbsoluteOffset::<f32>::default(),
    )
}

fn reconcile_group_flags(
    groups: &mut [crate::store::SelectionGroup],
    observed: &mut HashMap<String, crate::store::SelectionObservation>,
    confirmed: &Mail,
) {
    let Some(before) = observed.get_mut(&confirmed.id) else {
        return;
    };
    if before.account != confirmed.account_id || before.folder != confirmed.folder {
        return;
    }
    if let Some(group) = groups
        .iter_mut()
        .find(|group| group.account == before.account && group.folder == before.folder)
    {
        add_count(
            &mut group.unread,
            isize::from(confirmed.unread) - isize::from(before.unread),
        );
    }
    before.unread = confirmed.unread;
    before.starred = confirmed.starred;
}

fn message_count(count: usize) -> String {
    format!(
        "{count} {}",
        if count == 1 { "message" } else { "messages" }
    )
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

    async fn pending_review(
        unread_only: bool,
    ) -> (
        App,
        Store,
        tokio::sync::mpsc::Receiver<Command>,
        String,
        Mail,
    ) {
        let store = Store::memory().expect("fixture store");
        let mail = (0..12)
            .map(|i| {
                parse_mail(
                    "work",
                    &i.to_string(),
                    "INBOX",
                    format!("From: fixture@example.test\r\nSubject: {i:03}\r\n\r\nBody")
                        .into_bytes(),
                    true,
                    false,
                )
                .expect("fixture mail")
            })
            .collect();
        store.upsert(mail).await.expect("fixture mail saved");
        let (sender, mut commands, _network) = engine::CommandSender::close_test_channels();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.query.sort = MailSort::Subject;
        app.query.unread_only = unread_only;
        app.set_mail_page(Arc::new(
            store.query(app.query.clone()).await.expect("page"),
        ));
        let ids: Vec<_> = app
            .page
            .rows
            .iter()
            .take(10)
            .map(|mail| mail.id.clone())
            .collect();
        let source = MailSelectionId::default();
        store
            .capture_selection(source, 0, app.query.clone(), false, ids.clone())
            .await
            .expect("capture");
        let snapshot = store
            .change_selection(
                source,
                0,
                crate::store::SelectionChange::Range {
                    anchor: ids[0].clone(),
                    target: ids[9].clone(),
                    additive: false,
                },
                ids.clone(),
            )
            .await
            .expect("ten selected");
        app.mail_selection.mode = true;
        app.mail_selection.count = 10;
        app.mail_selection.visible = ids.into_iter().collect();
        app.mail_selection.snapshot = Some(Arc::new(snapshot));
        app.toggle_mail_flag(app.page.rows[0].clone(), true);
        let Command::AdmitMail(request, sent, _, _) = commands.try_recv().expect("read admitted")
        else {
            panic!("Expected read action");
        };
        app.begin_bulk(Intent::Move {
            account: None,
            folder: "Trash".into(),
            foreign: false,
        });
        app.pump_bulk();
        let Command::ReviewSelection(serial, id, revision, visible) =
            commands.try_recv().expect("review does not await read")
        else {
            panic!("Expected local review");
        };
        let review = store
            .review_selection(id, revision, visible)
            .await
            .expect("review");
        app.bulk_event(Event::BulkReview(serial, Ok(Arc::new(review))));
        assert_eq!(app.mail_actions.pending(), 1);
        assert_eq!(
            app.bulk.review.as_ref().map(|review| review.selected),
            Some(10)
        );
        (app, store, commands, request, sent)
    }

    #[tokio::test]
    async fn review_and_delete_paint_before_prior_read_finishes_without_double_counting() {
        for unread_only in [false, true] {
            for success in [false, true] {
                let (mut app, store, mut commands, request, sent) =
                    pending_review(unread_only).await;
                let _ = app.handle_bulk(Message::Confirm);
                assert!(app.dialog.is_none());
                assert_eq!((app.page.total, app.page.unread), (2, 2));
                assert_eq!(app.page.inbox_unread.get("work"), Some(&2));
                assert_eq!(
                    app.action_toasts
                        .current
                        .as_ref()
                        .map(|toast| toast.label()),
                    Some("Deleted 10 messages".into())
                );
                let Command::BulkStart(id, selection, action) =
                    commands.try_recv().expect("immediate group admission")
                else {
                    panic!("Expected ordered local admission");
                };
                if success {
                    let job = store
                        .start_individual_mail_action(
                            request.clone(),
                            sent.clone(),
                            BulkAction::Flags(Flags {
                                unread: Some(false),
                                starred: None,
                            }),
                        )
                        .await
                        .expect("saved read");
                    app.mail_admitted(request, Ok(Arc::new(job)));
                } else {
                    app.mail_admitted(request, Err("Rejected read admission".into()));
                }
                assert_eq!((app.page.total, app.page.unread), (2, 2));
                assert_eq!(app.page.inbox_unread.get("work"), Some(&2));
                app.pump_bulk();
                let job = store
                    .start_bulk(id.clone(), selection, action)
                    .await
                    .expect("admitted");
                app.bulk_event(Event::BulkStarted(id, Ok(Arc::new(job))));
                app.set_mail_page(Arc::new(
                    store
                        .query(app.query.clone())
                        .await
                        .expect("projected page"),
                ));
                assert_eq!((app.page.total, app.page.unread), (2, 2));
                assert_eq!(app.page.inbox_unread.get("work"), Some(&2));
                assert!(app.bulk.prediction.is_none());
            }
        }
    }

    #[tokio::test]
    async fn undo_before_group_admission_queues_after_it_and_preserves_read() {
        let (mut app, store, mut commands, request, sent) = pending_review(false).await;
        let _ = app.handle_bulk(Message::Confirm);
        let id = app.bulk.staging.clone().expect("group staging");
        let token = app
            .bulk
            .tokens
            .iter()
            .find_map(|(token, job)| (job == &id).then_some(*token))
            .expect("group toast token");
        assert!(app.request_group_undo(id.clone()));
        assert_eq!(app.bulk.staging.as_ref(), Some(&id));
        assert!(app.bulk.undo.contains_key(&id));
        assert_eq!(app.mail_actions.pending(), 1, "read remains independent");
        assert_eq!((app.page.total, app.page.unread), (12, 11));
        let read = store
            .start_individual_mail_action(
                request.clone(),
                sent,
                BulkAction::Flags(Flags {
                    unread: Some(false),
                    starred: None,
                }),
            )
            .await
            .expect("read");
        app.mail_admitted(request, Ok(Arc::new(read)));
        let Command::BulkStart(started, selection, action) =
            commands.try_recv().expect("admission first")
        else {
            panic!("expected admission")
        };
        let forward = store
            .start_bulk(started.clone(), selection, action)
            .await
            .expect("group");
        let Command::BulkUndo(undo) = commands.try_recv().expect("undo follows admission") else {
            panic!("expected undo")
        };
        assert_eq!(started, undo);
        let restored = store
            .request_bulk_undo(undo.clone())
            .await
            .expect("cancel durable group");
        assert_eq!(restored.cancelled, 10);
        app.bulk_event(Event::BulkStarted(started, Ok(Arc::new(forward))));
        app.bulk_event(Event::BulkUpdate(Arc::new(restored)));
        let mut query = app.query.clone();
        query.observe_bulk = app.bulk_observed_ids();
        app.set_mail_page(Arc::new(store.query(query).await.expect("page")));
        assert_eq!((app.page.total, app.page.unread), (12, 11));
        app.undo_combined_actions(vec![token]);
    }

    #[tokio::test]
    async fn rejected_group_admission_restores_rows_and_preserves_successful_read() {
        let (mut app, store, mut commands, request, sent) = pending_review(false).await;
        let _ = app.handle_bulk(Message::Confirm);
        let read = store
            .start_individual_mail_action(
                request.clone(),
                sent,
                BulkAction::Flags(Flags {
                    unread: Some(false),
                    starred: None,
                }),
            )
            .await
            .expect("read");
        app.mail_admitted(request, Ok(Arc::new(read)));
        let Command::BulkStart(id, ..) = commands.try_recv().expect("group admission") else {
            panic!("expected admission")
        };
        app.bulk_event(Event::BulkStarted(
            id,
            Err("Fixture storage refused".into()),
        ));
        assert!(app.bulk.staging.is_none());
        assert!(app.bulk.prediction.is_none());
        assert_eq!((app.page.total, app.page.unread), (12, 11));
        assert!(app.action_toasts.current.is_none());
        assert!(app.notice.as_ref().is_some_and(|(_, error, _)| *error));
    }

    fn job(id: &str, revision: u64, remaining: usize) -> Arc<Job> {
        Arc::new(Job {
            id: id.into(),
            action: BulkAction::Flags(Flags {
                unread: Some(false),
                starred: None,
            }),
            undo_requested: false,
            paused: false,
            total: 2,
            remaining,
            running: usize::from(remaining > 0),
            completed: 2 - remaining,
            restored: 0,
            failed: 0,
            uncertain: 0,
            cancelled: 0,
            revision,
        })
    }

    #[tokio::test]
    async fn history_pages_keep_active_jobs_and_do_not_reorder_on_worker_updates() {
        let (mut app, _) = App::new();
        app.bulk.history_serial = 4;
        app.dialog = Some(Dialog::BulkHistory);
        app.remember_job(job("active", 5, 1));
        app.bulk_event(Event::BulkJobs(
            4,
            Ok(Arc::new(vec![(*job("older", 1, 0)).clone()])),
        ));
        assert_eq!(app.bulk.jobs[0].id, "active");
        assert_eq!(app.bulk.history_jobs[0].id, "older");
        app.remember_job(job("active", 6, 0));
        assert_eq!(
            app.bulk.history_jobs[0].id, "older",
            "An unrelated update must not jump an older History page"
        );
        app.bulk_event(Event::BulkJobs(
            4,
            Ok(Arc::new(vec![(*job("active", 2, 2)).clone()])),
        ));
        assert_eq!(
            app.bulk.history_jobs[0].revision, 6,
            "A late History read must retain newer receipts"
        );
    }

    #[tokio::test]
    async fn close_before_the_engine_is_ready_does_not_wait_for_a_missing_worker() {
        let (mut app, _) = App::new();
        let task = app.handle(super::super::Message::WindowClose(
            iced::window::Id::unique(),
        ));
        assert!(app.pending_close.is_none());
        assert_eq!(task.units(), 1);
        assert!(!app.bulk.stop_requested);
    }

    #[tokio::test]
    async fn window_close_quiesces_the_worker_even_without_a_running_job_on_the_ui_page() {
        let (sender, mut commands) = engine::CommandSender::selection_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        let window = iced::window::Id::unique();
        let _ = app.handle(super::super::Message::WindowClose(window));
        assert_eq!(app.pending_close, Some(window));
        app.pump_bulk();
        assert!(matches!(commands.try_recv(), Ok(Command::BulkStop(1))));
        assert!(app.bulk.stop_requested);
        let _ = app.handle(super::super::Message::Backend(Event::BulkStopped(1)));
        assert!(app.bulk.stopped);
        assert!(app.pending_close.is_none());
    }

    #[tokio::test]
    async fn resolution_accepts_confirmation_keys_and_cannot_leak_into_a_reopened_history() {
        let (sender, mut commands) = engine::CommandSender::selection_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        app.dialog = Some(Dialog::BulkHistory);
        for key in [
            Key::Named(keyboard::key::Named::Escape),
            Key::Character("n".into()),
        ] {
            app.bulk.resolving = Some("unconfirmed".into());
            let _ = app.key(key, keyboard::Modifiers::default(), false);
            assert!(app.bulk.resolving.is_none());
            assert_eq!(app.dialog, Some(Dialog::BulkHistory));
            assert!(commands.try_recv().is_err());
        }
        for key in [
            Key::Named(keyboard::key::Named::Enter),
            Key::Character("y".into()),
        ] {
            app.bulk.resolving = Some("unconfirmed".into());
            let _ = app.key(key, keyboard::Modifiers::default(), false);
            assert!(app.bulk.resolving.is_none());
            assert!(
                matches!(commands.try_recv(),Ok(Command::BulkResolve(id)) if id=="unconfirmed")
            );
        }
        app.bulk.resolving = Some("unconfirmed".into());
        app.bulk.selected_job = Some("unconfirmed".into());
        let _ = app.handle(super::super::Message::Close);
        assert!(app.bulk.resolving.is_none());
        let _ = app.handle_bulk(Message::History);
        assert!(app.bulk.selected_job.is_none());
        assert!(!app.bulk_confirming());
    }

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
            observed: frozen.observed.clone(),
            projection: shep_action_core::Projection::Pending,
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
