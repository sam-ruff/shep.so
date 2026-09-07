use super::*;
use crate::mail_actions::{
    journal::{MoveRecord, MoveStage, RecoveryAction},
    runner,
};

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
        anyhow::ensure!(
            action != RecoveryAction::Retry || record.stage != MoveStage::Started,
            "Review the unconfirmed move before choosing how to recover it."
        );
        let mut ids = vec![
            record.original.account_id.clone(),
            record.receipt.account.clone(),
        ];
        ids.sort();
        ids.dedup();
        let mut guards = Vec::new();
        for id in ids {
            guards.push(self.account_lock(&id).await);
        }
        let saved = self.store.mail_move(record.token.clone()).await?;
        anyhow::ensure!(
            serde_json::to_string(&saved)? == serde_json::to_string(&record)?,
            "The move changed since this review. Close and reopen its recovery options."
        );
        anyhow::ensure!(!saved.finished(), "This move is already resolved.");
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
        let source_secret = providers::read_secret(&source.id).await?;
        let destination = if source.id != record.receipt.account {
            let account = self.account(&record.receipt.account).await?;
            let secret = providers::read_secret(&account.id).await?;
            Some((account, secret))
        } else {
            None
        };
        let mut connection =
            providers::mail::moves::ImapMoveConnection::new(source, source_secret, destination);
        runner::recover_reviewed(
            &self.store,
            &mut connection,
            record,
            action == RecoveryAction::UseExistingCopy,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_actions::{Fingerprint, Flags, MoveReceipt};
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
