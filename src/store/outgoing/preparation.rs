use super::*;

pub(crate) struct Preparation {
    pub info: OutgoingInfo,
    pub account: Account,
    pub draft: Draft,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct FrozenDraft {
    draft: Draft,
    attachments: Vec<DraftAttachment>,
}

fn pending_preparation(c: &Connection, attempt: &str) -> anyhow::Result<Option<OutgoingInfo>> {
    let data: Option<String> = c
        .query_row(
            "SELECT data FROM outgoing WHERE attempt=? AND stage='Preparing'",
            [attempt],
            |row| row.get(0),
        )
        .optional()?;
    Ok(data.map(|data| serde_json::from_str(&data)).transpose()?)
}

impl Store {
    pub(crate) async fn admit_outgoing(
        &self,
        draft: Draft,
        account: Account,
    ) -> anyhow::Result<OutgoingInfo> {
        self.run(move |c| {
            let tx = c.transaction()?;
            connections::allow(&tx, ConnectionKind::Account, &draft.account_id)?;
            folder_actions::idle(&tx, &draft.account_id)?;
            anyhow::ensure!(account.id == draft.account_id, "The sending account changed.");
            anyhow::ensure!(!drafts::sent(&tx, &draft)?, "This draft was sent or discarded.");
            let latest = drafts::snapshot(&tx)?.drafts.into_iter()
                .find(|saved| saved.id == draft.id).context("Save this draft before sending.")?;
            anyhow::ensure!(serde_json::to_string(&latest)? == serde_json::to_string(&draft)?
                && latest.attachments == draft.attachments,
                "The draft changed. Review its text and attachments before sending.");
            let previous: Option<String> = tx.query_row(
                "SELECT data FROM outgoing WHERE draft=?", [&draft.id], |r| r.get(0),
            ).optional()?;
            if let Some(previous) = previous {
                let previous: OutgoingInfo = serde_json::from_str(&previous)?;
                anyhow::ensure!(matches!(previous.delivery, DeliveryState::Rejected | DeliveryState::Released),
                    "This draft already has a delivery record. Review Outbox before sending again.");
            }
            let attempt = uuid::Uuid::new_v4().to_string();
            let info = OutgoingInfo {
                message_id: format!("<{attempt}@shep.local>"),
                attempt,
                draft_id: draft.id.clone(),
                draft_revision: draft.revision,
                account_id: account.id.clone(),
                from: account.email.clone(),
                subject: draft.subject.clone(),
                to: if !draft.to.is_empty() { draft.to.clone() } else if !draft.cc.is_empty() {
                    draft.cc.clone()
                } else { "Hidden recipients".into() },
                created: chrono::Utc::now().timestamp(),
                delivery: DeliveryState::Preparing,
                sent: SentState::Pending,
                folder: None,
                error: None,
            };
            tx.execute(
                "INSERT INTO outgoing(draft,attempt,account,stage,created,data,logical_id,config,preparation)
                 VALUES(?1,?2,?3,'Preparing',?4,?5,?6,?7,?8)
                 ON CONFLICT(draft) DO UPDATE SET attempt=excluded.attempt,account=excluded.account,
                 stage=excluded.stage,created=excluded.created,data=excluded.data,logical_id=excluded.logical_id,
                 config=excluded.config,preparation=excluded.preparation,envelope=NULL,raw=NULL",
                params![info.draft_id,info.attempt,account.id,info.created,serde_json::to_string(&info)?,
                    logical_id(&info.message_id),serde_json::to_string(&account)?,serde_json::to_string(&FrozenDraft {
                        attachments: draft.attachments.clone(), draft,
                    })?],
            )?;
            changed(&tx)?;
            tx.commit()?;
            Ok(info)
        }).await
    }

    pub(crate) async fn outgoing_preparation(
        &self,
        attempt: String,
    ) -> anyhow::Result<Option<Preparation>> {
        self.run(move |c| {
            let Some(saved) = pending_preparation(c, &attempt)? else {
                return Ok(None);
            };
            let (config, draft): (String, String) = c.query_row(
                "SELECT config,preparation FROM outgoing WHERE attempt=?",
                [&attempt],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let frozen: FrozenDraft = serde_json::from_str(&draft)?;
            let mut draft = frozen.draft;
            draft.attachments = frozen.attachments;
            Ok(Some(Preparation {
                info: saved,
                account: serde_json::from_str(&config)?,
                draft,
            }))
        })
        .await
    }

    pub(crate) async fn complete_outgoing_preparation(
        &self,
        attempt: String,
        submission: Submission,
    ) -> anyhow::Result<bool> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let Some(mut saved) = pending_preparation(&tx, &attempt)? else {
                return Ok(false);
            };
            connections::allow(&tx, ConnectionKind::Account, &saved.account_id)?;
            anyhow::ensure!(
                saved.message_id == submission.info.message_id
                    && saved.draft_id == submission.info.draft_id
                    && saved.draft_revision == submission.info.draft_revision
                    && saved.account_id == submission.account.id
                    && !submission.raw.is_empty()
                    && submission.raw.len() <= MAX_MESSAGE_BYTES,
                "The prepared message no longer matches its admitted request."
            );
            let config: String = tx.query_row(
                "SELECT config FROM outgoing WHERE attempt=?",
                [&attempt],
                |r| r.get(0),
            )?;
            anyhow::ensure!(
                serde_json::from_str::<Account>(&config)? == submission.account,
                "The prepared sending account changed."
            );
            tx.execute(
                "UPDATE outgoing SET envelope=?,raw=?,preparation=NULL WHERE attempt=?",
                params![
                    serde_json::to_string(&submission.envelope)?,
                    submission.raw,
                    attempt
                ],
            )?;
            saved.delivery = DeliveryState::Queued;
            write(&tx, &saved)?;
            tx.commit()?;
            Ok(true)
        })
        .await
    }

    pub(crate) async fn fail_outgoing_preparation(
        &self,
        attempt: String,
        error: String,
    ) -> anyhow::Result<bool> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let Some(mut saved) = pending_preparation(&tx, &attempt)? else {
                return Ok(false);
            };
            saved.delivery = DeliveryState::Rejected;
            saved.error = Some(error);
            write(&tx, &saved)?;
            tx.execute(
                "UPDATE outgoing SET preparation=NULL WHERE attempt=?",
                [&attempt],
            )?;
            tx.commit()?;
            Ok(true)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account() -> anyhow::Result<Account> {
        Ok(serde_json::from_value(serde_json::json!({
            "id":"work", "name":"Work", "email":"sender@example.test",
            "protocol":"Imap", "host":"imap.example.test", "port":993,
            "username":"sender", "smtp_host":"smtp.example.test", "smtp_port":465
        }))?)
    }
    fn draft() -> Draft {
        Draft {
            id: "preparing".into(),
            account_id: "work".into(),
            to: "reader@example.test".into(),
            subject: "Prepare later".into(),
            body: "Retain this text".into(),
            revision: 1,
            ..Default::default()
        }
    }
    fn build(preparation: Preparation) -> anyhow::Result<Submission> {
        let message = crate::compose::build_with_message_id(
            &preparation.account,
            &preparation.draft,
            vec![],
            &preparation.info.message_id,
        )?;
        Submission::new(preparation.account, &preparation.draft, message)
    }
    async fn admit(store: &Store) -> anyhow::Result<OutgoingInfo> {
        store.save_account(account()?).await?;
        store.save_draft(draft()).await?;
        store.admit_outgoing(draft(), account()?).await
    }

    #[tokio::test]
    async fn preparing_restarts_with_one_identity_and_cannot_dispatch_before_bytes_commit()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("outbox.sqlite");
        let store = Store::open(&path)?;
        let admitted = admit(&store).await?;
        assert!(!store.claim_outgoing(admitted.attempt.clone()).await?);
        assert!(
            store
                .outgoing_submission(admitted.attempt.clone())
                .await
                .is_err()
        );
        drop(store);
        let store = Store::open(path)?;
        let preparation = store
            .outgoing_preparation(admitted.attempt.clone())
            .await?
            .context("saved preparation")?;
        assert_eq!(preparation.info.message_id, admitted.message_id);
        assert_eq!(preparation.draft.body, draft().body);
        let submission = build(preparation)?;
        let bytes = submission.raw.clone();
        assert!(
            store
                .complete_outgoing_preparation(admitted.attempt.clone(), submission)
                .await?
        );
        assert_eq!(
            store
                .outgoing_submission(admitted.attempt.clone())
                .await?
                .raw,
            bytes
        );
        assert!(store.claim_outgoing(admitted.attempt.clone()).await?);
        assert!(!store.claim_outgoing(admitted.attempt).await?);
        Ok(())
    }

    #[tokio::test]
    async fn cancellation_fences_late_preparation_and_restores_editing() -> anyhow::Result<()> {
        let store = Store::memory()?;
        let admitted = admit(&store).await?;
        let prepared = build(
            store
                .outgoing_preparation(admitted.attempt.clone())
                .await?
                .context("preparation")?,
        )?;
        let mut newer = draft();
        newer.revision += 1;
        newer.body = "Newer local text".into();
        assert!(store.save_draft(newer.clone()).await.is_err());
        assert!(
            store
                .remove_draft_file(draft().id, "file".into())
                .await
                .is_err()
        );
        assert!(store.delete_draft(draft().id).await.is_err());
        store
            .cancel_queued_outgoing(admitted.attempt.clone())
            .await?;
        store.save_draft(newer.clone()).await?;
        assert!(
            !store
                .complete_outgoing_preparation(admitted.attempt.clone(), prepared)
                .await?
        );
        store
            .fail_outgoing_preparation(admitted.attempt.clone(), "Late failure".into())
            .await?;
        assert_eq!(
            store.outgoing_info(admitted.attempt).await?.delivery,
            DeliveryState::Released
        );
        assert_eq!(store.draft_state().await?.drafts[0].body, newer.body);
        assert!(store.next_queued_outgoing().await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn preparing_freezes_attachment_identities_and_blocks_late_file_changes()
    -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("exact.bin");
        std::fs::write(&file, [0, 255, 3, 13, 10])?;
        let store = Store::memory()?;
        store.save_account(account()?).await?;
        store.add_draft_files(draft(), vec![file.clone()]).await?;
        let saved = store.draft_state().await?.drafts.remove(0);
        let admitted = store.admit_outgoing(saved.clone(), account()?).await?;
        assert!(
            store
                .add_draft_files(saved.clone(), vec![file])
                .await
                .is_err()
        );
        assert!(
            store
                .remove_draft_file(saved.id.clone(), saved.attachments[0].id.clone())
                .await
                .is_err()
        );
        let preparation = store
            .outgoing_preparation(admitted.attempt)
            .await?
            .context("saved preparation")?;
        assert_eq!(preparation.draft.attachments, saved.attachments);
        let files = store.draft_files(preparation.draft).await?;
        assert_eq!(files[0].bytes, [0, 255, 3, 13, 10]);
        Ok(())
    }

    #[tokio::test]
    async fn removal_retires_preparation_and_ignores_late_results() -> anyhow::Result<()> {
        let store = Store::memory()?;
        let admitted = admit(&store).await?;
        let prepared = build(
            store
                .outgoing_preparation(admitted.attempt.clone())
                .await?
                .context("preparation")?,
        )?;
        let preview = store
            .removal_preview(ConnectionRef {
                kind: ConnectionKind::Account,
                id: "work".into(),
            })
            .await?;
        store.remove_connection(preview, true).await?;
        assert!(
            store
                .outgoing_preparation(admitted.attempt.clone())
                .await?
                .is_none()
        );
        assert!(
            !store
                .complete_outgoing_preparation(admitted.attempt.clone(), prepared)
                .await?
        );
        store
            .fail_outgoing_preparation(admitted.attempt, "Late failure".into())
            .await?;
        assert!(store.draft_state().await?.drafts.is_empty());
        assert!(store.next_queued_outgoing().await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn wrong_wire_identity_cannot_replace_preparation_and_failure_stops_resumption()
    -> anyhow::Result<()> {
        let store = Store::memory()?;
        let admitted = admit(&store).await?;
        let other = Submission::new(
            account()?,
            &draft(),
            crate::compose::build(&account()?, &draft(), vec![])?,
        )?;
        assert!(
            store
                .complete_outgoing_preparation(admitted.attempt.clone(), other)
                .await
                .is_err()
        );
        assert_eq!(
            store
                .outgoing_info(admitted.attempt.clone())
                .await?
                .delivery,
            DeliveryState::Preparing
        );
        store
            .fail_outgoing_preparation(admitted.attempt.clone(), "Invalid recipient".into())
            .await?;
        let failed = store.outgoing_info(admitted.attempt.clone()).await?;
        assert_eq!(failed.delivery, DeliveryState::Rejected);
        assert_eq!(failed.error.as_deref(), Some("Invalid recipient"));
        assert!(store.next_queued_outgoing().await?.is_none());
        store.release_outgoing(admitted.attempt).await?;
        assert_eq!(store.draft_state().await?.drafts[0].body, draft().body);
        Ok(())
    }
}
