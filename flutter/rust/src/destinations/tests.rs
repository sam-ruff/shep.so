use super::*;
mod cache;
mod mutation;
mod restart;
use crate::folders::execute::MockCreationApi;
use crate::tests::{profile, request, seed};
use mockall::predicate::eq;
use serde_json::json;
use shep_mail_core::{
    folder_actions::creation::{CreateOutcome, PlanRejected},
    folders::NameEncoding,
    model::Protocol,
};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct HeldApi {
    base: MockCreationApi,
    boundary: &'static str,
    held: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl HeldApi {
    async fn wait(&self, boundary: &str) {
        if boundary == self.boundary && !self.held.swap(true, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
}
#[async_trait::async_trait]
impl crate::folders::execute::CreationApi for HeldApi {
    async fn catalogue(&self) -> Result<Vec<Mailbox>> {
        self.wait("catalogue").await;
        self.base.catalogue().await
    }
    async fn plan(&self, parent: Option<String>, name: String) -> Result<Mailbox> {
        self.wait("plan").await;
        self.base.plan(parent, name).await
    }
    async fn inspect(&self, target: Mailbox) -> Result<Option<Mailbox>> {
        self.wait("inspect").await;
        self.base.inspect(target).await
    }
    async fn create(&self, target: Mailbox) -> Result<CreateOutcome> {
        self.wait("create").await;
        self.base.create(target).await
    }
}

async fn setup(
    role: Role,
) -> Result<(
    tempfile::TempDir,
    crate::api::MobileProfile,
    Destination,
    String,
)> {
    let (directory, profile) = profile().await;
    seed(&profile, 1).await;
    let mail = profile
        .database
        .write(|db| {
            let mut account = operations::stored_account(db, "fixture")?;
            account.protocol = Protocol::Imap;
            db.execute(
                "UPDATE accounts SET settings=?1 WHERE id='fixture'",
                [serde_json::to_string(&account)?],
            )?;
            Ok(db.query_row("SELECT id FROM mail LIMIT 1", [], |row| {
                row.get::<_, String>(0)
            })?)
        })
        .await?;
    let id = uuid::Uuid::new_v4().to_string();
    let admitted = request(
        &profile,
        json!({"op":"mutate","action_id":id,"id":mail,"folder":role.local(),"logical_role":role}),
    )
    .await;
    assert_eq!(admitted["status"], "waiting");
    let owner = id.clone();
    let destination = profile
        .database
        .read(move |db| get(db, "individual", &owner, "fixture")?.context("admitted destination"))
        .await?;
    Ok((directory, profile, destination, mail))
}

fn target(name: &str, role: FolderRole) -> Mailbox {
    Mailbox {
        name: name.into(),
        delimiter: Some('.'),
        selectable: true,
        encoding: NameEncoding::ImapUtf7,
        no_inferiors: false,
        non_existent: false,
        role: Some(role),
    }
}

#[tokio::test]
async fn cached_special_use_wins_without_broad_discovery_and_preserves_exact_encoding() -> Result<()>
{
    let (_dir, profile, destination, _mail) = setup(Role::Trash).await?;
    let mailbox = target("INBOX.&ZeVnLIqe-", FolderRole::Trash);
    let catalogue = vec![mailbox.clone()];
    profile
        .database
        .write(move |db| folders::save_catalogue(db, "fixture", &catalogue))
        .await?;
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(0);
    api.expect_plan().times(0);
    api.expect_create().times(0);
    let found = mailbox.clone();
    api.expect_inspect()
        .with(eq(mailbox.clone()))
        .times(1)
        .returning(move |_| Ok(Some(found.clone())));
    let Resolution::Ready(actual) = resolve(&profile.database, &api, destination).await? else {
        anyhow::bail!("expected resolved destination");
    };
    assert_eq!(actual, mailbox);
    assert!(
        profile
            .database
            .read(|db| Ok(db
                .query_row("SELECT COUNT(*) FROM folder_creations", [], |row| row
                    .get::<_, i64>(0))?))
            .await?
            == 0
    );
    Ok(())
}

#[tokio::test]
async fn failed_list_keeps_literal_candidate_and_next_admission_lists_again() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
    let mut api = MockCreationApi::new();
    api.expect_catalogue()
        .times(2)
        .returning(|| anyhow::bail!("LIST transport failure"));
    api.expect_plan()
        .with(eq(None), eq("Archive".to_owned()))
        .times(2)
        .returning(|_, _| anyhow::bail!("namespace unavailable"));
    api.expect_inspect().times(0);
    api.expect_create().times(0);
    assert!(matches!(
        resolve(&profile.database, &api, destination).await?,
        Resolution::Waiting { .. }
    ));
    assert_eq!(
        profile
            .database
            .read(|db| Ok(
                db.query_row("SELECT COUNT(*) FROM folder_catalogues", [], |row| row
                    .get::<_, i64>(0))?
            ))
            .await?,
        0
    );
    let id = uuid::Uuid::new_v4().to_string();
    request(
        &profile,
        json!({"op":"mutate","action_id":id,"id":mail,"folder":"Archive","logical_role":"archive"}),
    )
    .await;
    let destination = profile
        .database
        .read(move |db| get(db, "individual", &id, "fixture")?.context("second admission"))
        .await?;
    assert!(matches!(
        resolve(&profile.database, &api, destination).await?,
        Resolution::Waiting { .. }
    ));
    assert_eq!(
        profile
            .database
            .read(|db| Ok(db.query_row(
                "SELECT COUNT(*) FROM folder_creations WHERE acknowledged=1",
                [],
                |row| row.get::<_, i64>(0)
            )?))
            .await?,
        0
    );
    Ok(())
}

#[tokio::test]
async fn typed_namespace_rejection_is_retained_without_create_or_transient_retry() -> Result<()> {
    let (_dir, profile, destination, _mail) = setup(Role::Spam).await?;
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(1).returning(|| Ok(vec![]));
    api.expect_plan()
        .with(eq(None), eq("Junk".to_owned()))
        .times(1)
        .returning(|_, _| Err(PlanRejected("No selectable namespace".into()).into()));
    api.expect_inspect().times(0);
    api.expect_create().times(0);
    assert!(matches!(
        resolve(&profile.database, &api, destination.clone()).await?,
        Resolution::Rejected { .. }
    ));
    let saved = profile
        .database
        .read(move |db| {
            get(
                db,
                &destination.kind,
                &destination.owner,
                &destination.account,
            )?
            .context("rejected prerequisite")
        })
        .await?;
    assert_eq!(saved.phase, "rejected");
    assert!(matches!(
        resolve(&profile.database, &api, saved).await?,
        Resolution::Rejected { .. }
    ));
    Ok(())
}

#[tokio::test]
async fn missing_namespace_target_has_one_frozen_creation_and_ack_before_ready() -> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let plan = planned.clone();
    api.expect_plan()
        .with(eq(None), eq("Archive".to_owned()))
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
        .with(eq(planned.clone()))
        .times(1)
        .in_sequence(&mut sequence)
        .returning(|_| Ok(CreateOutcome::Acknowledged));
    let observed = planned.clone();
    api.expect_inspect()
        .times(1)
        .in_sequence(&mut sequence)
        .returning(move |_| Ok(Some(observed.clone())));
    let Resolution::Ready(actual) = resolve(&profile.database, &api, destination.clone()).await?
    else {
        anyhow::bail!("expected acknowledged destination");
    };
    assert_eq!(actual, planned);
    let creation = destination.creation.clone();
    let saved = profile
        .database
        .read(move |db| folders::get(db, &creation)?.context("linked creation"))
        .await?;
    assert!(saved.acknowledged);
    assert_eq!(saved.target, Some(planned));
    assert_eq!(saved.status, "succeeded");
    let folder = profile
        .database
        .read(move |db| Ok(operations::stored_mail(db, &mail)?.folder))
        .await?;
    assert_eq!(folder, "INBOX", "destination preparation cannot MOVE mail");
    let same = profile
        .database
        .read(move |db| {
            get(
                db,
                &destination.kind,
                &destination.owner,
                &destination.account,
            )?
            .context("saved destination")
        })
        .await?;
    let mut recovery = MockCreationApi::new();
    recovery.expect_create().times(0);
    assert!(matches!(
        resolve(&profile.database, &recovery, same).await?,
        Resolution::Ready(_)
    ));
    Ok(())
}

#[tokio::test]
async fn lost_create_ack_never_resumes_from_existence_and_new_admission_can_use_checked_folder()
-> Result<()> {
    let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
    let planned = target("INBOX.Archive", FolderRole::Archive);
    let mut api = MockCreationApi::new();
    api.expect_catalogue().times(1).returning(|| Ok(vec![]));
    let plan = planned.clone();
    api.expect_plan()
        .times(1)
        .returning(move |_, _| Ok(plan.clone()));
    api.expect_inspect().times(2).returning(|_| Ok(None));
    api.expect_create()
        .times(1)
        .returning(|_| Ok(CreateOutcome::Uncertain("lost tagged reply".into())));
    assert!(matches!(
        resolve(&profile.database, &api, destination.clone()).await?,
        Resolution::Waiting { .. }
    ));
    let id = destination.creation.clone();
    let saved = profile
        .database
        .read(move |db| folders::get(db, &id)?.context("uncertain creation"))
        .await?;
    assert!(!saved.acknowledged);
    let creation = saved.id.clone();
    let checked = profile
        .database
        .write(move |db| folders::decide(db, &creation, saved.revision, "check"))
        .await?;
    let mut inspection = MockCreationApi::new();
    let found = planned.clone();
    inspection
        .expect_inspect()
        .times(1)
        .returning(move |_| Ok(Some(found.clone())));
    inspection.expect_create().times(0);
    let observed = folders::execute(&profile.database, &inspection, checked).await?;
    assert_eq!(observed.status, "succeeded");
    assert!(
        !observed.acknowledged,
        "read-only existence cannot fabricate CREATE acknowledgement"
    );
    let owner = destination.owner.clone();
    let retained = profile
        .database
        .read(move |db| get(db, "individual", &owner, "fixture")?.context("retained destination"))
        .await?;
    let mut no_commands = MockCreationApi::new();
    no_commands.expect_catalogue().times(0);
    no_commands.expect_inspect().times(0);
    no_commands.expect_create().times(0);
    assert!(matches!(
        resolve(&profile.database, &no_commands, retained).await?,
        Resolution::Waiting { .. }
    ));
    let fresh = uuid::Uuid::new_v4().to_string();
    request(&profile,json!({"op":"mutate","action_id":fresh,"id":mail,"folder":"Archive","logical_role":"archive"})).await;
    let fresh_destination = profile
        .database
        .read(move |db| get(db, "individual", &fresh, "fixture")?.context("fresh destination"))
        .await?;
    let mut fresh_api = MockCreationApi::new();
    fresh_api.expect_catalogue().times(0);
    fresh_api.expect_plan().times(0);
    fresh_api.expect_create().times(0);
    fresh_api
        .expect_inspect()
        .times(1)
        .returning(move |_| Ok(Some(planned.clone())));
    assert!(matches!(
        resolve(&profile.database, &fresh_api, fresh_destination).await?,
        Resolution::Ready(_)
    ));
    Ok(())
}

#[tokio::test]
async fn cancelled_intent_at_every_held_boundary_prevents_move_and_retains_late_create_ack()
-> Result<()> {
    for boundary in ["catalogue", "plan", "inspect", "create"] {
        let (_dir, profile, destination, mail) = setup(Role::Archive).await?;
        let planned = target("INBOX.Archive", FolderRole::Archive);
        let mut base = MockCreationApi::new();
        base.expect_catalogue().returning(|| Ok(vec![]));
        let plan = planned.clone();
        base.expect_plan().returning(move |_, _| Ok(plan.clone()));
        let inspected = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let inspections = inspected.clone();
        base.expect_inspect().returning(move |_| {
            Ok((inspections.fetch_add(1, Ordering::SeqCst) >= 2).then(|| planned.clone()))
        });
        base.expect_create()
            .times(usize::from(boundary == "create"))
            .returning(|_| Ok(CreateOutcome::Acknowledged));
        let api = Arc::new(HeldApi {
            base,
            boundary,
            held: AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        });
        let worker = api.clone();
        let db = profile.database.clone();
        let checked = destination.clone();
        let running = tokio::spawn(async move { resolve(&db, worker.as_ref(), checked).await });
        api.entered.notified().await;
        request(
            &profile,
            json!({"op":"cancel_mail_action","id":destination.owner}),
        )
        .await;
        api.release.notify_one();
        assert!(
            matches!(
                running.await.context("resolver task")??,
                Resolution::Obsolete
            ),
            "{boundary}"
        );
        let folder = profile
            .database
            .read(move |db| Ok(operations::stored_mail(db, &mail)?.folder))
            .await?;
        assert_eq!(
            folder, "INBOX",
            "{boundary}: prerequisite must not dispatch MOVE"
        );
        let creation = destination.creation.clone();
        let saved = profile
            .database
            .read(move |db| folders::get(db, &creation))
            .await?;
        if boundary == "create" {
            assert!(saved.context("late receipt retained")?.acknowledged);
        } else {
            assert!(
                saved.is_none(),
                "{boundary}: cancelled read cannot admit CREATE"
            );
        }
    }
    Ok(())
}
