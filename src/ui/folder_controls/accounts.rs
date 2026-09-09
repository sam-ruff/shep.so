//! Resolve a common folder to one explicit account before reviewing a mutation.
use super::*;
use iced::widget::column;

impl App {
    pub(super) fn folder_account_choices(&self) -> Vec<FolderSelection> {
        let Some(scope) = &self.folder_controls.account_scope else {
            return Vec::new();
        };
        self.workspace
            .accounts
            .iter()
            .filter(|account| scope.account.as_ref().is_none_or(|id| id == &account.id))
            .filter_map(|account| {
                let folder = if scope.sent_only && !account.sent_folder.is_empty() {
                    account.sent_folder.clone()
                } else {
                    scope.folder.clone()
                };
                // Explicit account targets still reach the backend's reviewed error
                // state if their catalog changes. Aggregate choices show real folders.
                (scope.account.is_some()
                    || self
                        .folder_tree(&account.id)
                        .is_some_and(|tree| tree.node(&folder).is_some()))
                .then(|| FolderSelection {
                    account: Some(account.id.clone()),
                    folder,
                    sent_only: false,
                })
            })
            .collect()
    }

    pub(super) fn begin_folder_accounts(&mut self, target: FolderSelection, deleting: bool) {
        if target.folder.eq_ignore_ascii_case("INBOX")
            || target
                .account
                .as_ref()
                .is_some_and(|account| self.folder_busy(account))
        {
            self.notice(
                "This folder cannot be changed now. Open Folder changes to review pending work.",
                true,
            );
            return;
        }
        self.release_folder_preview();
        self.open(Dialog::FolderChange);
        let state = &mut self.folder_controls;
        state.account_scope = Some(target);
        state.account.clear();
        state.source.clear();
        state.serial += 1;
        state.preview = None;
        state.error = None;
        state.query.clear();
        state.options = Arc::default();
        state.filtered.clear();
        state.action = deleting.then_some(Change::Delete);
        state.loading = false;
        state.choosing_account = true;
        state.account_index = 0;
        let choices = self.folder_account_choices();
        self.folder_controls.account_focus =
            choices.first().and_then(|target| target.account.clone());
        if choices.len() == 1 {
            self.select_folder_account(choices[0].account.as_deref().unwrap());
        }
    }

    pub(super) fn show_folder_accounts(&mut self) {
        if self.dialog != Some(Dialog::FolderChange) || self.folder_controls.account_scope.is_none()
        {
            return;
        }
        self.release_folder_preview();
        self.pending_focus = None;
        self.focused_input = None;
        let state = &mut self.folder_controls;
        let previous = state.account.clone();
        state.account.clear();
        state.source.clear();
        state.serial += 1;
        state.choosing_account = true;
        state.account_index = 0;
        state.preview = None;
        state.error = None;
        state.loading = false;
        state.options = Arc::default();
        state.filtered.clear();
        let choices = self.folder_account_choices();
        let index = choices
            .iter()
            .position(|target| target.account.as_deref() == Some(&previous))
            .unwrap_or(0);
        self.folder_controls.account_index = index;
        self.folder_controls.account_focus =
            choices.get(index).and_then(|target| target.account.clone());
    }

    pub(super) fn reconcile_folder_account_focus(&mut self) {
        let Some(focused) = &self.folder_controls.account_focus else {
            return;
        };
        if let Some(index) = self
            .folder_account_choices()
            .iter()
            .position(|target| target.account.as_ref() == Some(focused))
        {
            self.folder_controls.account_index = index;
        } else {
            self.folder_controls.account_focus = None;
            self.folder_controls.account_index = 0;
            self.folder_controls.error = Some(
                "This account or folder is no longer available. Choose another account.".into(),
            );
        }
    }

    pub(super) fn select_folder_account(&mut self, account: &str) {
        if self.dialog != Some(Dialog::FolderChange) || !self.folder_controls.choosing_account {
            return;
        }
        let Some(target) = self
            .folder_account_choices()
            .into_iter()
            .find(|choice| choice.account.as_deref() == Some(account))
        else {
            self.folder_controls.error = Some(
                "This account or folder is no longer available. Choose another account.".into(),
            );
            return;
        };
        if self.folder_busy(account) {
            self.folder_controls.error = Some("This account has a folder change pending. Choose another account or review Folder changes.".into());
            return;
        }
        let state = &mut self.folder_controls;
        state.account = account.into();
        state.source = target.folder;
        state.choosing_account = false;
        state.query.clear();
        if state.action != Some(Change::Delete) {
            state.action = None;
        }
        self.handle_folders(Message::RefreshReview);
    }

    pub(in crate::ui) fn folder_parent_visible(&self) -> bool {
        self.dialog == Some(Dialog::FolderChange)
            && !self.folder_controls.choosing_account
            && self.folder_controls.action.is_none()
    }

    pub(in crate::ui) fn move_folder_account_choice(
        &mut self,
        direction: isize,
    ) -> Task<crate::ui::Message> {
        let choices = self.folder_account_choices();
        let count = choices.len();
        if count > 0 {
            self.folder_controls.account_index = choices
                .iter()
                .position(|target| target.account == self.folder_controls.account_focus)
                .map_or_else(
                    || if direction < 0 { count - 1 } else { 0 },
                    |index| (index as isize + direction).rem_euclid(count as isize) as usize,
                );
            self.folder_controls.account_focus =
                choices[self.folder_controls.account_index].account.clone();
            return widget::operation::snap_to(
                "folder-account-list",
                widget::scrollable::RelativeOffset {
                    x: 0.,
                    y: self.folder_controls.account_index as f32
                        / count.saturating_sub(1).max(1) as f32,
                },
            );
        }
        Task::none()
    }
    pub(in crate::ui) fn choose_focused_folder_account(&mut self) {
        if let Some(account) = self.folder_controls.account_focus.clone() {
            self.select_folder_account(&account);
        } else {
            self.folder_controls.error =
                Some("Choose an available account before continuing.".into());
        }
    }

    pub(super) fn folder_accounts_form(&self) -> Element<'_, crate::ui::Message> {
        let state = &self.folder_controls;
        let mut body = column![
            text("Choose an account").size(16).font(BOLD),
            text("The folder will change only in the account you choose.").size(13)
        ]
        .spacing(14);
        if let Some(error) = &state.error {
            body = body.push(text(error).size(13));
        }
        let choices = self.folder_account_choices();
        let mut rows = column![].spacing(4);
        for target in &choices {
            let Some(account) = self
                .workspace
                .accounts
                .iter()
                .find(|account| Some(&account.id) == target.account.as_ref())
            else {
                continue;
            };
            let busy = self.folder_busy(&account.id);
            let folder = self
                .workspace
                .folder_label(Some(&account.id), &target.folder);
            let label = if busy {
                format!("{folder} · folder change pending")
            } else {
                folder.to_string()
            };
            rows = rows.push(
                button(
                    column![
                        text(format!("{} · {}", account.name, account.email)).size(13),
                        muted(label).size(12)
                    ]
                    .spacing(4),
                )
                .width(Length::Fill)
                .padding(12)
                .style(if state.account_focus.as_deref() == Some(&account.id) {
                    selected
                } else {
                    ghost
                })
                .on_press_maybe((!busy).then(|| wrap(Message::SelectAccount(account.id.clone())))),
            );
        }
        if choices.is_empty() {
            rows = rows.push(text("No account currently has this folder. Close this review and refresh your accounts.").size(13));
        }
        body = body.push(
            container(
                scrollable(rows)
                    .id("folder-account-list")
                    .height(Length::Shrink),
            )
            .max_height(280),
        );
        body.push(action("Cancel", crate::ui::Message::Close))
            .into()
    }
}

#[cfg(test)]
#[path = "account_tests.rs"]
mod tests;
