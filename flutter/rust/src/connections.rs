//! A database pointer activates an independently saved device credential pair.
//! Only opaque slot identifiers and account configuration enter SQLite.
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use shep_mail_core::model::Account;

fn snapshot(db: &Connection, id: &str) -> Result<String> {
    let account: Option<String> = db
        .query_row("SELECT settings FROM accounts WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .optional()?;
    let slot: Option<String> = db
        .query_row(
            "SELECT slot FROM account_credentials WHERE account_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(serde_json::to_string(&(account, slot))?)
}
fn without_sent(mut account: Account) -> Account {
    account.smtp_security = Some(account.smtp_security());
    account.sent_copy = Default::default();
    account.sent_folder.clear();
    account
}
pub(crate) fn prepare(
    db: &mut Connection,
    mut account: Account,
    expected: Option<Account>,
) -> Result<Value> {
    account.validate()?;
    anyhow::ensure!(
        !account.id.is_empty() && account.id.len() <= 128,
        "Give the account a valid identity."
    );
    let tx = db.transaction()?;
    crate::accounts::available(&tx, &account.id)?;
    let prior: Option<String> = tx
        .query_row(
            "SELECT settings FROM accounts WHERE id=?1",
            [&account.id],
            |r| r.get(0),
        )
        .optional()?;
    match (prior, expected) {
        (None, None) => {}
        (Some(prior), Some(expected)) => {
            let prior: Account = serde_json::from_str(&prior)?;
            anyhow::ensure!(
                without_sent(prior.clone()) == without_sent(expected),
                "This account changed. Reopen its settings before reconnecting."
            );
            // Reconnect changes credentials only. Connection editing will need
            // its own reviewed cache-identity migration before lifting this rule.
            anyhow::ensure!(
                without_sent(prior.clone()) == without_sent(account.clone()),
                "Reconnect keeps the saved connection settings. Reopen Preferences or add a new account."
            );
            account.sent_copy = prior.sent_copy;
            account.sent_folder = prior.sent_folder;
        }
        _ => anyhow::bail!("This account changed. Reopen Preferences before connecting."),
    }
    let collision: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM credential_slots WHERE slot=?1 AND account_id!=?1)",
        [&account.id],
        |r| r.get(0),
    )?;
    anyhow::ensure!(!collision, "Choose a new account identity.");
    let slot = format!("credential-{}", uuid::Uuid::new_v4());
    let expected = snapshot(&tx, &account.id)?;
    tx.execute("INSERT INTO credential_slots(slot,account_id,settings,expected,state) VALUES(?1,?2,?3,?4,'prepared')", params![slot, account.id, serde_json::to_string(&account)?, expected])?;
    tx.commit()?;
    Ok(json!({"slot":slot,"account":account}))
}
pub(crate) fn owner(db: &Connection, slot: &str) -> Result<String> {
    db.query_row(
        "SELECT account_id FROM credential_slots WHERE slot=?1",
        [slot],
        |r| r.get(0),
    )
    .optional()?
    .context("This connection attempt is no longer available. Reconnect in Preferences.")
}
pub(crate) fn activate(db: &mut Connection, slot: &str) -> Result<()> {
    let tx = db.transaction()?;
    let id = owner(&tx, slot)?;
    crate::accounts::available(&tx, &id)?;
    let active: Option<String> = tx
        .query_row(
            "SELECT slot FROM account_credentials WHERE account_id=?1",
            [&id],
            |r| r.get(0),
        )
        .optional()?;
    if active.as_deref() == Some(slot) {
        return Ok(());
    }
    let (settings, expected): (String, String) = tx
        .query_row(
            "SELECT settings,expected FROM credential_slots WHERE slot=?1 AND state='prepared'",
            [slot],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .context("This connection attempt was replaced. Reconnect in Preferences.")?;
    anyhow::ensure!(
        snapshot(&tx, &id)? == expected,
        "The connection changed while passwords were being saved. Reopen Preferences and reconnect; the previous connection is preserved."
    );
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id=?1)",
        [&id],
        |r| r.get(0),
    )?;
    if let Some(old) = active {
        tx.execute(
            "UPDATE credential_slots SET state='cleanup' WHERE slot=?1",
            [old],
        )?;
    } else if exists {
        // The migrated profile's pair still lives under its original account ID.
        tx.execute(
            "INSERT INTO credential_slots(slot,account_id,state) VALUES(?1,?1,'cleanup')",
            [&id],
        )?;
    }
    tx.execute("INSERT INTO accounts(id,settings) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET settings=excluded.settings", params![id, settings])?;
    tx.execute("INSERT INTO account_credentials(account_id,slot) VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET slot=excluded.slot", params![id, slot])?;
    tx.execute(
        "UPDATE credential_slots SET state='active',settings=NULL,expected=NULL WHERE slot=?1",
        [slot],
    )?;
    tx.commit().context("Could not activate the saved connection. The previous connection is preserved; retry reconnecting.")?;
    Ok(())
}
pub(crate) fn target(db: &Connection, account: Account) -> Result<String> {
    let current = crate::operations::stored_account(db, &account.id)?;
    anyhow::ensure!(
        without_sent(current) == without_sent(account.clone()),
        "This account changed. Reopen Preferences and refresh before retrying."
    );
    Ok(db
        .query_row(
            "SELECT slot FROM account_credentials WHERE account_id=?1",
            [&account.id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(account.id))
}
/// Call only while holding the account operation lock, before provider work.
pub(crate) fn check_binding(db: &Connection, id: &str, slot: Option<&str>) -> Result<()> {
    crate::accounts::available(db, id)?;
    let current: Option<String> = db
        .query_row(
            "SELECT slot FROM account_credentials WHERE account_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    anyhow::ensure!(
        match current {
            Some(current) => slot == Some(current.as_str()),
            None => slot.is_none_or(|s| s == id),
        },
        "The account credentials changed. Refresh or reopen the action before retrying; no provider operation was started."
    );
    Ok(())
}
pub(crate) fn cleanup(db: &Connection) -> Result<Vec<String>> {
    let mut q = db.prepare("WITH candidates(slot) AS (SELECT slot FROM credential_slots WHERE state!='active' UNION SELECT id FROM removed_accounts WHERE cleanup=1) SELECT slot FROM candidates c WHERE NOT EXISTS(SELECT 1 FROM account_credentials a WHERE a.slot=c.slot) AND NOT EXISTS(SELECT 1 FROM accounts a WHERE a.id=c.slot AND NOT EXISTS(SELECT 1 FROM account_credentials b WHERE b.account_id=a.id)) ORDER BY slot")?;
    Ok(q.query_map([], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}
pub(crate) fn cleanup_done(db: &Connection, slot: &str) -> Result<()> {
    let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM account_credentials WHERE slot=?1) OR EXISTS(SELECT 1 FROM accounts a WHERE a.id=?1 AND NOT EXISTS(SELECT 1 FROM account_credentials c WHERE c.account_id=a.id))", [slot], |r| r.get(0))?;
    anyhow::ensure!(
        !active,
        "This account is still connected; its credential cleanup was refused."
    );
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "DELETE FROM credential_slots WHERE slot=?1 AND state!='active'",
        [slot],
    )?;
    tx.execute("UPDATE removed_accounts SET cleanup=0 WHERE id=?1", [slot])?;
    tx.commit()?;
    Ok(())
}
