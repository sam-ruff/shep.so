use super::*;
use crate::mail_actions::{MoveReceipt, connection_key};

impl Engine {
    async fn authorize_mail_mutation(
        &self,
        id: &str,
        group: Option<&crate::bulk::Item>,
    ) -> anyhow::Result<()> {
        if let Some(item) = group {
            self.store
                .claim_bulk_identity(item.clone(), id.to_owned())
                .await
        } else {
            anyhow::ensure!(
                self.store.bulk_owner(id.to_owned()).await?.is_none(),
                "A group action is pending for this message. Finish or review it before making another change."
            );
            Ok(())
        }
    }
    pub(super) async fn transfer_message(
        &self,
        mail: &Mail,
        destination: String,
        folder: String,
        output: Output,
        group: Option<&crate::bulk::Item>,
    ) -> anyhow::Result<(Account, MoveReceipt)> {
        self.transfer_with_policy(mail, destination, folder, output, true, group)
            .await
    }

    async fn transfer_with_policy(
        &self,
        mail: &Mail,
        destination: String,
        folder: String,
        mut output: Output,
        check_preference: bool,
        group: Option<&crate::bulk::Item>,
    ) -> anyhow::Result<(Account, MoveReceipt)> {
        let preferences: Preferences = self.store.get("preferences").await?;
        anyhow::ensure!(
            preferences.cross_account_moves || !check_preference,
            "Enable moving between accounts in Preferences first."
        );
        anyhow::ensure!(
            mail.account_id != destination,
            "Choose a different destination account."
        );
        anyhow::ensure!(
            !mail.remote_id.starts_with("local-sent-"),
            "This copy is stored locally. Save and sync its server Sent copy before moving it to another account."
        );
        // Always lock in the same order to prevent opposing transfers deadlocking.
        let mut ids = [mail.account_id.clone(), destination.clone()];
        ids.sort();
        let (_first, _second) = tokio::time::timeout(Duration::from_secs(600), async {
            let first = self.account_lock(&ids[0]).await;
            let second = self.account_lock(&ids[1]).await;
            (first, second)
        })
        .await
        .context("The accounts are still busy. Try moving again.")?;
        self.authorize_mail_mutation(&mail.id, group).await?;
        let current = self.store.mail_metadata(mail.id.clone()).await?;
        anyhow::ensure!(
            current.account_id == mail.account_id && current.folder == mail.folder,
            "The message moved since it was selected. Refresh its folder."
        );
        let mail = &current;
        let source = self.account(&mail.account_id).await?;
        let destination = self.account(&destination).await?;
        anyhow::ensure!(
            source.protocol == Protocol::Imap && destination.protocol == Protocol::Imap,
            "Moving between accounts requires two IMAP accounts. POP3 keeps server originals."
        );
        let fingerprint = self.store.message_fingerprint(mail.id.clone()).await?;
        let mut remote_id = None;
        if self.demo {
            #[cfg(feature = "test-support")]
            crate::test_support::mail_action_delay().await?;
            remote_id = Some(format!("preview-moved-{}", uuid::Uuid::new_v4()));
        } else {
            let source_secret = providers::read_secret(&source.id).await?;
            let destination_secret = providers::read_secret(&destination.id).await?;
            let journal_key = format!("transfer:{}", mail.id);
            let journal: Option<(String, String, String)> = self.store.get(&journal_key).await?;
            if let Some((account, target, stage)) = &journal {
                anyhow::ensure!(
                    account == &destination.id && target == &folder,
                    "A transfer is already pending for this message. Resume with the same destination."
                );
                anyhow::ensure!(
                    stage == "copied",
                    "The previous upload was interrupted. The original is safe. Check the destination in webmail before moving it there manually; Shep will not upload a possible duplicate."
                );
            }
            tokio::time::timeout(
                Duration::from_secs(35),
                providers::mail::prepare_transfer(&source, &source_secret, mail),
            )
            .await??;
            if journal.is_none() {
                self.store
                    .put(
                        &journal_key,
                        Some((
                            destination.id.clone(),
                            folder.clone(),
                            "uploading".to_owned(),
                        )),
                    )
                    .await?;
                let raw = self.store.raw_message(mail.id.clone()).await?;
                remote_id = tokio::time::timeout(Duration::from_secs(60), providers::mail::append_transfer(&destination, &destination_secret, mail, &folder, raw)).await
                            .context("Upload timed out; the source is retained. Check the destination before retrying.")??;
                self.store
                    .put(
                        &journal_key,
                        Some((destination.id.clone(), folder.clone(), "copied".to_owned())),
                    )
                    .await?;
            }
            tokio::time::timeout(Duration::from_secs(35), providers::mail::finish_transfer(&source, &source_secret, mail)).await
                        .context("The destination has a copy; source removal timed out. Retry the same destination to finish without uploading again.")?
                        .context("The destination has a copy; source removal could not be confirmed. Retry the same destination to finish.")?;
        }
        let mut receipt =
            MoveReceipt::server(mail, &destination.id, &folder, remote_id, fingerprint);
        receipt.connections = vec![
            (source.id.clone(), connection_key(&source)),
            (destination.id.clone(), connection_key(&destination)),
        ];
        let journal_key = format!("transfer:{}", mail.id);
        let cleanup = async {
            if let Some(current) = &receipt.current {
                self.store
                    .relocate_mail(mail.clone(), current.clone())
                    .await?;
            } else {
                self.store.remove(mail.id.clone()).await?;
            }
            self.store
                .put(&journal_key, Option::<(String, String, String)>::None)
                .await
        }
        .await;
        if cleanup.is_err() {
            output
                .send(Event::Error(
                    "The message was moved, but its cached folders need to refresh. Try Refresh."
                        .into(),
                ))
                .await?;
        }
        Ok((destination, receipt))
    }

    pub(super) async fn change_folder(
        &self,
        mail: &Mail,
        folder: &str,
        mut output: Output,
        group: Option<&crate::bulk::Item>,
    ) -> anyhow::Result<(Option<Account>, MoveReceipt)> {
        let _guard = tokio::time::timeout(
            Duration::from_secs(600),
            self.account_lock(&mail.account_id),
        )
        .await
        .context("The account is still busy. Try moving again.")?;
        self.authorize_mail_mutation(&mail.id, group).await?;
        let current = self.store.mail_metadata(mail.id.clone()).await?;
        anyhow::ensure!(
            current.account_id == mail.account_id && current.folder == mail.folder,
            "The message moved since it was selected. Refresh its folder."
        );
        let mail = &current;
        #[cfg(feature = "test-support")]
        if self.demo {
            crate::test_support::mail_action_delay().await?;
        }
        if self.demo {
            let fingerprint = self.store.message_fingerprint(mail.id.clone()).await?;
            let mut receipt = MoveReceipt::server(
                mail,
                &mail.account_id,
                folder,
                Some(format!("preview-moved-{}", uuid::Uuid::new_v4())),
                fingerprint,
            );
            if let Ok(account) = self.account(&mail.account_id).await {
                receipt
                    .connections
                    .push((account.id.clone(), connection_key(&account)));
            }
            self.store
                .relocate_mail(mail.clone(), receipt.current.clone().unwrap())
                .await?;
            return Ok((None, receipt));
        }
        if !mail.remote_id.starts_with("local-sent-") {
            let account = self.account(&mail.account_id).await?;
            if account.protocol == Protocol::Imap {
                let fingerprint = self.store.message_fingerprint(mail.id.clone()).await?;
                let remote_id = tokio::time::timeout(Duration::from_secs(45), async {
                    let password = providers::read_secret(&account.id).await?;
                    providers::mail::provider(account.protocol)
                        .move_mail(&account, &password, mail, folder)
                        .await
                })
                .await
                .context("The server did not confirm the move. Refresh before trying again.")??;
                let mut receipt =
                    MoveReceipt::server(mail, &mail.account_id, folder, remote_id, fingerprint);
                receipt
                    .connections
                    .push((account.id.clone(), connection_key(&account)));
                let cleanup = if let Some(current) = &receipt.current {
                    self.store
                        .relocate_mail(mail.clone(), current.clone())
                        .await
                } else {
                    self.store.remove(mail.id.clone()).await
                };
                if cleanup.is_err() {
                    output
                        .send(Event::Error(
                            "The server moved the message. Its cached folders need to refresh."
                                .into(),
                        ))
                        .await?;
                }
                return Ok((Some(account), receipt));
            }
        }
        let receipt = MoveReceipt::local(mail, folder);
        self.store
            .relocate_mail(mail.clone(), receipt.current.clone().unwrap())
            .await?;
        Ok((None, receipt))
    }

    pub(super) async fn undo_move(
        &self,
        original: Mail,
        receipt: &MoveReceipt,
        output: Output,
        group: Option<&crate::bulk::Item>,
    ) -> anyhow::Result<(Option<Account>, MoveReceipt)> {
        #[cfg(feature = "test-support")]
        if self.demo && std::env::args().any(|arg| arg == "--undo-failure-once") {
            let failed: bool = self.store.get("preview-undo-failed").await?;
            if !failed {
                self.store.put("preview-undo-failed", true).await?;
                tokio::time::sleep(Duration::from_millis(1800)).await;
                anyhow::bail!("Fixture server rejected Undo. Retry is available.");
            }
        }
        if receipt.fingerprint.is_none() {
            anyhow::ensure!(
                receipt.account == original.account_id,
                "A local move cannot change accounts."
            );
            let _guard = tokio::time::timeout(
                Duration::from_secs(600),
                self.account_lock(&receipt.account),
            )
            .await
            .context("The account is still busy. Retry Undo.")?;
            let known = receipt
                .current
                .as_ref()
                .context("The moved message has no local identity.")?;
            let current = self.store.mail_metadata(known.id.clone()).await?;
            self.authorize_mail_mutation(&current.id, group).await?;
            anyhow::ensure!(
                current.account_id == receipt.account && current.folder == receipt.folder,
                "This message moved again. Refresh before undoing."
            );
            let restored = MoveReceipt::local(&current, &original.folder);
            self.store
                .relocate_mail(current, restored.current.clone().unwrap())
                .await?;
            return Ok((None, restored));
        }
        let expected: HashSet<_> = [receipt.account.as_str(), original.account_id.as_str()]
            .into_iter()
            .collect();
        if !self.demo || !receipt.connections.is_empty() {
            let actual: HashSet<_> = receipt
                .connections
                .iter()
                .map(|(id, _)| id.as_str())
                .collect();
            anyhow::ensure!(
                expected == actual && actual.len() == receipt.connections.len(),
                "The move's account identity is unavailable. Refresh its folders."
            );
            for (id, key) in &receipt.connections {
                anyhow::ensure!(
                    connection_key(&self.account(id).await?) == *key,
                    "The account's incoming server changed since this move. Restore its connection settings before undoing."
                );
            }
        }
        let current = if self.demo {
            let known = receipt
                .current
                .as_ref()
                .context("The fixture move has no destination identity.")?;
            let current = self.store.mail_metadata(known.id.clone()).await?;
            anyhow::ensure!(
                current.account_id == receipt.account && current.folder == receipt.folder,
                "This message moved again. Refresh before undoing."
            );
            anyhow::ensure!(
                Some(self.store.message_fingerprint(current.id.clone()).await?)
                    == receipt.fingerprint,
                "The moved message changed. Refresh before undoing."
            );
            current
        } else {
            let account = self.account(&receipt.account).await?;
            let _guard =
                tokio::time::timeout(Duration::from_secs(600), self.account_lock(&account.id))
                    .await
                    .context("The account is still busy. Retry Undo.")?;
            let mut resolved = tokio::time::timeout(Duration::from_secs(120), async {
                let secret = providers::read_secret(&account.id).await?;
                providers::mail::recovery::resolve(&account, &secret, receipt).await
            })
            .await
            .context("Finding the moved message timed out. Retry Undo.")??;
            resolved.summary.timestamp = original.timestamp;
            let current = resolved.summary.clone();
            self.store.upsert(vec![resolved]).await?;
            current
        };
        if current.account_id == original.account_id {
            self.change_folder(&current, &original.folder, output, group)
                .await
        } else {
            // Undo reverses an already authorized transfer even if the optional
            // cross-account move preference was switched off afterward.
            self.transfer_with_policy(
                &current,
                original.account_id,
                original.folder,
                output,
                false,
                group,
            )
            .await
            .map(|(account, receipt)| (Some(account), receipt))
        }
    }

    pub(super) async fn change_flags(
        &self,
        mail: &Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        let _guard = tokio::time::timeout(
            Duration::from_secs(600),
            self.account_lock(&mail.account_id),
        )
        .await
        .context("The account is still busy. Try the change again.")?;
        self.authorize_mail_mutation(&mail.id, None).await?;
        self.write_flags(mail, changes).await
    }
    pub(super) async fn change_bulk_flags(
        &self,
        original: &Mail,
        changes: crate::mail_actions::Flags,
        expected: Option<crate::mail_actions::Flags>,
        item: &crate::bulk::Item,
    ) -> anyhow::Result<crate::bulk::Receipt> {
        let _guard = tokio::time::timeout(
            Duration::from_secs(600),
            self.account_lock(&original.account_id),
        )
        .await
        .context("The account is still busy. Try the change again.")?;
        self.authorize_mail_mutation(&original.id, Some(item))
            .await?;
        let mail = self.store.mail_metadata(original.id.clone()).await?;
        anyhow::ensure!(
            mail.account_id == original.account_id && mail.folder == original.folder,
            "The message changed folders. Refresh it before Undo."
        );
        if let Some(expected) = expected {
            anyhow::ensure!(
                expected.unread.is_none_or(|v| v == mail.unread)
                    && expected.starred.is_none_or(|v| v == mail.starred),
                "This message has a newer read or flag change. It was left unchanged."
            );
        }
        let changes = crate::mail_actions::Flags {
            unread: changes.unread.filter(|v| *v != mail.unread),
            starred: changes.starred.filter(|v| *v != mail.starred),
        };
        if changes.is_empty() {
            return Ok(crate::bulk::Receipt::Unchanged);
        }
        let before = crate::mail_actions::Flags {
            unread: changes.unread.map(|_| mail.unread),
            starred: changes.starred.map(|_| mail.starred),
        };
        self.write_flags(&mail, changes).await?;
        Ok(crate::bulk::Receipt::Flags {
            before,
            after: changes,
        })
    }
    async fn write_flags(
        &self,
        mail: &Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        #[cfg(feature = "test-support")]
        if self.demo {
            crate::test_support::mail_action_delay().await?;
        }
        if !self.demo && !mail.remote_id.starts_with("local-sent-") {
            let account = self.account(&mail.account_id).await?;
            if account.protocol == Protocol::Imap {
                tokio::time::timeout(Duration::from_secs(45), async {
                    let password = providers::read_secret(&account.id).await?;
                    providers::mail::provider(account.protocol)
                        .set_flags(&account, &password, mail, changes)
                        .await
                })
                .await
                .context("The server did not confirm the change. Refresh and try again.")??;
            }
        }
        self.store.patch_flags(mail.clone(), changes).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn transfers_report_typed_results_through_dispatch_and_preserve_read_state() {
        let engine = crate::engine::calendar_tests::engine();
        let store = engine.store.clone();
        crate::test_support::seed_demo(&store).await.unwrap();
        let page = store.query(MailQuery::default()).await.unwrap();
        let mut mail = page.rows[0].clone();
        mail.unread = false;
        store.flags(mail.clone()).await.unwrap();
        let (sender, input) = CommandSender::channel();
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let task = tokio::spawn(engine.run(input, output));
        for (request, enabled) in [(1, false), (2, true)] {
            let mut prefs: Preferences = store.get("preferences").await.unwrap();
            prefs.cross_account_moves = enabled;
            store.put("preferences", prefs).await.unwrap();
            sender
                .try_send(Command::Transfer(
                    request,
                    mail.clone(),
                    "preview-personal".into(),
                    "Archive".into(),
                ))
                .unwrap();
            let result = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Event::TransferFinished(id, moved, result) = events.next().await.unwrap()
                    {
                        assert_eq!(id, request);
                        assert_eq!(moved.id, mail.id);
                        break result;
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(result.is_ok(), enabled);
            assert_eq!(store.detail(mail.id.clone()).await.is_err(), enabled);
        }
        let archived = store
            .query(MailQuery {
                account: Some("preview-personal".into()),
                folder: "Archive".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(archived.total, 1);
        assert_eq!(archived.rows[0].subject, mail.subject);
        assert!(!archived.rows[0].unread);
        assert_eq!(archived.rows[0].starred, mail.starred);
        assert_ne!(archived.rows[0].id, mail.id);
        task.abort();
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn undo_uses_the_new_identity_and_restores_original_account_folder_and_flags() {
        for cross_account in [false, true] {
            let engine = crate::engine::calendar_tests::engine();
            let store = engine.store.clone();
            crate::test_support::seed_demo(&store).await.unwrap();
            let mut prefs: Preferences = store.get("preferences").await.unwrap();
            prefs.cross_account_moves = true;
            store.put("preferences", prefs.clone()).await.unwrap();
            let original = store.query(MailQuery::default()).await.unwrap().rows[0].clone();
            let (sender, input) = CommandSender::channel();
            let (output, mut events) = futures::channel::mpsc::channel(32);
            let task = tokio::spawn(engine.run(input, output));
            sender
                .try_send(if cross_account {
                    Command::Transfer(
                        1,
                        original.clone(),
                        "preview-personal".into(),
                        "Keep".into(),
                    )
                } else {
                    Command::Move(1, original.clone(), "Archive".into())
                })
                .unwrap();
            let receipt = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    match events.next().await.unwrap() {
                        Event::MoveFinished(1, _, _, result)
                        | Event::TransferFinished(1, _, result) => break result.unwrap(),
                        _ => {}
                    }
                }
            })
            .await
            .unwrap();
            let moved = receipt.current.as_ref().unwrap().clone();
            assert_ne!(moved.id, original.id);
            assert!(store.detail(original.id.clone()).await.is_err());
            prefs.cross_account_moves = false;
            store.put("preferences", prefs).await.unwrap();
            sender
                .try_send(Command::UndoMove(2, original.clone(), receipt.clone()))
                .unwrap();
            let restored = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Event::UndoFinished(2, _, result) = events.next().await.unwrap() {
                        break result.unwrap().current.clone().unwrap();
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(restored.folder, original.folder);
            assert_eq!(restored.account_id, original.account_id);
            assert_eq!(restored.unread, original.unread);
            assert_eq!(restored.starred, original.starred);
            assert_ne!(restored.id, original.id);
            assert_ne!(restored.id, moved.id);
            assert!(store.detail(moved.id).await.is_err());
            assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 120);
            sender
                .try_send(Command::UndoMove(3, original, receipt))
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Event::UndoFinished(3, _, result) = events.next().await.unwrap() {
                        assert!(result.is_err());
                        break;
                    }
                }
            })
            .await
            .unwrap();
            assert_eq!(store.query(MailQuery::default()).await.unwrap().total, 120);
            task.abort();
        }
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn pop3_undo_is_local_and_preserves_stable_identity_and_flags() {
        let mut engine = crate::engine::calendar_tests::engine();
        crate::test_support::seed_demo(&engine.store).await.unwrap();
        let original = engine.store.query(MailQuery::default()).await.unwrap().rows[0].clone();
        let mut account = engine.account(&original.account_id).await.unwrap();
        account.protocol = Protocol::Pop3;
        engine.store.save_account(account).await.unwrap();
        engine.demo = false;
        let (output, _events) = futures::channel::mpsc::channel(32);
        let (_, receipt) = engine
            .change_folder(&original, "Trash", output.clone(), None)
            .await
            .unwrap();
        assert!(receipt.fingerprint.is_none());
        assert_eq!(receipt.current.as_ref().unwrap().id, original.id);
        let (_, restored) = engine
            .undo_move(original.clone(), &receipt, output, None)
            .await
            .unwrap();
        let current = restored.current.unwrap();
        assert_eq!(current.id, original.id);
        assert_eq!(current.folder, original.folder);
        assert_eq!(current.unread, original.unread);
        assert_eq!(current.starred, original.starred);
        assert_eq!(
            engine.store.mail_metadata(current.id).await.unwrap().folder,
            original.folder
        );
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn changing_an_incoming_server_does_not_redirect_an_old_undo() {
        let engine = crate::engine::calendar_tests::engine();
        crate::test_support::seed_demo(&engine.store).await.unwrap();
        let original = engine.store.query(MailQuery::default()).await.unwrap().rows[0].clone();
        let (output, _events) = futures::channel::mpsc::channel(32);
        let (_, receipt) = engine
            .change_folder(&original, "Archive", output.clone(), None)
            .await
            .unwrap();
        let moved = receipt.current.as_ref().unwrap().clone();
        let mut account = engine.account(&original.account_id).await.unwrap();
        account.host = "different.example.test".into();
        engine.store.save_account(account).await.unwrap();
        let error = engine
            .undo_move(original, &receipt, output, None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("incoming server changed"));
        assert_eq!(
            engine.store.mail_metadata(moved.id).await.unwrap().folder,
            "Archive"
        );
    }

    #[tokio::test]
    async fn read_unread_and_flag_updates_cross_the_dispatcher_and_survive_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mail.sqlite");
        let store = Store::open(&path).unwrap();
        let mut engine = crate::engine::calendar_tests::engine();
        engine.store = store.clone();
        let mail = parse_mail(
            "fixture",
            "42.7",
            "INBOX",
            b"From: fixture@example.test\r\nSubject: Flags\r\n\r\nBody".to_vec(),
            true,
            true,
        )
        .unwrap();
        let summary = mail.summary.clone();
        store.upsert(vec![mail]).await.unwrap();
        let (sender, input) = CommandSender::channel();
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let task = tokio::spawn(engine.run(input, output));
        for (request, unread) in [(1, false), (2, true)] {
            sender
                .try_send(Command::Flags(
                    request,
                    summary.clone(),
                    crate::mail_actions::Flags {
                        unread: Some(unread),
                        starred: None,
                    },
                ))
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Event::FlagsFinished(id, _, result) = events.next().await.unwrap() {
                        assert_eq!(id, request);
                        result.unwrap();
                        break;
                    }
                }
            })
            .await
            .unwrap();
            let detail = store.detail(summary.id.clone()).await.unwrap();
            assert_eq!(detail.summary.unread, unread);
            assert!(detail.summary.starred, "Read action must preserve the flag");
            let page = store
                .query(MailQuery {
                    folder: "INBOX".into(),
                    unread_only: true,
                    ..Default::default()
                })
                .await
                .unwrap();
            assert_eq!(page.total, usize::from(unread));
        }
        task.abort();
        let reopened = Store::open(&path).unwrap();
        assert!(
            reopened
                .detail(summary.id.clone())
                .await
                .unwrap()
                .summary
                .unread
        );
        store.remove(summary.id.clone()).await.unwrap();
        assert!(
            store
                .patch_flags(
                    summary,
                    crate::mail_actions::Flags {
                        unread: Some(false),
                        starred: None
                    }
                )
                .await
                .is_err()
        );
    }
}
