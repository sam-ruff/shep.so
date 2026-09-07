//! Display pending destinations without exposing a speculative provider UID.
use super::*;
#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;

impl Actions {
    pub(in crate::ui) fn move_target(&self, id: &str) -> Option<(&Mail, &str, &str)> {
        if let Some(entry) = self.transfers.get(id) {
            Some((&entry.mail, &entry.account, &entry.folder))
        } else {
            self.moves.get(id).map(|entry| {
                (
                    &entry.mail,
                    entry.mail.account_id.as_str(),
                    entry.destination.as_str(),
                )
            })
        }
    }

    pub(in crate::ui) fn projected_moves(&self) -> Vec<MailMoveProjection> {
        let mut result: Vec<_> = self
            .moves
            .keys()
            .chain(self.transfers.keys())
            .filter(|id| !self.restoring(id))
            .filter_map(|id| self.move_target(id))
            .map(|(mail, account, folder)| {
                let effective = self.effective(mail);
                MailMoveProjection {
                    id: mail.id.clone(),
                    source_account: mail.account_id.clone(),
                    source_folder: mail.folder.clone(),
                    account: account.into(),
                    folder: folder.into(),
                    unread: effective.unread,
                    starred: effective.starred,
                }
            })
            .collect();
        result.sort_by(|a, b| a.id.cmp(&b.id));
        result
    }
}

impl App {
    pub(super) fn confirm_move_display(&mut self, original: &Mail, receipt: &MoveReceipt) {
        let pending = receipt.current.is_none() && receipt.recovery.is_some();
        let display = receipt.current.clone().or_else(|| {
            pending.then(|| {
                let mut mail = original.clone();
                mail.account_id = receipt.account.clone();
                mail.folder = receipt.folder.clone();
                mail.remote_id.clear();
                mail
            })
        });
        let mut base = (*self.mail_actions.base_page).clone();
        counts::confirm_move(&mut base, original, display.as_ref());
        self.mail_actions.base_page = Arc::new(base);
        self.reconcile_move_row(original, display.as_ref(), false);
        if pending {
            let base = Arc::make_mut(&mut self.mail_actions.base_page);
            base.move_placeholders.insert(original.id.clone());
            let record = crate::mail_actions::journal::MoveRecord {
                token: receipt.recovery.clone().unwrap(),
                original: original.clone(),
                receipt: receipt.clone(),
                stage: crate::mail_actions::journal::MoveStage::Committed,
                error: None,
                attempted: 0,
                retained: None,
            };
            base.move_recovery.insert(original.id.clone(), record);
        }
    }

    pub(in crate::ui) fn move_recovered(
        &mut self,
        record: &crate::mail_actions::journal::MoveRecord,
    ) {
        use crate::mail_actions::journal::MoveStage;
        let matches = |undo: &undo::Record| {
            undo.receipt
                .as_ref()
                .is_some_and(|r| r.recovery.as_deref() == Some(record.token.as_str()))
        };
        if record.stage == MoveStage::Kept {
            let retired: Vec<_> = self
                .mail_actions
                .undo
                .iter()
                .filter(|(_, r)| matches(r))
                .map(|(token, _)| *token)
                .collect();
            for token in retired {
                self.mail_actions.undo.remove(&token);
                self.action_toasts.failed(token);
            }
        } else {
            for undo in self.mail_actions.undo.values_mut().filter(|r| matches(r)) {
                undo.receipt = Some(Arc::new(record.receipt.clone()));
            }
        }
        self.detail_revision += 1;
        if record.stage == MoveStage::Kept {
            // This is a local resolution, not a new server acknowledgment.
            if let Some(mail) = record.retained.as_ref() {
                let mut display = MoveReceipt::local(&record.original, &mail.folder);
                display.current = Some(mail.clone());
                self.confirm_move_display(&record.original, &display);
            }
        } else {
            self.confirm_move_display(&record.original, &record.receipt);
        }
        if self.dialog == Some(Dialog::MoveRecovery) {
            if self
                .move_recovery
                .selected
                .as_ref()
                .is_some_and(|r| r.token == record.token)
            {
                self.move_recovery.selected = None;
                self.move_recovery.confirmed = false;
            }
            self.load_move_recoveries(None);
        }
        self.project_mail_flags();
    }

    pub(in crate::ui) fn cached_placeholder_detail(&mut self, id: &str) -> Option<Arc<MailDetail>> {
        let mut detail = self.cached_detail(id)?;
        if let Some(mail) = self.page.rows.iter().find(|m| m.id == id) {
            Arc::make_mut(&mut detail).summary = mail.clone();
        }
        Some(detail)
    }

    pub(super) fn reconcile_move_row(
        &mut self,
        original: &Mail,
        current: Option<&Mail>,
        failed: bool,
    ) {
        let mut base = (*self.mail_actions.base_page).clone();
        base.move_recovery.remove(&original.id);
        if let Some(index) = base.rows.iter().position(|m| m.id == original.id) {
            let was_placeholder = base.move_placeholders.remove(&original.id);
            let replacement = if failed {
                was_placeholder.then_some(original)
            } else {
                current
            };
            let replacement = replacement.filter(|mail| {
                self.bulk_scope_contains(&self.query, &mail.account_id, &mail.folder)
            });
            // A failure in the original scope leaves its existing row intact.
            if !failed || was_placeholder {
                if let Some(mail) = replacement {
                    let previous = std::mem::replace(&mut base.rows[index], mail.clone());
                    base.unread = base.unread.saturating_sub(usize::from(previous.unread))
                        + usize::from(mail.unread);
                    if self.selected.as_ref() == Some(&original.id) {
                        self.selected = Some(mail.id.clone());
                        self.conversation = Default::default();
                        self.bulk.waiting_reader = None;
                        if let Some(detail) = &mut self.detail
                            && detail.summary.id == original.id
                        {
                            Arc::make_mut(detail).summary = mail.clone();
                        }
                    }
                    for detail in &mut self.detail_cache {
                        if detail.summary.id == original.id {
                            Arc::make_mut(detail).summary = mail.clone();
                        }
                    }
                } else {
                    let removed = base.rows.remove(index);
                    base.total = base.total.saturating_sub(1);
                    base.unread = base.unread.saturating_sub(usize::from(removed.unread));
                    if self.selected.as_ref() == Some(&original.id) {
                        self.selected = None;
                        self.detail = None;
                        self.conversation = Default::default();
                        self.bulk.waiting_reader = None;
                    }
                }
            }
        }
        self.mail_actions.base_page = Arc::new(base);
    }
}
