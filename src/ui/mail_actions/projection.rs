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
