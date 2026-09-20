use super::*;
use crate::engine::CalendarActionResult;
use shep_action_core::Projection;
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct State {
    pub base: Arc<Vec<CalendarEvent>>,
    serial: u64,
    pending: BTreeMap<u64, Change>,
}

struct Change {
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

    pub fn needs_review(&self) -> bool {
        self.pending
            .values()
            .any(|change| change.projection.needs_review())
    }
}

impl App {
    pub(super) fn begin_calendar_action(&mut self, event: CalendarEvent, deleting: bool) {
        if self.calendar_actions.pending.len() >= CHANNEL_CAPACITY {
            self.notice(
                "Finish or review a calendar change before starting another.",
                true,
            );
            return;
        }
        if self
            .calendar_actions
            .pending
            .values()
            .any(|change| change.event.key() == event.key() && !change.projection.can_retry())
        {
            self.notice(
                "This event has a pending change. Review its calendar status before saving again.",
                true,
            );
            return;
        }
        let request = self.calendar_actions.serial + 1;
        if !self.try_command(Command::CalendarAction(request, event.clone(), deleting)) {
            return;
        }
        self.calendar_actions
            .pending
            .retain(|_, change| change.event.key() != event.key());
        self.calendar_actions.serial = request;
        self.calendar_actions.pending.insert(
            request,
            Change {
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

    pub(super) fn project_calendar_actions(&mut self) {
        let base = &self.calendar_actions.base;
        self.calendar_actions.pending.retain(|_, change| {
            if change.projection == Projection::Repair && self.events_revision > change.review_after
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
            !change.projection.observed_at(self.events_revision)
        });
        let mut events = (*self.calendar_actions.base).clone();
        for change in self
            .calendar_actions
            .pending
            .values()
            .filter(|change| change.projection.is_visible())
        {
            events.retain(|event| {
                event.key() != change.event.key()
                    && change.observed_key.as_ref() != Some(&event.key())
            });
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
        if !self.try_command(Command::InspectCalendarAction(
            request,
            change.event.clone(),
        )) {
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
        self.calendar_actions.pending.remove(&request);
        self.project_calendar_actions();
    }

    pub(super) fn calendar_changes_view(&self) -> Element<'_, Message> {
        let mut changes = widget::column![].spacing(8);
        for (request, change) in &self.calendar_actions.pending {
            let status = change.error.as_deref().unwrap_or("Saving calendar change…");
            let mut controls = widget::row![
                widget::text(format!("{}: {status}", change.event.title))
                    .size(12)
                    .width(iced::Length::Fill)
            ]
            .spacing(8)
            .align_y(iced::Alignment::Center);
            if change.error.is_some() {
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
                        widget::text(if change.projection.is_visible() {
                            "Use server state"
                        } else {
                            "Dismiss"
                        })
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
