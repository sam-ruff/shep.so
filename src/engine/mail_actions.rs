use super::*;

impl Engine {
    pub(super) async fn transfer_message(
        &self,
        mail: &Mail,
        destination: String,
        folder: String,
        mut output: Output,
    ) -> anyhow::Result<Account> {
        let preferences: Preferences = self.store.get("preferences").await?;
        anyhow::ensure!(
            preferences.cross_account_moves,
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
        let source = self.account(&mail.account_id).await?;
        let destination = self.account(&destination).await?;
        anyhow::ensure!(
            source.protocol == Protocol::Imap && destination.protocol == Protocol::Imap,
            "Moving between accounts requires two IMAP accounts. POP3 keeps server originals."
        );
        let raw = self.store.raw_message(mail.id.clone()).await?;
        if self.demo {
            #[cfg(feature = "test-support")]
            crate::test_support::mail_action_delay().await?;
            let moved = parse_mail(
                &destination.id,
                &format!("local-sent-transfer-{}", uuid::Uuid::new_v4()),
                &folder,
                raw,
                mail.unread,
                mail.starred,
            )?;
            self.store.upsert(vec![moved]).await?;
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
                tokio::time::timeout(Duration::from_secs(60), providers::mail::append_transfer(&destination, &destination_secret, mail, &folder, raw)).await
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
        let journal_key = format!("transfer:{}", mail.id);
        let cleanup = async {
            self.store.remove(mail.id.clone()).await?;
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
        Ok(destination)
    }

    pub(super) async fn change_folder(
        &self,
        mail: &Mail,
        folder: &str,
        mut output: Output,
    ) -> anyhow::Result<Option<Account>> {
        let _guard = tokio::time::timeout(
            Duration::from_secs(600),
            self.account_lock(&mail.account_id),
        )
        .await
        .context("The account is still busy. Try moving again.")?;
        #[cfg(feature = "test-support")]
        if self.demo {
            crate::test_support::mail_action_delay().await?;
        }
        if !self.demo && !mail.remote_id.starts_with("local-sent-") {
            let account = self.account(&mail.account_id).await?;
            tokio::time::timeout(Duration::from_secs(45), async {
                let password = providers::read_secret(&account.id).await?;
                providers::mail::provider(account.protocol)
                    .move_mail(&account, &password, mail, folder)
                    .await
            })
            .await
            .context("The server did not confirm the move. Refresh before trying again.")??;
            if account.protocol == Protocol::Imap {
                // Once MOVE is acknowledged, a cache/refresh failure must not
                // turn it into an apparent server rejection or a repeated MOVE.
                if self.store.remove(mail.id.clone()).await.is_err() {
                    output
                        .send(Event::Error(
                            "The server moved the message. Its cached folders need to refresh."
                                .into(),
                        ))
                        .await?;
                }
                return Ok(Some(account));
            }
        }
        self.store
            .move_local(mail.id.clone(), folder.to_owned())
            .await?;
        Ok(None)
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
