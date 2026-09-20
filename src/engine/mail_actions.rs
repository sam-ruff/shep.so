use super::*;
use crate::mail_actions::{MoveReceipt, connection_key};

impl Engine {
    pub(super) async fn finish_retarget_result(
        result: anyhow::Result<crate::mail_actions::journal::MoveRecord>,
        previous: Option<crate::mail_actions::journal::MoveRecord>,
        output: &mut Output,
    ) -> anyhow::Result<crate::mail_actions::journal::MoveRecord> {
        if let Some(previous) = previous {
            let _ = output.send(Event::MoveRecovered(Arc::new(previous))).await;
        }
        result
    }
    pub(super) async fn refresh_recovered_folders(
        &self,
        mut record: crate::mail_actions::journal::MoveRecord,
    ) -> crate::mail_actions::journal::MoveRecord {
        if self.demo || !record.finished() {
            return record;
        }
        let refresh = async {
            use crate::folder_actions::Connection;
            let account = self.account(&record.receipt.account).await?;
            let password = self.credentials.account_password(&account, false).await?;
            let mut connection =
                providers::mail::folders::ImapFolders::open(&account, &password).await?;
            let catalogue = connection.catalog().await?;
            self.store.save_folder_catalog(account.id, catalogue).await
        }
        .await;
        if let Err(error) = refresh {
            record.error = Some(format!(
                "The message was moved, but refreshing folders failed. Refresh the account to update its folders. {error:#}"
            ));
        }
        record
    }
    /// The wire folder a logical destination such as `Trash` stands for in
    /// `account`, so moves and their cached rows use the physical name.
    pub(super) async fn resolve_destination(
        &self,
        account: &str,
        folder: &str,
    ) -> anyhow::Result<String> {
        let catalog = self.store.cached_folder_catalog(account.to_owned()).await?;
        Ok(crate::folders::resolve_destination(&catalog, folder))
    }
    async fn authorize_mail_mutation(
        &self,
        id: &str,
        group: Option<&crate::bulk::Item>,
    ) -> anyhow::Result<()> {
        let mail = self.store.mail_metadata(id.to_owned()).await?;
        self.store.ensure_folder_idle(mail.account_id).await?;
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
            !mail.is_local_copy(),
            "This copy is stored locally. You can move it between folders in this account or export it."
        );
        // Always lock in the same order to prevent opposing transfers deadlocking.
        let mut ids = [mail.account_id.clone(), destination.clone()];
        ids.sort();
        let (_first, _second) = tokio::time::timeout(Duration::from_secs(600), async {
            let first = self.account_access(&ids[0]).await;
            let second = self.account_access(&ids[1]).await;
            (first, second)
        })
        .await
        .context("The accounts are still busy. Try moving again.")?;
        self.authorize_mail_mutation(&mail.id, group).await?;
        self.store.ensure_folder_idle(destination.clone()).await?;
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
        let folder = self.resolve_destination(&destination.id, &folder).await?;
        if !self.demo {
            let receipt = self
                .durable_move(&source, Some(&destination), mail, &folder, &mut output)
                .await?;
            return Ok((destination, receipt));
        }
        #[cfg(feature = "test-support")]
        crate::test_support::mail_action_delay().await?;
        let fingerprint = self.store.message_fingerprint(mail.id.clone()).await?;
        let mut receipt = MoveReceipt::server(
            mail,
            &destination.id,
            &folder,
            Some(format!("preview-moved-{}", uuid::Uuid::new_v4())),
            fingerprint,
        );
        receipt.connections = vec![
            (source.id.clone(), connection_key(&source)),
            (destination.id.clone(), connection_key(&destination)),
        ];
        self.store
            .relocate_mail(mail.clone(), receipt.current.clone().unwrap())
            .await?;
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
            self.account_access(&mail.account_id),
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
        let folder = &self.resolve_destination(&mail.account_id, folder).await?;
        if mail.is_local_copy() {
            let receipt = MoveReceipt::local(mail, folder);
            self.store
                .relocate_mail(mail.clone(), receipt.current.clone().unwrap())
                .await?;
            return Ok((None, receipt));
        }
        #[cfg(feature = "test-support")]
        if self.demo
            && let Err(error) = crate::test_support::mail_action_delay().await
        {
            return self.preview_local_fallback(mail, folder, error).await;
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
        if !mail.is_local_copy() {
            let account = self.account(&mail.account_id).await?;
            if account.protocol == Protocol::Imap {
                let receipt = self
                    .durable_move(&account, None, mail, folder, &mut output)
                    .await?;
                return Ok((Some(account), receipt));
            }
        }
        let receipt = MoveReceipt::local(mail, folder);
        self.store
            .relocate_mail(mail.clone(), receipt.current.clone().unwrap())
            .await?;
        Ok((None, receipt))
    }

    /// The isolated fixture refuses like a real server would: the row moves on
    /// this device only through the same journal stage production uses.
    #[cfg(feature = "test-support")]
    async fn preview_local_fallback(
        &self,
        mail: &Mail,
        folder: &str,
        error: anyhow::Error,
    ) -> anyhow::Result<(Option<Account>, MoveReceipt)> {
        use crate::mail_actions::{MoveFailure, classify_move_failure, journal::MoveRecord};
        if classify_move_failure(&error) != MoveFailure::Refused {
            return Err(error);
        }
        let account = self.account(&mail.account_id).await?;
        let fingerprint = self.store.message_fingerprint(mail.id.clone()).await?;
        let mut receipt = MoveReceipt::server(mail, &account.id, folder, None, fingerprint);
        receipt.connections = vec![(account.id.clone(), connection_key(&account))];
        let record = self
            .store
            .begin_local_mail_move(MoveRecord::new(mail.clone(), receipt), format!("{error:#}"))
            .await?;
        let mut receipt = record.receipt;
        receipt.local_only = true;
        Ok((None, receipt))
    }

    pub(super) async fn recover_completed_moves(&self, mut output: Output) -> anyhow::Result<()> {
        use crate::mail_actions::{journal::MoveStage, runner};
        let now = chrono::Utc::now().timestamp();
        for record in self.store.mail_move_lookups(now).await? {
            let mut accounts = vec![
                record.original.account_id.clone(),
                record.receipt.account.clone(),
            ];
            accounts.sort();
            accounts.dedup();
            let mut guards = Vec::new();
            for id in &accounts {
                guards.push(self.account_access(id).await);
            }
            let Some(saved) = self
                .store
                .mail_move_for_source(record.original.id.clone())
                .await?
            else {
                continue;
            };
            if saved.token != record.token
                || !matches!(
                    saved.stage,
                    MoveStage::Started | MoveStage::Committed | MoveStage::Local
                )
                || saved.attempted != record.attempted
            {
                continue;
            }
            if self
                .store
                .bulk_owner(record.original.id.clone())
                .await?
                .is_some()
            {
                continue;
            }
            let record = self.store.begin_mail_move_lookup(saved, now).await?;
            let result = async {
                for id in &accounts {
                    self.store.ensure_folder_idle(id.clone()).await?;
                }
                let source = self.account(&record.original.account_id).await?;
                let destination = if source.id != record.receipt.account {
                    Some(self.account(&record.receipt.account).await?)
                } else {
                    None
                };
                let mut connection = self.move_connections.open(source, destination).await?;
                let recovered =
                    runner::recover(&self.store, connection.as_mut(), record.clone()).await?;
                Ok::<_, anyhow::Error>(self.refresh_recovered_folders(recovered).await)
            }
            .await;
            match result {
                Ok(record) => {
                    // A device-only move that is refused again stays quiet; the
                    // row already shows its state and the next pass retries.
                    if let Some(error) = &record.error
                        && record.stage != MoveStage::Local
                    {
                        output.send(Event::Error(error.clone())).await?;
                    }
                    output.send(Event::MoveRecovered(Arc::new(record))).await?;
                    self.workspace(&mut output).await?;
                }
                Err(_) if record.stage.unsubmitted() => {}
                Err(error) => output
                    .send(Event::Error(format!(
                        "The cached message is retained. Move recovery needs attention: {error:#}"
                    )))
                    .await?,
            }
            output.send(Event::Changed).await?;
        }
        Ok(())
    }

    async fn durable_move(
        &self,
        source: &Account,
        destination: Option<&Account>,
        mail: &Mail,
        folder: &str,
        output: &mut Output,
    ) -> anyhow::Result<MoveReceipt> {
        use crate::mail_actions::{journal::MoveRecord, runner};
        let target = destination.unwrap_or(source);
        let mut connection = self
            .move_connections
            .open(source.clone(), destination.cloned())
            .await?;
        let connection = connection.as_mut();
        let mut current_mail = mail.clone();
        let mut previous_recovery = None;
        let mut existing = self.store.mail_move_for_source(mail.id.clone()).await?;
        if let Some(record) = &existing
            && (record.receipt.account != target.id || record.receipt.folder != folder)
        {
            anyhow::ensure!(
                record.receipt.account == source.id,
                "The earlier cross-account transfer needs its destination checked before moving this message elsewhere."
            );
            let mut previous = self.move_connections.open(source.clone(), None).await?;
            current_mail =
                runner::retarget_source(&self.store, previous.as_mut(), record.clone()).await?;
            if current_mail.id != record.original.id {
                previous_recovery = Some(self.store.mail_move(record.token.clone()).await?);
            }
            existing = None;
        }
        let mail = &current_mail;
        let result = async {
        let record = if let Some(record) = existing {
            anyhow::ensure!(
                record.receipt.account == target.id && record.receipt.folder == folder,
                "A move is pending for this message. Review its original destination before moving elsewhere."
            );
            runner::recover(&self.store, connection, record).await?
        } else {
            let fingerprint = self.store.message_fingerprint(mail.id.clone()).await?;
            let mut receipt = MoveReceipt::server(mail, &target.id, folder, None, fingerprint);
            receipt.connections = vec![(source.id.clone(), connection_key(source))];
            if let Some(account) = destination {
                receipt
                    .connections
                    .push((account.id.clone(), connection_key(account)));
            }
            let record = MoveRecord::new(mail.clone(), receipt);
            let legacy: Option<(String, String, String)> =
                self.store.get(&format!("transfer:{}", mail.id)).await?;
            if legacy.is_some() {
                let record = self.store.adopt_legacy_mail_move(record).await?;
                runner::recover(&self.store, connection, record).await?
            } else {
                runner::start(&self.store, connection, record).await?
            }
        };
        Ok(record)
        }.await;
        let record = Self::finish_retarget_result(result, previous_recovery, output).await?;
        if record.stage == crate::mail_actions::journal::MoveStage::Local {
            // The refusal reason travels in the receipt notice, not as an error.
            let mut receipt = record.receipt;
            receipt.local_only = true;
            return Ok(receipt);
        }
        if let Some(error) = record.error {
            // A closed view cannot negate an already durable acknowledgment.
            let _ = output.send(Event::Error(error)).await;
        }
        Ok(record.receipt)
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
            let fail = self
                .store
                .run(|db| {
                    Ok(db.execute(
                        "INSERT OR IGNORE INTO kv(key,value) VALUES('preview-undo-failed','true')",
                        [],
                    )? == 1)
                })
                .await?;
            if fail {
                tokio::time::sleep(Duration::from_millis(1800)).await;
                anyhow::bail!("Fixture server rejected Undo. Retry is available.");
            }
        }
        if let Some(restored) = self.undo_local_move(&original, receipt, group).await? {
            return Ok((None, restored));
        }
        if receipt.fingerprint.is_none() {
            anyhow::ensure!(
                receipt.account == original.account_id,
                "A local move cannot change accounts."
            );
            let _guard = tokio::time::timeout(
                Duration::from_secs(600),
                self.account_access(&receipt.account),
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
        if let Some(token) = &receipt.recovery {
            self.store
                .mail_move_for_undo(token.clone(), original.clone(), receipt.clone())
                .await?;
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
                tokio::time::timeout(Duration::from_secs(600), self.account_access(&account.id))
                    .await
                    .context("The account is still busy. Retry Undo.")?;
            self.store.ensure_folder_idle(account.id.clone()).await?;
            let mut resolved = tokio::time::timeout(Duration::from_secs(120), async {
                let secret = self.credentials.account_password(&account, false).await?;
                providers::mail::recovery::resolve(&account, &secret, receipt).await
            })
            .await
            .context("Finding the moved message timed out. Retry Undo.")??;
            resolved.summary.timestamp = original.timestamp;
            let current = resolved.summary.clone();
            if let Some(token) = &receipt.recovery {
                let record = self
                    .store
                    .mail_move_for_undo(token.clone(), original.clone(), receipt.clone())
                    .await?;
                if record.stage == crate::mail_actions::journal::MoveStage::Committed {
                    self.store.resolve_mail_move(record, resolved).await?;
                } else {
                    anyhow::ensure!(
                        record.stage == crate::mail_actions::journal::MoveStage::Located,
                        "Finish recovering the original move before Undo."
                    );
                    self.store.upsert(vec![resolved]).await?;
                }
            } else {
                self.store.upsert(vec![resolved]).await?;
            }
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

    /// A device-only move never reached the server, so Undo releases its record
    /// and the untouched row shows in its source folder again.
    async fn undo_local_move(
        &self,
        original: &Mail,
        receipt: &MoveReceipt,
        group: Option<&crate::bulk::Item>,
    ) -> anyhow::Result<Option<MoveReceipt>> {
        use crate::mail_actions::journal::MoveStage;
        let Some(token) = &receipt.recovery else {
            return Ok(None);
        };
        let Some(record) = self
            .store
            .mail_move_for_source(original.id.clone())
            .await?
            .filter(|record| record.token == *token && record.stage == MoveStage::Local)
        else {
            return Ok(None);
        };
        let _guard = tokio::time::timeout(
            Duration::from_secs(600),
            self.account_access(&record.original.account_id),
        )
        .await
        .context("The account is still busy. Retry Undo.")?;
        record.validate_receipt(receipt)?;
        self.authorize_mail_mutation(&record.original.id, group)
            .await?;
        self.store.reject_mail_move(record.clone()).await?;
        let current = self.store.mail_metadata(record.original.id).await?;
        let restored = MoveReceipt::local(&current, &current.folder);
        Ok(Some(restored))
    }

    pub(super) async fn change_flags(
        &self,
        mail: &Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        let _guard = tokio::time::timeout(
            Duration::from_secs(600),
            self.account_access(&mail.account_id),
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
            self.account_access(&original.account_id),
        )
        .await
        .context("The account is still busy. Try the change again.")?;
        self.authorize_mail_mutation(&original.id, Some(item))
            .await?;
        let changes = self
            .store
            .accepted_bulk_flags(item.clone(), changes)
            .await?;
        if changes.is_empty() {
            return Ok(crate::bulk::Receipt::Superseded);
        }
        let mail = self.store.mail_metadata(original.id.clone()).await?;
        anyhow::ensure!(
            mail.account_id == original.account_id
                && mail.folder == original.folder
                && mail.remote_id == original.remote_id,
            "The message changed folders. Refresh it before Undo."
        );
        if let Some(expected) = expected {
            anyhow::ensure!(
                (changes.unread.is_none() || expected.unread.is_none_or(|v| v == mail.unread))
                    && (changes.starred.is_none()
                        || expected.starred.is_none_or(|v| v == mail.starred)),
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
        self.send_flags(&mail, changes).await?;
        let receipt = crate::bulk::Receipt::Flags {
            before,
            after: changes,
        };
        self.store
            .acknowledge_bulk_flags(item.clone(), receipt.clone())
            .await?;
        Ok(receipt)
    }
    async fn write_flags(
        &self,
        mail: &Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        self.send_flags(mail, changes).await?;
        self.store.patch_flags(mail.clone(), changes).await
    }
    async fn send_flags(
        &self,
        mail: &Mail,
        changes: crate::mail_actions::Flags,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.store
                .mail_move_for_source(mail.id.clone())
                .await?
                .is_none_or(|record| record.finished()),
            "This message has an unfinished move. Review its recovery before changing flags."
        );
        #[cfg(feature = "test-support")]
        if self.demo {
            crate::test_support::mail_action_delay().await?;
        }
        if !self.demo && !mail.is_local_copy() {
            let account = self.account(&mail.account_id).await?;
            if account.protocol == Protocol::Imap {
                tokio::time::timeout(Duration::from_secs(45), async {
                    let password = self.credentials.account_password(&account, false).await?;
                    providers::mail::provider(account.protocol)
                        .set_flags(&account, &password, mail, changes)
                        .await
                })
                .await
                .context("The server did not confirm the change. Refresh and try again.")??;
            }
        }
        Ok(())
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

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn logical_trash_moves_land_in_the_special_use_folder_and_undo_restores_inbox() {
        use crate::folders::{FolderRole, Mailbox};
        let engine = crate::engine::calendar_tests::engine();
        crate::test_support::seed_demo(&engine.store).await.unwrap();
        let original = engine.store.query(MailQuery::default()).await.unwrap().rows[0].clone();
        engine
            .store
            .save_folder_catalog(
                original.account_id.clone(),
                vec![
                    Mailbox::flat("INBOX".into()),
                    Mailbox {
                        role: Some(FolderRole::Trash),
                        ..Mailbox::flat("Deleted Items".into())
                    },
                ],
            )
            .await
            .unwrap();
        let (output, _events) = futures::channel::mpsc::channel(32);
        let (_, receipt) = engine
            .change_folder(&original, "Trash", output.clone(), None)
            .await
            .unwrap();
        assert_eq!(receipt.folder, "Deleted Items");
        let moved = receipt.current.clone().unwrap();
        assert_eq!(moved.folder, "Deleted Items");
        assert_eq!(
            engine
                .store
                .mail_metadata(moved.id.clone())
                .await
                .unwrap()
                .folder,
            "Deleted Items"
        );
        let trash = engine
            .store
            .query(MailQuery {
                folder: "Trash".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(trash.rows.iter().any(|row| row.id == moved.id));
        let (_, restored) = engine
            .undo_move(original.clone(), &receipt, output.clone(), None)
            .await
            .unwrap();
        let restored = restored.current.unwrap();
        assert_eq!(restored.folder, original.folder);
        let (_, archived) = engine
            .change_folder(&restored, "Archive", output, None)
            .await
            .unwrap();
        assert_eq!(
            archived.folder, "Archive",
            "no special-use folder keeps the literal name"
        );
    }

    /// Scripts the provider behind the engine's move-connection seam. No
    /// credentials are read and nothing leaves the process.
    #[cfg(feature = "test-support")]
    struct ScriptedMoves {
        reply: std::sync::Mutex<ScriptedReply>,
    }
    #[cfg(feature = "test-support")]
    #[derive(Clone, Copy)]
    enum ScriptedReply {
        Refuse,
        Unconfirmed,
        Accept,
    }
    #[cfg(feature = "test-support")]
    #[async_trait::async_trait]
    impl providers::mail::moves::MoveConnections for ScriptedMoves {
        async fn open(
            &self,
            source: Account,
            destination: Option<Account>,
        ) -> anyhow::Result<Box<dyn crate::mail_actions::runner::Connection>> {
            use crate::mail_actions::runner::{Inspection, MockConnection, SubmissionError};
            assert!(destination.is_none(), "same-account moves only");
            let reply = *self.reply.lock().unwrap();
            let identities = vec![(source.id.clone(), connection_key(&source))];
            let mut connection = MockConnection::new();
            connection
                .expect_identities()
                .returning(move || identities.clone());
            connection
                .expect_inspect()
                .returning(|_| Ok(Inspection::SourceIntactNoDestinationCopy));
            connection.expect_prepare().returning(|_| Ok(()));
            connection
                .expect_submit()
                .returning(move |_, _| match reply {
                    ScriptedReply::Refuse => Err(SubmissionError::NotApplied(
                        "NO [CANNOT] read-only mailbox".into(),
                    )),
                    ScriptedReply::Unconfirmed => {
                        Err(SubmissionError::Unconfirmed("connection lost".into()))
                    }
                    ScriptedReply::Accept => Ok(Some("91.38".into())),
                });
            Ok(Box::new(connection))
        }
    }
    #[cfg(feature = "test-support")]
    async fn scripted_engine(reply: ScriptedReply) -> (Engine, Arc<ScriptedMoves>, Mail) {
        let mut engine = crate::engine::calendar_tests::engine();
        crate::test_support::seed_demo(&engine.store).await.unwrap();
        let scripted = Arc::new(ScriptedMoves {
            reply: std::sync::Mutex::new(reply),
        });
        engine.move_connections = scripted.clone();
        engine.demo = false;
        let original = engine.store.query(MailQuery::default()).await.unwrap().rows[0].clone();
        (engine, scripted, original)
    }
    #[cfg(feature = "test-support")]
    async fn in_folder(store: &Store, folder: &str) -> MailPage {
        store
            .query(MailQuery {
                folder: folder.into(),
                ..Default::default()
            })
            .await
            .unwrap()
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn refused_server_move_completes_on_this_device_and_a_later_check_finishes_it() {
        use crate::mail_actions::journal::MoveStage;
        let (mut engine, scripted, original) = scripted_engine(ScriptedReply::Refuse).await;
        let (output, mut events) = futures::channel::mpsc::channel(32);
        let (_, receipt) = engine
            .change_folder(&original, "Archive", output.clone(), None)
            .await
            .unwrap();
        assert!(receipt.local_only);
        assert!(receipt.current.is_none());
        assert_eq!(receipt.folder, "Archive");
        assert!(
            events.try_recv().is_err(),
            "a refusal that completed locally is not an error event"
        );
        let store = engine.store.clone();
        assert!(
            !in_folder(&store, "INBOX")
                .await
                .rows
                .iter()
                .any(|m| m.id == original.id)
        );
        let archive = in_folder(&store, "Archive").await;
        let row = archive.rows.iter().find(|m| m.id == original.id).unwrap();
        assert!(row.remote_id.is_empty());
        assert_eq!(archive.move_recovery[&original.id].stage, MoveStage::Local);
        store
            .apply_sync(MailSyncItem::Reconcile {
                account: original.account_id.clone(),
                folder: "INBOX".into(),
                live_ids: [original.id.clone()].into_iter().collect(),
            })
            .await
            .unwrap();
        assert!(
            !in_folder(&store, "INBOX")
                .await
                .rows
                .iter()
                .any(|m| m.id == original.id)
        );
        assert!(
            in_folder(&store, "Archive")
                .await
                .rows
                .iter()
                .any(|m| m.id == original.id)
        );

        // The next check retries the server move; once accepted the row gains
        // its real destination identity and the record clears.
        *scripted.reply.lock().unwrap() = ScriptedReply::Accept;
        engine.demo = true;
        engine.recover_completed_moves(output).await.unwrap();
        let recovered = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Event::MoveRecovered(record) = events.next().await.unwrap() {
                    break record;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(recovered.stage, MoveStage::Located);
        let current = recovered.receipt.current.clone().unwrap();
        assert_eq!(
            (current.folder.as_str(), current.remote_id.as_str()),
            ("Archive", "91.38")
        );
        assert!(store.detail(current.id.clone()).await.is_ok());
        assert!(
            store
                .pending_mail_moves(None, None)
                .await
                .unwrap()
                .is_empty()
        );
        let archive = in_folder(&store, "Archive").await;
        assert!(
            archive
                .rows
                .iter()
                .any(|m| m.id == current.id && m.remote_id == "91.38")
        );
    }

    #[cfg(feature = "test-support")]
    #[tokio::test]
    async fn transient_failure_keeps_the_row_in_place_and_undo_releases_a_device_only_move() {
        let (engine, scripted, original) = scripted_engine(ScriptedReply::Unconfirmed).await;
        let (output, _events) = futures::channel::mpsc::channel(32);
        let error = engine
            .change_folder(&original, "Archive", output.clone(), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unconfirmed"), "{error:#}");
        let store = engine.store.clone();
        let record = store
            .mail_move_for_source(original.id.clone())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.stage,
            crate::mail_actions::journal::MoveStage::Started
        );
        assert!(
            !in_folder(&store, "Archive")
                .await
                .rows
                .iter()
                .any(|m| m.id == original.id),
            "an unconfirmed move is never shown at its destination"
        );
        assert!(record.error.as_deref().unwrap().contains("unconfirmed"));

        let (engine, _, original) = scripted_engine(ScriptedReply::Refuse).await;
        *scripted.reply.lock().unwrap() = ScriptedReply::Refuse;
        let (_, receipt) = engine
            .change_folder(&original, "Trash", output.clone(), None)
            .await
            .unwrap();
        assert!(receipt.local_only);
        let (refresh, restored) = engine
            .undo_move(original.clone(), &receipt, output, None)
            .await
            .unwrap();
        assert!(refresh.is_none(), "no server round trip");
        let current = restored.current.unwrap();
        assert_eq!(current.id, original.id);
        assert_eq!(current.folder, original.folder);
        assert_eq!(current.remote_id, original.remote_id);
        let store = engine.store.clone();
        assert!(
            store
                .mail_move_for_source(original.id.clone())
                .await
                .unwrap()
                .is_none()
        );
        let inbox = in_folder(&store, "INBOX").await;
        let row = inbox.rows.iter().find(|m| m.id == original.id).unwrap();
        assert_eq!(row.remote_id, original.remote_id);
        assert!(!inbox.move_placeholders.contains(&original.id));
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
