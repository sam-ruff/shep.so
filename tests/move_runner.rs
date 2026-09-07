use shep::{
    mail_actions::{
        journal::*,
        runner::{self, Connection, SubmissionError},
        *,
    },
    model::*,
    store::Store,
};

struct Server {
    store: Store,
    identities: Vec<(String, String)>,
    raw: Vec<u8>,
    reply: Option<Result<Option<String>, SubmissionError>>,
    preflight_error: bool,
    cleanup_error: bool,
    lookup_error: bool,
    wrong_content: bool,
    wrong_identity: bool,
    prepared: usize,
    submitted: usize,
    removed: usize,
    looked_up: usize,
}
#[async_trait::async_trait]
impl Connection for Server {
    fn identities(&self) -> Vec<(String, String)> {
        self.identities.clone()
    }
    async fn prepare(&mut self, _: &MoveRecord) -> anyhow::Result<()> {
        self.prepared += 1;
        anyhow::ensure!(!self.preflight_error, "Source UIDVALIDITY changed");
        Ok(())
    }
    async fn submit(
        &mut self,
        record: &MoveRecord,
        raw: Option<Vec<u8>>,
    ) -> Result<Option<String>, SubmissionError> {
        self.submitted += 1;
        // Drive the real store, so this proves durable preparation precedes the
        // provider write, not just that a fake saw a callback in some order.
        let persisted = self.store.mail_move(record.token.clone()).await.unwrap();
        assert_eq!(persisted.stage, MoveStage::Started);
        assert_eq!(persisted.receipt.connections, self.identities);
        assert!(self.store.remove(record.original.id.clone()).await.is_err());
        if record.original.account_id != record.receipt.account {
            assert_eq!(raw.unwrap(), self.raw);
        } else {
            assert!(raw.is_none());
        }
        self.reply
            .take()
            .expect("Do not submit the same operation twice")
    }
    async fn finish_source(&mut self, record: &MoveRecord) -> anyhow::Result<()> {
        self.removed += 1;
        let persisted = self.store.mail_move(record.token.clone()).await.unwrap();
        assert_eq!(persisted.stage, MoveStage::Copied);
        assert_eq!(
            serde_json::to_string(&persisted.receipt).unwrap(),
            serde_json::to_string(&record.receipt).unwrap()
        );
        anyhow::ensure!(!self.cleanup_error, "Connection lost after source EXPUNGE");
        Ok(())
    }
    async fn locate(&mut self, receipt: &MoveReceipt) -> anyhow::Result<StoredMail> {
        self.looked_up += 1;
        anyhow::ensure!(!self.lookup_error, "Destination is temporarily unavailable");
        let raw = if self.wrong_content {
            b"Subject: Wrong message\r\n\r\nDifferent".to_vec()
        } else {
            self.raw.clone()
        };
        let mut message = parse_mail(
            &receipt.account,
            receipt
                .current
                .as_ref()
                .map_or("91.38", |m| m.remote_id.as_str()),
            &receipt.folder,
            raw,
            false,
            true,
        )?;
        if self.wrong_identity {
            message.summary.remote_id.clear();
        }
        Ok(message)
    }
}
async fn setup(store: &Store, transfer: bool) -> (MoveRecord, Server) {
    let original = parse_mail(
        "work",
        "42.7",
        "INBOX",
        b"Message-ID: <keepsake@example.test>\r\nSubject: Keepsake\r\n\r\nExact original".to_vec(),
        true,
        false,
    )
    .unwrap();
    store.upsert(vec![original.clone()]).await.unwrap();
    let destination = if transfer { "personal" } else { "work" };
    let mut receipt = MoveReceipt::server(
        &original.summary,
        destination,
        "Keep",
        None,
        Fingerprint::of(&original.raw),
    );
    receipt.connections = vec![("work".into(), "fixture-source".into())];
    if transfer {
        receipt
            .connections
            .push(("personal".into(), "fixture-destination".into()));
    }
    let identities = receipt.connections.clone();
    (
        MoveRecord::new(original.summary, receipt),
        Server {
            store: store.clone(),
            identities,
            raw: original.raw,
            reply: Some(Ok(Some("91.38".into()))),
            preflight_error: false,
            cleanup_error: false,
            lookup_error: false,
            wrong_content: false,
            wrong_identity: false,
            prepared: 0,
            submitted: 0,
            removed: 0,
            looked_up: 0,
        },
    )
}

#[tokio::test]
async fn acknowledged_missing_uid_returns_without_lookup_and_recovers_after_reopen_without_repeating_move()
 {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let (record, mut server) = setup(&store, false).await;
    server.reply = Some(Ok(None));
    let committed = runner::start(&store, &mut server, record.clone())
        .await
        .unwrap();
    assert_eq!(committed.stage, MoveStage::Committed);
    assert_eq!(
        (server.submitted, server.looked_up),
        (1, 0),
        "Acknowledgment is not delayed by recovery"
    );
    server.store = Store::memory().unwrap();
    drop(store);
    server.store = Store::open(&path).unwrap();
    let store = server.store.clone();
    let saved = store.mail_move(record.token).await.unwrap();
    let found = runner::recover(&store, &mut server, saved).await.unwrap();
    assert_eq!(found.stage, MoveStage::Located);
    assert_eq!(
        (server.submitted, server.removed, server.looked_up),
        (1, 0, 1)
    );
    assert_eq!(
        store
            .detail(found.receipt.current.unwrap().id)
            .await
            .unwrap()
            .body,
        "Exact original"
    );
}

#[tokio::test]
async fn copied_uid_survives_cleanup_failure_then_retry_verifies_it_without_reupload() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("mail.sqlite");
    let store = Store::open(&path).unwrap();
    let (record, mut server) = setup(&store, true).await;
    server.cleanup_error = true;
    let error = runner::start(&store, &mut server, record.clone())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("confirmed copy"));
    let saved = store.mail_move(record.token.clone()).await.unwrap();
    assert_eq!(saved.stage, MoveStage::Copied);
    assert_eq!(saved.receipt.current.as_ref().unwrap().remote_id, "91.38");
    server.store = Store::memory().unwrap();
    drop(store);
    server.store = Store::open(&path).unwrap();
    let store = server.store.clone();
    server.cleanup_error = false;
    let completed = runner::recover(&store, &mut server, saved).await.unwrap();
    assert_eq!(completed.stage, MoveStage::Located);
    assert_eq!(
        (server.submitted, server.removed, server.looked_up),
        (1, 2, 1)
    );
    assert_eq!(
        store
            .raw_message(completed.receipt.current.unwrap().id)
            .await
            .unwrap(),
        server.raw
    );
    assert!(
        store
            .mail_metadata(record.original.id.clone())
            .await
            .is_err()
    );
    assert_eq!(
        store
            .detail(record.original.id)
            .await
            .unwrap()
            .summary
            .remote_id,
        "91.38"
    );
}

#[tokio::test]
async fn known_uid_completes_cache_without_extra_provider_reads() {
    for transfer in [false, true] {
        let store = Store::memory().unwrap();
        let (record, mut server) = setup(&store, transfer).await;
        let saved = runner::start(&store, &mut server, record.clone())
            .await
            .unwrap();
        assert_eq!(saved.stage, MoveStage::Located);
        assert_eq!(
            (server.submitted, server.removed, server.looked_up),
            (1, usize::from(transfer), 0)
        );
        assert_eq!(saved.receipt.recovery.as_ref(), Some(&record.token));
        assert!(
            store
                .pending_mail_moves(None, None)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn unconfirmed_submission_is_never_replayed_or_reclassified_as_rejection() {
    for transfer in [false, true] {
        let store = Store::memory().unwrap();
        let (record, mut server) = setup(&store, transfer).await;
        server.reply = Some(Err(SubmissionError::Unconfirmed(
            "Tagged MOVE NO or lost acknowledgment".into(),
        )));
        assert!(
            runner::start(&store, &mut server, record.clone())
                .await
                .is_err()
        );
        let pending = store.mail_move(record.token).await.unwrap();
        assert_eq!(pending.stage, MoveStage::Started);
        assert!(pending.error.as_ref().unwrap().contains("unconfirmed"));
        assert!(runner::recover(&store, &mut server, pending).await.is_err());
        assert_eq!(
            (server.submitted, server.removed, server.looked_up),
            (1, 0, 0)
        );
        assert_eq!(
            store.raw_message(record.original.id).await.unwrap(),
            server.raw
        );
    }
}

#[tokio::test]
async fn failed_preflight_and_proven_atomic_upload_rejection_allow_retry_without_retained_operation()
 {
    let store = Store::memory().unwrap();
    let (record, mut server) = setup(&store, true).await;
    server.preflight_error = true;
    assert!(
        runner::start(&store, &mut server, record.clone())
            .await
            .is_err()
    );
    assert_eq!(server.submitted, 0);
    assert!(
        store
            .pending_mail_moves(None, None)
            .await
            .unwrap()
            .is_empty()
    );
    server.preflight_error = false;
    server.reply = Some(Err(SubmissionError::NotApplied("APPEND NO quota".into())));
    assert!(
        runner::start(&store, &mut server, record.clone())
            .await
            .is_err()
    );
    assert_eq!(server.removed, 0);
    assert!(
        store
            .pending_mail_moves(None, None)
            .await
            .unwrap()
            .is_empty()
    );
    server.reply = Some(Ok(Some("91.38".into())));
    assert_eq!(
        runner::start(&store, &mut server, record)
            .await
            .unwrap()
            .stage,
        MoveStage::Located
    );
}

#[tokio::test]
async fn changed_connection_bad_lookup_and_stale_recovery_never_remove_source() {
    let store = Store::memory().unwrap();
    let (record, mut server) = setup(&store, true).await;
    server.cleanup_error = true;
    assert!(
        runner::start(&store, &mut server, record.clone())
            .await
            .is_err()
    );
    let copied = store.mail_move(record.token.clone()).await.unwrap();
    server.identities[0].1 = "changed-host".into();
    assert!(
        runner::recover(&store, &mut server, copied.clone())
            .await
            .is_err()
    );
    assert_eq!(server.looked_up, 0);
    server.identities = copied.receipt.connections.clone();
    server.lookup_error = true;
    assert!(
        runner::recover(&store, &mut server, copied.clone())
            .await
            .is_err()
    );
    server.lookup_error = false;
    assert!(
        runner::recover(&store, &mut server, copied).await.is_err(),
        "A stale error/retry snapshot cannot proceed"
    );
    let copied = store.mail_move(record.token.clone()).await.unwrap();
    server.wrong_content = true;
    assert!(runner::recover(&store, &mut server, copied).await.is_err());
    assert_eq!(server.removed, 1, "Only the original cleanup attempt ran");
    assert_eq!(
        store.raw_message(record.original.id).await.unwrap(),
        server.raw
    );
    server.wrong_content = false;
    server.cleanup_error = false;
    let copied = store.mail_move(record.token).await.unwrap();
    assert_eq!(
        runner::recover(&store, &mut server, copied)
            .await
            .unwrap()
            .stage,
        MoveStage::Located
    );
}

#[tokio::test]
async fn failed_cache_commit_still_returns_server_acknowledgment_and_recovers_without_second_write()
{
    let store = Store::memory().unwrap();
    let (record, mut server) = setup(&store, false).await;
    store
        .upsert(vec![
            parse_mail(
                "work",
                "91.38",
                "Keep",
                b"Subject: Conflict\r\n\r\nKeep this too".to_vec(),
                false,
                false,
            )
            .unwrap(),
        ])
        .await
        .unwrap();
    let committed = runner::start(&store, &mut server, record.clone())
        .await
        .unwrap();
    assert_eq!(committed.stage, MoveStage::Committed);
    assert!(
        committed
            .error
            .as_ref()
            .unwrap()
            .contains("message was moved")
    );
    assert_eq!(
        store.raw_message(record.original.id).await.unwrap(),
        server.raw
    );
    // A separately confirmed cache conflict resolution permits cache recovery;
    // the confirmed provider move itself is never retried.
    store.remove("work:Keep:91.38".into()).await.unwrap();
    assert_eq!(
        runner::recover(&store, &mut server, committed)
            .await
            .unwrap()
            .stage,
        MoveStage::Located
    );
    assert_eq!(server.submitted, 1);
}

#[tokio::test]
async fn malformed_destination_identity_cannot_authorize_source_cleanup() {
    let store = Store::memory().unwrap();
    let (record, mut server) = setup(&store, true).await;
    server.cleanup_error = true;
    assert!(
        runner::start(&store, &mut server, record.clone())
            .await
            .is_err()
    );
    let copied = store.mail_move(record.token.clone()).await.unwrap();
    server.cleanup_error = false;
    server.wrong_identity = true;
    assert!(runner::recover(&store, &mut server, copied).await.is_err());
    assert_eq!(server.removed, 1);
    assert_eq!(
        store.mail_move(record.token).await.unwrap().stage,
        MoveStage::Copied
    );
    assert_eq!(
        store.raw_message(record.original.id).await.unwrap(),
        server.raw
    );
}

#[tokio::test]
async fn reviewed_unconfirmed_move_verifies_and_checkpoints_existing_copy_before_cleanup_without_resubmission()
 {
    for transfer in [false, true] {
        let store = Store::memory().unwrap();
        let (record, mut server) = setup(&store, transfer).await;
        store.prepare_mail_move(record.clone()).await.unwrap();
        assert!(
            runner::recover(&store, &mut server, record.clone())
                .await
                .is_err()
        );
        assert_eq!(
            (server.submitted, server.looked_up, server.removed),
            (0, 0, 0)
        );
        server.cleanup_error = true;
        assert!(
            runner::recover_reviewed(&store, &mut server, record.clone(), true)
                .await
                .is_err()
        );
        let copied = store.mail_move(record.token).await.unwrap();
        assert_eq!(copied.stage, MoveStage::Copied);
        assert_eq!(copied.receipt.current.as_ref().unwrap().remote_id, "91.38");
        assert!(store.remove(copied.original.id.clone()).await.is_err());
        assert_eq!(
            (server.submitted, server.looked_up, server.removed),
            (0, 1, 1)
        );
        server.cleanup_error = false;
        let recovered = runner::recover(&store, &mut server, copied).await.unwrap();
        assert_eq!(recovered.stage, MoveStage::Located);
        assert_eq!(
            (server.submitted, server.looked_up, server.removed),
            (0, 2, 2)
        );
        assert_eq!(
            store
                .raw_message(recovered.receipt.current.unwrap().id)
                .await
                .unwrap(),
            server.raw
        );
    }
}

#[tokio::test]
async fn reviewed_unconfirmed_move_never_deletes_source_without_exact_unique_destination_identity()
{
    for failure in ["missing", "content", "identity"] {
        let store = Store::memory().unwrap();
        let (record, mut server) = setup(&store, true).await;
        store.prepare_mail_move(record.clone()).await.unwrap();
        server.lookup_error = failure == "missing";
        server.wrong_content = failure == "content";
        server.wrong_identity = failure == "identity";
        assert!(
            runner::recover_reviewed(&store, &mut server, record.clone(), true)
                .await
                .is_err()
        );
        let saved = store.mail_move(record.token.clone()).await.unwrap();
        assert_eq!(saved.stage, MoveStage::Started);
        assert!(saved.error.is_some());
        assert_eq!(
            (server.submitted, server.looked_up, server.removed),
            (0, 1, 0)
        );
        assert_eq!(
            store.raw_message(record.original.id.clone()).await.unwrap(),
            server.raw
        );
        assert!(
            runner::recover_reviewed(&store, &mut server, record, true)
                .await
                .is_err(),
            "A stale review cannot submit or delete"
        );
        assert_eq!(
            (server.submitted, server.looked_up, server.removed),
            (0, 1, 0)
        );
    }
}
