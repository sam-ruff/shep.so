use super::*;
use crate::mail_actions::{
    journal::{MoveRecord, RecoveryAction},
    runner,
};

fn recovery_snapshot(
    reviewed: &MoveRecord,
    saved: MoveRecord,
    action: RecoveryAction,
) -> anyhow::Result<MoveRecord> {
    if action == RecoveryAction::Retry {
        reviewed.validate_receipt(&saved.receipt)?;
        anyhow::ensure!(
            reviewed.token == saved.token
                && serde_json::to_string(&reviewed.original)?
                    == serde_json::to_string(&saved.original)?,
            "The original move changed. Refresh before retrying."
        );
        return Ok(saved);
    }
    anyhow::ensure!(
        serde_json::to_string(&saved)? == serde_json::to_string(reviewed)?,
        "The move changed since this review. Close and reopen its recovery options."
    );
    Ok(saved)
}

impl Engine {
    pub(super) async fn recover_mail_move(
        &self,
        record: MoveRecord,
        action: RecoveryAction,
        confirmed: bool,
    ) -> anyhow::Result<MoveRecord> {
        anyhow::ensure!(
            action == RecoveryAction::Retry || confirmed,
            "Review and confirm this recovery choice first."
        );
        let mut ids = vec![
            record.original.account_id.clone(),
            record.receipt.account.clone(),
        ];
        ids.sort();
        ids.dedup();
        let mut guards = Vec::new();
        for id in ids {
            guards.push(self.account_access(&id).await);
        }
        let saved = self.store.mail_move(record.token.clone()).await?;
        let record = recovery_snapshot(&record, saved, action)?;
        if action == RecoveryAction::Retry && record.finished() {
            return Ok(record);
        }
        anyhow::ensure!(!record.finished(), "This move is already resolved.");
        anyhow::ensure!(
            self.store
                .bulk_owner(record.original.id.clone())
                .await?
                .is_none(),
            "This move belongs to a pending group action. Review that group in Mail changes first."
        );
        for id in [&record.original.account_id, &record.receipt.account] {
            self.store.ensure_folder_idle(id.clone()).await?;
        }
        if action == RecoveryAction::KeepLocal {
            return self.store.keep_mail_move(record, true).await;
        }
        if self.demo {
            #[cfg(feature = "test-support")]
            return crate::test_support::recover_move(&self.store, record, action).await;
            #[cfg(not(feature = "test-support"))]
            anyhow::bail!("Move recovery needs a connected account.");
        }
        let source = self.account(&record.original.account_id).await?;
        let source_secret = self.credentials.read(&source.id).await?;
        let destination = if source.id != record.receipt.account {
            let account = self.account(&record.receipt.account).await?;
            let secret = self.credentials.read(&account.id).await?;
            Some((account, secret))
        } else {
            None
        };
        let mut connection =
            providers::mail::moves::ImapMoveConnection::new(source, source_secret, destination);
        if action == RecoveryAction::Retry {
            let recovered = runner::recover(&self.store, &mut connection, record).await?;
            return Ok(self.refresh_recovered_folders(recovered).await);
        }
        let recovered = runner::recover_reviewed(
            &self.store,
            &mut connection,
            record,
            action == RecoveryAction::UseExistingCopy,
        )
        .await?;
        Ok(self.refresh_recovered_folders(recovered).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_actions::{Fingerprint, Flags, MoveReceipt};

    async fn acknowledged_move(
        store: &Store,
        original: Mail,
        folder: &str,
        remote: &str,
    ) -> MoveRecord {
        let fingerprint = store
            .message_fingerprint(original.id.clone())
            .await
            .unwrap();
        let record = MoveRecord::new(
            original.clone(),
            MoveReceipt::server(
                &original,
                &original.account_id,
                folder,
                None,
                fingerprint.clone(),
            ),
        );
        store.prepare_mail_move(record.clone()).await.unwrap();
        let mut receipt = record.receipt.clone();
        receipt.current = MoveReceipt::server(
            &original,
            &original.account_id,
            folder,
            Some(remote.into()),
            fingerprint,
        )
        .current;
        let committed = store
            .checkpoint_mail_move(
                record,
                crate::mail_actions::journal::MoveStage::Committed,
                receipt,
            )
            .await
            .unwrap();
        store.commit_mail_move_cache(committed).await.unwrap()
    }

    #[tokio::test]
    async fn engine_undo_retains_initial_folder_through_verified_retarget_chain() {
        let engine = crate::engine::calendar_tests::engine();
        let message = parse_mail(
            "work",
            "42.7",
            "INBOX",
            b"Subject: Keepsake\r\n\r\nOriginal content".to_vec(),
            false,
            true,
        )
        .unwrap();
        let original = message.summary.clone();
        engine.store.upsert(vec![message.clone()]).await.unwrap();
        let first = acknowledged_move(&engine.store, original.clone(), "Archive", "91.38").await;
        let current = first.receipt.current.unwrap();
        let second = acknowledged_move(&engine.store, current.clone(), "Projects", "92.44").await;
        let (output, _) = futures::channel::mpsc::channel(32);
        let mut wrong = original.clone();
        wrong.folder = "Elsewhere".into();
        assert!(
            engine
                .undo_move(wrong, &second.receipt, output.clone(), None)
                .await
                .is_err()
        );
        let mut wrong = original.clone();
        wrong.remote_id = "42.8".into();
        assert!(
            engine
                .undo_move(wrong, &second.receipt, output.clone(), None)
                .await
                .is_err()
        );
        let mut wrong_receipt = second.receipt.clone();
        wrong_receipt.fingerprint = Some(Fingerprint::of(b"Unrelated content"));
        assert!(
            engine
                .undo_move(original.clone(), &wrong_receipt, output.clone(), None)
                .await
                .is_err()
        );
        let (_, restored) = engine
            .undo_move(original.clone(), &second.receipt, output, None)
            .await
            .unwrap();
        let restored = restored.current.unwrap();
        assert_eq!(restored.folder, original.folder);
        assert!(!restored.unread);
        assert!(restored.starred);
        assert_eq!(
            engine.store.raw_message(restored.id).await.unwrap(),
            message.raw
        );
        assert!(engine.store.mail_metadata(current.id).await.is_err());
    }

    #[tokio::test]
    async fn retarget_success_publishes_physical_source_before_known_or_missing_uid_receipt() {
        use crate::mail_actions::runner::{self, MockConnection};
        for known_uid in [false, true] {
            let engine = crate::engine::calendar_tests::engine();
            let message = parse_mail(
                "work",
                "42.7",
                "INBOX",
                b"Subject: Keepsake\r\n\r\nOriginal content".to_vec(),
                false,
                true,
            )
            .unwrap();
            engine.store.upsert(vec![message.clone()]).await.unwrap();
            let previous =
                acknowledged_move(&engine.store, message.summary.clone(), "Archive", "91.38").await;
            let current = previous.receipt.current.clone().unwrap();
            let mut receipt = MoveReceipt::server(
                &current,
                "work",
                "Projects",
                None,
                Fingerprint::of(&message.raw),
            );
            receipt.connections = vec![("work".into(), "mock".into())];
            let mut connection = MockConnection::new();
            connection
                .expect_identities()
                .returning(|| vec![("work".into(), "mock".into())]);
            connection.expect_prepare().times(1).returning(|_| Ok(()));
            connection
                .expect_submit()
                .times(1)
                .returning(move |_, raw| {
                    assert!(raw.is_none());
                    Ok(known_uid.then(|| "92.44".into()))
                });
            let result = runner::start(
                &engine.store,
                &mut connection,
                MoveRecord::new(current.clone(), receipt),
            )
            .await;
            let (mut output, mut events) = futures::channel::mpsc::channel(4);
            let result = Engine::finish_retarget_result(result, Some(previous), &mut output)
                .await
                .unwrap();
            let Some(Event::MoveRecovered(prior)) = events.next().await else {
                panic!("The physical source must precede the final receipt");
            };
            assert_eq!(prior.original.id, message.summary.id);
            assert_eq!(
                prior.receipt.current.as_ref().unwrap().id,
                result.original.id
            );
            assert_eq!(result.original.id, current.id);
            assert_eq!(result.receipt.current.is_some(), known_uid);
            let final_record = if known_uid {
                result
            } else {
                assert_eq!(
                    result.stage,
                    crate::mail_actions::journal::MoveStage::Committed
                );
                let placeholder = engine.store.detail(current.id.clone()).await.unwrap();
                assert_eq!(placeholder.summary.folder, "Projects");
                assert!(placeholder.summary.remote_id.is_empty());
                assert_eq!(
                    engine.store.raw_message(current.id.clone()).await.unwrap(),
                    message.raw
                );
                let alias = engine
                    .store
                    .detail(message.summary.id.clone())
                    .await
                    .unwrap();
                assert_eq!(alias.summary.id, current.id);
                assert_eq!(alias.summary.folder, "Projects");
                assert!(
                    engine
                        .store
                        .raw_message(message.summary.id.clone())
                        .await
                        .is_err()
                );
                let mut review = result.clone();
                review.original = prior.receipt.current.clone().unwrap();
                assert!(recovery_snapshot(&review, result.clone(), RecoveryAction::Retry).is_ok());
                review.original = message.summary.clone();
                assert!(recovery_snapshot(&review, result.clone(), RecoveryAction::Retry).is_err());
                let mut found = parse_mail(
                    "work",
                    "92.44",
                    "Projects",
                    message.raw.clone(),
                    false,
                    true,
                )
                .unwrap();
                found.summary.timestamp = message.summary.timestamp;
                engine.store.resolve_mail_move(result, found).await.unwrap()
            };
            let checked = engine
                .store
                .mail_move_for_undo(
                    final_record.token,
                    message.summary.clone(),
                    final_record.receipt,
                )
                .await
                .unwrap();
            assert_eq!(checked.original.id, current.id);
            assert_eq!(checked.receipt.current.as_ref().unwrap().folder, "Projects");
        }
    }

    #[tokio::test]
    async fn failed_retarget_reports_verified_previous_copy_before_returning_failure() {
        use crate::mail_actions::runner::{self, MockConnection};
        let engine = crate::engine::calendar_tests::engine();
        let message = parse_mail(
            "work",
            "42.7",
            "INBOX",
            b"Subject: Keepsake\r\n\r\nOriginal content".to_vec(),
            false,
            true,
        )
        .unwrap();
        engine.store.upsert(vec![message.clone()]).await.unwrap();
        let previous =
            acknowledged_move(&engine.store, message.summary.clone(), "Archive", "91.38").await;
        let current = previous.receipt.current.clone().unwrap();
        let mut receipt = MoveReceipt::server(
            &current,
            "work",
            "Projects",
            None,
            Fingerprint::of(&message.raw),
        );
        receipt.connections = vec![("work".into(), "mock".into())];
        let mut connection = MockConnection::new();
        connection
            .expect_identities()
            .returning(|| vec![("work".into(), "mock".into())]);
        connection
            .expect_prepare()
            .times(1)
            .returning(|_| anyhow::bail!("The requested destination is unavailable"));
        connection.expect_submit().times(0);
        let result = runner::start(
            &engine.store,
            &mut connection,
            MoveRecord::new(current.clone(), receipt),
        )
        .await;
        let (mut output, mut events) = futures::channel::mpsc::channel(4);
        assert!(
            Engine::finish_retarget_result(result, Some(previous), &mut output)
                .await
                .is_err()
        );
        let Some(Event::MoveRecovered(recovered)) = events.next().await else {
            panic!("Expected physical source recovery before the latest failure");
        };
        assert_eq!(recovered.original.id, message.summary.id);
        assert_eq!(recovered.receipt.current.as_ref().unwrap().id, current.id);
        assert_eq!(
            engine.store.raw_message(current.id.clone()).await.unwrap(),
            message.raw
        );
        assert!(
            engine
                .store
                .mail_move_for_source(current.id)
                .await
                .unwrap()
                .is_none()
        );
    }
    #[test]
    fn retry_adopts_background_progress_but_reviewed_decisions_require_exact_snapshot() {
        let original = parse_mail(
            "work",
            "42.7",
            "INBOX",
            b"Subject: receipt\r\n\r\nOriginal".to_vec(),
            true,
            false,
        )
        .unwrap();
        let record = MoveRecord::new(
            original.summary.clone(),
            MoveReceipt::server(
                &original.summary,
                "work",
                "Archive",
                None,
                Fingerprint::of(&original.raw),
            ),
        );
        let mut saved = record.clone();
        saved.attempted = 1000;
        saved.error = Some("Earlier background lookup failed".into());
        assert_eq!(
            recovery_snapshot(&record, saved.clone(), RecoveryAction::Retry)
                .unwrap()
                .attempted,
            1000
        );
        for action in [RecoveryAction::KeepLocal, RecoveryAction::UseExistingCopy] {
            assert!(recovery_snapshot(&record, saved.clone(), action).is_err());
        }
        saved.stage = crate::mail_actions::journal::MoveStage::Located;
        assert!(
            recovery_snapshot(&record, saved.clone(), RecoveryAction::Retry)
                .unwrap()
                .finished()
        );
        saved.receipt.folder = "Other".into();
        assert!(recovery_snapshot(&record, saved, RecoveryAction::Retry).is_err());
    }
    #[tokio::test]
    async fn local_recovery_needs_current_review_and_retained_mail_stays_locally_actionable() {
        let mut engine = crate::engine::calendar_tests::engine();
        // Production mode: all exercised paths must complete without a real
        // account, a provider or the user's keychain.
        engine.demo = false;
        let stored = parse_mail(
            "work",
            "42.7",
            "INBOX",
            b"Subject: Keep me\r\n\r\nOriginal bytes".to_vec(),
            true,
            false,
        )
        .unwrap();
        let raw = stored.raw.clone();
        let original = stored.summary.clone();
        engine.store.upsert(vec![stored]).await.unwrap();
        let record = MoveRecord::new(
            original.clone(),
            MoveReceipt::server(&original, "work", "Keep", None, Fingerprint::of(&raw)),
        );
        engine
            .store
            .prepare_mail_move(record.clone())
            .await
            .unwrap();
        for (action, confirmed) in [
            (RecoveryAction::KeepLocal, false),
            (RecoveryAction::UseExistingCopy, false),
            (RecoveryAction::Retry, true),
        ] {
            assert!(
                engine
                    .recover_mail_move(record.clone(), action, confirmed)
                    .await
                    .is_err()
            );
        }
        let latest = engine
            .store
            .fail_mail_move(record.clone(), "Unconfirmed submission".into())
            .await
            .unwrap();
        assert!(
            engine
                .recover_mail_move(record, RecoveryAction::KeepLocal, true)
                .await
                .is_err()
        );
        let kept = engine
            .recover_mail_move(latest, RecoveryAction::KeepLocal, true)
            .await
            .unwrap();
        let mail = kept.retained.unwrap();
        let (output, _events) = futures::channel::mpsc::channel(32);
        engine
            .change_flags(
                &mail,
                Flags {
                    unread: Some(false),
                    starred: Some(true),
                },
            )
            .await
            .unwrap();
        let (_, receipt) = engine
            .change_folder(&mail, "Archive", output.clone(), None)
            .await
            .unwrap();
        assert!(receipt.fingerprint.is_none());
        let (_, restored) = engine
            .undo_move(mail, &receipt, output, None)
            .await
            .unwrap();
        let current = restored.current.unwrap();
        assert!(current.is_local_copy());
        let detail = engine.store.detail(current.id.clone()).await.unwrap();
        assert!(!detail.summary.unread);
        assert!(detail.summary.starred);
        assert_eq!(engine.store.raw_message(current.id).await.unwrap(), raw);
    }
}
