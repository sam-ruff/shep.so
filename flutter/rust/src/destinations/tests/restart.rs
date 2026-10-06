use super::*;

#[tokio::test]
async fn create_rejection_retains_frozen_target_and_requires_explicit_folder_review() -> Result<()>
{
    let (_dir, profile, destination, _mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let returned = planned.clone();
    api.expect_plan()
        .times(1)
        .returning(move |_, _| Ok(returned.clone()));
    api.expect_inspect().times(2).returning(|_| Ok(None));
    api.expect_create()
        .with(eq(planned.clone()))
        .times(1)
        .returning(|_| Ok(CreateOutcome::Rejected("private provider detail".into())));
    assert!(matches!(
        resolve(&profile.database, &api, destination.clone()).await?,
        Resolution::Waiting { .. }
    ));
    let creation = destination.creation.clone();
    let saved = profile
        .database
        .read(move |db| folders::get(db, &creation)?.context("rejected creation"))
        .await?;
    assert_eq!(saved.status, "rejected");
    assert_eq!(saved.target, Some(planned));
    assert!(!saved.acknowledged);
    assert!(
        !saved
            .error
            .context("public recovery")?
            .contains("private provider detail")
    );
    let owner = destination.owner;
    let destination = profile
        .database
        .read(move |db| get(db, "individual", &owner, "fixture")?.context("retained destination"))
        .await?;
    let mut quiet = MockCreationApi::new();
    quiet.expect_catalogue().times(0);
    quiet.expect_plan().times(0);
    quiet.expect_inspect().times(0);
    quiet.expect_create().times(0);
    assert!(matches!(
        resolve(&profile.database, &quiet, destination).await?,
        Resolution::Waiting { .. }
    ));
    Ok(())
}

#[tokio::test]
async fn unknown_create_restart_retains_exact_uuid_encoding_and_does_not_discover_or_replay()
-> Result<()> {
    let (directory, profile, destination, _mail) = setup(Role::Spam).await?;
    let planned = target("INBOX.Junk", FolderRole::Junk);
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let returned = planned.clone();
    api.expect_plan()
        .with(eq(None), eq("Junk".to_owned()))
        .times(1)
        .returning(move |_, _| Ok(returned.clone()));
    api.expect_inspect().times(2).returning(|_| Ok(None));
    api.expect_create()
        .with(eq(planned.clone()))
        .times(1)
        .returning(|_| Ok(CreateOutcome::Uncertain("lost acknowledgement".into())));
    assert!(matches!(
        resolve(&profile.database, &api, destination.clone()).await?,
        Resolution::Waiting { .. }
    ));
    drop(profile);
    let profile = crate::api::MobileProfile::open(
        directory
            .path()
            .join("mail.sqlite3")
            .to_string_lossy()
            .into_owned(),
    )
    .await?;
    let owner = destination.owner.clone();
    let resumed = profile
        .database
        .read(move |db| get(db, "individual", &owner, "fixture")?.context("reopened destination"))
        .await?;
    assert_eq!(resumed.creation, destination.creation);
    assert_eq!(resumed.target, Some(planned.clone()));
    let creation = resumed.creation.clone();
    let saved = profile
        .database
        .read(move |db| folders::get(db, &creation)?.context("reopened creation"))
        .await?;
    assert_eq!(saved.status, "uncertain");
    assert_eq!(saved.target, Some(planned));
    assert!(!saved.acknowledged);
    let mut quiet = MockCreationApi::new();
    quiet.expect_catalogue().times(0);
    quiet.expect_plan().times(0);
    quiet.expect_inspect().times(0);
    quiet.expect_create().times(0);
    assert!(matches!(
        resolve(&profile.database, &quiet, resumed).await?,
        Resolution::Waiting { .. }
    ));
    Ok(())
}
