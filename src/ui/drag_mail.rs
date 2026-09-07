//! Dragging uses metadata and cached destination rules. Persistence follows the
//! same optimistic single-message or frozen group-review path as the toolbar.
#[cfg(test)]
mod tests;
mod widget;
use super::*;
pub(super) use widget::{Handle, Region};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub account: Option<String>,
    pub folder: String,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reveal {
    Inbox,
    Account(String),
    Folder(String, String),
}
#[derive(Debug, Clone)]
pub enum Payload {
    Single(Box<Mail>),
    Group(Arc<crate::store::SelectionSnapshot>),
}
impl Payload {
    pub fn count(&self) -> usize {
        match self {
            Self::Single(_) => 1,
            Self::Group(s) => s.selected,
        }
    }
    fn groups(&self) -> impl Iterator<Item = (&str, &str)> {
        let single = match self {
            Self::Single(m) => Some((m.account_id.as_str(), m.folder.as_str())),
            _ => None,
        };
        let group = match self {
            Self::Group(s) => Some(s.groups.as_slice()),
            _ => None,
        };
        single.into_iter().chain(
            group
                .into_iter()
                .flatten()
                .map(|g| (g.account.as_str(), g.folder.as_str())),
        )
    }
}
#[derive(Clone)]
pub(super) struct Rules {
    workspace: Arc<Workspace>,
    cross_account: bool,
}
fn same_folder(a: &str, b: &str) -> bool {
    a == b || a.eq_ignore_ascii_case("INBOX") && b.eq_ignore_ascii_case("INBOX")
}
impl Rules {
    pub fn check(&self, payload: &Payload, target: &Target) -> Result<(), &'static str> {
        if payload.groups().next().is_none() {
            return Err("No messages are available to move.");
        }
        let mut changes = false;
        for (source, folder) in payload.groups() {
            let destination = target.account.as_deref().unwrap_or(source);
            let from = self
                .workspace
                .accounts
                .iter()
                .find(|a| a.id == source)
                .ok_or("The source account is no longer connected.")?;
            let to = self
                .workspace
                .accounts
                .iter()
                .find(|a| a.id == destination)
                .ok_or("The destination account is no longer connected.")?;
            if source != destination {
                if !self.cross_account {
                    return Err("Enable moving between accounts in Preferences.");
                }
                if from.protocol != Protocol::Imap || to.protocol != Protocol::Imap {
                    return Err("Moving between accounts requires two IMAP accounts.");
                }
            }
            if !self
                .workspace
                .account_folders
                .get(destination)
                .is_some_and(|folders| folders.iter().any(|f| same_folder(f, &target.folder)))
            {
                return Err(
                    "This folder is unavailable for one or more accounts. Refresh mail and try again.",
                );
            }
            changes |= source != destination || !same_folder(folder, &target.folder);
        }
        if !changes {
            return Err("These messages are already in this folder.");
        }
        Ok(())
    }
}
impl App {
    pub(super) fn drag_rules(&self) -> Rules {
        Rules {
            workspace: self.workspace.clone(),
            cross_account: self.preferences.cross_account_moves,
        }
    }
    pub(super) fn drag_payload(&self, mail: &Mail) -> Option<Arc<Payload>> {
        if self.bulk_owns_mail(&mail.id)
            || self.mail_actions.restoring(&mail.id)
            || self.mail_actions.moving(&mail.id)
        {
            return None;
        }
        if self.mail_selection.mode && self.mail_selection.visible.contains(&mail.id) {
            self.mail_selection.ready_for_drag().then(|| {
                Arc::new(Payload::Group(
                    self.mail_selection.snapshot.clone().unwrap(),
                ))
            })
        } else {
            Some(Arc::new(Payload::Single(Box::new(mail.clone()))))
        }
    }
    pub(super) fn drop_mail(&mut self, payload: Arc<Payload>, target: Option<Target>) {
        self.last_click = None;
        let Some(target) = target else {
            return;
        };
        if self.tab != Tab::Mail || self.full_reader || self.dialog.is_some() {
            return;
        }
        if let Err(error) = self.drag_rules().check(&payload, &target) {
            self.notice(error, true);
            return;
        }
        match payload.as_ref() {
            Payload::Single(original) => {
                let Some(mail) = self
                    .page
                    .rows
                    .iter()
                    .find(|m| m.id == original.id)
                    .cloned()
                    .filter(|m| m.account_id == original.account_id && m.folder == original.folder)
                else {
                    self.notice(
                        "This message changed while dragging. Select it and try again.",
                        true,
                    );
                    return;
                };
                if self.drag_payload(&mail).is_none() {
                    self.notice(
                        "This message is still being changed. Finish or review it before moving.",
                        true,
                    );
                    return;
                }
                if let Some(account) = target.account.filter(|a| a != &mail.account_id) {
                    self.transfer_mail(mail, account, target.folder);
                } else {
                    self.move_mail(mail, target.folder);
                }
            }
            Payload::Group(snapshot) => {
                if !self.mail_selection.ready_for_drag()
                    || !self
                        .mail_selection
                        .snapshot
                        .as_ref()
                        .is_some_and(|current| {
                            current.id == snapshot.id && current.revision == snapshot.revision
                        })
                {
                    self.notice(
                        "The selection changed while dragging. Check it and try again.",
                        true,
                    );
                    return;
                }
                self.begin_bulk(bulk::Intent::Move {
                    account: target.account,
                    folder: target.folder,
                });
            }
        }
    }
    pub(super) fn reveal_drag_folders(&mut self, reveal: Reveal) {
        if !self.mail_drag.active() || self.tab != Tab::Mail || self.dialog.is_some() {
            return;
        }
        match reveal {
            Reveal::Inbox => self.inbox_expanded = true,
            Reveal::Folder(account, path) => self.set_folder_expanded(&account, &path, true),
            Reveal::Account(id) => {
                if self.preferences.collapsed_accounts.contains(&id) {
                    self.preferences
                        .collapsed_accounts
                        .retain(|account| account != &id);
                    self.save_preferences();
                }
            }
        }
    }
    pub(super) fn sidebar_drag_reveal(&self, action: &Message) -> Option<Reveal> {
        if let Some((account, path)) = self.sidebar_group(action)
            && !self.folder_expanded(&account, &path)
        {
            return Some(Reveal::Folder(account, path));
        }
        match action {
            Message::AccountFolderUnified if !self.inbox_expanded => Some(Reveal::Inbox),
            Message::ToggleAccountFolders(id)
                if self.preferences.collapsed_accounts.contains(id) =>
            {
                Some(Reveal::Account(id.clone()))
            }
            _ => None,
        }
    }
    pub(super) fn sidebar_drop_target(&self, action: &Message) -> Option<Target> {
        let (account, folder) = match action {
            Message::AccountFolder(account, folder) => (Some(account.clone()), folder.clone()),
            Message::AccountFolderUnified => (None, "INBOX".into()),
            Message::Folder(folder) => (None, folder.clone()),
            // Sent is a combined query over discovered server folders and local
            // copies, not a literal destination. Use an actual account folder.
            _ => return None,
        };
        Some(Target { account, folder })
    }
}
