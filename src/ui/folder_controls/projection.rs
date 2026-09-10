//! Query-wide folder deletion projection, with at most one retained metadata page.
use super::*;

impl App {
    pub(super) fn folder_count_query(&self) -> MailQuery {
        let mut query = self.query.clone();
        query.project_moves = self.mail_actions.projected_moves();
        query.observe = self.mail_actions.observed_ids();
        query.observe_bulk = self.bulk_observed_ids();
        if let Some(id) = &self.selected
            && !query.observe.contains(id)
        {
            query.observe.push(id.clone());
        }
        query
    }

    pub(super) fn release_folder_projection(&mut self, preview: &Preview) {
        if let Some((token, _)) = &preview.projection {
            self.send(Command::Folder(Request::Release(token.clone())));
        }
    }

    pub(in crate::ui) fn release_folder_preview(&mut self) {
        if let Some(preview) = self.folder_controls.preview.take()
            && !self
                .folder_controls
                .pending
                .values()
                .any(|pending| Arc::ptr_eq(&pending.preview, &preview))
        {
            self.release_folder_projection(&preview);
        }
    }

    pub(super) fn request_folder_page(&mut self) {
        self.request_page();
        self.folder_controls.retained_reader =
            self.selected.clone().map(|id| (id, self.list_revision));
        if self.detail.is_none()
            && let Some(id) = self.reader_id().map(str::to_owned)
        {
            self.focus_conversation_message(id);
        }
    }

    pub(in crate::ui) fn retain_folder_reader(&mut self) -> bool {
        let retain = self
            .folder_controls
            .retained_reader
            .as_ref()
            .is_some_and(|(id, revision)| {
                self.selected.as_ref() == Some(id)
                    && *revision == self.list_revision
                    && self
                        .page
                        .observed
                        .get(id)
                        .and_then(Option::as_ref)
                        .is_some_and(|member| {
                            self.bulk_scope_contains(&self.query, &member.account, &member.folder)
                                && (!self.query.unread_only || member.unread)
                                && (!self.query.read_only || !member.unread)
                                && (!self.query.starred_only
                                    || self
                                        .detail
                                        .as_ref()
                                        .is_some_and(|detail| detail.summary.starred))
                        })
            });
        if !retain {
            self.folder_controls.retained_reader = None;
        }
        retain
    }
    pub(in crate::ui) fn pending_folder_deletions(&self) -> Vec<FolderSelection> {
        let mut folders: Vec<_> = self
            .folder_controls
            .pending
            .values()
            .filter(|pending| pending.preview.review.plan.action == Change::Delete)
            .flat_map(|pending| {
                pending
                    .preview
                    .review
                    .plan
                    .members
                    .iter()
                    .map(|member| FolderSelection {
                        account: Some(pending.preview.review.account.clone()),
                        folder: member.mailbox.name.clone(),
                        sent_only: false,
                    })
            })
            .collect();
        folders.sort_by(|a, b| (&a.account, &a.folder).cmp(&(&b.account, &b.folder)));
        folders.dedup();
        folders
    }

    pub(super) fn query_without_deleted_folders(
        &self,
        query: &MailQuery,
        review: &crate::folder_actions::Review,
    ) -> MailQuery {
        let mut result = query.clone();
        let affected = |account: &str, folder: &str| {
            account == review.account
                && review
                    .plan
                    .members
                    .iter()
                    .any(|member| member.mailbox.name == folder)
        };
        if !query.searches_all_folders() {
            if let Some(folders) = &query.folders {
                result.folders = Some(
                    folders
                        .iter()
                        .flat_map(|folder| {
                            let accounts: Vec<_> = if let Some(account) = &folder.account {
                                vec![account.clone()]
                            } else if review
                                .plan
                                .members
                                .iter()
                                .any(|member| member.mailbox.name == folder.folder)
                            {
                                self.workspace
                                    .accounts
                                    .iter()
                                    .map(|account| account.id.clone())
                                    .collect()
                            } else {
                                return vec![folder.clone()];
                            };
                            accounts
                                .into_iter()
                                .filter_map(|account| {
                                    let path = if folder.sent_only {
                                        self.workspace
                                            .accounts
                                            .iter()
                                            .find(|candidate| candidate.id == account)
                                            .filter(|candidate| !candidate.sent_folder.is_empty())
                                            .map_or(folder.folder.as_str(), |candidate| {
                                                candidate.sent_folder.as_str()
                                            })
                                    } else {
                                        &folder.folder
                                    };
                                    (!affected(&account, path)).then(|| FolderSelection {
                                        account: Some(account),
                                        ..folder.clone()
                                    })
                                })
                                .collect::<Vec<_>>()
                        })
                        .collect(),
                );
            } else if query
                .account
                .as_deref()
                .is_some_and(|account| affected(account, &query.folder))
            {
                result.folder = "INBOX".into();
                result.sent_only = false;
            }
        }
        if result != *query {
            result.offset = 0;
        }
        result
    }

    pub(super) fn rollback_folder_projection(
        &mut self,
        pending: Pending,
        remaining: Option<(usize, usize)>,
    ) {
        if pending.redirect.as_ref() != Some(&self.query)
            || pending.redirect_revision != self.list_revision
        {
            return;
        }
        self.query = pending.origin;
        // Restore only the deleted membership. Unaffected rows and group counts
        // belong to the latest page, including newer moves or emptied folders.
        let affected = |account: &str, folder: &str| {
            account == pending.preview.review.account
                && pending
                    .preview
                    .review
                    .plan
                    .members
                    .iter()
                    .any(|member| member.mailbox.name == folder)
        };
        let current = &self.mail_actions.base_page;
        let mut restored = (**current).clone();
        let (total, unread) = remaining
            .or(pending.original_page.folder_count)
            .unwrap_or_default();
        restored.total = restored.total.saturating_add(total);
        restored.unread = restored.unread.saturating_add(unread);
        restored.rows = pending
            .original_page
            .rows
            .iter()
            .filter_map(|mail| {
                if affected(&mail.account_id, &mail.folder) {
                    Some(mail.clone())
                } else {
                    current
                        .rows
                        .iter()
                        .find(|newer| newer.id == mail.id)
                        .cloned()
                }
            })
            .collect();
        for mail in &current.rows {
            if !restored.rows.iter().any(|retained| retained.id == mail.id) {
                restored.rows.push(mail.clone());
            }
        }
        restored.rows.truncate(PAGE_SIZE);
        self.mail_actions.base_page = Arc::new(restored);
        self.project_mail_flags();
    }

    pub(super) fn remove_folder_rows(
        &mut self,
        excluded: &[FolderSelection],
        counts: (usize, usize),
    ) {
        let matches = |mail: &Mail| {
            excluded.iter().any(|folder| {
                folder.account.as_deref() == Some(mail.account_id.as_str())
                    && folder.folder == mail.folder
            })
        };
        let page = Arc::make_mut(&mut self.mail_actions.base_page);
        page.total = page.total.saturating_sub(counts.0);
        page.unread = page.unread.saturating_sub(counts.1);
        page.folder_count = None;
        page.rows.retain(|mail| !matches(mail));
        if self
            .detail
            .as_ref()
            .is_some_and(|detail| matches(&detail.summary))
            || self.selected.as_ref().is_some_and(|id| {
                self.page
                    .rows
                    .iter()
                    .any(|mail| &mail.id == id && matches(mail))
            })
        {
            self.selected = None;
            self.detail = None;
            self.conversation = Default::default();
        }
        self.project_mail_flags();
        if self.selected.is_none()
            && let Some(mail) = self.page.rows.first()
        {
            self.select(mail.id.clone());
        }
    }
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
