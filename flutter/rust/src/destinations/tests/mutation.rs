use super::*;

#[tokio::test]
async fn source_replacement_during_create_retains_folder_ack_without_dispatching_old_mail()
-> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let mut base = MockCreationApi::new();
    base.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let returned = planned.clone();
    base.expect_plan()
        .times(1)
        .returning(move |_, _| Ok(returned.clone()));
    let inspections = std::sync::atomic::AtomicUsize::new(0);
    base.expect_inspect().times(3).returning(move |_| {
        Ok((inspections.fetch_add(1, Ordering::SeqCst) >= 2).then(|| planned.clone()))
    });
    base.expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    let api = Arc::new(HeldApi {
        base,
        boundary: "create",
        held: AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    *profile
        .operations
        .folder_provider
        .lock()
        .expect("fixture folder provider") = Some(api.clone());
    let mut provider = MockDestinationProvider::new();
    provider.expect_move_mail().times(0);
    provider.expect_move_planned_mail().times(0);
    provider.expect_set_flags().times(0);
    *profile
        .operations
        .provider
        .lock()
        .expect("fixture mail provider") = Some(Arc::new(provider));
    let worker = crate::api::MobileProfile {
        database: profile.database.clone(),
        operations: profile.operations.clone(),
    };
    let action = destination.owner.clone();
    let source = mail.clone();
    let running = tokio::spawn(async move {
        request(&worker,json!({"op":"mutate","action_id":action,"id":source,"folder":"Archive","logical_role":"archive","password":"fixture-password"})).await
    });
    api.entered.notified().await;
    let replaced = mail.clone();
    profile
        .database
        .write(move |db| {
            db.execute(
                "UPDATE mail SET raw=?2 WHERE id=?1",
                params![
                    replaced,
                    b"Subject: Replacement\r\n\r\nDifferent message with the same id and UID"
                        .to_vec()
                ],
            )?;
            Ok(())
        })
        .await?;
    api.release.notify_one();
    let result = running.await?;
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["committed"], false);
    assert_eq!(
        result["warning"],
        "This message changed identity while the action was waiting. Refresh and review it before retrying."
    );
    let creation = destination.creation;
    let saved = profile
        .database
        .read(move |db| folders::get(db, &creation)?.context("late folder acknowledgement"))
        .await?;
    assert!(saved.acknowledged && saved.receipt.is_some());
    assert_eq!(
        profile
            .database
            .read(move |db| operations::stored_mail(db, &mail))
            .await?
            .folder,
        "INBOX"
    );
    Ok(())
}

#[tokio::test]
async fn prerequisite_waiting_raw_response_never_claims_committed_mail_move() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
    let mut creation = MockCreationApi::new();
    creation
        .expect_catalogue()
        .times(1)
        .returning(|| Ok(vec![]));
    creation
        .expect_plan()
        .times(1)
        .returning(|_, _| Ok(target("INBOX.Archive", FolderRole::Archive)));
    creation.expect_inspect().times(2).returning(|_| Ok(None));
    creation.expect_create().times(1).returning(|_| {
        Ok(CreateOutcome::Uncertain(
            "lost CREATE acknowledgement".into(),
        ))
    });
    *profile
        .operations
        .folder_provider
        .lock()
        .expect("fixture folder provider") = Some(Arc::new(creation));
    let mut provider = MockDestinationProvider::new();
    provider.expect_move_mail().times(0);
    provider.expect_move_planned_mail().times(0);
    provider.expect_set_flags().times(0);
    *profile
        .operations
        .provider
        .lock()
        .expect("fixture mail provider") = Some(Arc::new(provider));
    let result=request(&profile,json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Archive","logical_role":"archive","password":"fixture-password"})).await;
    assert_eq!(result["status"], "waiting");
    assert_eq!(result["committed"], false);
    assert_eq!(result["folder_creation"], destination.creation);
    let source = profile
        .database
        .read(move |db| operations::stored_mail(db, &mail))
        .await?;
    assert_eq!(source.folder, "INBOX");
    Ok(())
}

#[tokio::test]
async fn already_in_resolved_special_use_folder_retains_choice_without_move_receipt_or_inverse()
-> Result<()> {
    let (_dir, profile) = profile().await;
    seed(&profile, 1).await;
    let planned = target("INBOX.Corbeille", FolderRole::Trash);
    let catalog = vec![planned.clone()];
    profile
        .database
        .write(move |db| {
            db.execute(
                "UPDATE accounts SET settings=json_set(settings,'$.protocol','Imap')",
                [],
            )?;
            db.execute("UPDATE mail SET folder='INBOX.Corbeille'", [])?;
            folders::save_catalogue(db, "fixture", &catalog)
        })
        .await?;
    let mut creation = MockCreationApi::new();
    creation.expect_catalogue().times(0);
    creation.expect_plan().times(0);
    creation.expect_create().times(0);
    creation
        .expect_inspect()
        .with(eq(planned.clone()))
        .times(1)
        .returning(move |_| Ok(Some(planned.clone())));
    *profile
        .operations
        .folder_provider
        .lock()
        .expect("fixture folder provider") = Some(Arc::new(creation));
    let mut provider = MockDestinationProvider::new();
    provider.expect_move_mail().times(0);
    provider.expect_move_planned_mail().times(0);
    provider.expect_set_flags().times(0);
    *profile
        .operations
        .provider
        .lock()
        .expect("fixture mail provider") = Some(Arc::new(provider));
    let exact = json!({"op":"mutate","action_id":"already-trashed","id":"fixture:INBOX:0","folder":"Trash","logical_role":"trash","password":"fixture-password"});
    for _ in 0..2 {
        let result = request(&profile, exact.clone()).await;
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result["unchanged"], true);
        assert_eq!(result["committed"], false);
        assert_eq!(
            result["applied_fields"],
            json!({"folder":"INBOX.Corbeille"})
        );
    }
    profile.database.read(|db| {
        assert_eq!(db.query_row("SELECT COUNT(*) FROM individual_mail_action_receipts",[],|row|row.get::<_,i64>(0))?,0);
        assert_eq!(db.query_row("SELECT COUNT(*) FROM move_receipts",[],|row|row.get::<_,i64>(0))?,0);
        assert!(db.query_row("SELECT a.intent_revision=f.revision FROM individual_mail_actions a JOIN mail_intents f ON f.mail=a.mail AND f.field='folder' WHERE a.id='already-trashed'",[],|row|row.get::<_,bool>(0))?);
        Ok(())
    }).await?;
    let refused:serde_json::Value=serde_json::from_str(&profile.request(json!({"op":"undo_mail_action","id":"already-trashed","password":"fixture-password"}).to_string()).await?)?;
    assert!(
        refused["error"]
            .as_str()
            .context("no inverse")?
            .contains("confirmed")
    );
    Ok(())
}

#[tokio::test]
async fn changed_connection_waiting_reports_no_mail_commit_and_never_contacts_provider()
-> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
    profile.database.write(|db| {db.execute("UPDATE accounts SET settings=json_set(settings,'$.host','replacement.example.test')",[])?;Ok(())}).await?;
    let mut creation = MockCreationApi::new();
    creation.expect_catalogue().times(0);
    creation.expect_plan().times(0);
    creation.expect_inspect().times(0);
    creation.expect_create().times(0);
    *profile
        .operations
        .folder_provider
        .lock()
        .expect("fixture folder provider") = Some(Arc::new(creation));
    let mut provider = MockDestinationProvider::new();
    provider.expect_move_mail().times(0);
    provider.expect_move_planned_mail().times(0);
    provider.expect_set_flags().times(0);
    *profile
        .operations
        .provider
        .lock()
        .expect("fixture mail provider") = Some(Arc::new(provider));
    let result=request(&profile,json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Archive","logical_role":"archive","password":"fixture-password"})).await;
    assert_eq!(result["status"], "waiting");
    assert_eq!(result["committed"], false);
    assert_eq!(
        profile
            .database
            .read(move |db| operations::stored_mail(db, &mail))
            .await?
            .folder,
        "INBOX"
    );
    Ok(())
}

#[tokio::test]
async fn credential_reconnect_keeps_the_admitted_destination_for_the_same_mailbox() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Trash).await?;
    let planned = target("INBOX.Corbeille", FolderRole::Trash);
    let catalogue = vec![planned.clone()];
    profile
        .database
        .write(move |db| {
            folders::save_catalogue(db, "fixture", &catalogue)?;
            db.execute(
                "INSERT INTO credential_slots(slot,account_id,state) VALUES('reconnected-slot','fixture','active')",
                [],
            )?;
            db.execute(
                "INSERT INTO account_credentials(account_id,slot) VALUES('fixture','reconnected-slot')",
                [],
            )?;
            Ok(())
        })
        .await?;
    let mut creation = MockCreationApi::new();
    creation.expect_catalogue().times(0);
    creation.expect_plan().times(0);
    creation.expect_create().times(0);
    let found = planned.clone();
    creation
        .expect_inspect()
        .with(eq(planned.clone()))
        .times(1)
        .returning(move |_| Ok(Some(found.clone())));
    *profile
        .operations
        .folder_provider
        .lock()
        .expect("fixture folder provider") = Some(Arc::new(creation));
    let mut provider = MockDestinationProvider::new();
    provider.expect_move_mail().times(0);
    provider.expect_set_flags().times(0);
    let expected = planned.clone();
    provider
        .expect_move_planned_mail()
        .withf(move |_, _, _, target| *target == expected)
        .times(1)
        .returning(|_, _, _, _| Ok(Some("1.900".into())));
    *profile
        .operations
        .provider
        .lock()
        .expect("fixture mail provider") = Some(Arc::new(provider));
    let result = request(
        &profile,
        json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Trash","logical_role":"trash","password":"fixture-password","credential_slot":"reconnected-slot"}),
    )
    .await;
    assert_eq!(result["status"], "succeeded", "{result}");
    assert_eq!(result["applied_fields"]["folder"], "INBOX.Corbeille");
    Ok(())
}

#[tokio::test]
async fn logical_mutation_and_undo_use_actual_special_use_identity_and_receipt() -> Result<()> {
    for (role, physical) in [
        (Role::Archive, "INBOX.Archiv"),
        (Role::Trash, "INBOX.Corbeille"),
        (Role::Spam, "INBOX.&ZeVnLIqe-"),
    ] {
        let (_dir, profile, destination, mail) = setup(role).await?;
        let planned = target(physical, role.special());
        let catalogue = vec![planned.clone()];
        profile
            .database
            .write(move |db| folders::save_catalogue(db, "fixture", &catalogue))
            .await?;
        let mut creation = MockCreationApi::new();
        creation.expect_catalogue().times(0);
        creation.expect_plan().times(0);
        creation.expect_create().times(0);
        let found = planned.clone();
        creation
            .expect_inspect()
            .with(eq(planned.clone()))
            .times(1)
            .returning(move |_| Ok(Some(found.clone())));
        *profile
            .operations
            .folder_provider
            .lock()
            .expect("fixture folder provider") = Some(Arc::new(creation));
        let mut provider = MockDestinationProvider::new();
        provider.expect_sync().times(0);
        provider.expect_set_flags().times(0);
        let expected = planned.clone();
        provider
            .expect_move_planned_mail()
            .withf(move |_, _, source, target| source.folder == "INBOX" && *target == expected)
            .times(1)
            .returning(|_, _, _, _| Ok(Some("1.900".into())));
        provider
            .expect_move_mail()
            .withf(move |_, _, source, folder| {
                source.folder == physical && source.remote_id == "1.900" && folder == "INBOX"
            })
            .times(1)
            .returning(|_, _, _, _| Ok(Some("1.901".into())));
        *profile
            .operations
            .provider
            .lock()
            .expect("fixture mail provider") = Some(Arc::new(provider));
        let id = destination.owner;
        let result=request(&profile,json!({"op":"mutate","action_id":id,"id":mail,"folder":role.local(),"logical_role":role,"password":"fixture-password"})).await;
        assert_eq!(result["status"], "succeeded");
        assert_eq!(result["applied_fields"]["folder"], physical);
        let action = id.clone();
        profile
            .database
            .read(move |db| {
                let (receipt,source): (String,String) = db.query_row(
                    "SELECT r.result,a.physical FROM individual_mail_action_receipts r JOIN individual_mail_actions a ON a.id=r.action WHERE a.id=?1",
                    [action],
                    |row| Ok((row.get(0)?,row.get(1)?)),
                )?;
                let receipt: serde_json::Value = serde_json::from_str(&receipt)?;
                assert_eq!(receipt["receipt"]["current"]["folder"], physical);
                assert_eq!(receipt["receipt"]["current"]["remote_id"], "1.900");
                let source:serde_json::Value=serde_json::from_str(&source)?;
                assert_eq!(source["folder"], "INBOX");
                Ok(())
            })
            .await?;
        assert_eq!(
            request(
                &profile,
                json!({"op":"undo_mail_action","id":id,"password":"fixture-password"})
            )
            .await["status"],
            "succeeded"
        );
        let cached = request(&profile, json!({"op":"page","folder":"Inbox"})).await;
        assert_eq!(
            cached["mail"].as_array().context("Inbox metadata")?.len(),
            1
        );
    }
    Ok(())
}

#[tokio::test]
async fn pop3_logical_roles_remain_local_and_spam_keeps_its_local_name() -> Result<()> {
    for role in [Role::Archive, Role::Trash, Role::Spam] {
        let (_dir, profile) = profile().await;
        seed(&profile, 1).await;
        let mut creation = MockCreationApi::new();
        creation.expect_catalogue().times(0);
        creation.expect_plan().times(0);
        creation.expect_inspect().times(0);
        creation.expect_create().times(0);
        *profile
            .operations
            .folder_provider
            .lock()
            .expect("fixture folder provider") = Some(Arc::new(creation));
        let result=request(&profile,json!({"op":"mutate","action_id":uuid::Uuid::new_v4().to_string(),"id":"fixture:INBOX:0","folder":role.local(),"logical_role":role})).await;
        assert_eq!(result["status"], "succeeded");
        assert_eq!(result["applied_fields"]["folder"], role.local());
        assert_eq!(
            request(&profile, json!({"op":"folder_creations"})).await,
            json!([])
        );
    }
    Ok(())
}
