use super::*;
use crate::outgoing::{DeliveryState, OUTGOING_PAGE_SIZE, RecoveryAction, SentState, Submission};
use providers::mail::DeliveryFailure;

impl Engine {
    async fn outgoing_changed(&self, output: &mut Output) -> anyhow::Result<()> {
        self.workspace(output).await?;
        output.send(Event::OutgoingChanged).await?;
        Ok(())
    }
    pub(super) async fn send_draft(&self, draft: Draft, output: &mut Output) -> anyhow::Result<()> {
        let _guard = self.account_access(&draft.account_id).await;
        self.store
            .ensure_folder_idle(draft.account_id.clone())
            .await?;
        if let Some(previous) = self.store.outgoing_for_draft(draft.id.clone()).await?
            && !matches!(
                previous.delivery,
                DeliveryState::Rejected | DeliveryState::Released
            )
        {
            output
                .send(Event::ReviewOutgoing(draft.id.clone(), draft.revision))
                .await?;
            anyhow::bail!(
                "This draft already has a delivery record. Review Outbox before sending again."
            );
        }
        self.store.ensure_draft_unsent(draft.clone()).await?;
        self.store.save_draft(draft.clone()).await?;
        output
            .send(Event::DraftSaved(
                draft.id.clone(),
                draft.revision,
                Ok(Arc::new(self.store.draft_state().await?)),
            ))
            .await?;
        let account = self.account(&draft.account_id).await?;
        let files = self.store.draft_files(draft.clone()).await?;
        let build_draft = draft.clone();
        let submission = tokio::task::spawn_blocking(move || {
            let message = crate::compose::build(&account, &build_draft, files)?;
            Submission::new(account, &build_draft, message)
        })
        .await??;
        #[cfg(feature = "test-support")]
        if self.demo && std::env::args().any(|arg| arg == "--mail-actions=slow") {
            // Hold fixture preparation so native tests can switch editors before
            // the existing preview refusal. No SMTP or keychain access occurs.
            tokio::time::sleep(Duration::from_millis(1600)).await;
        }
        anyhow::ensure!(
            !self.demo,
            "Sending is disabled in preview. Your draft is saved locally."
        );
        let info = self.store.begin_outgoing(submission, draft.clone()).await?;
        self.outgoing_changed(output).await?;
        output
            .send(Event::SubmissionQueued(draft.id.clone(), draft.revision))
            .await?;
        let submission = self.store.outgoing_submission(info.attempt.clone()).await?;
        match self.outbound.submit(&submission).await {
            Ok(()) => {
                let recorded = self
                    .store
                    .record_delivery(info.attempt.clone(), DeliveryState::Accepted, None)
                    .await;
                output.send(Event::Sent(draft.id, draft.revision)).await?;
                recorded.context("SMTP accepted this message, but its acknowledgment could not be saved. Review Outbox; do not resend it.")?;
                self.finish_outgoing_local(&info.attempt, output).await?;
                let result = self.copy_outgoing(&info.attempt, false, false).await;
                self.outgoing_changed(output).await?;
                output.send(Event::Changed).await?;
                match result {
                    Ok(()) => output.send(Event::Notice("Message sent.".into())).await?,
                    Err(error) => anyhow::bail!(
                        "Message sent. Its Sent copy needs attention in Outbox: {error:#}"
                    ),
                }
            }
            Err(error) => {
                let state = if matches!(error, DeliveryFailure::Rejected(_)) {
                    DeliveryState::Rejected
                } else {
                    DeliveryState::Uncertain
                };
                self.store
                    .record_delivery(info.attempt, state, Some(error.to_string()))
                    .await?;
                self.outgoing_changed(output).await?;
                if state == DeliveryState::Uncertain {
                    output
                        .send(Event::ReviewOutgoing(draft.id.clone(), draft.revision))
                        .await?;
                }
                return Err(error.into());
            }
        }
        Ok(())
    }
    async fn finish_outgoing_local(
        &self,
        attempt: &str,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        let submission = self.store.outgoing_submission(attempt.into()).await?;
        let info = submission.info.clone();
        let mail = tokio::task::spawn_blocking(move || {
            parse_mail(
                &submission.info.account_id,
                &submission.info.local_remote_id(),
                "Sent",
                submission.raw,
                false,
                false,
            )
        })
        .await??;
        let state = self
            .store
            .outgoing_local_sent(attempt.into(), mail)
            .await
            .context("The message was sent, but its local Sent copy needs recovery in Outbox.")?;
        output
            .send(Event::DraftSaved(
                info.draft_id,
                info.draft_revision,
                Ok(Arc::new(state)),
            ))
            .await?;
        Ok(())
    }
    async fn copy_account(&self, submission: &Submission) -> anyhow::Result<Account> {
        let account = self.account(&submission.info.account_id).await?;
        let old = &submission.account;
        anyhow::ensure!(
            account.protocol == old.protocol
                && account.host == old.host
                && account.port == old.port
                && account.username == old.username
                && account.email == old.email,
            "The mail account changed after this send. Keep the local copy or restore the original incoming connection before checking Sent."
        );
        Ok(account)
    }
    async fn copy_outgoing(
        &self,
        attempt: &str,
        check_only: bool,
        confirmed: bool,
    ) -> anyhow::Result<()> {
        let submission = self.store.outgoing_submission(attempt.into()).await?;
        let info = &submission.info;
        anyhow::ensure!(
            info.delivery == DeliveryState::Accepted,
            "Review delivery before saving a Sent copy."
        );
        let mut account = self.copy_account(&submission).await?;
        if account.protocol == Protocol::Pop3 || account.sent_copy == SentCopyPolicy::LocalOnly {
            self.store
                .record_sent_copy(attempt.into(), SentState::LocalOnly, None, None)
                .await?;
            return Ok(());
        }
        anyhow::ensure!(
            !self.demo,
            "Checking or saving server copies is disabled in preview."
        );
        if matches!(info.sent, SentState::Appending | SentState::Uncertain) {
            if let Some(folder) = &info.folder {
                account.sent_folder = folder.clone();
            }
            anyhow::ensure!(
                check_only || confirmed,
                "The previous Sent upload was not acknowledged. Check the server or confirm another copy before retrying."
            );
        }
        let result=async {
            let mut connection=tokio::time::timeout(Duration::from_secs(60),self.outbound.sent(&account)).await.context("Connecting to Sent timed out")??;
            let folder=connection.folder().to_owned();
            let found=tokio::time::timeout(Duration::from_secs(60),connection.find(&info.message_id)).await.context("Checking Sent timed out")??;
            if found.is_some() {
                self.store.record_sent_copy(attempt.into(),SentState::Saved,Some(folder),None).await?;
                return Ok(());
            }
            anyhow::ensure!(!check_only,"No matching copy was found in Sent. Delivery is still confirmed; saving a copy does not resend it.");
            anyhow::ensure!(account.sent_copy!=SentCopyPolicy::ServerManaged,"Your server is configured to save Sent automatically. Check again after syncing, or keep the local copy.");
            if matches!(info.sent,SentState::Appending|SentState::Uncertain) {
                self.store.record_sent_copy(attempt.into(),SentState::Pending,Some(folder.clone()),None).await?;
            }
            self.store.record_sent_copy(attempt.into(),SentState::Appending,Some(folder.clone()),None).await?;
            match tokio::time::timeout(Duration::from_secs(60),connection.append(&submission.raw,info.created)).await {
                Ok(Ok(_))=> {self.store.record_sent_copy(attempt.into(),SentState::Saved,Some(folder),None).await?;Ok(())}
                outcome=> {
                    self.store.record_sent_copy(attempt.into(),SentState::Uncertain,Some(folder),Some("The server did not acknowledge the Sent copy. Check before uploading another copy.".into())).await?;
                    match outcome {Ok(Err(e))=>Err(e),Err(_)=>anyhow::bail!("Sent-copy upload timed out"),_=>unreachable!()}
                }
            }
        }.await;
        if let Err(error) = &result {
            let latest = self.store.outgoing_info(attempt.into()).await?;
            if latest.delivery == DeliveryState::Accepted {
                self.store
                    .record_sent_copy(
                        attempt.into(),
                        if latest.sent == SentState::Appending {
                            SentState::Uncertain
                        } else {
                            latest.sent
                        },
                        latest.folder,
                        Some(error.to_string().chars().take(1024).collect()),
                    )
                    .await?;
            }
        }
        result
    }
    pub(super) async fn resolve_outgoing(
        &self,
        attempt: String,
        action: RecoveryAction,
        confirmed: bool,
        output: &mut Output,
    ) -> anyhow::Result<()> {
        let initial = self.store.outgoing_info(attempt.clone()).await?;
        let _guard = self.account_access(&initial.account_id).await;
        self.store
            .ensure_folder_idle(initial.account_id.clone())
            .await?;
        let info = self.store.outgoing_info(attempt.clone()).await?;
        match action {
            RecoveryAction::ReturnDraft => {
                anyhow::ensure!(
                    confirmed || info.delivery == DeliveryState::Rejected,
                    "Confirm that you reviewed the uncertain delivery before returning it to drafts."
                );
                self.store.release_outgoing(attempt).await?;
                output
                    .send(Event::Notice(
                        "Returned to drafts. Sending again requires a new Send action.".into(),
                    ))
                    .await?;
            }
            RecoveryAction::MarkSent => {
                anyhow::ensure!(
                    confirmed && info.needs_delivery_review(),
                    "Confirm your delivery review before recording this message as sent."
                );
                self.store
                    .record_delivery(attempt.clone(), DeliveryState::Accepted, None)
                    .await?;
                self.finish_outgoing_local(&attempt, output).await?;
                output
                    .send(Event::Notice(
                        "Recorded as sent. Its server copy can be checked separately.".into(),
                    ))
                    .await?;
            }
            RecoveryAction::KeepLocal => {
                anyhow::ensure!(
                    info.delivery == DeliveryState::Accepted,
                    "Review delivery first."
                );
                self.finish_outgoing_local(&attempt, output).await?;
                self.store
                    .record_sent_copy(attempt, SentState::LocalOnly, None, None)
                    .await?;
                output
                    .send(Event::Notice(
                        "Sent copy kept locally. No message was resent.".into(),
                    ))
                    .await?;
            }
            RecoveryAction::RetryCopy => {
                self.finish_outgoing_local(&attempt, output).await?;
                self.copy_outgoing(&attempt, false, confirmed).await?;
                output
                    .send(Event::Notice(
                        "Sent copy saved. No message was resent.".into(),
                    ))
                    .await?;
            }
            RecoveryAction::CheckSent if info.delivery == DeliveryState::Accepted => {
                self.finish_outgoing_local(&attempt, output).await?;
                self.copy_outgoing(&attempt, true, false).await?;
                output
                    .send(Event::Notice("Matching copy found in Sent.".into()))
                    .await?;
            }
            RecoveryAction::CheckSent => {
                anyhow::ensure!(
                    info.needs_delivery_review(),
                    "This message does not need a delivery check."
                );
                anyhow::ensure!(!self.demo, "Checking server copies is disabled in preview.");
                let submission = self.store.outgoing_submission(attempt.clone()).await?;
                let account = self.copy_account(&submission).await?;
                anyhow::ensure!(
                    account.protocol == Protocol::Imap,
                    "POP3 cannot check a server Sent folder. Review delivery with your provider."
                );
                let found = tokio::time::timeout(Duration::from_secs(60), async {
                    let mut connection = self.outbound.sent(&account).await?;
                    connection.find(&info.message_id).await
                })
                .await
                .context("Checking Sent timed out")??;
                if let Some(receipt) = found {
                    self.store
                        .record_delivery(attempt.clone(), DeliveryState::Accepted, None)
                        .await?;
                    self.finish_outgoing_local(&attempt, output).await?;
                    self.store
                        .record_sent_copy(attempt, SentState::Saved, Some(receipt.folder), None)
                        .await?;
                    output
                        .send(Event::Notice(
                            "Matching message found in Sent; recorded as sent.".into(),
                        ))
                        .await?;
                } else {
                    let error = "No matching copy was found. This does not prove the message was not sent. Review delivery before returning it to drafts.";
                    self.store
                        .record_delivery(attempt, DeliveryState::Uncertain, Some(error.into()))
                        .await?;
                    anyhow::bail!(error);
                }
            }
        }
        Ok(())
    }
    pub(super) async fn repair_outgoing(&self, output: &mut Output) -> anyhow::Result<()> {
        let mut offset = 0;
        loop {
            let page = self.store.outgoing_page(offset).await?;
            for info in page.rows {
                if info.delivery == DeliveryState::Accepted {
                    let _guard = self.account_access(&info.account_id).await;
                    self.store
                        .ensure_folder_idle(info.account_id.clone())
                        .await?;
                    if self
                        .store
                        .outgoing_info(info.attempt.clone())
                        .await?
                        .delivery
                        == DeliveryState::Accepted
                    {
                        self.finish_outgoing_local(&info.attempt, output).await?;
                    }
                }
            }
            offset += OUTGOING_PAGE_SIZE;
            if offset >= page.total {
                break;
            }
        }
        self.outgoing_changed(output).await?;
        output.send(Event::Changed).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::mail::sent::SentReceipt;
    use crate::providers::outgoing::{Outbound, SentConnection};
    use std::sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    #[derive(Default)]
    struct State {
        uncertain: AtomicBool,
        open_fails: AtomicBool,
        append_fails: AtomicBool,
        found: AtomicBool,
        sends: AtomicUsize,
        sent_connections: AtomicUsize,
        appends: AtomicUsize,
        wires: Mutex<Vec<Vec<u8>>>,
    }
    struct Fake(Arc<State>);
    #[async_trait::async_trait]
    impl Outbound for Fake {
        async fn submit(&self, s: &Submission) -> Result<(), DeliveryFailure> {
            self.0.sends.fetch_add(1, Ordering::SeqCst);
            self.0.wires.lock().unwrap().push(s.raw.clone());
            if self.0.uncertain.load(Ordering::SeqCst) {
                Err(DeliveryFailure::Uncertain)
            } else {
                Ok(())
            }
        }
        async fn sent(&self, _: &Account) -> anyhow::Result<Box<dyn SentConnection>> {
            self.0.sent_connections.fetch_add(1, Ordering::SeqCst);
            anyhow::ensure!(
                !self.0.open_fails.load(Ordering::SeqCst),
                "Fixture IMAP unavailable"
            );
            Ok(Box::new(Fake(self.0.clone())))
        }
    }
    #[async_trait::async_trait]
    impl SentConnection for Fake {
        fn folder(&self) -> &str {
            "Sent Mail"
        }
        async fn find(&mut self, _: &str) -> anyhow::Result<Option<SentReceipt>> {
            Ok(self.0.found.load(Ordering::SeqCst).then(|| SentReceipt {
                folder: "Sent Mail".into(),
                remote_id: Some("42.7".into()),
            }))
        }
        async fn append(&mut self, raw: &[u8], _: i64) -> anyhow::Result<SentReceipt> {
            self.0.appends.fetch_add(1, Ordering::SeqCst);
            assert_eq!(raw, self.0.wires.lock().unwrap()[0]);
            anyhow::ensure!(
                !self.0.append_fails.load(Ordering::SeqCst),
                "Fixture APPEND acknowledgment lost"
            );
            Ok(SentReceipt {
                folder: "Sent Mail".into(),
                remote_id: None,
            })
        }
    }
    fn account() -> Account {
        serde_json::from_value(serde_json::json!({"id":"work","name":"Work","email":"sender@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"sender","smtp_host":"smtp.example.test","smtp_port":465})).unwrap()
    }
    fn draft() -> Draft {
        Draft {
            id: "draft".into(),
            account_id: "work".into(),
            to: "friend@example.test".into(),
            subject: "Recovery".into(),
            body: "Keep these bytes".into(),
            revision: 1,
            ..Default::default()
        }
    }
    async fn engine(state: Arc<State>) -> Engine {
        let mut e = super::super::calendar_tests::engine();
        e.demo = false;
        e.outbound = Arc::new(Fake(state));
        e.store.save_account(account()).await.unwrap();
        e
    }
    #[tokio::test]
    async fn uncertain_submission_requires_explicit_review_and_never_resubmits_automatically() {
        let state = Arc::new(State::default());
        state.uncertain.store(true, Ordering::SeqCst);
        let engine = engine(state.clone()).await;
        let (mut output, _rx) = futures::channel::mpsc::channel(32);
        assert!(engine.send_draft(draft(), &mut output).await.is_err());
        let info = engine
            .store
            .outgoing_for_draft("draft".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.delivery, DeliveryState::Uncertain);
        engine.repair_outgoing(&mut output).await.unwrap();
        assert!(engine.send_draft(draft(), &mut output).await.is_err());
        assert!(
            engine
                .resolve_outgoing(
                    info.attempt.clone(),
                    RecoveryAction::ReturnDraft,
                    false,
                    &mut output
                )
                .await
                .is_err()
        );
        assert_eq!(state.sends.load(Ordering::SeqCst), 1);
        engine
            .resolve_outgoing(info.attempt, RecoveryAction::ReturnDraft, true, &mut output)
            .await
            .unwrap();
        assert_eq!(state.sends.load(Ordering::SeqCst), 1);
        assert_eq!(engine.store.draft_state().await.unwrap().drafts.len(), 1);
    }
    #[tokio::test]
    async fn sent_copy_failure_recovers_without_another_smtp_delivery() {
        let state = Arc::new(State::default());
        state.open_fails.store(true, Ordering::SeqCst);
        let engine = engine(state.clone()).await;
        let (mut output, _rx) = futures::channel::mpsc::channel(32);
        assert!(
            engine
                .send_draft(draft(), &mut output)
                .await
                .unwrap_err()
                .to_string()
                .contains("Message sent")
        );
        assert!(engine.store.draft_state().await.unwrap().drafts.is_empty());
        assert_eq!(engine.store.export().await.unwrap().len(), 1);
        let info = engine
            .store
            .outgoing_for_draft("draft".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.delivery, DeliveryState::Accepted);
        state.open_fails.store(false, Ordering::SeqCst);
        state.append_fails.store(true, Ordering::SeqCst);
        assert!(
            engine
                .resolve_outgoing(
                    info.attempt.clone(),
                    RecoveryAction::RetryCopy,
                    false,
                    &mut output
                )
                .await
                .is_err()
        );
        assert_eq!(
            engine
                .store
                .outgoing_info(info.attempt.clone())
                .await
                .unwrap()
                .sent,
            SentState::Uncertain
        );
        assert!(
            engine
                .resolve_outgoing(
                    info.attempt.clone(),
                    RecoveryAction::RetryCopy,
                    false,
                    &mut output
                )
                .await
                .is_err()
        );
        assert_eq!(state.appends.load(Ordering::SeqCst), 1);
        state.found.store(true, Ordering::SeqCst);
        engine
            .resolve_outgoing(
                info.attempt.clone(),
                RecoveryAction::CheckSent,
                false,
                &mut output,
            )
            .await
            .unwrap();
        assert_eq!(state.sends.load(Ordering::SeqCst), 1);
        assert_eq!(state.appends.load(Ordering::SeqCst), 1);
        assert_eq!(
            engine
                .store
                .outgoing_info(info.attempt)
                .await
                .unwrap()
                .delivery,
            DeliveryState::Complete
        );
    }
    #[tokio::test]
    async fn accepted_smtp_with_failed_ack_persistence_remains_reviewable_after_restart() {
        let state = Arc::new(State::default());
        let mut engine = engine(state.clone()).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outgoing.sqlite");
        engine.store = Store::open(&path).unwrap();
        engine.store.save_account(account()).await.unwrap();
        engine.store.run(|c|{c.execute_batch("CREATE TRIGGER fail_ack BEFORE UPDATE ON outgoing WHEN NEW.stage='Accepted' BEGIN SELECT RAISE(ABORT,'fixture disk failure'); END;")?;Ok(())}).await.unwrap();
        let (mut output, _rx) = futures::channel::mpsc::channel(32);
        assert!(
            engine
                .send_draft(draft(), &mut output)
                .await
                .unwrap_err()
                .to_string()
                .contains("SMTP accepted")
        );
        engine.store = Store::open(path).unwrap();
        engine.repair_outgoing(&mut output).await.unwrap();
        assert!(engine.send_draft(draft(), &mut output).await.is_err());
        assert_eq!(state.sends.load(Ordering::SeqCst), 1);
        let info = engine
            .store
            .outgoing_for_draft("draft".into())
            .await
            .unwrap()
            .unwrap();
        assert!(info.needs_delivery_review());
        engine
            .store
            .run(|c| {
                c.execute_batch("DROP TRIGGER fail_ack")?;
                Ok(())
            })
            .await
            .unwrap();
        engine
            .resolve_outgoing(info.attempt, RecoveryAction::MarkSent, true, &mut output)
            .await
            .unwrap();
        assert_eq!(state.sends.load(Ordering::SeqCst), 1);
        assert!(engine.store.draft_state().await.unwrap().drafts.is_empty());
    }

    #[tokio::test]
    async fn local_and_pop3_policies_finish_without_opening_a_sent_connection() {
        for (protocol, policy) in [
            (Protocol::Imap, SentCopyPolicy::LocalOnly),
            (Protocol::Pop3, SentCopyPolicy::Automatic),
        ] {
            let state = Arc::new(State::default());
            state.open_fails.store(true, Ordering::SeqCst);
            let engine = engine(state.clone()).await;
            let mut account = account();
            account.protocol = protocol;
            account.sent_copy = policy;
            engine.store.save_account(account).await.unwrap();
            let (mut output, _rx) = futures::channel::mpsc::channel(32);
            engine.send_draft(draft(), &mut output).await.unwrap();
            let info = engine
                .store
                .outgoing_for_draft("draft".into())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(info.delivery, DeliveryState::Complete);
            assert_eq!(info.sent, SentState::LocalOnly);
            assert_eq!(state.sends.load(Ordering::SeqCst), 1);
            assert_eq!(state.sent_connections.load(Ordering::SeqCst), 0);
            assert_eq!(engine.store.export().await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn server_managed_policy_checks_existing_copies_and_never_appends_or_resends() {
        let state = Arc::new(State::default());
        let engine = engine(state.clone()).await;
        let mut account = account();
        account.sent_copy = SentCopyPolicy::ServerManaged;
        engine.store.save_account(account).await.unwrap();
        let (mut output, _rx) = futures::channel::mpsc::channel(32);
        assert!(
            engine
                .send_draft(draft(), &mut output)
                .await
                .unwrap_err()
                .to_string()
                .contains("Message sent")
        );
        let info = engine
            .store
            .outgoing_for_draft("draft".into())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(info.delivery, DeliveryState::Accepted);
        assert_eq!(state.appends.load(Ordering::SeqCst), 0);
        state.found.store(true, Ordering::SeqCst);
        engine
            .resolve_outgoing(
                info.attempt.clone(),
                RecoveryAction::CheckSent,
                false,
                &mut output,
            )
            .await
            .unwrap();
        assert_eq!(
            engine
                .store
                .outgoing_info(info.attempt)
                .await
                .unwrap()
                .delivery,
            DeliveryState::Complete
        );
        assert_eq!(state.sends.load(Ordering::SeqCst), 1);
        assert_eq!(state.appends.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn automatic_policy_uses_a_copy_the_server_already_saved() {
        let state = Arc::new(State::default());
        state.found.store(true, Ordering::SeqCst);
        let engine = engine(state.clone()).await;
        let (mut output, _rx) = futures::channel::mpsc::channel(32);
        engine.send_draft(draft(), &mut output).await.unwrap();
        assert_eq!(engine.store.outgoing_page(0).await.unwrap().total, 0);
        assert_eq!(state.sends.load(Ordering::SeqCst), 1);
        assert_eq!(state.appends.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn local_sent_flags_and_moves_do_not_use_the_imap_or_credential_service() {
        let state = Arc::new(State::default());
        let engine = engine(state.clone()).await;
        let mut account = account();
        account.sent_copy = SentCopyPolicy::LocalOnly;
        engine.store.save_account(account).await.unwrap();
        let (mut output, _rx) = futures::channel::mpsc::channel(32);
        engine.send_draft(draft(), &mut output).await.unwrap();
        let info = engine
            .store
            .outgoing_for_draft("draft".into())
            .await
            .unwrap()
            .unwrap();
        let mut mail = engine.store.detail(info.local_id()).await.unwrap().summary;
        mail.starred = true;
        engine
            .execute(
                Command::Flags(
                    1,
                    mail.clone(),
                    crate::mail_actions::Flags {
                        starred: Some(true),
                        unread: None,
                    },
                ),
                output.clone(),
            )
            .await
            .unwrap();
        engine
            .execute(Command::Move(2, mail, "Archive".into()), output)
            .await
            .unwrap();
        let mail = engine.store.detail(info.local_id()).await.unwrap().summary;
        assert!(mail.starred);
        assert_eq!(mail.folder, "Archive");
        assert_eq!(state.sent_connections.load(Ordering::SeqCst), 0);
    }
}
