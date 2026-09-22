//! Device-local account admission and credential references. Secret bytes stay
//! in the credential service; activation commits settings and references together.
use super::*;
use rusqlite::OptionalExtension;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slots {
    pub incoming: String,
    pub smtp: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    Admitted,
    Staged,
    Checked,
    Activated,
    Failed,
    Interrupted,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub id: String,
    pub account: Account,
    pub previous: Option<Account>,
    pub slots: Slots,
    pub stage: Stage,
    pub error: Option<String>,
}

pub(super) fn schema(c: &Connection) -> anyhow::Result<()> {
    c.execute_batch(
        "CREATE TABLE IF NOT EXISTS account_setup_attempts(
        id TEXT PRIMARY KEY,account TEXT NOT NULL,data TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS account_setup_current(
        account TEXT PRIMARY KEY,attempt TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS account_credential_slots(
        account TEXT PRIMARY KEY,attempt TEXT NOT NULL,data TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS account_setup_stage ON account_setup_attempts(json_extract(data,'$.stage'));
        CREATE INDEX IF NOT EXISTS account_setup_account ON account_setup_attempts(account);",
    )?;
    let has_config = c
        .prepare("PRAGMA table_info(account_credential_slots)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == "config");
    if !has_config {
        c.execute(
            "ALTER TABLE account_credential_slots ADD COLUMN config TEXT",
            [],
        )?;
    }
    Ok(())
}

fn check_mailbox_identity(c: &Connection, account: &Account) -> anyhow::Result<()> {
    let previous = get::<Vec<Account>>(c, "accounts")?
        .into_iter()
        .find(|a| a.id == account.id);
    let Some(previous) = previous else {
        return Ok(());
    };
    if crate::mail_actions::connection_key(&previous)
        == crate::mail_actions::connection_key(account)
    {
        return Ok(());
    }
    let cached: bool = c.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE account=?)",
        [&account.id],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        !cached,
        "This account contains cached mail. Reconnect using its existing incoming settings, or add the changed mailbox as a new account. Moving cached mail to another connection requires a reviewed migration."
    );
    Ok(())
}

fn same_connection(a: &Account, b: &Account) -> bool {
    a.id == b.id
        && a.email == b.email
        && crate::mail_actions::connection_key(a) == crate::mail_actions::connection_key(b)
        && (
            a.smtp_host.as_str(),
            a.smtp_port,
            a.smtp_username.as_str(),
            a.smtp_security,
            a.smtp_auth,
            a.smtp_separate_password,
        ) == (
            b.smtp_host.as_str(),
            b.smtp_port,
            b.smtp_username.as_str(),
            b.smtp_security,
            b.smtp_auth,
            b.smtp_separate_password,
        )
}

fn checked_slots(c: &Connection, account: &Account) -> anyhow::Result<Option<Slots>> {
    let saved: Option<(String, Option<String>)> = c
        .query_row(
            "SELECT data,config FROM account_credential_slots WHERE account=?",
            [&account.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((data, config)) = saved else {
        return Ok(None);
    };
    let checked: Account = serde_json::from_str(
        &config.context("Reconnect this account to verify its credential binding.")?,
    )?;
    anyhow::ensure!(
        same_connection(&checked, account),
        "The account connection changed. Reconnect before using its saved credentials."
    );
    Ok(Some(serde_json::from_str(&data)?))
}

pub(crate) fn fence_import(c: &Connection, import_id: &str) -> anyhow::Result<()> {
    schema(c)?;
    c.execute("INSERT OR IGNORE INTO imported_operations SELECT ?,'account-setup',id,data FROM account_setup_attempts", [import_id])?;
    c.execute_batch("DELETE FROM account_setup_current; DELETE FROM account_setup_attempts; DELETE FROM account_credential_slots;")?;
    Ok(())
}

pub(super) fn remove(c: &Connection, account: &str) -> anyhow::Result<()> {
    let active: Option<String> = c
        .query_row(
            "SELECT data FROM account_credential_slots WHERE account=?",
            [account],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(active) = active {
        cleanup(c, account, &serde_json::from_str(&active)?)?;
    }
    let records = c
        .prepare("SELECT data FROM account_setup_attempts WHERE account=?")?
        .query_map([account], |r| r.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    for record in records {
        let attempt: Attempt = serde_json::from_str(&record)?;
        cleanup(c, account, &attempt.slots)?;
    }
    c.execute(
        "DELETE FROM account_setup_current WHERE account=?",
        [account],
    )?;
    c.execute(
        "DELETE FROM account_setup_attempts WHERE account=?",
        [account],
    )?;
    c.execute(
        "DELETE FROM account_credential_slots WHERE account=?",
        [account],
    )?;
    Ok(())
}

pub(super) fn in_use(c: &Connection, key: &str) -> anyhow::Result<bool> {
    Ok(c.query_row("SELECT EXISTS(SELECT 1 FROM account_credential_slots WHERE json_extract(data,'$.incoming')=?1 OR json_extract(data,'$.smtp')=?1)
        OR EXISTS(SELECT 1 FROM account_setup_attempts a JOIN account_setup_current p ON p.attempt=a.id
        WHERE json_extract(a.data,'$.stage') IN ('Admitted','Staged','Checked') AND (json_extract(a.data,'$.slots.incoming')=?1 OR json_extract(a.data,'$.slots.smtp')=?1))", [key], |r| r.get(0))?)
}

fn same(a: &Option<Account>, b: &Option<Account>) -> anyhow::Result<bool> {
    Ok(serde_json::to_value(a)? == serde_json::to_value(b)?)
}
fn load(c: &Connection, id: &str) -> anyhow::Result<Attempt> {
    let data: String = c.query_row(
        "SELECT data FROM account_setup_attempts WHERE id=?",
        [id],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&data)?)
}
fn save(c: &Connection, attempt: &Attempt) -> anyhow::Result<()> {
    c.execute(
        "INSERT INTO account_setup_attempts(id,account,data) VALUES(?,?,?)
        ON CONFLICT(id) DO UPDATE SET data=excluded.data",
        params![
            attempt.id,
            attempt.account.id,
            serde_json::to_string(attempt)?
        ],
    )?;
    Ok(())
}
fn current(c: &Connection, attempt: &Attempt) -> anyhow::Result<()> {
    connections::allow(c, ConnectionKind::Account, &attempt.account.id)?;
    let id: Option<String> = c
        .query_row(
            "SELECT attempt FROM account_setup_current WHERE account=?",
            [&attempt.account.id],
            |r| r.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        id.as_deref() == Some(&attempt.id),
        "A newer account setup replaced this request. Reopen Accounts."
    );
    let previous = get::<Vec<Account>>(c, "accounts")?
        .into_iter()
        .find(|a| a.id == attempt.account.id);
    anyhow::ensure!(
        same(&previous, &attempt.previous)?,
        "Account settings changed after this request. Reopen Accounts."
    );
    Ok(())
}
fn cleanup(c: &Connection, account: &str, slots: &Slots) -> anyhow::Result<()> {
    for key in std::iter::once(&slots.incoming).chain(slots.smtp.iter()) {
        c.execute(
            "INSERT OR IGNORE INTO credential_cleanup(kind,id,key) VALUES('account',?,?)",
            params![account, key],
        )?;
    }
    Ok(())
}

impl Store {
    pub async fn check_account_mailbox_identity(&self, account: Account) -> anyhow::Result<()> {
        self.run(move |c| check_mailbox_identity(c, &account)).await
    }

    pub async fn resolve_account_credential(&self, logical: String) -> anyhow::Result<String> {
        self.run(move |c| {
            let (id, smtp) = logical
                .strip_suffix(":smtp")
                .map_or((logical.as_str(), false), |id| (id, true));
            let account = get::<Vec<Account>>(c, "accounts")?
                .into_iter()
                .find(|a| a.id == id);
            let Some(account) = account else {
                return Ok(logical);
            };
            connections::allow(c, ConnectionKind::Account, id)?;
            let Some(slots) = checked_slots(c, &account)? else {
                return Ok(logical);
            };
            if smtp {
                slots
                    .smtp
                    .context("Separate SMTP credentials are unavailable. Reconnect this account.")
            } else {
                Ok(slots.incoming)
            }
        })
        .await
    }

    pub async fn interrupt_account_setup(&self, id: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let data: Option<String> = tx
                .query_row(
                    "SELECT data FROM account_setup_attempts WHERE id=?",
                    [&id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(data) = data else { return Ok(()) };
            let mut attempt: Attempt = serde_json::from_str(&data)?;
            if matches!(
                attempt.stage,
                Stage::Admitted | Stage::Staged | Stage::Checked
            ) {
                attempt.stage = Stage::Interrupted;
                attempt.error = Some(
                    "Connection checking was interrupted. Re-enter credentials to retry.".into(),
                );
                cleanup(&tx, &attempt.account.id, &attempt.slots)?;
                save(&tx, &attempt)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    /// Startup only, after the previous process and credential writer stopped.
    pub async fn interrupt_account_setups(&self) -> anyhow::Result<()> {
        self.run(|c| {
            let tx = c.transaction()?;
            let ids = tx.prepare("SELECT id FROM account_setup_attempts WHERE json_extract(data,'$.stage') IN ('Admitted','Staged','Checked')")?
                .query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
            for id in ids {
                let mut attempt = load(&tx, &id)?;
                attempt.stage = Stage::Interrupted;
                attempt.error = Some("Account setup was interrupted. Re-enter your credentials to retry.".into());
                cleanup(&tx, &attempt.account.id, &attempt.slots)?;
                save(&tx, &attempt)?;
            }
            tx.commit()?;
            Ok(())
        }).await
    }

    pub async fn admit_account_setup(
        &self,
        id: String,
        account: Account,
        previous: Option<Account>,
    ) -> anyhow::Result<Attempt> {
        uuid::Uuid::parse_str(&id).context("Invalid account setup identity")?;
        account.validate()?;
        crate::credentials::connection_id(&account.id, false)?;
        self.run(move |c| {
            let tx = c.transaction()?;
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM account_setup_attempts WHERE id=?)", [&id], |r| r.get(0))?;
            if exists {
                let saved = load(&tx, &id)?;
                anyhow::ensure!(same(&Some(saved.account.clone()), &Some(account))? && same(&saved.previous, &previous)?, "This setup identity belongs to another request.");
                return Ok(saved);
            }
            connections::allow(&tx, ConnectionKind::Account, &account.id)?;
            check_mailbox_identity(&tx, &account)?;
            let actual = get::<Vec<Account>>(&tx, "accounts")?.into_iter().find(|a| a.id == account.id);
            anyhow::ensure!(same(&actual, &previous)?, "Account settings changed. Reopen Accounts.");
            let count: i64 = tx.query_row("SELECT COUNT(*) FROM account_setup_attempts WHERE account != ? AND json_extract(data,'$.stage') IN ('Admitted','Staged','Checked')", [&account.id], |r| r.get(0))?;
            anyhow::ensure!(count < 128, "Review pending account setup before adding another request.");
            let old: Option<String> = tx.query_row("SELECT attempt FROM account_setup_current WHERE account=?", [&account.id], |r| r.get(0)).optional()?;
            if let Some(old) = old {
                let mut old = load(&tx, &old)?;
                if old.stage != Stage::Activated {
                    old.stage = Stage::Cancelled;
                    cleanup(&tx, &old.account.id, &old.slots)?;
                    save(&tx, &old)?;
                }
            }
            let slots = Slots {
                incoming: format!("{}:setup:{id}:incoming", account.id),
                smtp: (account.smtp_separate_password && account.smtp_auth != SmtpAuth::None)
                    .then(|| format!("{}:setup:{id}:smtp", account.id)),
            };
            let attempt = Attempt { id, account, previous, slots, stage: Stage::Admitted, error: None };
            save(&tx, &attempt)?;
            tx.execute("INSERT INTO account_setup_current(account,attempt) VALUES(?,?) ON CONFLICT(account) DO UPDATE SET attempt=excluded.attempt", params![attempt.account.id, attempt.id])?;
            tx.commit()?;
            Ok(attempt)
        }).await
    }

    pub async fn account_setup(&self, id: String) -> anyhow::Result<Attempt> {
        self.run(move |c| load(c, &id)).await
    }

    pub async fn validate_account_setup(&self, id: String) -> anyhow::Result<Attempt> {
        self.run(move |c| {
            let attempt = load(c, &id)?;
            current(c, &attempt)?;
            Ok(attempt)
        })
        .await
    }

    pub async fn account_setup_page(&self, after: Option<String>) -> anyhow::Result<Vec<Attempt>> {
        self.run(move |c| {
            c.prepare("SELECT data FROM account_setup_attempts WHERE (?1 IS NULL OR id>?1) ORDER BY id LIMIT 21")?
                .query_map([after], |r| r.get::<_, String>(0))?
                .map(|row| Ok(serde_json::from_str(&row?)?)).collect()
        }).await
    }

    pub async fn advance_account_setup(
        &self,
        id: String,
        expected: Stage,
        next: Stage,
    ) -> anyhow::Result<Attempt> {
        anyhow::ensure!(
            matches!(
                (expected, next),
                (Stage::Admitted, Stage::Staged) | (Stage::Staged, Stage::Checked)
            ),
            "Invalid account setup transition"
        );
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut attempt = load(&tx, &id)?;
            current(&tx, &attempt)?;
            anyhow::ensure!(
                attempt.stage == expected,
                "Account setup progress changed. Reopen Accounts."
            );
            attempt.stage = next;
            save(&tx, &attempt)?;
            tx.commit()?;
            Ok(attempt)
        })
        .await
    }

    pub async fn fail_account_setup(&self, id: String, error: String) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let present: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM account_setup_attempts WHERE id=?1)",
                [&id],
                |row| row.get(0),
            )?;
            if !present {
                return Ok(());
            }
            let mut attempt = load(&tx, &id)?;
            if matches!(
                attempt.stage,
                Stage::Activated | Stage::Cancelled | Stage::Interrupted
            ) {
                return Ok(());
            }
            attempt.stage = Stage::Failed;
            attempt.error = Some(error);
            cleanup(&tx, &attempt.account.id, &attempt.slots)?;
            save(&tx, &attempt)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub async fn activate_account_setup(&self, id: String) -> anyhow::Result<Attempt> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let attempt = activate(&tx, &id)?;
            tx.commit()?;
            Ok(attempt)
        })
        .await
    }

    pub async fn account_credential_key(
        &self,
        account: Account,
        smtp: bool,
    ) -> anyhow::Result<String> {
        self.run(move |c| {
            connections::allow(c, ConnectionKind::Account, &account.id)?;
            let actual = get::<Vec<Account>>(c, "accounts")?
                .into_iter()
                .find(|a| a.id == account.id)
                .context("This account was removed. Refresh Accounts.")?;
            anyhow::ensure!(
                if smtp {
                    same_connection(&actual, &account)
                } else {
                    crate::mail_actions::connection_key(&actual)
                        == crate::mail_actions::connection_key(&account)
                },
                "Account settings changed before connecting. Refresh and retry."
            );
            if let Some(slots) = checked_slots(c, &actual)? {
                return if smtp && account.smtp_separate_password {
                    slots.smtp.context(
                        "Separate SMTP credentials are unavailable. Reconnect this account.",
                    )
                } else {
                    Ok(slots.incoming)
                };
            }
            Ok(if smtp && account.smtp_separate_password {
                format!("{}:smtp", account.id)
            } else {
                account.id
            })
        })
        .await
    }
}

impl Store {
    /// Accounts ready to sync, each with its incoming connection identity:
    /// the server settings plus the active credential binding. A long-lived
    /// connection opened with other values is stale.
    pub(crate) async fn accounts_ready_to_watch(&self) -> anyhow::Result<Vec<(Account, String)>> {
        self.run(|c| {
            super::profile_sync::join::ready_to_sync(c)?
                .into_iter()
                .map(|account| {
                    let slot: Option<String> = c
                        .query_row(
                            "SELECT json_extract(data,'$.incoming') FROM account_credential_slots WHERE account=?",
                            [&account.id],
                            |r| r.get(0),
                        )
                        .optional()?
                        .flatten();
                    let identity = format!(
                        "{}\n{}",
                        crate::mail_actions::connection_key(&account),
                        slot.unwrap_or_default()
                    );
                    Ok((account, identity))
                })
                .collect()
        })
        .await
    }
}

pub(crate) fn activate(c: &Connection, id: &str) -> anyhow::Result<Attempt> {
    let mut attempt = load(c, id)?;
    if attempt.stage == Stage::Activated {
        connections::allow(c, ConnectionKind::Account, &attempt.account.id)?;
        let owns: bool = c.query_row(
            "SELECT EXISTS(SELECT 1 FROM account_credential_slots s JOIN account_setup_current q ON q.account=s.account WHERE s.account=? AND s.attempt=? AND q.attempt=?)",
            params![attempt.account.id, id, id], |row| row.get(0),
        )?;
        let actual = get::<Vec<Account>>(c, "accounts")?
            .into_iter()
            .find(|a| a.id == attempt.account.id);
        anyhow::ensure!(
            owns && same(&actual, &Some(attempt.account.clone()))?,
            "A newer account setup replaced this activation. Refresh Accounts."
        );
        return Ok(attempt);
    }
    current(c, &attempt)?;
    anyhow::ensure!(
        attempt.stage == Stage::Checked,
        "Verify both connection credentials before activation."
    );
    folder_actions::idle(c, &attempt.account.id)?;
    let old: Option<String> = c
        .query_row(
            "SELECT data FROM account_credential_slots WHERE account=?",
            [&attempt.account.id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(old) = old {
        cleanup(c, &attempt.account.id, &serde_json::from_str(&old)?)?;
    } else if attempt.previous.is_some() {
        cleanup(
            c,
            &attempt.account.id,
            &Slots {
                incoming: attempt.account.id.clone(),
                smtp: Some(format!("{}:smtp", attempt.account.id)),
            },
        )?;
    }
    save_account_fields(c, &attempt.account)?;
    c.execute("INSERT INTO account_credential_slots(account,attempt,data,config) VALUES(?,?,?,?) ON CONFLICT(account) DO UPDATE SET attempt=excluded.attempt,data=excluded.data,config=excluded.config", params![attempt.account.id, attempt.id, serde_json::to_string(&attempt.slots)?, serde_json::to_string(&attempt.account)?])?;
    attempt.stage = Stage::Activated;
    attempt.error = None;
    save(c, &attempt)?;
    Ok(attempt)
}

pub(super) fn save_account_fields(c: &Connection, account: &Account) -> anyhow::Result<()> {
    let mut accounts: Vec<Account> = get(c, "accounts")?;
    let previous = accounts.iter().find(|a| a.id == account.id).cloned();
    accounts.retain(|a| a.id != account.id);
    if !account.sent_folder.is_empty() {
        c.execute("INSERT INTO sent_folders(account,folder) VALUES(?,?) ON CONFLICT(account) DO UPDATE SET folder=excluded.folder", params![account.id, account.sent_folder])?;
    } else {
        c.execute("DELETE FROM sent_folders WHERE account=?", [&account.id])?;
    }
    profile_sync::join::reconnected(c, &account.id)?;
    accounts.push(account.clone());
    put(c, "accounts", &accounts)?;
    connections::changed(c)?;
    profile_sync::state::record_native_account_fields(c, account, previous.as_ref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn full_setup_capacity_allows_replacing_its_own_pending_request() {
        let store = Store::memory().expect("store");
        let prototype: Account = serde_json::from_value(serde_json::json!({"id":"fixture","name":"Fixture","email":"fixture@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"fixture","smtp_host":"smtp.example.test","smtp_port":465})).expect("account");
        let mut first = None;
        for index in 0..128 {
            let mut account = prototype.clone();
            account.id = format!("fixture-{index}");
            let saved = store
                .admit_account_setup(uuid::Uuid::new_v4().to_string(), account, None)
                .await
                .expect("capacity");
            if index == 0 {
                first = Some(saved);
            }
        }
        assert!(
            store
                .admit_account_setup(uuid::Uuid::new_v4().to_string(), prototype, None)
                .await
                .is_err()
        );
        let first = first.expect("first request");
        let retry = store
            .admit_account_setup(
                uuid::Uuid::new_v4().to_string(),
                first.account.clone(),
                None,
            )
            .await
            .expect("explicit replacement remains available");
        assert_eq!(retry.stage, Stage::Admitted);
        assert_eq!(
            store
                .account_setup(first.id)
                .await
                .expect("old attempt")
                .stage,
            Stage::Cancelled
        );
    }

    #[tokio::test]
    async fn imported_slots_are_archived_without_binding_the_new_device() {
        let dir = tempfile::tempdir().expect("directory");
        let store = Store::open(dir.path().join("cache.sqlite")).expect("store");
        let account: Account = serde_json::from_value(serde_json::json!({"id":"fixture","name":"Fixture","email":"fixture@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"fixture","smtp_host":"smtp.example.test","smtp_port":465})).expect("account");
        let attempt = store
            .admit_account_setup(uuid::Uuid::new_v4().to_string(), account.clone(), None)
            .await
            .expect("admit");
        store
            .advance_account_setup(attempt.id.clone(), Stage::Admitted, Stage::Staged)
            .await
            .expect("staged");
        store
            .advance_account_setup(attempt.id.clone(), Stage::Staged, Stage::Checked)
            .await
            .expect("checked");
        store
            .activate_account_setup(attempt.id.clone())
            .await
            .expect("activate");
        store
            .run(|c| {
                let tx = c.transaction()?;
                super::super::import_archive_schema(&tx)?;
                fence_import(&tx, "new-device")?;
                let archived: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM imported_operations WHERE kind='account-setup'",
                    [],
                    |r| r.get(0),
                )?;
                anyhow::ensure!(
                    archived == 1,
                    "Preserve the original operation for inspection"
                );
                tx.commit()?;
                Ok(())
            })
            .await
            .expect("fence");
        assert!(store.account_setup(attempt.id).await.is_err());
        assert_eq!(
            store
                .account_credential_key(account, false)
                .await
                .expect("new device legacy namespace"),
            "fixture"
        );
        assert!(
            !store
                .credential_in_use(attempt.slots.incoming)
                .await
                .expect("foreign slot not bound")
        );
    }

    async fn activate_setup(store: &Store, account: Account, previous: Option<Account>) {
        let attempt = store
            .admit_account_setup(uuid::Uuid::new_v4().to_string(), account, previous)
            .await
            .expect("admit");
        for (from, to) in [
            (Stage::Admitted, Stage::Staged),
            (Stage::Staged, Stage::Checked),
        ] {
            store
                .advance_account_setup(attempt.id.clone(), from, to)
                .await
                .expect("advance");
        }
        store
            .activate_account_setup(attempt.id)
            .await
            .expect("activate");
    }

    #[tokio::test]
    async fn watch_identity_follows_incoming_settings_and_credential_binding() {
        let store = Store::memory().expect("store");
        let identity = || async {
            let targets = store.accounts_ready_to_watch().await.expect("targets");
            assert_eq!(targets.len(), 1);
            targets[0].1.clone()
        };
        let account: Account = serde_json::from_value(serde_json::json!({"id":"fixture","name":"Fixture","email":"fixture@example.test","protocol":"Imap","host":"imap.example.test","port":993,"username":"fixture","smtp_host":"smtp.example.test","smtp_port":465})).expect("account");
        activate_setup(&store, account.clone(), None).await;
        let first = identity().await;
        assert_eq!(identity().await, first, "stable while nothing changes");

        let mut renamed = account.clone();
        renamed.name = "Renamed".into();
        renamed.smtp_port = 587;
        store.save_account(renamed.clone()).await.expect("rename");
        assert_eq!(
            identity().await,
            first,
            "names and outgoing settings do not affect the watcher"
        );

        activate_setup(&store, renamed.clone(), Some(renamed.clone())).await;
        let rebound = identity().await;
        assert_ne!(rebound, first, "a new credential slot restarts the watcher");

        let mut moved = renamed.clone();
        moved.port = 143;
        moved.incoming_security = ConnectionSecurity::StartTls;
        activate_setup(&store, moved, Some(renamed)).await;
        assert_ne!(
            identity().await,
            rebound,
            "new incoming settings restart it"
        );
    }
}
