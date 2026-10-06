//! Exact logical mail destinations are prerequisites of the existing action
//! and folder journals. This module never dispatches a mail mutation.
#[cfg(test)]
mod tests;
use crate::{database::Database, folders, operations};
use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use shep_mail_core::{
    folders::{FolderRole, Mailbox},
    mail_actions::connection_key,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Role {
    Archive,
    Trash,
    Spam,
}
impl Role {
    pub fn local(self) -> &'static str {
        match self {
            Self::Archive => "Archive",
            Self::Trash => "Trash",
            Self::Spam => "Spam",
        }
    }
    pub fn requested(self) -> &'static str {
        match self {
            Self::Spam => "Junk",
            _ => self.local(),
        }
    }
    pub fn special(self) -> FolderRole {
        match self {
            Self::Archive => FolderRole::Archive,
            Self::Trash => FolderRole::Trash,
            Self::Spam => FolderRole::Junk,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct Destination {
    pub kind: String,
    pub owner: String,
    pub account: String,
    pub role: Role,
    pub connection: String,
    pub creation: String,
    pub phase: String,
    pub target: Option<Mailbox>,
    pub candidate: Option<Mailbox>,
    pub error: Option<String>,
    pub revision: i64,
}

pub(crate) enum Resolution {
    Ready(Mailbox),
    Waiting {
        creation: Option<String>,
        message: String,
    },
    Obsolete,
    Rejected {
        message: String,
    },
}

pub(crate) fn get(
    db: &Connection,
    kind: &str,
    owner: &str,
    account: &str,
) -> Result<Option<Destination>> {
    let row = db.query_row(
        "SELECT role,connection,creation_id,phase,target,candidate,error,revision FROM logical_mail_destinations WHERE owner_kind=?1 AND owner=?2 AND account=?3",
        params![kind, owner, account],
        |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,Option<String>>(4)?,row.get::<_,Option<String>>(5)?,row.get::<_,Option<String>>(6)?,row.get::<_,i64>(7)?)),
    ).optional()?;
    row.map(
        |(role, connection, creation, phase, target, candidate, error, revision)| {
            Ok(Destination {
                kind: kind.into(),
                owner: owner.into(),
                account: account.into(),
                role: serde_json::from_str(&role)?,
                connection,
                creation,
                phase,
                target: target
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                candidate: candidate
                    .map(|value| serde_json::from_str(&value))
                    .transpose()?,
                error,
                revision,
            })
        },
    )
    .transpose()
}

pub(crate) fn admit(
    db: &Connection,
    kind: &str,
    owner: &str,
    account: &str,
    role: Role,
) -> Result<Destination> {
    if let Some(saved) = get(db, kind, owner, account)? {
        ensure!(
            saved.role == role,
            "This action identity belongs to another logical destination."
        );
        return Ok(saved);
    }
    crate::accounts::available(db, account)?;
    let settings = operations::stored_account(db, account)?;
    db.execute("INSERT INTO logical_mail_destinations(owner_kind,owner,account,role,connection,creation_id) VALUES(?1,?2,?3,?4,?5,?6)",
        params![kind,owner,account,serde_json::to_string(&role)?,connection_key(&settings),uuid::Uuid::new_v4().to_string()])?;
    get(db, kind, owner, account)?.context("The destination prerequisite was not saved.")
}

/// The mailbox identity must be unchanged; a reconnect with a new credential
/// slot keeps it, and dispatch checks the current slot separately.
pub(crate) fn binding(db: &Connection, destination: &Destination) -> Result<()> {
    crate::accounts::available(db, &destination.account)?;
    let settings = operations::stored_account(db, &destination.account)?;
    ensure!(
        connection_key(&settings) == destination.connection,
        "The account connection changed. Cancel this mail action and review its destination again."
    );
    Ok(())
}

/// Only a waiting individual action that still owns its folder intent and
/// exact source may continue its destination.
pub(crate) fn owns(db: &Connection, destination: &Destination) -> Result<bool> {
    if destination.phase == "cancelled" || destination.kind != "individual" {
        return Ok(false);
    }
    let saved: Option<(String,String)> = db.query_row("SELECT a.mail,a.physical FROM individual_mail_actions a JOIN mail_intents i ON i.mail=a.mail AND i.field='folder' AND i.revision=a.intent_revision WHERE a.id=?1 AND a.account=?2 AND a.status IN ('queued','waiting')",
        params![destination.owner,destination.account], |row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    let Some((id, physical)) = saved else {
        return Ok(false);
    };
    binding(db, destination)?;
    let Ok(mail) = operations::stored_mail(db, &id) else {
        return Ok(false);
    };
    operations::action_source_matches(db, &mail, &serde_json::from_str(&physical)?)
}

fn save(db: &Connection, before: &Destination, after: &Destination) -> Result<Destination> {
    ensure!(
        before
            .target
            .as_ref()
            .is_none_or(|target| after.target.as_ref() == Some(target)),
        "The frozen destination cannot be replaced."
    );
    ensure!(db.execute("UPDATE logical_mail_destinations SET phase=?1,target=?2,candidate=?3,error=?4,revision=revision+1 WHERE owner_kind=?5 AND owner=?6 AND account=?7 AND revision=?8",
        params![after.phase,after.target.as_ref().map(serde_json::to_string).transpose()?,after.candidate.as_ref().map(serde_json::to_string).transpose()?,after.error,before.kind,before.owner,before.account,before.revision])? == 1,
        "The destination changed. Refresh this saved action.");
    get(db, &before.kind, &before.owner, &before.account)?
        .context("This saved destination was removed.")
}

pub(crate) fn linked(db: &Connection, creation: &str) -> Result<Option<Destination>> {
    let key: Option<(String, String, String)> = db
        .query_row(
            "SELECT owner_kind,owner,account FROM logical_mail_destinations WHERE creation_id=?1",
            [creation],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()?;
    key.map(|(kind, owner, account)| {
        get(db, &kind, &owner, &account)?.context("The folder dependency was removed.")
    })
    .transpose()
}

pub(crate) fn allow_creation(db: &Connection, creation: &str) -> Result<bool> {
    linked(db, creation)?.map_or(Ok(true), |destination| owns(db, &destination))
}

pub(crate) fn allow_cache(db: &Connection, creation: &str) -> Result<()> {
    if let Some(destination) = linked(db, creation)? {
        binding(db, &destination)?;
    }
    Ok(())
}

pub(crate) fn local(db: &Connection, destination: &Destination) -> Result<Mailbox> {
    ensure!(
        owns(db, destination)?,
        "This local destination no longer owns its action."
    );
    if destination.phase == "ready" {
        return destination
            .target
            .clone()
            .context("The saved local destination is missing.");
    }
    let catalogue: Vec<Mailbox> = db
        .query_row(
            "SELECT mailboxes FROM folder_catalogues WHERE account_id=?1",
            [&destination.account],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default();
    let resolved =
        shep_mail_core::folders::resolve_destination(&catalogue, destination.role.requested());
    let target = catalogue
        .iter()
        .find(|mailbox| mailbox.usable() && mailbox.name == resolved)
        .cloned()
        .unwrap_or_else(|| Mailbox::flat(destination.role.local().into()));
    let mut after = destination.clone();
    after.phase = "ready".into();
    after.target = Some(target.clone());
    save(db, destination, &after)?;
    note_target(db, destination, &target)?;
    Ok(target)
}

pub(crate) async fn provider(
    profile: &crate::api::MobileProfile,
    destination: Destination,
    account: shep_mail_core::model::Account,
    password: secrecy::SecretString,
) -> Result<Resolution> {
    #[cfg(test)]
    {
        let injected = profile
            .operations
            .folder_provider
            .lock()
            .expect("folder provider")
            .clone();
        if let Some(api) = injected {
            return resolve(&profile.database, api.as_ref(), destination).await;
        }
    }
    resolve(
        &profile.database,
        &folders::ImapCreation { account, password },
        destination,
    )
    .await
}

async fn alive(db: &Database, destination: &Destination) -> Result<bool> {
    let checked = destination.clone();
    db.read(move |db| owns(db, &checked)).await
}

fn role_name(role: Role) -> &'static str {
    match role {
        Role::Archive => "Archive",
        Role::Trash => "Trash",
        Role::Spam => "Junk",
    }
}

fn note_target(db: &Connection, destination: &Destination, target: &Mailbox) -> Result<()> {
    let encoding = serde_json::to_value(target.encoding)?;
    db.execute(
        "DELETE FROM folder_role_names WHERE account_id=?1 AND role=?2 AND observed=1",
        params![destination.account, role_name(destination.role)],
    )?;
    db.execute("INSERT INTO folder_role_names(account_id,role,name,encoding,observed) VALUES(?1,?2,?3,?4,1) ON CONFLICT(account_id,role,name) DO UPDATE SET encoding=excluded.encoding",
        params![destination.account,role_name(destination.role),target.name,encoding.as_str().context("Invalid destination encoding.")?])?;
    Ok(())
}

async fn ready(db: &Database, destination: &Destination, target: Mailbox) -> Result<Resolution> {
    let before = destination.clone();
    let retained = target.clone();
    let saved = db
        .write(move |db| {
            let tx = db.transaction()?;
            if !owns(&tx, &before)? {
                return Ok(false);
            }
            let mut after = before.clone();
            after.phase = "ready".into();
            after.target = Some(retained.clone());
            after.error = None;
            save(&tx, &before, &after)?;
            note_target(&tx, &before, &retained)?;
            tx.commit()?;
            Ok(true)
        })
        .await?;
    Ok(if saved {
        Resolution::Ready(target)
    } else {
        Resolution::Obsolete
    })
}

fn waiting(destination: &Destination, message: &str) -> Resolution {
    Resolution::Waiting {
        creation: (destination.phase == "creating" || destination.phase == "blocked")
            .then(|| destination.creation.clone()),
        message: message.into(),
    }
}

async fn block(
    db: &Database,
    destination: &Destination,
    phase: &str,
    message: &str,
) -> Result<Resolution> {
    if !alive(db, destination).await? {
        return Ok(Resolution::Obsolete);
    }
    let before = destination.clone();
    let mut after = before.clone();
    after.phase = phase.into();
    after.error = Some(message.into());
    let saved = db
        .write(move |db| {
            if !owns(db, &before)? {
                return Ok(None);
            }
            save(db, &before, &after).map(Some)
        })
        .await?;
    Ok(saved.map_or(Resolution::Obsolete, |saved| waiting(&saved, message)))
}

pub(crate) async fn cached(db: &Database, destination: &Destination) -> Result<Option<Resolution>> {
    if !alive(db, destination).await? {
        return Ok(Some(Resolution::Obsolete));
    }
    if destination.phase == "rejected" {
        return Ok(Some(Resolution::Rejected {
            message: destination
                .error
                .clone()
                .unwrap_or_else(|| "The destination needs a fresh review.".into()),
        }));
    }
    if destination.phase == "ready" {
        return Ok(Some(Resolution::Ready(
            destination
                .target
                .clone()
                .context("The saved destination is missing.")?,
        )));
    }
    let id = destination.creation.clone();
    let job = db.read(move |db| folders::get(db, &id)).await?;
    let Some(job) = job else {
        return Ok(None);
    };
    if job.acknowledged && job.receipt.is_some() {
        if job.status != "succeeded" {
            let repairing = job.clone();
            db.write(move |db| folders::apply_receipt(db, &repairing))
                .await?;
        }
        let target = destination
            .target
            .clone()
            .context("The saved folder target is missing.")?;
        return ready(db, destination, target).await.map(Some);
    }
    if matches!(
        job.status.as_str(),
        "uncertain" | "running" | "checking" | "rejected" | "cancelled" | "dismissed" | "succeeded"
    ) {
        return block(db,destination,"blocked","This destination needs review in Folder activity. An existence check cannot confirm the original CREATE. Cancel this mail action before choosing a new checked destination.").await.map(Some);
    }
    Ok(None)
}

async fn rejected(db: &Database, destination: &Destination) -> Result<Resolution> {
    let message = "The destination name or namespace is unavailable. Cancel this action and review its destination before trying again.";
    let saved = block(db, destination, "rejected", message).await?;
    Ok(if matches!(saved, Resolution::Obsolete) {
        Resolution::Obsolete
    } else {
        Resolution::Rejected {
            message: message.into(),
        }
    })
}

fn planned_parts(candidate: &Mailbox) -> (Option<String>, String) {
    let tree = shep_mail_core::folders::Tree::new(std::slice::from_ref(candidate));
    if let Some(node) = tree.nodes.iter().find(|node| node.path == candidate.name) {
        return (
            node.parent.map(|parent| tree.nodes[parent].path.clone()),
            node.label.clone(),
        );
    }
    (
        None,
        candidate.encoding.display(&candidate.name).into_owned(),
    )
}

pub(crate) async fn resolve(
    db: &Database,
    api: &dyn folders::execute::CreationApi,
    mut destination: Destination,
) -> Result<Resolution> {
    if let Some(result) = cached(db, &destination).await? {
        return Ok(result);
    }
    if !alive(db, &destination).await? {
        return Ok(Resolution::Obsolete);
    }
    if destination.target.is_none() {
        let account = destination.account.clone();
        let mut catalogue: Vec<Mailbox> = db
            .read(move |db| {
                db.query_row(
                    "SELECT mailboxes FROM folder_catalogues WHERE account_id=?1",
                    [account],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .map(|value| serde_json::from_str(&value).map_err(anyhow::Error::from))
                .transpose()
                .map(Option::unwrap_or_default)
            })
            .await?;
        if catalogue.is_empty()
            && let Ok(listed) = api.catalogue().await
        {
            ensure!(
                listed.len() <= 8192,
                "The folder catalogue exceeded the supported limit."
            );
            let checked = destination.clone();
            let stored = listed.clone();
            let retained = db
                .write(move |db| {
                    let tx = db.transaction()?;
                    if !owns(&tx, &checked)? {
                        return Ok(false);
                    }
                    folders::save_catalogue(&tx, &checked.account, &stored)?;
                    tx.commit()?;
                    Ok(true)
                })
                .await?;
            if !retained {
                return Ok(Resolution::Obsolete);
            }
            catalogue = listed;
        }
        if !alive(db, &destination).await? {
            return Ok(Resolution::Obsolete);
        }
        let requested =
            shep_mail_core::folders::resolve_destination(&catalogue, destination.role.requested());
        let known = catalogue
            .iter()
            .find(|mailbox| mailbox.usable() && mailbox.name == requested)
            .cloned();
        let mut target = if let Some(candidate) = known.as_ref() {
            match api.inspect(candidate.clone()).await {
                Ok(Some(observed)) if observed.usable() && observed.name==candidate.name && observed.encoding==candidate.encoding => return ready(db,&destination,candidate.clone()).await,
                Ok(None) => {},
                _ => return block(db,&destination,"waiting","Could not verify the saved destination. Reconnect or refresh its folders, then retry this action.").await,
            }
            let (parent, name) = planned_parts(candidate);
            match api.plan(parent,name).await {
                Ok(planned) if planned.name==candidate.name && planned.encoding==candidate.encoding => planned,
                Err(error) if error.downcast_ref::<shep_mail_core::folder_actions::creation::PlanRejected>().is_some() => return rejected(db,&destination).await,
                Ok(_) => return rejected(db,&destination).await,
                _ => return block(db,&destination,"waiting","The saved folder namespace is unavailable. Refresh its folders and review this action.").await,
            }
        } else {
            match api.plan(None,requested).await {
                Ok(planned) => planned,
                Err(error) if error.downcast_ref::<shep_mail_core::folder_actions::creation::PlanRejected>().is_some() => return rejected(db,&destination).await,
                Err(_) => return block(db,&destination,"waiting","Could not discover the destination namespace. Reconnect or refresh its folders, then retry this action.").await,
            }
        };
        target.role = Some(destination.role.special());
        shep_mail_core::folder_actions::creation::valid_path(&target.name)?;
        match api.inspect(target.clone()).await {
            Ok(Some(observed))
                if observed.usable()
                    && observed.name == target.name
                    && observed.encoding == target.encoding =>
            {
                return ready(db, &destination, target).await;
            }
            Ok(None) => {}
            _ => {
                return block(
                    db,
                    &destination,
                    "waiting",
                    "Could not check the exact destination. No folder or mail command was sent.",
                )
                .await;
            }
        }
        if !alive(db, &destination).await? {
            return Ok(Resolution::Obsolete);
        }
        let before = destination.clone();
        let planned = target.clone();
        destination = db
            .write(move |db| {
                let tx = db.transaction()?;
                let mut after = before.clone();
                after.phase = "creating".into();
                after.target = Some(planned.clone());
                after.candidate = known;
                folders::admit_destination(&tx, &after, &planned)?;
                let saved = save(&tx, &before, &after)?;
                tx.commit()?;
                Ok(saved)
            })
            .await?;
    }
    let id = destination.creation.clone();
    let job = db
        .read(move |db| {
            folders::get(db, &id)?.context("The saved destination folder request is missing.")
        })
        .await?;
    let created = match folders::execute(db,api,job).await {
        Ok(created) => created,
        Err(_) => return block(db,&destination,"creating","Could not finish the saved destination. Open Folder activity to inspect or repair it; MOVE has not started.").await,
    };
    if !alive(db, &destination).await? {
        return Ok(Resolution::Obsolete);
    }
    if created.status == "succeeded" && created.acknowledged && created.receipt.is_some() {
        return ready(
            db,
            &destination,
            destination
                .target
                .clone()
                .context("The frozen destination is missing.")?,
        )
        .await;
    }
    block(db,&destination,"creating","The saved destination has not confirmed CREATE. Open Folder activity before continuing this action.").await
}
