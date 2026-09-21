use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use shep_mail_core::{folders::Mailbox, mail_actions::connection_key, model::Account};
pub(crate) mod changes;
pub(crate) mod execute;
#[cfg(test)]
mod tests;
pub(crate) use execute::{ImapCreation, execute};

const COLUMNS: &str = "id,account_id,connection,parent,name,status,target,receipt,acknowledged,error,revision,created,mutation";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Creation {
    pub id: String,
    pub account: String,
    pub connection: String,
    pub parent: Option<String>,
    pub name: String,
    pub status: String,
    pub target: Option<Mailbox>,
    pub receipt: Option<Mailbox>,
    pub acknowledged: bool,
    pub error: Option<String>,
    pub revision: i64,
    pub created: i64,
    pub mutation: Option<changes::Mutation>,
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<Creation> {
    fn mailbox(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<Mailbox>> {
        row.get::<_, Option<String>>(index)?
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        index,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .transpose()
    }
    Ok(Creation {
        id: row.get(0)?,
        account: row.get(1)?,
        connection: row.get(2)?,
        parent: row.get(3)?,
        name: row.get(4)?,
        status: row.get(5)?,
        target: mailbox(row, 6)?,
        receipt: mailbox(row, 7)?,
        acknowledged: row.get(8)?,
        error: row.get(9)?,
        revision: row.get(10)?,
        created: row.get(11)?,
        mutation: row
            .get::<_, Option<String>>(12)?
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        12,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
            })
            .transpose()?,
    })
}

pub fn get(db: &Connection, id: &str) -> Result<Option<Creation>> {
    Ok(db
        .query_row(
            &format!("SELECT {COLUMNS} FROM folder_creations WHERE id=?1"),
            [id],
            decode,
        )
        .optional()?)
}

pub fn history(db: &Connection) -> Result<Vec<Creation>> {
    let mut active = db.prepare(&format!("SELECT {COLUMNS} FROM folder_creations INDEXED BY folder_creation_active WHERE status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain') ORDER BY created,id LIMIT 32"))?;
    let mut result = active
        .query_map([], decode)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut completed = db.prepare(&format!("SELECT {COLUMNS} FROM folder_creations INDEXED BY folder_creation_history WHERE status IN ('succeeded','cancelled') ORDER BY created DESC,id LIMIT ?1"))?;
    let remaining = 50_i64 - i64::try_from(result.len())?;
    result.extend(
        completed
            .query_map([remaining], decode)?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    );
    Ok(result)
}

pub fn options(db: &Connection) -> Result<serde_json::Value> {
    let mut query = db.prepare("SELECT a.settings,COALESCE(f.names,'[]'),COALESCE(c.mailboxes,'[]') FROM accounts a LEFT JOIN folders f ON f.account_id=a.id LEFT JOIN folder_catalogues c ON c.account_id=a.id WHERE NOT EXISTS(SELECT 1 FROM removed_accounts r WHERE r.id=a.id) ORDER BY a.id")?;
    let rows = query.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut result = Vec::new();
    for row in rows {
        let (settings, names, catalogue) = row?;
        let account: Account = serde_json::from_str(&settings)?;
        let catalogue: Vec<Mailbox> = serde_json::from_str(&catalogue)?;
        let mut labels: std::collections::BTreeMap<_, _> = catalogue
            .iter()
            .map(|mailbox| {
                (
                    mailbox.name.clone(),
                    mailbox.encoding.display(&mailbox.name).into_owned(),
                )
            })
            .collect();
        let tree = shep_mail_core::folders::Tree::new(&catalogue);
        for node in &tree.nodes {
            labels.insert(node.path.clone(), node.display_path.clone());
        }
        let change_names: Vec<String> =
            if catalogue.is_empty() || account.protocol == shep_mail_core::model::Protocol::Pop3 {
                serde_json::from_str(&names)?
            } else {
                tree.nodes.iter().map(|node| node.path.clone()).collect()
            };
        result.push(serde_json::json!({ "account": account.id, "label":account.name, "email":account.email,
            "connection":connection_key(&account), "protocol":account.protocol,
            "names":serde_json::from_str::<Vec<String>>(&names)?, "change_names":change_names, "catalogue":catalogue, "parent_labels":labels }));
    }
    Ok(serde_json::to_value(result)?)
}

pub fn wait(db: &Connection, job: &Creation) -> Result<Creation> {
    if !matches!(job.status.as_str(), "queued" | "waiting") {
        return Ok(job.clone());
    }
    let mut after = job.clone();
    after.status = "waiting".into();
    after.error = Some("Waiting for this account's background work.".into());
    save(db, job, &after)
}

pub fn create_local(db: &mut Connection, job: &Creation) -> Result<Creation> {
    anyhow::ensure!(
        matches!(job.status.as_str(), "queued" | "waiting"),
        "This folder request is no longer queued."
    );
    anyhow::ensure!(
        !job.name.contains('/'),
        "Use a single folder name and select its parent separately."
    );
    let mut after = job.clone();
    let name = job.parent.as_ref().map_or_else(
        || job.name.clone(),
        |parent| format!("{parent}/{}", job.name),
    );
    let mailbox = Mailbox::flat(name);
    after.target = Some(mailbox.clone());
    after.receipt = Some(mailbox);
    after.status = "repair".into();
    let saved = save(db, job, &after)?;
    apply_receipt(db, &saved)
}

pub fn checked_account(db: &Connection, job: &Creation) -> Result<Account> {
    crate::accounts::available(db, &job.account)?;
    let account = crate::operations::stored_account(db, &job.account)?;
    anyhow::ensure!(
        connection_key(&account) == job.connection,
        "The account connection changed. Stop tracking this request and create the folder again."
    );
    Ok(account)
}

pub fn admit(
    db: &mut Connection,
    id: &str,
    account: &str,
    expected: &str,
    parent: Option<String>,
    name: String,
) -> Result<Creation> {
    uuid::Uuid::parse_str(id).context("This folder request has an invalid identity.")?;
    shep_mail_core::folder_actions::creation::valid_path(&name)?;
    if let Some(parent) = parent.as_deref() {
        shep_mail_core::folder_actions::creation::valid_path(parent)?;
    }
    let tx = db.transaction()?;
    crate::accounts::available(&tx, account)?;
    if let Some(saved) = get(&tx, id)? {
        anyhow::ensure!(
            saved.account == account
                && saved.connection == expected
                && saved.parent == parent
                && saved.name == name,
            "This request identity belongs to another folder. Reopen New folder."
        );
        tx.commit()?;
        return Ok(saved);
    }
    let settings = crate::operations::stored_account(&tx, account)?;
    changes::available(&tx, account)?;
    anyhow::ensure!(
        settings.protocol != shep_mail_core::model::Protocol::Pop3 || !name.contains('/'),
        "Use a single folder name and select its parent separately."
    );
    anyhow::ensure!(
        connection_key(&settings) == expected,
        "The account changed. Reopen New folder before continuing."
    );
    let count: i64 = tx.query_row("SELECT count(*) FROM folder_creations INDEXED BY folder_creation_active WHERE status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain')", [], |row| row.get(0))?;
    anyhow::ensure!(
        count < 32,
        "Folder requests are catching up. Review pending folders before adding another."
    );
    let duplicate: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM folder_creations INDEXED BY folder_creation_active WHERE account_id=?1 AND connection=?2 AND parent IS ?3 AND name=?4 AND status IN ('queued','waiting','planning','running','checking','repair','rejected','uncertain'))", params![account, expected, parent, name], |row| row.get(0))?;
    anyhow::ensure!(
        !duplicate,
        "This folder already has a pending request. Open Folder activity."
    );
    tx.execute("INSERT INTO folder_creations(id,account_id,connection,parent,name,status,created) VALUES(?1,?2,?3,?4,?5,'queued',?6)", params![id,account,expected,parent,name,chrono::Utc::now().timestamp_millis()])?;
    let saved = get(&tx, id)?.context("The folder request was not saved.")?;
    tx.commit()?;
    Ok(saved)
}

pub fn save(db: &Connection, before: &Creation, after: &Creation) -> Result<Creation> {
    anyhow::ensure!(
        (!before.acknowledged || after.acknowledged)
            && before
                .target
                .as_ref()
                .is_none_or(|target| after.target.as_ref() == Some(target))
            && before
                .receipt
                .as_ref()
                .is_none_or(|receipt| after.receipt.as_ref() == Some(receipt)),
        "A saved folder identity or acknowledgment cannot be replaced."
    );
    anyhow::ensure!(
        before.id == after.id
            && before.account == after.account
            && before.connection == after.connection
            && before.name == after.name
            && before.parent == after.parent,
        "The folder request identity changed."
    );
    let target = after
        .target
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let receipt = after
        .receipt
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    anyhow::ensure!(db.execute("UPDATE folder_creations SET status=?1,target=?2,receipt=?3,acknowledged=?4,error=?5,revision=revision+1,mutation=?8 WHERE id=?6 AND revision=?7", params![after.status,target,receipt,after.acknowledged,after.error,before.id,before.revision,after.mutation.as_ref().map(serde_json::to_string).transpose()?])? == 1,
        "This folder request changed. Refresh Folder activity.");
    get(db, &before.id)?.context("The folder request was removed.")
}

pub fn decide(db: &mut Connection, id: &str, revision: i64, decision: &str) -> Result<Creation> {
    let tx = db.transaction()?;
    let before = get(&tx, id)?.context("This folder request is no longer available.")?;
    if before.mutation.is_some() {
        let saved = changes::decide(&tx, &before, revision, decision)?;
        tx.commit()?;
        return Ok(saved);
    }
    anyhow::ensure!(
        before.revision == revision,
        "This folder request changed. Refresh Folder activity."
    );
    if matches!(decision, "retry" | "check") {
        checked_account(&tx, &before)?;
    }
    let mut after = before.clone();
    after.status = match decision {
        "cancel"
            if matches!(
                before.status.as_str(),
                "queued" | "waiting" | "planning" | "rejected"
            ) && !before.acknowledged =>
        {
            "cancelled"
        }
        "retry" if before.status == "rejected" && !before.acknowledged => "queued",
        "check" if matches!(before.status.as_str(), "uncertain" | "repair" | "running") => {
            "checking"
        }
        "dismiss"
            if matches!(
                before.status.as_str(),
                "uncertain" | "repair" | "rejected" | "succeeded" | "cancelled"
            ) =>
        {
            "dismissed"
        }
        _ => anyhow::bail!("This folder request cannot take that action. Refresh Folder activity."),
    }
    .into();
    after.error = None;
    let saved = save(&tx, &before, &after)?;
    tx.commit()?;
    Ok(saved)
}

pub fn recover(db: &Connection) -> Result<()> {
    db.execute("UPDATE folder_creations SET status=CASE WHEN mutation IS NOT NULL THEN CASE WHEN json_extract(mutation,'$.receipt') IS NOT NULL THEN 'repair' WHEN status='planning' THEN 'queued' ELSE 'uncertain' END WHEN acknowledged=1 OR receipt IS NOT NULL THEN 'repair' WHEN status='planning' THEN 'queued' ELSE 'uncertain' END,error='The app closed before this request finished. Check its saved state before continuing.',revision=revision+1 WHERE status IN ('planning','running','checking')", [])?;
    Ok(())
}

pub fn apply_receipt(db: &mut Connection, job: &Creation) -> Result<Creation> {
    let tx = db.transaction()?;
    checked_account(&tx, job)?;
    let mailbox = job
        .receipt
        .as_ref()
        .context("Check the created folder before updating the cache.")?;
    let mut names: Vec<String> = tx
        .query_row(
            "SELECT names FROM folders WHERE account_id=?1",
            [&job.account],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default();
    if mailbox.usable() && !names.contains(&mailbox.name) {
        names.push(mailbox.name.clone());
    }
    tx.execute("INSERT INTO folders(account_id,names) VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET names=excluded.names", params![job.account,serde_json::to_string(&names)?])?;
    let mut catalogue: Vec<Mailbox> = tx
        .query_row(
            "SELECT mailboxes FROM folder_catalogues WHERE account_id=?1",
            [&job.account],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|value| serde_json::from_str(&value))
        .transpose()?
        .unwrap_or_default();
    catalogue.retain(|value| value.name != mailbox.name);
    catalogue.push(mailbox.clone());
    tx.execute("INSERT INTO folder_catalogues(account_id,mailboxes) VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET mailboxes=excluded.mailboxes", params![job.account,serde_json::to_string(&catalogue)?])?;
    let mut after = job.clone();
    after.status = "succeeded".into();
    after.error = None;
    let saved = save(&tx, job, &after)?;
    tx.commit()?;
    Ok(saved)
}

pub fn save_catalogue(db: &Connection, account: &str, folders: &[Mailbox]) -> Result<()> {
    crate::accounts::available(db, account)?;
    let names: Vec<&str> = folders
        .iter()
        .filter(|folder| folder.usable())
        .map(|folder| folder.name.as_str())
        .collect();
    db.execute("INSERT INTO folders VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET names=excluded.names",params![account,serde_json::to_string(&names)?])?;
    db.execute("INSERT INTO folder_catalogues VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET mailboxes=excluded.mailboxes",params![account,serde_json::to_string(folders)?])?;
    Ok(())
}
