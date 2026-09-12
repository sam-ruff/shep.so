use super::*;
use crate::folders::Mailbox;
use rusqlite::OptionalExtension;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PendingCreation {
    pub account: String,
    pub connection: String,
    pub parent: Option<String>,
    pub name: String,
}

pub(super) fn pending(c: &Connection) -> anyhow::Result<Vec<PendingCreation>> {
    let mut pending = Vec::new();
    let mut statement = c.prepare("SELECT request FROM folder_creations WHERE account=? AND connection=? ORDER BY rowid DESC LIMIT 32")?;
    for account in get::<Vec<Account>>(c, "accounts")? {
        let connection = crate::mail_actions::connection_key(&account);
        for request in statement.query_map(params![account.id, connection], |row| {
            row.get::<_, String>(0)
        })? {
            let (parent, name) = serde_json::from_str(&request?)?;
            pending.push(PendingCreation {
                account: account.id.clone(),
                connection: connection.clone(),
                parent,
                name,
            });
        }
    }
    Ok(pending)
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS folder_creations(
        account TEXT NOT NULL, connection TEXT NOT NULL, request TEXT NOT NULL,
        target TEXT NOT NULL, PRIMARY KEY(account,connection,request));",
    )?;
    Ok(())
}

fn checked_account(c: &Connection, id: &str, expected: &str) -> anyhow::Result<Account> {
    connections::allow(c, ConnectionKind::Account, id)?;
    folder_actions::idle(c, id)?;
    let account = get::<Vec<Account>>(c, "accounts")?
        .into_iter()
        .find(|account| account.id == id)
        .context("This account was removed.")?;
    anyhow::ensure!(
        crate::mail_actions::connection_key(&account) == expected,
        "This account's connection changed. Open New folder again."
    );
    Ok(account)
}

fn save_catalog(c: &Connection, account: &str, catalog: Vec<Mailbox>) -> anyhow::Result<()> {
    let mut names: HashMap<String, Vec<String>> = get(c, "account_folders")?;
    let folders = catalog
        .iter()
        .filter(|mailbox| mailbox.selectable && !mailbox.non_existent)
        .map(|mailbox| mailbox.name.clone())
        .collect::<Vec<_>>();
    let sent: Option<String> = c
        .query_row(
            "SELECT folder FROM sent_folders WHERE account=?",
            [account],
            |row| row.get(0),
        )
        .optional()?;
    if sent.is_some_and(|folder| !folders.contains(&folder)) {
        c.execute("DELETE FROM sent_folders WHERE account=?", [account])?;
    }
    names.insert(account.into(), folders);
    let mut catalogs: HashMap<String, Vec<Mailbox>> = get(c, "folder_catalogs")?;
    catalogs.insert(account.into(), catalog);
    put(c, "account_folders", &names)?;
    put(c, "folder_catalogs", &catalogs)?;
    connections::changed(c)?;
    Ok(())
}

impl Store {
    pub(crate) async fn folder_creation_target(
        &self,
        account: String,
        expected: String,
        request: String,
        proposed: Option<Mailbox>,
    ) -> anyhow::Result<Option<Mailbox>> {
        self.run(move |c| {
            let tx = c.transaction()?;
            checked_account(&tx, &account, &expected)?;
            let saved: Option<String> = tx.query_row(
                "SELECT target FROM folder_creations WHERE account=? AND connection=? AND request=?",
                params![account, expected, request], |row| row.get(0)).optional()?;
            if let Some(saved) = saved {
                return Ok(Some(serde_json::from_str(&saved)?));
            }
            if let Some(target) = &proposed {
                crate::folder_actions::creation::valid_path(&target.name)?;
                let count: i64 = tx.query_row("SELECT COUNT(*) FROM folder_creations WHERE account=?", [&account], |row| row.get(0))?;
                anyhow::ensure!(count < CHANNEL_CAPACITY as i64, "Finish the pending folder creations before adding another.");
                tx.execute("INSERT INTO folder_creations(account,connection,request,target) VALUES(?,?,?,?)",
                    params![account, expected, request, serde_json::to_string(target)?])?;
            }
            tx.commit()?;
            Ok(proposed)
        }).await
    }

    pub(crate) async fn save_created_folder_catalog(
        &self,
        account: String,
        expected: String,
        catalog: Vec<Mailbox>,
        created: Mailbox,
        request: String,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            checked_account(&tx, &account, &expected)?;
            let frozen: String = tx.query_row("SELECT target FROM folder_creations WHERE account=? AND connection=? AND request=?", params![account, expected, request], |row| row.get(0)).optional()?.context("The saved folder request is no longer available.")?;
            let frozen: Mailbox = serde_json::from_str(&frozen)?;
            anyhow::ensure!(frozen.name == created.name, "The confirmed folder does not match the saved destination.");
            anyhow::ensure!(
                catalog.iter().any(|mailbox| mailbox.name == created.name
                    && mailbox.selectable
                    && !mailbox.non_existent),
                "The server has not confirmed the new folder yet. Try again."
            );
            save_catalog(&tx, &account, catalog)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn finish_folder_creation(
        &self,
        account: String,
        expected: String,
        request: String,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            c.execute(
                "DELETE FROM folder_creations WHERE account=? AND connection=? AND request=?",
                params![account, expected, request],
            )?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn create_local_folder(
        &self,
        account: String,
        expected: String,
        parent: Option<String>,
        name: String,
    ) -> anyhow::Result<Mailbox> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let config = checked_account(&tx, &account, &expected)?;
            anyhow::ensure!(
                config.protocol == Protocol::Pop3,
                "Local folders are only available for POP3 accounts."
            );
            let catalogs: HashMap<String, Vec<Mailbox>> = get(&tx, "folder_catalogs")?;
            let mut catalog = catalogs.get(&account).cloned().unwrap_or_default();
            let root = Mailbox {
                delimiter: Some('/'),
                encoding: catalog
                    .first()
                    .map(|mailbox| mailbox.encoding)
                    .unwrap_or_default(),
                ..Mailbox::flat(String::new())
            };
            let parent = parent
                .as_ref()
                .map(|path| {
                    catalog
                        .iter()
                        .find(|mailbox| &mailbox.name == path)
                        .context("The parent folder is no longer available.")
                })
                .transpose()?;
            let created = crate::folder_actions::creation::plan(&root, parent, &name)?;
            if let Some(existing) = catalog.iter().find(|mailbox| mailbox.name == created.name) {
                anyhow::ensure!(
                    existing.selectable && !existing.non_existent,
                    "This name belongs to an unavailable folder."
                );
                return Ok(existing.clone());
            }
            catalog.push(created.clone());
            save_catalog(&tx, &account, catalog)?;
            tx.commit()?;
            Ok(created)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn fixture() -> (Store, Account, Vec<Mailbox>) {
        let store = Store::memory().expect("fixture store");
        let account: Account = serde_json::from_value(serde_json::json!({
            "id":"folders","name":"Local folders","email":"folders@example.test","protocol":"Pop3",
            "host":"localhost","port":995,"username":"folders","smtp_host":"localhost","smtp_port":465
        })).expect("fixture account");
        store
            .save_account(account.clone())
            .await
            .expect("save account");
        let catalog = ["INBOX", "Projects"]
            .into_iter()
            .map(|name| Mailbox {
                delimiter: Some('/'),
                ..Mailbox::flat(name.into())
            })
            .collect::<Vec<_>>();
        store
            .save_folder_catalog(account.id.clone(), catalog.clone())
            .await
            .expect("save catalog");
        (store, account, catalog)
    }

    #[tokio::test]
    async fn local_creation_persists_parent_and_retry_does_not_duplicate() {
        let (store, account, _) = fixture().await;
        let connection = crate::mail_actions::connection_key(&account);
        let first = store
            .create_local_folder(
                account.id.clone(),
                connection.clone(),
                Some("Projects".into()),
                "Receipts".into(),
            )
            .await
            .expect("create child");
        assert_eq!(first.name, "Projects/Receipts");
        assert_eq!(
            store
                .create_local_folder(
                    account.id.clone(),
                    connection,
                    Some("Projects".into()),
                    "Receipts".into()
                )
                .await
                .expect("retry child"),
            first
        );
        let workspace = store.workspace().await.expect("read saved workspace");
        assert_eq!(
            workspace.account_folders[&account.id]
                .iter()
                .filter(|name| *name == "Projects/Receipts")
                .count(),
            1
        );
        assert!(
            workspace.folder_trees[&account.id]
                .node("Projects/Receipts")
                .is_some()
        );
    }

    #[tokio::test]
    async fn changed_connection_or_missing_account_cannot_publish_a_late_catalog() {
        let (store, account, catalog) = fixture().await;
        let connection = crate::mail_actions::connection_key(&account);
        let created = Mailbox::flat("Receipts".into());
        let mut updated = catalog.clone();
        updated.push(created.clone());
        let mut changed = account.clone();
        changed.host = "changed.example.test".into();
        store
            .save_account(changed)
            .await
            .expect("change connection");
        assert!(
            store
                .save_created_folder_catalog(
                    account.id.clone(),
                    connection.clone(),
                    updated.clone(),
                    created.clone(),
                    "request".into()
                )
                .await
                .is_err()
        );
        assert_eq!(
            store
                .current_folder_catalog(account.id.clone())
                .await
                .expect("retained catalog"),
            catalog
        );
        store
            .put("accounts", Vec::<Account>::new())
            .await
            .expect("remove fixture metadata");
        assert!(
            store
                .save_created_folder_catalog(
                    account.id,
                    connection,
                    updated,
                    created,
                    "request".into()
                )
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unconfirmed_creation_and_forbidden_parent_leave_catalog_unchanged() {
        let (store, account, catalog) = fixture().await;
        let connection = crate::mail_actions::connection_key(&account);
        assert!(
            store
                .save_created_folder_catalog(
                    account.id.clone(),
                    connection.clone(),
                    catalog.clone(),
                    Mailbox::flat("Missing".into()),
                    "request".into()
                )
                .await
                .is_err()
        );
        assert!(
            store
                .create_local_folder(
                    account.id.clone(),
                    connection.clone(),
                    Some("Missing".into()),
                    "Child".into()
                )
                .await
                .is_err()
        );
        assert!(
            store
                .create_local_folder(account.id.clone(), connection, None, "Nested/Leaf".into())
                .await
                .is_err()
        );
        assert_eq!(
            store
                .current_folder_catalog(account.id)
                .await
                .expect("unchanged catalog"),
            catalog
        );
    }

    #[tokio::test]
    async fn saved_target_survives_restart_namespace_change_and_failed_catalog_confirmation() {
        let (_, account, catalog) = fixture().await;
        let directory = tempfile::tempdir().expect("fixture directory");
        let path = directory.path().join("cache.sqlite");
        let connection = crate::mail_actions::connection_key(&account);
        let request = "[null,\"Receipts\"]".to_owned();
        let original = Mailbox {
            delimiter: Some('.'),
            ..Mailbox::flat("INBOX.Receipts".into())
        };
        {
            let store = Store::open(&path).expect("open fixture");
            store
                .save_account(account.clone())
                .await
                .expect("save account");
            store
                .save_folder_catalog(account.id.clone(), catalog.clone())
                .await
                .expect("save catalog");
            assert_eq!(
                store
                    .folder_creation_target(
                        account.id.clone(),
                        connection.clone(),
                        request.clone(),
                        Some(original.clone())
                    )
                    .await
                    .expect("reserve target"),
                Some(original.clone())
            );
        }
        let store = Store::open(&path).expect("reopen fixture");
        let workspace = store.workspace().await.expect("restart workspace");
        assert_eq!(workspace.folder_creations.len(), 1);
        assert_eq!(workspace.folder_creations[0].name, "Receipts");
        assert_eq!(workspace.folder_creations[0].parent, None);
        let redirected = Mailbox::flat("Changed/Receipts".into());
        assert_eq!(
            store
                .folder_creation_target(
                    account.id.clone(),
                    connection.clone(),
                    request.clone(),
                    Some(redirected)
                )
                .await
                .expect("retain exact destination"),
            Some(original.clone())
        );
        assert!(
            store
                .save_created_folder_catalog(
                    account.id.clone(),
                    connection.clone(),
                    catalog.clone(),
                    original.clone(),
                    request.clone()
                )
                .await
                .is_err()
        );
        assert_eq!(
            store
                .folder_creation_target(
                    account.id.clone(),
                    connection.clone(),
                    request.clone(),
                    None
                )
                .await
                .expect("retain after incomplete list"),
            Some(original.clone())
        );
        let mut confirmed = catalog;
        confirmed.push(original.clone());
        store
            .save_created_folder_catalog(
                account.id.clone(),
                connection.clone(),
                confirmed,
                original.clone(),
                request.clone(),
            )
            .await
            .expect("confirmed list");
        assert_eq!(
            store
                .folder_creation_target(
                    account.id.clone(),
                    connection.clone(),
                    request.clone(),
                    None
                )
                .await
                .expect("retain until workspace publication"),
            Some(original)
        );
        store
            .finish_folder_creation(account.id.clone(), connection.clone(), request.clone())
            .await
            .expect("published workspace");
        assert_eq!(
            store
                .folder_creation_target(account.id, connection, request, None)
                .await
                .expect("completed request removed"),
            None
        );
    }
}
