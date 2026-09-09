use super::*;
use anyhow::ensure;
use rusqlite::OptionalExtension;

#[derive(Clone, Debug)]
pub(crate) struct ReconnectReview {
    account: Account,
    generation: String,
}
pub(super) fn schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS profile_reconnect(account_id TEXT PRIMARY KEY, generation TEXT NOT NULL, reason TEXT NOT NULL);")?;
    Ok(())
}
pub(crate) fn required(db: &Connection, id: &str) -> anyhow::Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM profile_reconnect WHERE account_id=?)",
        [id],
        |r| r.get(0),
    )?)
}
pub(crate) fn mark(db: &Connection, id: &str) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO profile_reconnect(account_id,generation,reason) VALUES(?,?,?)",
        params![
            id,
            uuid::Uuid::new_v4().to_string(),
            "Imported profile account needs passwords on this device."
        ],
    )?;
    Ok(())
}
impl Store {
    pub(crate) async fn profile_reconnect_required(&self, id: String) -> anyhow::Result<bool> {
        self.run(move |db| required(db, &id)).await
    }
    pub(crate) async fn require_profile_active(&self, id: String) -> anyhow::Result<()> {
        self.run(move|db| {
            connections::allow(db,ConnectionKind::Account,&id)?;
            ensure!(!required(db,&id)?,"Reconnect this account in Preferences before using the mail server. Its passwords have not been approved on this device.");
            Ok(())
        }).await
    }
    pub(crate) async fn profile_reconnect_review(
        &self,
        id: String,
    ) -> anyhow::Result<ReconnectReview> {
        self.run(move |db| {
            connections::allow(db, ConnectionKind::Account, &id)?;
            let generation: String = db
                .query_row(
                    "SELECT generation FROM profile_reconnect WHERE account_id=?",
                    [&id],
                    |r| r.get(0),
                )
                .optional()?
                .context("This account is no longer waiting for reconnect. Reopen its settings.")?;
            let account = get::<Vec<Account>>(db, "accounts")?
                .into_iter()
                .find(|a| a.id == id)
                .context("This account was removed. Reopen Preferences.")?;
            Ok(ReconnectReview {
                account,
                generation,
            })
        })
        .await
    }
    pub(crate) async fn activate_profile_account(
        &self,
        review: ReconnectReview,
        account: Account,
    ) -> anyhow::Result<()> {
        self.run(move|db| {
            let tx=db.transaction()?;
            connections::allow(&tx,ConnectionKind::Account,&account.id)?;
            ensure!(account.id==review.account.id,"The reconnect account changed. Reopen its settings.");
            let mut accounts:Vec<Account>=get(&tx,"accounts")?;
            let existing=accounts.iter_mut().find(|a|a.id==account.id).context("The account was removed. Its reconnect remains unconfirmed.")?;
            ensure!(existing==&review.account,"Account settings changed during reconnect. Review them and enter the passwords again.");
            let generation: Option<String>=tx.query_row("SELECT generation FROM profile_reconnect WHERE account_id=?",[&account.id],|r|r.get(0)).optional()?;
            ensure!(generation.as_deref()==Some(&review.generation),"The reconnect request changed. Review the account before continuing.");
            *existing=account.clone();
            put(&tx,"accounts",&accounts)?;
            if account.sent_folder.is_empty() {tx.execute("DELETE FROM sent_folders WHERE account=?",[&account.id])?;}
            else {tx.execute("INSERT INTO sent_folders(account,folder) VALUES(?,?) ON CONFLICT(account) DO UPDATE SET folder=excluded.folder",params![account.id,account.sent_folder])?;}
            tx.execute("DELETE FROM profile_reconnect WHERE account_id=?",[&account.id])?;
            connections::changed(&tx)?;
            tx.commit()?;
            Ok(())
        }).await
    }
}
