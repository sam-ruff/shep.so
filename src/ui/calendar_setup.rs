use super::*;
use crate::providers::calendar::discovery::DiscoveredCalendar;
use iced::{
    Alignment, Length,
    widget::{button, checkbox, column, container, row, scrollable, space, text},
};

#[derive(Default)]
pub(super) struct CalendarSetup {
    pub generation: u64,
    pub discovering: bool,
    pub saving: Option<u64>,
    pub choices: Vec<DiscoveredCalendar>,
    pub selected: HashSet<String>,
    pub error: Option<String>,
}
impl CalendarSetup {
    pub fn invalidate(&mut self) {
        self.generation += 1;
        self.discovering = false;
        self.choices.clear();
        self.selected.clear();
        self.error = None;
    }
}
impl App {
    pub(super) fn discover_calendars(&mut self) {
        if self.calendar_setup.saving.is_some() || self.calendar_setup.discovering {
            return;
        }
        self.calendar_setup.invalidate();
        let request = self.calendar_setup.generation;
        let command = Command::DiscoverCalendars(
            request,
            self.field("url").trim().into(),
            self.field("username").trim().into(),
            self.field("password").to_string().into(),
        );
        if self.try_command(command) {
            self.calendar_setup.discovering = true;
        }
    }
    pub(super) fn calendars_discovered(
        &mut self,
        request: u64,
        result: Result<Vec<DiscoveredCalendar>, String>,
    ) {
        if request != self.calendar_setup.generation || self.dialog != Some(Dialog::Calendar) {
            return;
        }
        self.calendar_setup.discovering = false;
        match result {
            Ok(choices) => {
                self.calendar_setup.selected = choices.iter().map(|c| c.url.clone()).collect();
                self.calendar_setup.choices = choices;
            }
            Err(error) => self.calendar_setup.error = Some(error),
        }
    }
    pub(super) fn connect_calendars(&mut self) {
        if self.calendar_setup.saving.is_some() || self.calendar_setup.discovering {
            return;
        }
        let sources: Vec<_> = self
            .calendar_setup
            .choices
            .iter()
            .filter(|c| self.calendar_setup.selected.contains(&c.url))
            .map(|c| CalendarSource {
                id: String::new(),
                name: c.name.clone(),
                kind: CalendarKind::CalDav,
                url: c.url.clone(),
                username: self.field("username").trim().into(),
                access: c.access,
            })
            .collect();
        if sources.is_empty() {
            return;
        }
        let request = self.calendar_setup.generation;
        if self.try_command(Command::ConnectCalendars(
            request,
            self.workspace.connections_revision,
            sources,
            self.field("password").to_string().into(),
        )) {
            self.calendar_setup.saving = Some(request);
            self.calendar_setup.error = None;
        }
    }
    pub(super) fn calendars_connected(&mut self, request: u64, result: Result<(), String>) {
        if self.calendar_setup.saving != Some(request) {
            return;
        }
        self.calendar_setup.saving = None;
        let current =
            request == self.calendar_setup.generation && self.dialog == Some(Dialog::Calendar);
        match result {
            Ok(()) => {
                if current {
                    self.dialog = None;
                    self.fields.clear();
                    if self.tab == Tab::Preferences {
                        self.settings_fields();
                    }
                    self.calendar_setup.invalidate();
                }
                self.notice("Calendars connected.", false);
                self.send(Command::SyncCalendar);
            }
            Err(error) => {
                self.pending_close = None;
                self.composer.close = None;
                if current {
                    self.calendar_setup.error = Some(error);
                } else {
                    self.notice(
                        format!("Could not finish connecting calendars: {error}"),
                        true,
                    );
                }
            }
        }
    }
    pub(super) fn calendar_connection_form(&self) -> Element<'_, Message> {
        let setup = &self.calendar_setup;
        let saving = setup.saving.is_some();
        let mut form = column![].spacing(16);
        if setup.choices.is_empty() {
            for (label, placeholder, key, secure) in [
                (
                    "Server or calendar URL",
                    "https://cloud.example.com",
                    "url",
                    false,
                ),
                ("Username", "Your calendar username", "username", false),
                (
                    "Password / app password",
                    "Your server password",
                    "password",
                    true,
                ),
            ] {
                let field = iced::widget::text_input(placeholder, self.field(key))
                    .padding(12)
                    .size(12)
                    .style(components::field)
                    .secure(secure)
                    .on_input_maybe((!saving).then_some(move |v| Message::Field(key, v)));
                form = form.push(column![text(label).size(12).font(BOLD), field].spacing(8));
            }
            form = form.push(
                button(
                    text(if setup.discovering {
                        "Finding calendars…"
                    } else {
                        "Find calendars"
                    })
                    .size(12),
                )
                .padding([12, 18])
                .style(primary)
                .on_press_maybe(
                    (!setup.discovering && !saving).then_some(Message::DiscoverCalendars),
                ),
            );
        } else {
            let mut choices = column![].spacing(8);
            for calendar in &setup.choices {
                let url = calendar.url.clone();
                let access = if calendar.access.read_only() {
                    "Read only"
                } else if calendar.access == CalendarAccess::default() {
                    "Can edit"
                } else {
                    "Limited editing"
                };
                choices = choices.push(
                    container(
                        column![
                            row![
                                checkbox(setup.selected.contains(&calendar.url))
                                    .label(&calendar.name)
                                    .on_toggle_maybe((!saving).then_some(move |_| {
                                        Message::ChooseCalendar(url.clone())
                                    })),
                                space().width(Length::Fill),
                                muted(access).size(11)
                            ]
                            .spacing(12)
                            .align_y(Alignment::Center),
                            muted(&calendar.url).size(10),
                        ]
                        .spacing(6),
                    )
                    .padding(12)
                    .width(Length::Fill)
                    .style(card),
                );
            }
            form = form
                .push(text("Choose calendars to sync").font(BOLD).size(15))
                .push(scrollable(choices).height((setup.choices.len() as f32 * 80.).min(280.)))
                .push(
                    row![
                        button(text("Back").size(12))
                            .padding([12, 18])
                            .style(outline)
                            .on_press_maybe((!saving).then_some(Message::CalendarBack)),
                        space().width(Length::Fill),
                        button(
                            text(if saving {
                                "Connecting…".to_string()
                            } else {
                                format!("Connect {}", setup.selected.len())
                            })
                            .size(12)
                        )
                        .padding([12, 18])
                        .style(primary)
                        .on_press_maybe(
                            (!saving && !setup.selected.is_empty())
                                .then_some(Message::SaveCalendar)
                        )
                    ]
                    .spacing(12),
                );
        }
        if let Some(error) = &setup.error {
            form = form.push(container(text(error).size(12)).padding(12).style(subtle));
        }
        form.into()
    }
    pub(super) fn event_access(&self) -> CalendarAccess {
        let Some(source) = self
            .workspace
            .calendars
            .iter()
            .find(|s| s.id == self.field("source"))
        else {
            return CalendarAccess::READ_ONLY;
        };
        if self
            .editing_event
            .as_ref()
            .is_some_and(|event| source.kind == CalendarKind::CalDav && event.remote_url.is_none())
        {
            return CalendarAccess::READ_ONLY;
        }
        source.access
    }
    pub(super) fn read_only_event(&self) -> Element<'_, Message> {
        let Some(event) = &self.editing_event else {
            return space().into();
        };
        let source = self
            .workspace
            .calendars
            .iter()
            .find(|s| s.id == event.source_id);
        let date = if event.all_day {
            {
                let last = (event.end - chrono::Duration::days(1)).date_naive();
                if last == event.start.date_naive() {
                    format!("{} · all day", event.start.format("%d %b %Y"))
                } else {
                    format!(
                        "{} – {} · all day",
                        event.start.format("%d %b %Y"),
                        last.format("%d %b %Y")
                    )
                }
            }
        } else {
            format!(
                "{} – {}",
                event
                    .start
                    .with_timezone(&chrono::Local)
                    .format("%d %b %Y · %H:%M"),
                event
                    .end
                    .with_timezone(&chrono::Local)
                    .format("%d %b %Y · %H:%M")
            )
        };
        let mut body = column![
            text(&event.title).size(21).font(BOLD),
            muted(source.map(|s| s.name.as_str()).unwrap_or("Calendar")),
            text(date).size(13)
        ]
        .spacing(16);
        if !event.location.is_empty() {
            body = body.push(text(&event.location).size(13));
        }
        if !event.description.is_empty() {
            body = body.push(text(&event.description).size(13));
        }
        body = body.push(
            muted(if source.is_some_and(|s| s.access.read_only()) {
                "Read-only calendar"
            } else {
                "Editing is not available for this event."
            })
            .size(12),
        );
        if self.event_access().delete {
            body = body.push(action("Delete event", Message::DeleteEvent));
        }
        body.push(action("Close", Message::Close)).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn calendar() -> DiscoveredCalendar {
        DiscoveredCalendar {
            name: "Home".into(),
            url: "https://calendar.example.test/personal/".into(),
            access: Default::default(),
        }
    }
    #[test]
    fn calendar_discovery_rejects_results_after_credential_edits_or_form_changes() {
        let (mut app, _) = App::new();
        app.open(Dialog::Calendar);
        let request = app.calendar_setup.generation;
        let _ = app.handle(Message::Field("username", "different".into()));
        app.calendars_discovered(request, Ok(vec![calendar()]));
        assert!(app.calendar_setup.choices.is_empty());
        let request = app.calendar_setup.generation;
        app.calendars_discovered(request, Ok(vec![calendar()]));
        assert_eq!(app.calendar_setup.selected.len(), 1);
        app.open(Dialog::Calendar);
        app.calendars_discovered(request, Err("Old sign-in error".into()));
        assert!(app.calendar_setup.error.is_none());
        assert!(app.calendar_setup.choices.is_empty());
    }
    #[test]
    fn late_calendar_connection_does_not_close_another_editor() {
        let (mut app, _) = App::new();
        app.open(Dialog::Calendar);
        let request = app.calendar_setup.generation;
        app.calendar_setup.saving = Some(request);
        app.open(Dialog::Event);
        let _ = app.handle(Message::Field("title", "Keep this event".into()));
        app.calendars_connected(request, Ok(()));
        assert_eq!(app.dialog, Some(Dialog::Event));
        assert_eq!(app.field("title"), "Keep this event");
        assert!(app.calendar_setup.saving.is_none());
    }
}
