use super::*;

fn held(base: MockCreationApi, boundary: &'static str) -> Arc<HeldApi> {
    Arc::new(HeldApi {
        base,
        boundary,
        held: AtomicBool::new(false),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    })
}

async fn folder_owned(profile: &crate::api::MobileProfile, action: &str) -> Result<bool> {
    let action = action.to_owned();
    profile
        .database
        .read(move |db| {
            Ok(db.query_row(
                "SELECT EXISTS(SELECT 1 FROM individual_mail_actions a JOIN mail_intents i ON i.mail=a.mail AND i.field='folder' AND i.revision=a.intent_revision WHERE a.id=?1)",
                [action],
                |row| row.get(0),
            )?)
        })
        .await
}

async fn stored_folder(profile: &crate::api::MobileProfile, mail: &str) -> Result<String> {
    let mail = mail.to_owned();
    Ok(profile
        .database
        .read(move |db| operations::stored_mail(db, &mail))
        .await?
        .folder)
}

#[tokio::test]
async fn duplicate_execution_after_success_keeps_the_succeeded_intent() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Trash).await?;
    let planned = target("INBOX.Corbeille", FolderRole::Trash);
    let catalogue = vec![planned.clone()];
    profile
        .database
        .write(move |db| folders::save_catalogue(db, "fixture", &catalogue))
        .await?;
    let mut base = MockCreationApi::new();
    let found = planned.clone();
    base.expect_inspect()
        .returning(move |_| Ok(Some(found.clone())));
    let api = held(base, "inspect");
    let mut provider = MockDestinationProvider::new();
    provider
        .expect_move_planned_mail()
        .times(1)
        .returning(|_, _, _, _| Ok(Some("1.900".into())));
    install(&profile, api.clone(), provider);
    let body = json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Trash","logical_role":"trash","password":"fixture-password"});
    let first_worker = worker(&profile);
    let first_body = body.clone();
    let first = tokio::spawn(async move { request(&first_worker, first_body).await });
    api.entered.notified().await;
    profile.operations.mutation_waiting.notified().await;
    let second_worker = worker(&profile);
    let second = tokio::spawn(async move { request(&second_worker, body).await });
    profile.operations.mutation_waiting.notified().await;
    api.release.notify_one();
    assert_eq!(first.await?["status"], "succeeded");
    let second = second.await?;
    assert_eq!(second["status"], "succeeded", "{second}");
    assert_eq!(second["applied_fields"]["folder"], "INBOX.Corbeille");
    assert!(folder_owned(&profile, &destination.owner).await?);
    Ok(())
}

#[tokio::test]
async fn definite_create_refusal_rejects_only_that_action_and_retry_can_create() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let mut refused = MockCreationApi::new();
    refused.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let returned = planned.clone();
    refused
        .expect_plan()
        .times(1)
        .returning(move |_, _| Ok(returned.clone()));
    refused.expect_inspect().times(2).returning(|_| Ok(None));
    refused
        .expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Rejected("private provider detail".into())));
    let mut provider = MockDestinationProvider::new();
    provider.expect_move_planned_mail().times(0);
    provider.expect_move_mail().times(0);
    install(&profile, Arc::new(refused), provider);
    let body = json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Archive","logical_role":"archive","password":"fixture-password"});
    let result = request(&profile, body.clone()).await;
    assert_eq!(result["status"], "rejected", "{result}");
    assert_eq!(result["committed"], false);
    assert!(
        !result["warning"]
            .as_str()
            .context("visible refusal")?
            .contains("private provider detail")
    );
    assert!(!folder_owned(&profile, &destination.owner).await?);
    assert_eq!(stored_folder(&profile, &mail).await?, "INBOX");
    assert_eq!(request(&profile, body).await["status"], "rejected");

    let mut retry = MockCreationApi::new();
    retry.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let returned = planned.clone();
    retry
        .expect_plan()
        .times(1)
        .returning(move |_, _| Ok(returned.clone()));
    let mut sequence = mockall::Sequence::new();
    retry
        .expect_inspect()
        .times(2)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(None));
    retry
        .expect_create()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    let observed = planned.clone();
    retry
        .expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(move |_| Ok(Some(observed.clone())));
    let mut provider = MockDestinationProvider::new();
    provider
        .expect_move_planned_mail()
        .times(1)
        .returning(|_, _, _, _| Ok(Some("1.900".into())));
    install(&profile, Arc::new(retry), provider);
    let fresh = uuid::Uuid::new_v4().to_string();
    let result = request(
        &profile,
        json!({"op":"mutate","action_id":fresh,"id":mail,"folder":"Archive","logical_role":"archive","password":"fixture-password"}),
    )
    .await;
    assert_eq!(result["status"], "succeeded", "{result}");
    assert_eq!(result["applied_fields"]["folder"], "INBOX.Archive");
    Ok(())
}

#[tokio::test]
async fn spam_view_keeps_a_plain_junk_folder_beside_a_special_use_spam_folder() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Spam).await?;
    let special = target("Spam", FolderRole::Junk);
    let mut plain = target("Junk", FolderRole::Junk);
    plain.role = None;
    let catalogue = vec![special, plain.clone()];
    let saved = catalogue.clone();
    profile
        .database
        .write(move |db| folders::save_catalogue(db, "fixture", &saved))
        .await?;
    let mut creation = MockCreationApi::new();
    let found = plain.clone();
    creation
        .expect_inspect()
        .times(1)
        .returning(move |_| Ok(Some(found.clone())));
    let mut provider = MockDestinationProvider::new();
    provider
        .expect_move_planned_mail()
        .withf(|_, _, _, target| target.name == "Junk")
        .times(1)
        .returning(|_, _, _, _| Ok(Some("1.900".into())));
    install(&profile, Arc::new(creation), provider);
    let result = request(
        &profile,
        json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Spam","logical_role":"spam","password":"fixture-password"}),
    )
    .await;
    assert_eq!(result["applied_fields"]["folder"], "Junk", "{result}");
    profile
        .database
        .write(move |db| folders::save_catalogue(db, "fixture", &catalogue))
        .await?;
    let page = request(&profile, json!({"op":"page","folder":"Spam"})).await;
    assert_eq!(page["total"], 1, "{page}");
    assert_eq!(page["mail"][0]["folder"], "Junk");
    Ok(())
}

#[tokio::test]
async fn uncertain_logical_move_projects_its_physical_destination() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Trash).await?;
    let planned = target("INBOX.Corbeille", FolderRole::Trash);
    let catalogue = vec![planned.clone()];
    profile
        .database
        .write(move |db| folders::save_catalogue(db, "fixture", &catalogue))
        .await?;
    let mut creation = MockCreationApi::new();
    let found = planned.clone();
    creation
        .expect_inspect()
        .times(1)
        .returning(move |_| Ok(Some(found.clone())));
    let mut provider = MockDestinationProvider::new();
    provider
        .expect_move_planned_mail()
        .times(1)
        .returning(|_, _, _, _| anyhow::bail!("connection reset after MOVE"));
    install(&profile, Arc::new(creation), provider);
    let result = request(
        &profile,
        json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Trash","logical_role":"trash","password":"fixture-password"}),
    )
    .await;
    assert_eq!(result["status"], "uncertain", "{result}");
    assert_eq!(result["applied_fields"]["folder"], "INBOX.Corbeille");
    let page = request(&profile, json!({"op":"page","folder":"INBOX.Corbeille"})).await;
    assert_eq!(page["total"], 1, "{page}");
    let detail = request(&profile, json!({"op":"detail","id":mail})).await;
    assert_eq!(detail["summary"]["folder"], "INBOX.Corbeille");
    Ok(())
}

#[tokio::test]
async fn changed_folder_encoding_before_select_rejects_without_a_pending_move() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Trash).await?;
    let planned = target("INBOX.Corbeille", FolderRole::Trash);
    let catalogue = vec![planned.clone()];
    profile
        .database
        .write(move |db| folders::save_catalogue(db, "fixture", &catalogue))
        .await?;
    let mut creation = MockCreationApi::new();
    let found = planned.clone();
    creation
        .expect_inspect()
        .times(1)
        .returning(move |_| Ok(Some(found.clone())));
    let mut provider = MockDestinationProvider::new();
    provider
        .expect_move_planned_mail()
        .times(1)
        .returning(|_, _, _, _| {
            Err(shep_mail_core::mail_actions::DestinationChanged(
                "The server folder encoding changed.".into(),
            )
            .into())
        });
    install(&profile, Arc::new(creation), provider);
    let error: Value = serde_json::from_str(
        &profile
            .request(
                json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Trash","logical_role":"trash","password":"fixture-password"})
                    .to_string(),
            )
            .await?,
    )?;
    assert!(
        error["error"]
            .as_str()
            .is_some_and(|error| error.contains("encoding changed")),
        "{error}"
    );
    let action = destination.owner.clone();
    let (status, pending) = profile
        .database
        .read(move |db| {
            Ok((
                db.query_row(
                    "SELECT status FROM individual_mail_actions WHERE id=?1",
                    [action],
                    |row| row.get::<_, String>(0),
                )?,
                db.query_row("SELECT COUNT(*) FROM pending_moves", [], |row| {
                    row.get::<_, i64>(0)
                })?,
            ))
        })
        .await?;
    assert_eq!(status, "rejected");
    assert_eq!(pending, 0);
    assert!(!folder_owned(&profile, &destination.owner).await?);
    Ok(())
}

#[tokio::test]
async fn renamed_cached_role_folder_lists_again_instead_of_rejecting() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
    let stale = target("INBOX.Old Archive", FolderRole::Archive);
    let renamed = target("INBOX.Archives", FolderRole::Archive);
    let catalogue = vec![stale.clone()];
    profile
        .database
        .write(move |db| folders::save_catalogue(db, "fixture", &catalogue))
        .await?;
    let mut creation = MockCreationApi::new();
    let fresh = vec![renamed.clone()];
    creation
        .expect_catalogue()
        .returning(move || Ok(fresh.clone()));
    creation
        .expect_plan()
        .returning(|_, _| Ok(target("INBOX.Elsewhere", FolderRole::Archive)));
    creation.expect_create().times(0);
    let current = renamed.clone();
    creation
        .expect_inspect()
        .returning(move |checked| Ok((checked.name == current.name).then(|| current.clone())));
    let mut provider = MockDestinationProvider::new();
    let expected = renamed.clone();
    provider
        .expect_move_planned_mail()
        .withf(move |_, _, _, target| *target == expected)
        .times(1)
        .returning(|_, _, _, _| Ok(Some("1.900".into())));
    install(&profile, Arc::new(creation), provider);
    let result = request(
        &profile,
        json!({"op":"mutate","action_id":destination.owner,"id":mail,"folder":"Archive","logical_role":"archive","password":"fixture-password"}),
    )
    .await;
    assert_eq!(result["status"], "succeeded", "{result}");
    assert_eq!(result["applied_fields"]["folder"], "INBOX.Archives");
    Ok(())
}

#[tokio::test]
async fn folder_found_before_any_create_is_used_without_an_acknowledgement() -> Result<()> {
    let (_dir, profile, destination, _mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let returned = planned.clone();
    api.expect_plan()
        .times(1)
        .returning(move |_, _| Ok(returned.clone()));
    let mut sequence = mockall::Sequence::new();
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(None));
    let appeared = planned.clone();
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(move |_| Ok(Some(appeared.clone())));
    api.expect_create().times(0);
    let Resolution::Ready(actual) = resolve(&profile.database, &api, destination.clone()).await?
    else {
        anyhow::bail!("a folder observed before CREATE is checked state");
    };
    assert_eq!(actual, planned);
    let creation = destination.creation;
    let saved = profile
        .database
        .read(move |db| folders::get(db, &creation)?.context("observed creation"))
        .await?;
    assert_eq!(saved.status, "succeeded");
    assert!(!saved.acknowledged, "no CREATE acknowledgement is invented");
    Ok(())
}

#[tokio::test]
async fn cancelled_pruned_destination_is_removed_when_its_folder_request_ends() -> Result<()> {
    let (_dir, profile, destination, _mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let before = destination.clone();
    let frozen = planned.clone();
    profile
        .database
        .write(move |db| {
            let tx = db.transaction()?;
            let mut after = before.clone();
            after.phase = "creating".into();
            after.target = Some(frozen.clone());
            folders::admit_destination(&tx, &after, &frozen)?;
            save(&tx, &before, &after)?;
            tx.commit()?;
            Ok(())
        })
        .await?;
    request(
        &profile,
        json!({"op":"cancel_mail_action","id":destination.owner}),
    )
    .await;
    let owner = destination.owner.clone();
    profile
        .database
        .write(move |db| {
            db.execute("DELETE FROM individual_mail_actions WHERE id=?1", [owner])?;
            Ok(())
        })
        .await?;
    let creation = destination.creation.clone();
    let job = profile
        .database
        .read(move |db| folders::get(db, &creation)?.context("queued creation"))
        .await?;
    let mut quiet = MockCreationApi::new();
    quiet.expect_plan().times(0);
    quiet.expect_inspect().times(0);
    quiet.expect_create().times(0);
    assert_eq!(
        folders::execute(&profile.database, &quiet, job)
            .await?
            .status,
        "cancelled"
    );
    let remaining = profile
        .database
        .read(|db| {
            Ok(db.query_row(
                "SELECT COUNT(*) FROM logical_mail_destinations",
                [],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .await?;
    assert_eq!(remaining, 0);
    Ok(())
}

#[tokio::test]
async fn failed_list_plans_the_literal_candidate_without_caching_it() -> Result<()> {
    let (_dir, profile, destination, _mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let mut api = MockCreationApi::new();
    api.expect_catalogue()
        .times(1)
        .returning(|| anyhow::bail!("LIST transport failure"));
    let returned = planned.clone();
    api.expect_plan()
        .with(eq(None), eq("Archive".to_owned()))
        .times(1)
        .returning(move |_, _| Ok(returned.clone()));
    let found = planned.clone();
    api.expect_inspect()
        .times(1)
        .returning(move |_| Ok(Some(found.clone())));
    api.expect_create().times(0);
    let Resolution::Ready(actual) = resolve(&profile.database, &api, destination).await? else {
        anyhow::bail!("the planned literal candidate exists");
    };
    assert_eq!(actual.name, "INBOX.Archive");
    let cached = profile
        .database
        .read(|db| {
            Ok(
                db.query_row("SELECT COUNT(*) FROM folder_catalogues", [], |row| {
                    row.get::<_, i64>(0)
                })?,
            )
        })
        .await?;
    assert_eq!(cached, 0);
    Ok(())
}
