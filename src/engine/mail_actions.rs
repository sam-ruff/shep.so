use super::*;

impl Engine {
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
