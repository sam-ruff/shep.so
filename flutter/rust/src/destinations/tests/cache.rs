use super::*;

#[tokio::test]
async fn acknowledged_folder_receipt_repairs_cache_without_credentials_or_another_create()
-> Result<()> {
    let (_dir, profile, destination, _mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let plan = planned.clone();
    api.expect_plan()
        .times(1)
        .returning(move |_, _| Ok(plan.clone()));
    let mut sequence = mockall::Sequence::new();
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(None));
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(None));
    api.expect_create()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| anyhow::bail!("post-write discovery lost"));
    assert!(matches!(
        resolve(&profile.database, &api, destination.clone()).await?,
        Resolution::Waiting { .. }
    ));
    profile.database.write(|db| {db.execute_batch("CREATE TRIGGER refuse_destination_cache BEFORE INSERT ON folder_catalogues BEGIN SELECT RAISE(ABORT,'cache fixture failure'); END;")?;Ok(())}).await?;
    let creation = destination.creation.clone();
    let job = profile
        .database
        .read(move |db| folders::get(db, &creation)?.context("repairing creation"))
        .await?;
    let mut inspect = MockCreationApi::new();
    let found = planned.clone();
    inspect
        .expect_inspect()
        .times(1)
        .returning(move |_| Ok(Some(found.clone())));
    inspect.expect_create().times(0);
    assert!(
        folders::execute(&profile.database, &inspect, job)
            .await
            .is_err()
    );
    let creation = destination.creation.clone();
    let saved = profile
        .database
        .read(move |db| folders::get(db, &creation)?.context("retained receipt"))
        .await?;
    assert!(saved.acknowledged);
    assert_eq!(saved.receipt, Some(planned));
    profile
        .database
        .write(|db| {
            db.execute_batch("DROP TRIGGER refuse_destination_cache")?;
            Ok(())
        })
        .await?;
    let owner = destination.owner.clone();
    let fresh = profile
        .database
        .read(move |db| get(db, "individual", &owner, "fixture")?.context("current prerequisite"))
        .await?;
    let mut no_provider = MockCreationApi::new();
    no_provider.expect_catalogue().times(0);
    no_provider.expect_plan().times(0);
    no_provider.expect_inspect().times(0);
    no_provider.expect_create().times(0);
    assert!(matches!(
        resolve(&profile.database, &no_provider, fresh).await?,
        Resolution::Ready(_)
    ));
    Ok(())
}

#[tokio::test]
async fn fresh_catalogue_removes_stale_roles_and_logical_views_use_current_physical_names()
-> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Trash).await?;
    let mailbox = target("Corbeille", FolderRole::Trash);
    let catalogue = vec![mailbox.clone()];
    profile
        .database
        .write(move |db| folders::save_catalogue(db, "fixture", &catalogue))
        .await?;
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(0);
    api.expect_inspect()
        .times(1)
        .returning(move |_| Ok(Some(mailbox.clone())));
    assert!(matches!(
        resolve(&profile.database, &api, destination.clone()).await?,
        Resolution::Ready(_)
    ));
    request(
        &profile,
        json!({"op":"cancel_mail_action","id":destination.owner}),
    )
    .await;
    let mut obsolete = target("Corbeille", FolderRole::Trash);
    obsolete.selectable = false;
    let current = target("Bin", FolderRole::Trash);
    profile
        .database
        .write(move |db| {
            folders::save_catalogue(db, "fixture", &[obsolete, current])?;
            db.execute("UPDATE mail SET folder='Bin' WHERE id=?1", [mail])?;
            Ok(())
        })
        .await?;
    let aliases=profile.database.read(|db| {
        Ok(db.prepare("SELECT name FROM folder_role_names INDEXED BY folder_role_lookup WHERE role='Trash' AND account_id='fixture'")?.query_map([],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)
    }).await?;
    assert_eq!(aliases, vec!["Bin"]);
    let page = request(&profile, json!({"op":"page","folder":"Trash"})).await;
    assert_eq!(page["total"], 1);
    assert_eq!(page["mail"][0]["folder"], "Bin");
    assert_eq!(page["folder_membership"]["fixture"][0], "Bin");
    Ok(())
}

#[tokio::test]
async fn pruned_action_history_removes_its_finished_destinations() -> Result<()> {
    let (_dir, profile) = crate::tests::profile().await;
    seed(&profile, 1).await;
    for n in 0..110 {
        let role = if n % 2 == 0 {
            Role::Archive
        } else {
            Role::Trash
        };
        let mail = profile
            .database
            .read(|db| Ok(db.query_row("SELECT id FROM mail", [], |row| row.get::<_, String>(0))?))
            .await?;
        let result = request(
            &profile,
            json!({"op":"mutate","action_id":uuid::Uuid::new_v4().to_string(),"id":mail,"folder":role.local(),"logical_role":role}),
        )
        .await;
        assert_eq!(result["status"], "succeeded");
    }
    let (actions, destinations) = profile
        .database
        .read(|db| {
            Ok((
                db.query_row("SELECT COUNT(*) FROM individual_mail_actions", [], |row| {
                    row.get::<_, i64>(0)
                })?,
                db.query_row(
                    "SELECT COUNT(*) FROM logical_mail_destinations",
                    [],
                    |row| row.get::<_, i64>(0),
                )?,
            ))
        })
        .await?;
    assert_eq!(actions, 101);
    assert_eq!(destinations, actions);
    Ok(())
}
