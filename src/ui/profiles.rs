use super::*;
use crate::profiles::{Id, Request, Snapshot};
use iced::{
    Alignment, Length,
    widget::{button, column, row, space, text, text_input},
};

#[derive(Debug, Clone)]
pub enum Action {
    Refresh,
    Page(bool),
    Rename(Id),
    Name(String),
    SaveName,
    CancelName,
    Activate(Id),
}
#[derive(Default)]
pub(super) struct State {
    pub snapshot: Option<Arc<Snapshot>>,
    serial: u64,
    busy: bool,
    changing: bool,
    offset: u64,
    editing: Option<(Id, String)>,
    error: Option<String>,
}
impl State {
    pub fn changing(&self) -> bool {
        self.busy && self.changing
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn observation(&self) -> serde_json::Value {
        serde_json::json!({"busy":self.busy,"current":self.snapshot.as_ref().map(|s|s.current.id.key()),"next":self.snapshot.as_ref().map(|s|s.page.active.key()),"total":self.snapshot.as_ref().map(|s|s.page.total),"offset":self.offset,"editing":self.editing.as_ref().map(|(id,name)|serde_json::json!({"id":id.key(),"name":name})),"rows":self.snapshot.as_ref().map(|s|s.page.rows.iter().map(|p|serde_json::json!({"id":p.id.key(),"name":p.name,"ready":p.ready})).collect::<Vec<_>>()),"error":self.error})
    }
}
impl App {
    pub(super) fn profile_action(&mut self, action: Action) {
        if self.profiles.busy {
            return;
        }
        let state = &mut self.profiles;
        let mut changing = false;
        let request = match action {
            Action::Refresh => Request::List {
                offset: state.offset,
            },
            Action::Page(next) => {
                if next
                    && state
                        .snapshot
                        .as_ref()
                        .is_some_and(|s| state.offset + 50 < s.page.total)
                {
                    state.offset += 50;
                } else if !next {
                    state.offset = state.offset.saturating_sub(50);
                }
                Request::List {
                    offset: state.offset,
                }
            }
            Action::Rename(id) => {
                if let Some(profile) = state
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.page.rows.iter().find(|p| p.id == id))
                {
                    state.editing = Some((id, profile.name.clone()));
                }
                return;
            }
            Action::Name(name) => {
                if let Some((_, current)) = &mut state.editing {
                    *current = name;
                }
                return;
            }
            Action::CancelName => {
                state.editing = None;
                return;
            }
            Action::SaveName => {
                let Some((id, name)) = &state.editing else {
                    return;
                };
                let Some(snapshot) = &state.snapshot else {
                    return;
                };
                if let Err(error) = crate::profiles::name_checked(name.clone()) {
                    state.error = Some(error.to_string());
                    return;
                }
                changing = true;
                Request::Rename {
                    id: *id,
                    name: name.clone(),
                    revision: snapshot.page.revision,
                    offset: state.offset,
                }
            }
            Action::Activate(id) => {
                let Some(snapshot) = &state.snapshot else {
                    return;
                };
                changing = true;
                Request::Activate {
                    id,
                    revision: snapshot.page.revision,
                    offset: state.offset,
                }
            }
        };
        state.serial += 1;
        let serial = state.serial;
        if self.try_command(Command::Profiles(serial, request)) {
            self.profiles.busy = true;
            self.profiles.changing = changing;
            self.profiles.error = None;
        } else {
            self.profiles.error = Some("Profiles could not be queued. Refresh to retry.".into());
        }
    }
    pub(super) fn profiles_update(
        &mut self,
        request: u64,
        result: Result<Arc<Snapshot>, String>,
    ) -> Task<Message> {
        if request != self.profiles.serial {
            return Task::none();
        }
        self.profiles.busy = false;
        match result {
            Ok(snapshot) => {
                self.profiles.error = snapshot.warning.clone();
                if self.profiles.changing {
                    self.notice(
                        if self.profiles.editing.is_some() {
                            "Profile name saved"
                        } else {
                            "Profile selected for the next launch"
                        },
                        false,
                    );
                    self.profiles.editing = None;
                }
                self.profiles.snapshot = Some(snapshot);
            }
            Err(error) => {
                if let Some(snapshot) = &self.profiles.snapshot {
                    self.profiles.offset = snapshot.page.offset;
                }
                self.profiles.error = Some(error.clone());
                self.notice(format!("Could not update profiles: {error}"), true);
                self.pending_close = None;
            }
        }
        self.profiles.changing = false;
        if let Some(window) = self.pending_close.take() {
            return self.handle(Message::WindowClose(window));
        }
        Task::none()
    }
    pub(super) fn profiles_card(&self) -> Element<'_, Message> {
        use components::{action, muted, outline};
        let state = &self.profiles;
        let message = |a| Message::Profiles(a);
        let mut content = column![].spacing(12);
        if let Some(snapshot) = &state.snapshot {
            content = content.push(muted(format!("Open now: {}", snapshot.current.name)).size(12));
            for profile in &snapshot.page.rows {
                if let Some((id, name)) = &state.editing
                    && *id == profile.id
                {
                    content = content.push(
                        column![
                            text_input("Profile name", name)
                                .style(components::field)
                                .id("profile-name")
                                .on_input(move |v| message(Action::Name(v)))
                                .on_submit(message(Action::SaveName))
                                .padding(12)
                                .size(13),
                            row![
                                action("Save name", message(Action::SaveName)),
                                action("Cancel", message(Action::CancelName))
                            ]
                            .spacing(10)
                        ]
                        .spacing(8),
                    );
                } else {
                    let status = if !profile.ready {
                        "Needs recovery"
                    } else if profile.id == snapshot.page.active {
                        "Next launch"
                    } else {
                        ""
                    };
                    let choose = button(text("Use on next launch").size(12))
                        .padding([11, 14])
                        .style(outline)
                        .on_press_maybe(
                            (profile.ready && profile.id != snapshot.page.active && !state.busy)
                                .then(|| message(Action::Activate(profile.id))),
                        );
                    content = content.push(
                        column![
                            row![
                                text(&profile.name).size(14).font(BOLD),
                                space().width(Length::Fill),
                                muted(status).size(11)
                            ]
                            .spacing(10)
                            .align_y(Alignment::Center),
                            row![
                                choose,
                                action("Rename", message(Action::Rename(profile.id)))
                            ]
                            .spacing(10)
                        ]
                        .spacing(8),
                    );
                }
            }
            if snapshot.page.total > 50 {
                content = content.push(
                    row![
                        button(text("Previous").size(12))
                            .padding([11, 14])
                            .style(outline)
                            .on_press_maybe(
                                (state.offset > 0 && !state.busy)
                                    .then(|| message(Action::Page(false)))
                            ),
                        muted(format!(
                            "{}–{} of {}",
                            snapshot.page.offset + 1,
                            snapshot.page.offset + snapshot.page.rows.len() as u64,
                            snapshot.page.total
                        ))
                        .size(12),
                        button(text("Next").size(12))
                            .padding([11, 14])
                            .style(outline)
                            .on_press_maybe(
                                (state.offset + 50 < snapshot.page.total && !state.busy)
                                    .then(|| message(Action::Page(true)))
                            )
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center),
                );
            }
        } else {
            content = content.push(muted("Import a database to add another profile."));
        }
        content = content.push(
            row![
                action(
                    if state.changing() {
                        "Saving…"
                    } else if state.busy {
                        "Refreshing…"
                    } else {
                        "Refresh profiles"
                    },
                    message(Action::Refresh)
                ),
                action(
                    "Import database…",
                    Message::FindSetting(SettingsTab::Backups, "Database transfer")
                )
            ]
            .spacing(10),
        );
        if let Some(error) = &state.error {
            content = content.push(text(error).size(12));
        }
        self.settings_card("Profiles", "Choose which saved workspace opens when you launch Shep. Each keeps its own accounts and settings.", content.into())
    }
}
