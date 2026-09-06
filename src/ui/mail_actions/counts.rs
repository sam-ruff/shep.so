//! Unread Inbox counts are global, independent of the current page or filter.
//! Observe pending identities in the same database snapshot to avoid applying
//! an optimistic change twice after its cache write has already completed.
use super::*;
use std::collections::{BTreeMap, HashSet};

pub(super) fn member(page: &MailPage, mail: &Mail) -> Option<MailMembership> {
    page.observed.get(&mail.id).cloned().unwrap_or_else(|| {
        Some(
            page.rows
                .iter()
                .find(|m| m.id == mail.id)
                .unwrap_or(mail)
                .into(),
        )
    })
}

pub(super) fn adjust(
    counts: &mut BTreeMap<String, usize>,
    before: Option<&MailMembership>,
    after: Option<&MailMembership>,
) {
    let inbox = |value: &MailMembership| {
        (value.unread && value.folder.eq_ignore_ascii_case("INBOX"))
            .then_some(value.account.clone())
    };
    let before = before.and_then(inbox);
    let after = after.and_then(inbox);
    if before == after {
        return;
    }
    if let Some(account) = before {
        let count = counts.entry(account).or_default();
        *count = count.saturating_sub(1);
    }
    if let Some(account) = after {
        let count = counts.entry(account).or_default();
        *count = count.saturating_add(1);
    }
}

pub(super) fn confirm_flags(page: &mut MailPage, mail: &Mail) {
    let before = member(page, mail);
    let after = before.clone().map(|mut state| {
        state.unread = mail.unread;
        state
    });
    adjust(&mut page.inbox_unread, before.as_ref(), after.as_ref());
    page.observed.insert(mail.id.clone(), after);
}

pub(super) fn confirm_move(page: &mut MailPage, source: &Mail, current: Option<&Mail>) {
    let before = member(page, source);
    let after = current.map(MailMembership::from);
    // A missing observed source means this snapshot already includes the move.
    if before.is_some() {
        adjust(&mut page.inbox_unread, before.as_ref(), after.as_ref());
    }
    page.observed.insert(source.id.clone(), None);
    if let Some(mail) = current {
        page.observed.insert(mail.id.clone(), after);
    }
}

pub(super) fn confirm_restore(page: &mut MailPage, record: &undo::Record, receipt: &MoveReceipt) {
    let previous = record.receipt.as_ref().and_then(|r| r.current.as_ref());
    let before = previous
        .and_then(|mail| member(page, mail))
        .or_else(|| member(page, &record.original));
    let after = receipt.current.as_ref().map(MailMembership::from);
    if before.is_some() {
        adjust(&mut page.inbox_unread, before.as_ref(), after.as_ref());
    }
    page.observed.insert(record.original.id.clone(), None);
    if let Some(mail) = previous {
        page.observed.insert(mail.id.clone(), None);
    }
    if let Some(mail) = &receipt.current {
        page.observed.insert(mail.id.clone(), after);
    }
}

impl Actions {
    pub(in crate::ui) fn observed_ids(&self) -> Vec<String> {
        let mut ids: HashSet<String> = self
            .flags
            .keys()
            .chain(self.moves.keys())
            .chain(self.transfers.keys())
            .cloned()
            .collect();
        for record in self.undo.values().filter(|record| record.restoring()) {
            ids.insert(record.original.id.clone());
            if let Some(current) = record.receipt.as_ref().and_then(|r| r.current.as_ref()) {
                ids.insert(current.id.clone());
            }
        }
        let mut ids: Vec<_> = ids.into_iter().collect();
        ids.sort();
        ids
    }
}

impl App {
    pub(super) fn invalidate_action_snapshot(&mut self) {
        // A page requested before this intent has no observations for it. Its
        // counts must not replace the baseline after the action starts writing.
        self.generation += 1;
        self.prefetch_page = None;
        self.prefetch_query = None;
    }
    pub(super) fn project_inbox_counts(&self) -> BTreeMap<String, usize> {
        let actions = &self.mail_actions;
        let base = &actions.base_page;
        let mut counts = base.inbox_unread.clone();
        for (id, entry) in &actions.flags {
            if actions.moves.contains_key(id)
                || actions.transfers.contains_key(id)
                || actions.restoring(id)
            {
                continue;
            }
            let before = member(base, &entry.confirmed);
            let after = before.clone().map(|mut state| {
                state.unread = entry.desired.unread;
                state
            });
            adjust(&mut counts, before.as_ref(), after.as_ref());
        }
        // The source identity can disappear before its receipt reaches iced.
        // In that snapshot the destination is already counted by SQLite.
        for (mail, account, folder) in actions
            .moves
            .values()
            .map(|e| (&e.mail, &e.mail.account_id, &e.destination))
            .chain(
                actions
                    .transfers
                    .values()
                    .map(|e| (&e.mail, &e.account, &e.folder)),
            )
        {
            let destination = MailMembership {
                account: account.clone(),
                folder: folder.clone(),
                unread: mail.unread,
            };
            let before = member(base, mail).unwrap_or_else(|| destination.clone());
            let after = if actions.restoring(&mail.id) {
                MailMembership::from(actions.effective(mail))
            } else {
                MailMembership {
                    unread: actions.effective(mail).unread,
                    ..destination
                }
            };
            adjust(&mut counts, Some(&before), Some(&after));
        }
        for record in actions.undo.values().filter(|r| r.restoring()) {
            if actions.moves.contains_key(&record.original.id)
                || actions.transfers.contains_key(&record.original.id)
            {
                continue;
            }
            if let Some(current) = record.receipt.as_ref().and_then(|r| r.current.as_ref()) {
                let before = member(base, current).or_else(|| member(base, &record.original));
                let after = before
                    .as_ref()
                    .map(|_| MailMembership::from(actions.effective(&record.original)));
                adjust(&mut counts, before.as_ref(), after.as_ref());
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::fixture;
    use super::*;

    fn snapshot(app: &App, mail: &Mail, folder: &str, unread: bool, count: usize) -> Arc<MailPage> {
        let mut page = (*app.mail_actions.base_page).clone();
        page.rows.clear();
        page.total = 0;
        page.unread = 0;
        page.inbox_unread.insert(mail.account_id.clone(), count);
        page.observed.insert(
            mail.id.clone(),
            Some(MailMembership {
                account: mail.account_id.clone(),
                folder: folder.into(),
                unread,
            }),
        );
        Arc::new(page)
    }
    fn count(app: &App) -> usize {
        app.page.inbox_unread.values().sum()
    }

    #[tokio::test]
    async fn unread_counts_survive_empty_filtered_pages_and_already_saved_snapshots() {
        for saved_before_response in [false, true] {
            let (mut app, mut commands, detail) = fixture().await;
            app.toggle_mail_flag(detail.summary.clone(), true);
            let Command::Flags(request, sent, _) = commands.try_recv().unwrap() else {
                panic!()
            };
            assert_eq!(count(&app), 0);
            app.query.folder = "Archive".into();
            let page = snapshot(
                &app,
                &sent,
                "INBOX",
                !saved_before_response,
                usize::from(!saved_before_response),
            );
            app.set_mail_page(page);
            assert_eq!(
                count(&app),
                0,
                "The pending read is independent of visible rows"
            );
            let _ = app.flags_finished(request, sent, Ok(()));
            assert_eq!(
                count(&app),
                0,
                "Receipt cannot double-apply the read or restore an old count"
            );
        }
    }
    #[tokio::test]
    async fn failed_read_restores_count_after_leaving_the_inbox() {
        let (mut app, mut commands, detail) = fixture().await;
        app.toggle_mail_flag(detail.summary.clone(), true);
        let Command::Flags(request, sent, _) = commands.try_recv().unwrap() else {
            panic!()
        };
        app.query.folder = "Archive".into();
        app.set_mail_page(snapshot(&app, &sent, "INBOX", true, 1));
        assert_eq!(count(&app), 0);
        let _ = app.flags_finished(request, sent, Err("Rejected".into()));
        assert_eq!(count(&app), 1);
    }
    #[tokio::test]
    async fn inbox_moves_and_undo_count_once_before_and_after_cache_commit() {
        for cached_before_receipt in [false, true] {
            let (mut app, mut commands, detail) = fixture().await;
            let mut mail = detail.summary.clone();
            mail.folder = "Keep".into();
            app.set_mail_page(snapshot(&app, &mail, "Keep", true, 0));
            app.move_mail(mail.clone(), "INBOX".into());
            assert_eq!(count(&app), 1);
            let Command::Move(request, sent, _) = commands.try_recv().unwrap() else {
                panic!()
            };
            if cached_before_receipt {
                app.set_mail_page(snapshot(&app, &mail, "INBOX", true, 1));
                assert_eq!(count(&app), 1);
            }
            let _ = app.move_finished(request, sent, "INBOX".into(), Ok(()));
            assert_eq!(count(&app), 1);
            let tokens = app.action_toasts.current.as_ref().unwrap().undo_tokens();
            app.undo_actions(tokens);
            assert_eq!(count(&app), 0);
        }
    }
    #[tokio::test]
    async fn cross_account_inbox_move_and_undo_reconcile_rekeyed_source() {
        let (mut app, mut commands, detail) = fixture().await;
        let mail = detail.summary.clone();
        app.transfer_mail(mail.clone(), "personal".into(), "INBOX".into());
        assert_eq!(app.page.inbox_unread.get("fixture"), Some(&0));
        assert_eq!(app.page.inbox_unread.get("personal"), Some(&1));
        let Command::Transfer(request, _, _, _) = commands.try_recv().unwrap() else {
            panic!()
        };
        let receipt = Arc::new(MoveReceipt::server(
            &mail,
            "personal",
            "INBOX",
            Some("91.2".into()),
            crate::mail_actions::Fingerprint::of(b"fixture"),
        ));
        let current = receipt.current.as_ref().unwrap().clone();
        let mut page = MailPage::default();
        page.inbox_unread.insert("personal".into(), 1);
        page.observed.insert(mail.id.clone(), None);
        app.set_mail_page(Arc::new(page));
        assert_eq!(count(&app), 1);
        let _ = app.transfer_receipt(request, mail.clone(), Ok(receipt));
        assert_eq!(count(&app), 1);
        let tokens = app.action_toasts.current.as_ref().unwrap().undo_tokens();
        app.undo_actions(tokens);
        assert_eq!(app.page.inbox_unread.get("fixture"), Some(&1));
        assert_eq!(app.page.inbox_unread.get("personal"), Some(&0));
        let mut page = MailPage::default();
        page.inbox_unread.insert("fixture".into(), 1);
        page.observed.insert(current.id, None);
        page.observed.insert(mail.id, None); // Restored mail has yet another server UID.
        app.set_mail_page(Arc::new(page));
        assert_eq!(
            count(&app),
            1,
            "A committed restore is already in the snapshot"
        );
    }
}
