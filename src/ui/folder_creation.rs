use super::*;
use iced::Length;
use iced::widget::{button, column, pick_list, row, space, text, text_input};

#[derive(Debug, Clone)]
pub enum Message {
    Open,
    Resume(crate::store::PendingCreation),
    Account(Choice),
    Parent(Choice),
    Name(String),
    Submit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice(pub String, pub String);
impl std::fmt::Display for Choice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.1)
    }
}

#[derive(Default)]
pub(super) struct State {
    pub(super) serial: u64,
    account: String,
    connection: String,
    parent: String,
    name: String,
    pub busy: bool,
    error: Option<String>,
}

fn wrap(message: Message) -> super::Message {
    super::Message::FolderCreation(message)
}

impl App {
    pub(super) fn handle_folder_creation(&mut self, message: Message) -> Task<super::Message> {
        if matches!(message, Message::Open) {
            if !self.folder_creation.busy && self.folder_creation.error.is_none() {
                let account = self.query.account.clone().or_else(|| {
                    self.workspace
                        .accounts
                        .first()
                        .map(|account| account.id.clone())
                });
                self.folder_creation.account = account.unwrap_or_default();
                self.folder_creation.connection = self
                    .workspace
                    .accounts
                    .iter()
                    .find(|account| account.id == self.folder_creation.account)
                    .map(crate::mail_actions::connection_key)
                    .unwrap_or_default();
                self.folder_creation.parent.clear();
                self.folder_creation.name.clear();
                if let Some(saved) = self
                    .workspace
                    .folder_creations
                    .iter()
                    .find(|saved| saved.account == self.folder_creation.account)
                    .cloned()
                {
                    self.resume_folder_creation(saved);
                }
            }
            self.open(Dialog::FolderCreation);
            return focus_after_layout("new-folder-name");
        }
        if self.dialog != Some(Dialog::FolderCreation) || self.folder_creation.busy {
            return Task::none();
        }
        match message {
            Message::Resume(saved) => {
                self.resume_folder_creation(saved);
                return focus_after_layout("new-folder-name");
            }
            Message::Account(choice) => {
                self.folder_creation.account = choice.0;
                self.folder_creation.connection = self
                    .workspace
                    .accounts
                    .iter()
                    .find(|account| account.id == self.folder_creation.account)
                    .map(crate::mail_actions::connection_key)
                    .unwrap_or_default();
                self.folder_creation.parent.clear();
                self.folder_creation.error = None;
            }
            Message::Parent(choice) => {
                self.folder_creation.parent = choice.0;
                self.folder_creation.error = None;
            }
            Message::Name(name) => {
                self.folder_creation.name = name;
                self.folder_creation.error = None;
            }
            Message::Submit => {
                let state = &self.folder_creation;
                let name = state.name.trim();
                if name.is_empty() || name.chars().any(char::is_control) {
                    self.folder_creation.error =
                        Some("Enter a folder name without control characters.".into());
                    return Task::none();
                }
                let Some(account) = self
                    .workspace
                    .accounts
                    .iter()
                    .find(|a| a.id == state.account)
                else {
                    self.folder_creation.error = Some("Choose an account for this folder.".into());
                    return Task::none();
                };
                let request = engine::folder_creation::Request {
                    serial: state.serial.wrapping_add(1),
                    account: account.id.clone(),
                    connection: state.connection.clone(),
                    parent: (!state.parent.is_empty()).then(|| state.parent.clone()),
                    name: name.to_owned(),
                };
                let serial = request.serial;
                if self.try_command(Command::CreateFolder(request)) {
                    self.folder_creation.serial = serial;
                    self.folder_creation.busy = true;
                    self.folder_creation.error = None;
                } else {
                    self.folder_creation.error =
                        Some("Folder creation could not start. Try again.".into());
                }
            }
            Message::Open => {}
        }
        Task::none()
    }

    fn resume_folder_creation(&mut self, saved: crate::store::PendingCreation) {
        self.folder_creation.account = saved.account;
        self.folder_creation.connection = saved.connection;
        self.folder_creation.parent = saved.parent.unwrap_or_default();
        self.folder_creation.name = saved.name;
        self.folder_creation.error =
            Some("This folder request is unfinished. Try again to check it.".into());
    }

    fn current_folder_creation(&self) -> Option<crate::store::PendingCreation> {
        let state = &self.folder_creation;
        let account = self
            .workspace
            .accounts
            .iter()
            .find(|account| account.id == state.account)?;
        Some(crate::store::PendingCreation {
            account: account.id.clone(),
            connection: state.connection.clone(),
            parent: (!state.parent.is_empty()).then(|| state.parent.clone()),
            name: state.name.trim().into(),
        })
    }

    pub(super) fn folder_created(
        &mut self,
        serial: u64,
        result: Result<crate::folders::Mailbox, String>,
    ) {
        if serial != self.folder_creation.serial || !self.folder_creation.busy {
            return;
        }
        self.folder_creation.busy = false;
        match result {
            Ok(created) => {
                let closed = self.dialog == Some(Dialog::FolderCreation);
                if let Some(finished) = self.current_folder_creation() {
                    Arc::make_mut(&mut self.workspace)
                        .folder_creations
                        .retain(|saved| saved != &finished);
                }
                if closed {
                    self.dialog = None;
                    self.focused_input = None;
                    self.pending_focus = None;
                }
                let account = self.folder_creation.account.clone();
                let mut ancestors = Vec::new();
                if let Some(tree) = self.folder_tree(&account) {
                    let mut parent = tree.node(&created.name).and_then(|node| node.parent);
                    while let Some(index) = parent {
                        let node = &tree.nodes[index];
                        ancestors.push(node.path.clone());
                        parent = node.parent;
                    }
                }
                let was_collapsed = self.preferences.collapsed_accounts.contains(&account);
                self.preferences
                    .collapsed_accounts
                    .retain(|id| id != &account);
                let mut changed = was_collapsed;
                if !ancestors.is_empty() {
                    let expanded = self
                        .preferences
                        .expanded_folders
                        .entry(account)
                        .or_default();
                    let before = expanded.len();
                    expanded.extend(ancestors);
                    changed |= expanded.len() != before;
                }
                if changed {
                    self.save_preferences();
                }
                if closed
                    && self.sidebar_focus
                    && let Some(index) = self.sidebar_items().iter().position(|item| {
                        matches!(item.action, super::Message::FolderCreation(Message::Open))
                    })
                {
                    self.sidebar_index = index;
                }
                self.folder_creation.name.clear();
                self.folder_creation.error = None;
                self.notice("Folder created.", false);
            }
            Err(error) => {
                self.pending_close = None;
                self.folder_creation.error = Some(error.clone());
                self.notice(format!("Could not create the folder. {error}"), true);
            }
        }
    }

    pub(super) fn folder_creation_form(&self) -> Element<'_, super::Message> {
        let state = &self.folder_creation;
        let accounts: Vec<_> = self
            .workspace
            .accounts
            .iter()
            .map(|account| {
                Choice(
                    account.id.clone(),
                    format!("{} ({})", account.name, account.email),
                )
            })
            .collect();
        let selected = accounts
            .iter()
            .find(|choice| choice.0 == state.account)
            .cloned();
        let mut parents = vec![Choice(String::new(), "Account root".into())];
        if let Some(tree) = self.folder_tree(&state.account) {
            parents.extend(
                tree.nodes
                    .iter()
                    .filter(|node| {
                        !node.mailbox.no_inferiors
                            && !node.mailbox.non_existent
                            && node.mailbox.delimiter.is_some()
                    })
                    .map(|node| Choice(node.mailbox.name.clone(), node.display_path.clone())),
            );
        }
        let parent = parents
            .iter()
            .find(|choice| choice.0 == state.parent)
            .cloned();
        let mut body = column![].spacing(12);
        if state.busy {
            body = body.push(text("Creating folder…").size(13)).push(muted(
                "You can close this dialog and keep reading your mail.",
            ));
        } else {
            body = body
                .push(
                    column![
                        text("Account").size(12),
                        pick_list(accounts, selected, |choice| wrap(Message::Account(choice)))
                            .text_size(13)
                            .padding(12)
                            .style(select_input)
                            .menu_style(select_menu)
                            .width(Length::Fill)
                    ]
                    .spacing(8),
                )
                .push(
                    column![
                        text("Inside").size(12),
                        pick_list(parents, parent, |choice| wrap(Message::Parent(choice)))
                            .text_size(13)
                            .padding(12)
                            .style(select_input)
                            .menu_style(select_menu)
                            .width(Length::Fill)
                    ]
                    .spacing(8),
                )
                .push(
                    column![
                        text("Folder name").size(12),
                        text_input("New folder name", &state.name)
                            .id("new-folder-name")
                            .on_input(|name| wrap(Message::Name(name)))
                            .on_submit(wrap(Message::Submit))
                            .style(field)
                            .padding(12)
                            .size(13)
                    ]
                    .spacing(8),
                );
        }
        if !state.busy
            && let Some(saved) = self
                .workspace
                .folder_creations
                .iter()
                .find(|saved| saved.account == state.account)
        {
            let current = saved.name == state.name.trim()
                && saved.parent.as_deref().unwrap_or_default() == state.parent;
            body = body.push(
                row![
                    text(format!("Unfinished: {}", saved.name)).size(12),
                    space().width(Length::Fill),
                    button(text("Resume").size(12))
                        .style(ghost)
                        .padding([6, 10])
                        .on_press_maybe((!current).then(|| wrap(Message::Resume(saved.clone()))))
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center),
            );
        }
        if let Some(error) = &state.error {
            body = body.push(text(error).size(13));
        }
        let submit = button(
            text(if state.error.is_some() {
                "Try again"
            } else {
                "Create folder"
            })
            .size(13),
        )
        .style(primary)
        .padding([10, 16])
        .on_press_maybe(
            (!state.busy && !state.name.trim().is_empty()).then(|| wrap(Message::Submit)),
        );
        body.push(
            row![
                button(text(if state.busy { "Close" } else { "Cancel" }).size(13))
                    .style(ghost)
                    .padding([10, 16])
                    .on_press(super::Message::Close),
                space().width(Length::Fill),
                submit
            ]
            .spacing(10),
        )
        .into()
    }
}

#[cfg(feature = "test-support")]
impl App {
    pub(super) fn folder_creation_test_state(&self) -> serde_json::Value {
        let state = &self.folder_creation;
        serde_json::json!({"open":self.dialog == Some(Dialog::FolderCreation),
            "account":state.account,"parent":state.parent,"name":state.name,
            "busy":state.busy,"error":state.error,"saved":self.workspace.folder_creations})
    }
}

#[cfg(test)]
mod tests;
