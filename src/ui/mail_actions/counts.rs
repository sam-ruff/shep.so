//! Unread Inbox counts are global, independent of the current page or filter.
//! Observe pending identities in the same database snapshot to avoid applying
//! an optimistic change twice after its cache write has already completed.
use super::*;
use std::collections::{BTreeMap, BTreeSet, HashSet};

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

/// The published badge value: projected unread Inbox counts summed once per
/// connected account. Never an incremental counter.
pub(in crate::ui) fn badge_total(
    accounts: &[Account],
    counts: &BTreeMap<String, usize>,
    enabled: bool,
) -> u64 {
    if !enabled {
        return 0;
    }
    accounts
        .iter()
        .map(|account| account.id.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .fold(0u64, |total, id| {
            total.saturating_add(counts.get(id).copied().unwrap_or(0) as u64)
        })
}

pub(super) fn confirm_flags(page: &mut MailPage, previous: &Mail, mail: &Mail) {
    // A reviewed folder scalar belongs to the complete cache snapshot. An
    // intervening receipt must refresh that review before folder confirmation.
    page.folder_count = None;
    // An unobserved, unlisted identity means this snapshot predates the intent,
    // so its counts still hold the previously confirmed state, never the new one.
    let before = member(page, previous);
    let after = before.clone().map(|mut state| {
        state.unread = mail.unread;
        state
    });
    adjust(&mut page.inbox_unread, before.as_ref(), after.as_ref());
    page.observed.insert(mail.id.clone(), after);
}

pub(super) fn confirm_move(page: &mut MailPage, source: &Mail, current: Option<&Mail>) {
    page.folder_count = None;
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
    page.folder_count = None;
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
        ids.extend(
            self.moves
                .values()
                .filter_map(|entry| entry.recovered.as_ref().map(|mail| mail.id.clone())),
        );
        ids.extend(
            self.transfers
                .values()
                .filter_map(|entry| entry.recovered.as_ref().map(|mail| mail.id.clone())),
        );
        let mut ids: Vec<_> = ids.into_iter().collect();
        ids.sort();
        ids
    }
}

impl App {
    pub(in crate::ui) fn invalidate_action_snapshot(&mut self) {
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
        for (original, mail, account, folder) in actions
            .moves
            .values()
            .map(|e| {
                (
                    &e.mail,
                    e.recovered.as_ref().unwrap_or(&e.mail),
                    &e.mail.account_id,
                    &e.destination,
                )
            })
            .chain(actions.transfers.values().map(|e| {
                (
                    &e.mail,
                    e.recovered.as_ref().unwrap_or(&e.mail),
                    &e.account,
                    &e.folder,
                )
            }))
        {
            let destination = MailMembership {
                account: account.clone(),
                folder: folder.clone(),
                unread: mail.unread,
            };
            let before = member(base, mail).unwrap_or_else(|| destination.clone());
            let after = if actions.restoring(&original.id) {
                MailMembership::from(actions.effective(original))
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

    fn connect(app: &mut App, ids: &[&str]) {
        let workspace = Arc::make_mut(&mut app.workspace);
        for id in ids {
            workspace.accounts.push(
                serde_json::from_value(serde_json::json!({
                    "id": id, "name": id, "email": format!("{id}@example.test"),
                    "protocol": "Imap", "host": "imap.example.test", "port": 993,
                    "username": id, "smtp_host": "smtp.example.test", "smtp_port": 465
                }))
                .unwrap(),
            );
        }
    }
    fn inbox_mail(account: &str, uid: &str, folder: &str, unread: bool) -> StoredMail {
        parse_mail(
            account,
            uid,
            folder,
            format!("From: {account}@example.test\r\nSubject: Badge {uid}\r\n\r\nBody {uid}")
                .into_bytes(),
            unread,
            false,
        )
        .unwrap()
    }
    /// Answer the page request the app would send after `Event::Changed`,
    /// using the same observation list as `request_page`.
    async fn answer_page(app: &mut App, store: &crate::store::Store) {
        let mut query = app.query.clone();
        query.project_moves = app.mail_actions.projected_moves();
        query.observe = app.mail_actions.observed_ids();
        if let Some(id) = &app.selected
            && !query.observe.contains(id)
        {
            query.observe.push(id.clone());
        }
        let page = Arc::new(store.query(query).await.unwrap());
        let _ = app.handle(Message::Backend(Event::Page(app.generation, page, false)));
    }
    async fn badge_fixture(
        folder: &str,
    ) -> (
        App,
        tokio::sync::mpsc::Receiver<Command>,
        crate::store::Store,
        Mail,
    ) {
        let store = crate::store::Store::memory().unwrap();
        let mail = inbox_mail("fixture", "42.7", "INBOX", true);
        let summary = mail.summary.clone();
        store.upsert(vec![mail]).await.unwrap();
        let (sender, commands) = engine::CommandSender::network_test_channel();
        let (mut app, _) = App::new();
        app.tx = Some(sender);
        connect(&mut app, &["fixture"]);
        app.query.folder = folder.into();
        app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
        (app, commands, store, summary)
    }

    #[tokio::test]
    async fn badge_is_the_projected_inbox_total_of_connected_accounts_not_an_arrival_counter() {
        let (mut app, _commands, store, mail) = badge_fixture("Archive").await;
        assert_eq!(app.unread_badge_count(), 1);
        let arrival = Arc::new(crate::notifications::Arrival {
            account: "fixture".into(),
            message: mail.id.clone(),
            sender: "fixture@example.test".into(),
            subject: "Badge 42.7".into(),
        });
        for _ in 0..3 {
            let _ = app.handle(Message::Backend(Event::MailArrived(arrival.clone())));
        }
        assert_eq!(
            app.unread_badge_count(),
            1,
            "Arrival notifications never count separately from the projection"
        );
        store
            .upsert(vec![
                inbox_mail("fixture", "42.8", "INBOX", false),
                inbox_mail("fixture", "42.9", "Archive", true),
                inbox_mail("other", "1.1", "INBOX", true),
            ])
            .await
            .unwrap();
        let _ = app.handle(Message::Backend(Event::Changed));
        answer_page(&mut app, &store).await;
        assert_eq!(
            app.unread_badge_count(),
            1,
            "Read, non-Inbox and disconnected-account mail do not raise the badge"
        );
        connect(&mut app, &["other"]);
        assert_eq!(app.unread_badge_count(), 2);
        store
            .upsert(vec![inbox_mail("fixture", "42.10", "INBOX", true)])
            .await
            .unwrap();
        let _ = app.handle(Message::Backend(Event::Changed));
        answer_page(&mut app, &store).await;
        assert_eq!(app.unread_badge_count(), 3);
        Arc::make_mut(&mut app.workspace)
            .accounts
            .retain(|a| a.id != "other");
        assert_eq!(app.unread_badge_count(), 2);
        app.preferences.unread_badge = false;
        assert_eq!(app.unread_badge_count(), 0);
    }

    #[tokio::test]
    async fn acknowledged_flag_change_of_unlisted_inbox_mail_keeps_the_projected_badge() {
        for initially_unread in [true, false] {
            let (mut app, mut commands, store, mut mail) = badge_fixture("Archive").await;
            if !initially_unread {
                store
                    .apply_sync(MailSyncItem::Flags(vec![(mail.id.clone(), false, false)]))
                    .await
                    .unwrap();
                mail.unread = false;
                app.set_mail_page(Arc::new(store.query(app.query.clone()).await.unwrap()));
            }
            let (before, after) = if initially_unread { (1, 0) } else { (0, 1) };
            assert_eq!(app.unread_badge_count(), before);
            assert!(app.page.rows.is_empty(), "The Inbox message is not listed");
            assert!(!app.page.observed.contains_key(&mail.id));
            app.toggle_mail_flag(mail.clone(), true);
            assert_eq!(
                app.unread_badge_count(),
                after,
                "Intent applies immediately"
            );
            let Command::Flags(request, sent, _) = commands.try_recv().unwrap() else {
                panic!("Expected a flag change")
            };
            let _ = app.flags_finished(request, sent.clone(), Ok(()));
            assert_eq!(
                app.unread_badge_count(),
                after,
                "The acknowledgement must not restore the pre-write count (initially unread: {initially_unread})"
            );
            store
                .apply_sync(MailSyncItem::Flags(vec![(
                    mail.id.clone(),
                    sent.unread,
                    false,
                )]))
                .await
                .unwrap();
            answer_page(&mut app, &store).await;
            assert_eq!(app.unread_badge_count(), after, "The requery agrees");
        }
    }

    #[tokio::test]
    async fn badge_ignores_stale_generations_and_follows_inbox_changes_from_any_folder() {
        let (mut app, mut commands, store, mail) = badge_fixture("Archive").await;
        let stale = Arc::new(store.query(app.query.clone()).await.unwrap());
        let stale_generation = app.generation;
        app.toggle_mail_flag(mail.clone(), true);
        assert_eq!(app.unread_badge_count(), 0);
        let _ = app.handle(Message::Backend(Event::Page(
            stale_generation,
            stale,
            false,
        )));
        assert_eq!(
            app.unread_badge_count(),
            0,
            "A page requested before the intent cannot replace the projection"
        );
        // A snapshot taken after the intent but before its cache write.
        answer_page(&mut app, &store).await;
        assert_eq!(app.unread_badge_count(), 0);
        let Command::Flags(request, sent, _) = commands.try_recv().unwrap() else {
            panic!("Expected a flag change")
        };
        store
            .apply_sync(MailSyncItem::Flags(vec![(mail.id.clone(), false, false)]))
            .await
            .unwrap();
        answer_page(&mut app, &store).await;
        assert_eq!(app.unread_badge_count(), 0, "Snapshot includes the write");
        let _ = app.flags_finished(request, sent, Ok(()));
        assert_eq!(app.unread_badge_count(), 0, "Receipt cannot double-apply");
        // Sync marks Inbox mail unread again while another folder stays open.
        store
            .apply_sync(MailSyncItem::Flags(vec![(mail.id.clone(), true, false)]))
            .await
            .unwrap();
        store
            .upsert(vec![inbox_mail("fixture", "42.11", "INBOX", true)])
            .await
            .unwrap();
        let _ = app.handle(Message::Backend(Event::Changed));
        answer_page(&mut app, &store).await;
        assert_eq!(app.query.folder, "Archive");
        assert_eq!(app.unread_badge_count(), 2);
    }

    #[tokio::test]
    async fn archive_and_undo_of_unlisted_inbox_mail_keep_the_projected_badge() {
        let (mut app, mut commands, store, mail) = badge_fixture("Archive").await;
        assert_eq!(app.unread_badge_count(), 1);
        app.move_mail(mail.clone(), "Keep".into());
        assert_eq!(app.unread_badge_count(), 0);
        let Command::Move(request, sent, folder) = commands.try_recv().unwrap() else {
            panic!("Expected a move")
        };
        let _ = app.move_finished(request, sent.clone(), folder, Ok(()));
        assert_eq!(app.unread_badge_count(), 0, "Receipt keeps the archive");
        let mut kept = sent.clone();
        kept.folder = "Keep".into();
        store.relocate_mail(sent, kept.clone()).await.unwrap();
        answer_page(&mut app, &store).await;
        assert_eq!(app.unread_badge_count(), 0);
        let tokens = app.action_toasts.current.as_ref().unwrap().undo_tokens();
        app.undo_actions(tokens);
        assert_eq!(
            app.unread_badge_count(),
            1,
            "Undo restores the unread Inbox mail"
        );
        let Command::UndoMove(request, original, _) = commands.try_recv().unwrap() else {
            panic!("Expected an undo")
        };
        let restored = Arc::new(MoveReceipt::local(&original, "INBOX"));
        let _ = app.undo_finished(request, original, Ok(restored));
        assert_eq!(app.unread_badge_count(), 1);
        store.relocate_mail(kept, mail).await.unwrap();
        answer_page(&mut app, &store).await;
        assert_eq!(app.unread_badge_count(), 1);
    }

    #[test]
    fn badge_total_counts_each_connected_account_once_and_ignores_others() {
        let accounts: Vec<Account> = ["work", "work", "home"]
            .iter()
            .map(|id| {
                serde_json::from_value(serde_json::json!({
                    "id": id, "name": id, "email": "sam@example.test", "protocol": "Imap",
                    "host": "imap.example.test", "port": 993, "username": id,
                    "smtp_host": "smtp.example.test", "smtp_port": 465
                }))
                .unwrap()
            })
            .collect();
        let counts = BTreeMap::from([
            ("work".to_owned(), 3usize),
            ("home".to_owned(), 2),
            ("removed".to_owned(), 9),
        ]);
        assert_eq!(badge_total(&accounts, &counts, true), 5);
        assert_eq!(badge_total(&accounts, &counts, false), 0);
        assert_eq!(badge_total(&accounts[2..], &counts, true), 2);
        assert_eq!(badge_total(&[], &counts, true), 0);
    }
}
