use super::*;
use crate::engine::CalendarActionResult;
use shep_action_core::{Projection, Status};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct State {
    pub base: Arc<Vec<CalendarEvent>>,
    serial: u64,
    pending: BTreeMap<u64, Change>,
    journal_revision: u64,
    journal_ids: std::collections::HashSet<String>,
}

struct Change {
    id: String,
    job: Option<Arc<crate::store::CalendarJob>>,
    resolving: bool,
    event: CalendarEvent,
    deleting: bool,
    projection: Projection,
    review_after: u64,
    inspecting: bool,
    verified: Option<u64>,
    observed_key: Option<String>,
    error: Option<String>,
}

#[cfg(test)]
mod tests;

impl State {
    #[cfg(feature = "test-support")]
    pub fn observation(&self) -> serde_json::Value {
        serde_json::json!(
            self.pending
                .iter()
                .map(|(request, change)| serde_json::json!({
                    "request": request, "title": change.event.title, "deleting": change.deleting,
                    "admitted": change.job.is_some(), "status": change.job.as_ref().map(|job|job.status.as_str()),
                    "acknowledged": matches!(change.projection, Projection::Committed { .. } | Projection::Repair), "error": change.error,
                }))
                .collect::<Vec<_>>()
        )
    }

    pub fn has_changes(&self) -> bool {
        !self.pending.is_empty()
    }

    pub fn owns_source(&self, source: &str) -> bool {
        self.pending
            .values()
            .any(|change| change.event.source_id == source)
    }

    #[cfg(test)]
    pub fn needs_review(&self) -> bool {
        self.pending
            .values()
            .any(|change| change.projection.needs_review())
    }

    pub fn needs_flush(&self) -> bool {
        self.pending
            .values()
            .any(|change| change.job.is_none() && change.error.is_none() || change.resolving)
    }

    pub fn unsaved_review(&self) -> bool {
        self.pending
            .values()
            .any(|change| change.job.is_none() && change.error.is_some())
    }
}

impl App {
    pub(super) fn begin_calendar_action(&mut self, event: CalendarEvent, deleting: bool) {
        if self.bulk.stopped {
            if !self.try_command(Command::BulkResume(String::new())) {
                return;
            }
            self.bulk.stopped = false;
            self.bulk.stop_requested = false;
        }
        if self.calendar_actions.pending.len() >= CHANNEL_CAPACITY {
            self.notice(
                "Finish or review a calendar change before starting another.",
                true,
            );
            return;
        }
        let request = self.calendar_actions.serial + 1;
        let id = uuid::Uuid::new_v4().to_string();
        if !self.try_command(Command::AdmitCalendarAction(
            request,
            id.clone(),
            event.clone(),
            deleting,
        )) {
            return;
        }
        self.calendar_actions.pending.retain(|_, change| {
            change.job.is_some()
                || change.event.key() != event.key()
                || !change.projection.can_retry()
        });
        self.calendar_actions.serial = request;
        self.calendar_actions.pending.insert(
            request,
            Change {
                id,
                job: None,
                resolving: false,
                event,
                deleting,
                projection: Projection::Pending,
                review_after: self.events_revision,
                inspecting: false,
                verified: None,
                observed_key: None,
                error: None,
            },
        );
        self.dialog = None;
        self.editing_event = None;
        self.focused_input = None;
        self.pending_focus = None;
        self.project_calendar_actions();
    }

    pub(super) fn calendar_admitted(
        &mut self,
        request: u64,
        result: Result<Arc<crate::store::CalendarJob>, String>,
    ) {
        match result {
            Ok(job) => {
                if job.revision <= self.calendar_actions.journal_revision
                    && !self.calendar_actions.journal_ids.contains(&job.id)
                {
                    self.calendar_actions.pending.remove(&request);
                    self.project_calendar_actions();
                    return;
                }
                self.remember_calendar_job(job);
                if self.pending_close.is_none() {
                    self.send(Command::BulkRun(String::new()));
                }
            }
            Err(error) => {
                if let Some(change) = self.calendar_actions.pending.get_mut(&request) {
                    change.projection = Projection::Rejected;
                    change.error = Some(format!("Could not save locally. {error}"));
                }
                self.pending_close = None;
            }
        }
        self.project_calendar_actions();
    }

    fn remember_calendar_job(&mut self, job: Arc<crate::store::CalendarJob>) {
        let existing = self
            .calendar_actions
            .pending
            .iter()
            .find(|(_, change)| change.id == job.id)
            .map(|(request, _)| *request);
        if job.revision <= self.calendar_actions.journal_revision
            && !self.calendar_actions.journal_ids.contains(&job.id)
        {
            if let Some(request) = existing {
                self.calendar_actions.pending.remove(&request);
            }
            return;
        }
        if existing.is_some_and(|request| {
            self.calendar_actions.pending[&request]
                .job
                .as_ref()
                .is_some_and(|old| old.revision > job.revision)
        }) {
            return;
        }
        let Some(status) = Status::parse(&job.status) else {
            return;
        };
        if status.is_finished() {
            if let Some(request) = existing {
                self.calendar_actions.pending.remove(&request);
            }
            return;
        }
        let request = existing.unwrap_or_else(|| {
            self.calendar_actions.serial += 1;
            self.calendar_actions.serial
        });
        let unchanged = self
            .calendar_actions
            .pending
            .get(&request)
            .filter(|change| {
                change
                    .job
                    .as_ref()
                    .is_some_and(|old| old.revision == job.revision)
            });
        let inspecting = unchanged.is_some_and(|change| change.inspecting);
        let resolving = unchanged.is_some_and(|change| change.resolving);
        let projection = match status {
            Status::Repair => Projection::Repair,
            Status::Rejected => Projection::Rejected,
            Status::Uncertain => Projection::Uncertain,
            _ => Projection::Pending,
        };
        let event = job.receipt.as_ref().unwrap_or(&job.event).clone();
        self.calendar_actions.pending.insert(
            request,
            Change {
                id: job.id.clone(),
                event,
                deleting: job.deleting,
                projection,
                review_after: 0,
                inspecting,
                resolving,
                verified: job.checked.then_some(self.events_revision),
                observed_key: job.observed.as_ref().map(CalendarEvent::key),
                error: if job.checked {
                    Some(job.observed.as_ref().map_or_else(
                        || "The server confirms this event is absent. Use server state to accept its removal.".into(),
                        |event| format!("Server currently shows “{}”. Use server state to accept that event.",event.title)))
                } else { job.error.clone().or_else(|| {
                    status
                        .needs_review()
                        .then(|| "This calendar change needs checking.".into())
                }) },
                job: Some(job),
            },
        );
    }

    pub(super) fn calendar_journal(
        &mut self,
        revision: u64,
        journal_revision: u64,
        events: Arc<Vec<CalendarEvent>>,
        jobs: Arc<Vec<crate::store::CalendarJob>>,
    ) {
        if revision >= self.events_revision {
            self.events_revision = revision;
            self.calendar_actions.base = events;
        }
        if journal_revision >= self.calendar_actions.journal_revision {
            self.calendar_actions.pending.retain(|_, change| {
                change
                    .job
                    .as_ref()
                    .is_none_or(|job| job.revision > journal_revision)
                    || jobs.iter().any(|job| job.id == change.id)
            });
            for job in jobs.iter() {
                self.remember_calendar_job(Arc::new(job.clone()));
            }
            self.calendar_actions.journal_revision = journal_revision;
            self.calendar_actions.journal_ids = jobs.iter().map(|job| job.id.clone()).collect();
        }
        self.project_calendar_actions();
    }

    pub(super) fn calendar_job_update(
        &mut self,
        id: String,
        result: Result<Arc<crate::store::CalendarJob>, String>,
    ) {
        match result {
            Ok(job) => {
                let ready = job.status == "queued"
                    || Status::parse(&job.status).is_some_and(Status::is_finished);
                self.remember_calendar_job(job);
                if ready && self.pending_close.is_none() {
                    self.send(Command::BulkRun(String::new()));
                }
            }
            Err(error) => {
                if let Some(change) = self
                    .calendar_actions
                    .pending
                    .values_mut()
                    .find(|change| change.id == id)
                {
                    change.inspecting = false;
                    change.resolving = false;
                    change.error = Some(error.clone());
                }
                self.notice(error, true);
            }
        }
        self.project_calendar_actions();
    }

    pub(super) fn project_calendar_actions(&mut self) {
        let base = &self.calendar_actions.base;
        self.calendar_actions.pending.retain(|_, change| {
            if change.job.is_none()
                && change.projection == Projection::Repair
                && self.events_revision > change.review_after
            {
                let cached = base.iter().find(|event| event.key() == change.event.key());
                let repaired = if change.deleting {
                    cached.is_none()
                } else {
                    cached.is_some_and(|event| same_cached_event(event, &change.event))
                };
                if repaired {
                    return false;
                }
            }
            change.job.is_some() || !change.projection.observed_at(self.events_revision)
        });
        let mut events = (*self.calendar_actions.base).clone();
        let mut identities =
            std::collections::HashMap::<String, std::collections::HashSet<String>>::new();
        let mut by_key = std::collections::HashMap::<String, String>::new();
        for change in self.calendar_actions.pending.values() {
            let previous = change
                .job
                .as_ref()
                .and_then(|job| job.previous.as_ref())
                .or_else(|| by_key.get(&change.event.key()));
            let mut keys = previous
                .and_then(|id| identities.get(id))
                .cloned()
                .unwrap_or_default();
            keys.insert(change.event.key());
            if let Some(job) = &change.job {
                keys.insert(job.origin.clone());
            }
            if let Some(key) = &change.observed_key {
                keys.insert(key.clone());
            }
            for key in &keys {
                by_key.insert(key.clone(), change.id.clone());
            }
            identities.insert(change.id.clone(), keys.clone());
            if !change.projection.is_visible() {
                continue;
            }
            events.retain(|event| !keys.contains(&event.key()));
            if !change.deleting {
                events.push(change.event.clone());
            }
        }
        self.events = Arc::new(events);
    }

    pub(super) fn calendar_action_finished(&mut self, request: u64, result: CalendarActionResult) {
        let Some(change) = self.calendar_actions.pending.get_mut(&request) else {
            return;
        };
        match result {
            CalendarActionResult::Applied {
                event,
                revision,
                warning,
            } => {
                change.event = *event;
                change.projection = Projection::acknowledge(revision);
                change.error = warning;
            }
            CalendarActionResult::Rejected(error) => {
                change.projection = Projection::Rejected;
                change.error = Some(format!("Not saved. {error}"));
            }
            CalendarActionResult::Uncertain(error) => {
                change.projection = Projection::Uncertain;
                change.review_after = self.events_revision;
                change.error = Some(format!(
                    "Needs checking. Refresh and check the server before using its state. {error}"
                ));
            }
        }
        if let Some(error) = change.error.clone() {
            self.pending_close = None;
            self.notice(error, true);
        }
        self.project_calendar_actions();
    }

    pub(super) fn calendar_recovery_event(&self, key: &str) -> Option<CalendarEvent> {
        self.calendar_actions
            .pending
            .values()
            .find(|change| change.event.key() == key && change.error.is_some())
            .map(|change| change.event.clone())
    }

    pub(super) fn calendar_rejected_create(&self, event: &CalendarEvent) -> bool {
        event.etag.is_none()
            && event.remote_url.is_none()
            && self
                .calendar_actions
                .pending
                .values()
                .any(|change| change.event.key() == event.key() && change.projection.can_retry())
    }

    pub(super) fn retry_calendar_change(&mut self, request: u64) {
        let Some(change) = self.calendar_actions.pending.get(&request) else {
            return;
        };
        let Some(job) = change
            .job
            .as_ref()
            .filter(|job| job.status == "waiting" && !change.resolving)
        else {
            return;
        };
        if self.try_command(Command::RetryCalendarJob(job.id.clone(), job.revision))
            && let Some(change) = self.calendar_actions.pending.get_mut(&request)
        {
            change.resolving = true;
        }
    }

    pub(super) fn inspect_calendar_change(&mut self, request: u64) {
        let Some(change) = self.calendar_actions.pending.get(&request) else {
            return;
        };
        if change.inspecting
            || !matches!(
                change.projection,
                Projection::Uncertain | Projection::Repair
            )
        {
            return;
        }
        let command = change.job.as_ref().map_or_else(
            || Command::InspectCalendarAction(request, change.event.clone()),
            |job| Command::CheckCalendarJob(job.id.clone(), job.revision),
        );
        if !self.try_command(command) {
            return;
        }
        if let Some(change) = self.calendar_actions.pending.get_mut(&request) {
            change.inspecting = true;
            change.verified = None;
        }
    }

    pub(super) fn calendar_action_observed(
        &mut self,
        request: u64,
        result: Result<engine::CalendarObservation, String>,
    ) {
        let Some(change) = self
            .calendar_actions
            .pending
            .get_mut(&request)
            .filter(|change| change.inspecting)
        else {
            return;
        };
        change.inspecting = false;
        match result {
            Ok(observed) => {
                change.verified = Some(observed.revision);
                change.observed_key = observed.current.as_ref().map(CalendarEvent::key);
                change.error = Some(match observed.current {
                    Some(event) => format!("Server currently shows “{}”. Use server state to accept that event.", event.title),
                    None => "The server confirms this event is absent. Use server state to accept its removal.".into(),
                });
                if observed.revision >= self.events_revision {
                    self.events_revision = observed.revision;
                    self.calendar_actions.base = observed.events;
                }
            }
            Err(error) => {
                change.error = Some(format!(
                    "Could not check the server. Your change is retained. {error}"
                ));
                self.pending_close = None;
            }
        }
        self.project_calendar_actions();
    }

    fn can_dismiss_calendar_change(&self, request: u64) -> bool {
        let Some(change) = self.calendar_actions.pending.get(&request) else {
            return false;
        };
        if change.resolving {
            return false;
        }
        if let Some(job) = &change.job {
            return !change.inspecting
                && (matches!(job.status.as_str(), "rejected" | "waiting")
                    || job.checked && matches!(job.status.as_str(), "uncertain" | "repair"));
        }
        if change.projection.can_retry() {
            return true;
        }
        matches!(
            change.projection,
            Projection::Uncertain | Projection::Repair
        ) && !change.inspecting
            && change.verified.is_some_and(|revision| {
                revision > change.review_after && self.events_revision >= revision
            })
    }

    pub(super) fn dismiss_calendar_change(&mut self, request: u64) {
        if !self.can_dismiss_calendar_change(request) {
            return;
        }
        if let Some(job) = self
            .calendar_actions
            .pending
            .get(&request)
            .and_then(|change| change.job.clone())
        {
            if self.try_command(Command::ResolveCalendarJob(job.id.clone(), job.revision))
                && let Some(change) = self.calendar_actions.pending.get_mut(&request)
            {
                change.resolving = true;
            }
            return;
        }
        self.calendar_actions.pending.remove(&request);
        self.project_calendar_actions();
    }

    pub(super) fn calendar_changes_view(&self) -> Element<'_, Message> {
        let mut changes = widget::column![].spacing(8);
        for (request, change) in &self.calendar_actions.pending {
            let status = change.error.as_deref().unwrap_or_else(|| {
                match change.job.as_ref().map(|job| job.status.as_str()) {
                    Some("queued") => "Waiting to sync…",
                    Some("running") => "Saving on the server…",
                    _ => "Saving calendar change…",
                }
            });
            let mut controls = widget::row![
                widget::text(format!("{}: {status}", change.event.title))
                    .size(12)
                    .width(iced::Length::Fill)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);
            if change.error.is_some() {
                if change
                    .job
                    .as_ref()
                    .is_some_and(|job| job.status == "waiting")
                {
                    controls = controls.push(
                        widget::button(widget::text("Retry").size(12))
                            .padding([10, 14])
                            .style(components::outline)
                            .on_press_maybe(
                                (!change.resolving)
                                    .then_some(Message::RetryCalendarChange(*request)),
                            ),
                    );
                }
                controls = controls.push(components::action(
                    "Review",
                    Message::EditEvent(change.event.key()),
                ));
                if matches!(
                    change.projection,
                    Projection::Repair | Projection::Uncertain
                ) {
                    controls = controls.push(
                        widget::button(
                            widget::text(if change.inspecting {
                                "Checking…"
                            } else {
                                "Check server"
                            })
                            .size(12),
                        )
                        .padding([10, 14])
                        .style(components::outline)
                        .on_press_maybe(
                            (!change.inspecting)
                                .then_some(Message::InspectCalendarChange(*request)),
                        ),
                    );
                }
                controls = controls.push(
                    widget::button(
                        widget::text(
                            if change
                                .job
                                .as_ref()
                                .is_some_and(|job| job.status == "waiting")
                            {
                                "Cancel change"
                            } else if change.projection.is_visible() {
                                "Use server state"
                            } else {
                                "Dismiss"
                            },
                        )
                        .size(12),
                    )
                    .padding([10, 14])
                    .style(components::outline)
                    .on_press_maybe(
                        self.can_dismiss_calendar_change(*request)
                            .then_some(Message::DismissCalendarChange(*request)),
                    ),
                );
            }
            changes = changes.push(controls);
        }
        widget::container(widget::scrollable(changes))
            .height(iced::Length::Shrink)
            .max_height(140)
            .into()
    }
}

fn same_cached_event(a: &CalendarEvent, b: &CalendarEvent) -> bool {
    a.key() == b.key()
        && a.title == b.title
        && a.start == b.start
        && a.end == b.end
        && a.location == b.location
        && a.description == b.description
        && a.all_day == b.all_day
        && a.etag == b.etag
        && a.remote_url == b.remote_url
}
