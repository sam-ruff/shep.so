use crate::api::MobileProfile;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shep_action_core::Status;
use shep_mail_core::{
    mail_actions::{Fingerprint, Flags, MoveFailure, MoveReceipt, classify_move_failure},
    model::*,
    providers::mail,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{Mutex, Semaphore, mpsc};

pub struct Operations {
    profile_history: crate::profile_history::Runtime,
    profile_discovery: crate::profile_discovery::Runtime,
    admitted: Arc<Semaphore>,
    slots: Arc<Semaphore>,
    search: Arc<Semaphore>,
    rendering: Arc<Semaphore>,
    forwarding: Arc<Semaphore>,
    printing: Arc<Semaphore>,
    profile_records: Arc<Semaphore>,
    accounts: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// One owned group step at a time; account removal takes it as a fence.
    pub(crate) groups: Arc<Mutex<()>>,
    pub(crate) outgoing: crate::outgoing::Runtime,
    pub(crate) sent: crate::sent::Runtime,
    #[cfg(test)]
    pub(crate) provider: std::sync::Mutex<Option<Arc<dyn shep_mail_core::providers::MailProvider>>>,
    #[cfg(test)]
    pub(crate) mutation_waiting: tokio::sync::Notify,
}
impl Operations {
    #[cfg(test)]
    pub(crate) async fn hold_render_capacity(&self) -> tokio::sync::OwnedSemaphorePermit {
        self.rendering.clone().acquire_many_owned(2).await.unwrap()
    }
    pub fn new() -> Self {
        Self {
            profile_history: crate::profile_history::Runtime::default(),
            profile_discovery: crate::profile_discovery::Runtime::default(),
            admitted: Arc::new(Semaphore::new(40)),
            slots: Arc::new(Semaphore::new(8)),
            search: Arc::new(Semaphore::new(1)),
            rendering: Arc::new(Semaphore::new(2)),
            forwarding: Arc::new(Semaphore::new(2)),
            printing: Arc::new(Semaphore::new(2)),
            profile_records: Arc::new(Semaphore::new(1)),
            accounts: Mutex::new(HashMap::new()),
            groups: Arc::new(Mutex::new(())),
            outgoing: crate::outgoing::Runtime::default(),
            sent: crate::sent::Runtime::default(),
            #[cfg(test)]
            provider: std::sync::Mutex::new(None),
            #[cfg(test)]
            mutation_waiting: tokio::sync::Notify::new(),
        }
    }
    fn mail_provider(
        &self,
        protocol: Protocol,
    ) -> Arc<dyn shep_mail_core::providers::MailProvider> {
        #[cfg(test)]
        if let Some(provider) = self.provider.lock().unwrap().clone() {
            return provider;
        }
        Arc::from(mail::provider(protocol))
    }
    pub(crate) async fn account(&self, id: &str) -> tokio::sync::OwnedMutexGuard<()> {
        let lock = self
            .accounts
            .lock()
            .await
            .entry(id.to_owned())
            .or_default()
            .clone();
        lock.lock_owned().await
    }
}
impl Operations {
    #[cfg(test)]
    pub(crate) async fn hold_network_capacity(
        &self,
    ) -> (
        tokio::sync::OwnedSemaphorePermit,
        tokio::sync::OwnedSemaphorePermit,
    ) {
        (
            self.admitted.clone().acquire_many_owned(40).await.unwrap(),
            self.slots.clone().acquire_many_owned(8).await.unwrap(),
        )
    }
    pub(crate) async fn try_account(&self, id: &str) -> Result<tokio::sync::OwnedMutexGuard<()>> {
        self.accounts.lock().await.entry(id.to_owned()).or_default().clone()
            .try_lock_owned().context("This account has an operation in progress. Wait for it to finish, then check Outbox.")
    }
}
#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    OpenProfileDiscovery {
        session: uuid::Uuid,
        access_token: String,
        namespace: String,
        expected_principal: Option<String>,
    },
    ProfileDiscovery {
        session: uuid::Uuid,
        command: crate::profile_discovery::Command,
    },
    ProfileCreation {
        session: uuid::Uuid,
        command: crate::profile_discovery::creation::Command,
    },
    ProfileEnrollment {
        session: uuid::Uuid,
        command: crate::profile_discovery::enrollment::Command,
    },
    ProfileSync {
        session: uuid::Uuid,
        command: crate::profile_discovery::sync::Command,
    },
    CloseProfileDiscovery {
        session: uuid::Uuid,
    },
    ProfileHistory {
        binding: shep_profile_core::history::Binding,
        command: Box<shep_profile_core::history::Command>,
    },
    CloseProfileHistory {
        binding: shep_profile_core::history::Binding,
    },
    Accounts,
    ValidateProfileOperation {
        record: String,
    },
    Selection {
        command: crate::selection::Command,
        #[serde(default)]
        observed: Vec<String>,
    },
    Groups {
        command: crate::groups::Command,
    },
    MailActions {
        #[serde(default)]
        offset: u32,
        #[serde(default)]
        runnable: bool,
    },
    CancelMailAction {
        id: String,
    },
    UndoMailAction {
        id: String,
        credential_slot: Option<String>,
        password: Option<SecretString>,
    },
    InspectMailAction {
        id: String,
        credential_slot: Option<String>,
        password: Option<SecretString>,
    },
    FindText {
        blocks: Vec<String>,
        query: String,
        match_case: bool,
    },
    PrepareAccount {
        account: Account,
        expected: Option<Account>,
    },
    ActivateAccount {
        slot: String,
    },
    CredentialTarget {
        account: Account,
    },
    CheckAccount {
        id: String,
    },
    AccountRemovalPreview {
        id: String,
    },
    RemoveAccount {
        review: crate::accounts::Removal,
        #[serde(default)]
        discard_unresolved: bool,
    },
    CredentialCleanup,
    CredentialCleanupDone {
        id: String,
    },
    SaveSentPreferences {
        id: String,
        policy: SentCopyPolicy,
        folder: String,
    },
    SaveAccount {
        account: Account,
        #[serde(default)]
        preserve_sent: bool,
    },
    Probe {
        account: Account,
        password: SecretString,
        smtp: bool,
    },
    Page {
        folder: String,
        #[serde(default)]
        account: Option<String>,
        #[serde(default)]
        query: String,
        #[serde(default)]
        filter: String,
        #[serde(default)]
        oldest: bool,
        #[serde(default)]
        offset: u32,
        #[serde(default)]
        projection: std::collections::BTreeMap<String, crate::paging::Edit>,
    },
    Detail {
        id: String,
    },
    Print {
        id: String,
        options: shep_mail_core::printing::Options,
    },
    Formatted {
        id: String,
        options: shep_mail_core::document::Options,
    },
    Attachment {
        id: String,
        file: String,
    },
    Sync {
        #[serde(default)]
        credential_slot: Option<String>,
        account: String,
        password: SecretString,
    },
    Mutate {
        #[serde(default)]
        action_id: Option<String>,
        #[serde(default)]
        observed_lineage: Option<String>,
        #[serde(default)]
        require_observation: bool,
        #[serde(default)]
        credential_slot: Option<String>,
        id: String,
        #[serde(default)]
        password: Option<SecretString>,
        folder: Option<String>,
        unread: Option<bool>,
        starred: Option<bool>,
    },
    Drafts,
    DraftFiles {
        id: String,
    },
    AddDraftFiles {
        id: String,
        paths: Vec<crate::drafts::SelectedFile>,
    },
    RemoveDraftFile {
        id: String,
        file: String,
    },
    Reply {
        id: String,
        #[serde(default)]
        all: bool,
    },
    Forward {
        id: String,
        draft_id: String,
    },
    SaveDraft {
        draft: Draft,
    },
    DiscardDraft {
        id: String,
        revision: u64,
    },
    AdmitSend {
        attempt: String,
        id: String,
        revision: u64,
        #[serde(default)]
        file_revision: u64,
    },
    Send {
        attempt: String,
        #[serde(default)]
        credential_slot: Option<String>,
        password: SecretString,
        #[serde(default)]
        incoming_password: Option<SecretString>,
    },
    OutgoingAccount {
        id: String,
    },
    RunnableOutgoing,
    CancelOutgoing {
        id: String,
    },
    WaitOutgoing {
        id: String,
    },
    SentOutgoing {
        #[serde(default)]
        credential_slot: Option<String>,
        id: String,
        copy: bool,
        #[serde(default)]
        confirmed: bool,
        #[serde(default)]
        password: Option<SecretString>,
    },
    Delivery {
        id: String,
    },
    Outbox {
        #[serde(default)]
        offset: u32,
    },
    RecoverOutgoing {
        id: String,
        action: crate::outgoing::Recovery,
        #[serde(default)]
        confirmed: bool,
    },
}
pub(crate) fn summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<Mail> {
    Ok(Mail {
        id: row.get(0)?,
        account_id: row.get(1)?,
        remote_id: row.get(2)?,
        folder: row.get(3)?,
        sender: row.get(4)?,
        recipient: row.get(5)?,
        subject: row.get(6)?,
        preview: row.get(7)?,
        timestamp: row.get(8)?,
        unread: row.get(9)?,
        starred: row.get(10)?,
        attachment_count: row.get::<_, u32>(11)? as usize,
    })
}
pub(crate) const SUMMARY: &str = "id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count";
pub(crate) fn stored_account(db: &Connection, id: &str) -> Result<Account> {
    let json: String = db
        .query_row("SELECT settings FROM accounts WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .context("This account was removed. Reopen Preferences.")?;
    Ok(serde_json::from_str(&json)?)
}
pub(crate) fn stored_mail(db: &Connection, id: &str) -> Result<Mail> {
    db.query_row(
        &format!("SELECT {SUMMARY} FROM mail WHERE id=COALESCE((SELECT id FROM mail_aliases WHERE alias=?1),?1)"),
        [id],
        summary,
    )
    .context("This message moved or is no longer cached. Refresh its folder.")
}
fn mark_local_sent_edit(db: &Connection, message: &Mail) -> Result<()> {
    if let Some(attempt) = message.remote_id.strip_prefix("local-sent-")
        && message.id == format!("{}:Sent:local-sent-{attempt}", message.account_id)
    {
        // POP3 UIDLs are arbitrary server strings. A matching prefix alone
        // must not change another message/account's outgoing record.
        db.execute("UPDATE outgoing_sent SET local_edited=1 WHERE id=?1 AND EXISTS(SELECT 1 FROM outgoing o WHERE o.id=outgoing_sent.id AND o.account_id=?2)",params![attempt,message.account_id])?;
    }
    Ok(())
}
pub(crate) fn insert_mail(db: &Connection, mail: StoredMail, pop: bool) -> Result<()> {
    if db.is_autocommit() {
        let tx = db.unchecked_transaction()?;
        insert_mail(&tx, mail, pop)?;
        tx.commit()?;
        return Ok(());
    }
    let mut m = mail.summary;
    // Local ids outlive a server MOVE. Re-downloading its new UID must update
    // that same cache record instead of creating a second row.
    if !pop && let Some(id) = db
        .query_row(
            "SELECT id FROM mail WHERE account_id=?1 AND folder=?2 AND remote_id=?3 AND moved=0",
            params![m.account_id, m.folder, m.remote_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
    {
        m.id = id;
    }
    // POP3 has no server flags/folders: preserve this device's existing state.
    let prior = if pop {
        db.query_row(
            "SELECT folder,unread,starred FROM mail WHERE id=?1",
            [&m.id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            },
        )
        .optional()?
    } else {
        None
    };
    let (folder, unread, starred) = prior.unwrap_or((m.folder.clone(), m.unread, m.starred));
    db.execute("INSERT INTO mail(id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count,body,raw) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14) ON CONFLICT(id) DO UPDATE SET sender=excluded.sender,recipient=excluded.recipient,subject=excluded.subject,preview=excluded.preview,timestamp=excluded.timestamp,unread=excluded.unread,starred=excluded.starred,attachment_count=excluded.attachment_count,body=excluded.body,raw=excluded.raw", params![m.id,m.account_id,m.remote_id,folder,m.sender,m.recipient,m.subject,m.preview,m.timestamp,unread,starred,u32::try_from(m.attachment_count)?,mail.text,mail.raw])?;
    if !pop {
        crate::sent::reconcile(db, &m, &mail.raw)?;
    }
    Ok(())
}
// These writes keep raw bytes, FTS and local UI identity attached to one row.
// An existing destination must be the same bytes before it can be deduplicated.
fn deduplicate_move(
    db: &Connection,
    id: &str,
    account: &str,
    folder: &str,
    remote: &str,
) -> Result<()> {
    let other:Option<(String,Vec<u8>)>=db.query_row("SELECT id,raw FROM mail WHERE account_id=?1 AND folder=?2 AND remote_id=?3 AND id!=?4 AND moved=0",params![account,folder,remote,id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((other, raw)) = other {
        let source: Vec<u8> =
            db.query_row("SELECT raw FROM mail WHERE id=?1", [id], |r| r.get(0))?;
        anyhow::ensure!(
            Fingerprint::of(&source).matches(&raw),
            "The destination cache has conflicting content. Refresh before another action."
        );
        adopt_action_alias(db, &other, id)?;
        db.execute(
            "UPDATE mail_aliases SET id=?2 WHERE id=?1",
            params![other, id],
        )?;
        db.execute("DELETE FROM mail WHERE id=?1", [other])?;
    }
    Ok(())
}

pub(crate) fn adopt_action_alias(db: &Connection, source: &str, target: &str) -> Result<()> {
    db.execute("UPDATE mail_lineage_aliases SET target=(SELECT token FROM mail_lineage WHERE id=?2) WHERE target=(SELECT token FROM mail_lineage WHERE id=?1)",params![source,target])?;
    db.execute("INSERT INTO mail_lineage_aliases(source,target) SELECT s.token,t.token FROM mail_lineage s,mail_lineage t WHERE s.id=?1 AND t.id=?2 ON CONFLICT(source) DO UPDATE SET target=excluded.target",params![source,target])?;
    db.execute(
        "INSERT INTO mail_intents(mail,field,revision) SELECT ?2,field,revision FROM mail_intents WHERE mail=?1
         ON CONFLICT(mail,field) DO UPDATE SET revision=MAX(mail_intents.revision,excluded.revision)",
        params![source,target],
    )?;
    db.execute(
        "UPDATE individual_mail_actions SET mail=?2 WHERE mail=?1",
        params![source, target],
    )?;
    Ok(())
}
pub(crate) fn acknowledged_mail_write(
    db: &Connection,
    id: &str,
    write: impl FnOnce() -> Result<usize>,
) -> Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "Mail identity changes require an atomic cache write."
    );
    let lineage: String =
        db.query_row("SELECT token FROM mail_lineage WHERE id=?1", [id], |row| {
            row.get(0)
        })?;
    write()?;
    db.execute(
        "UPDATE mail_lineage SET token=?2 WHERE id=?1",
        params![id, lineage],
    )?;
    Ok(())
}

pub(crate) fn save_move(db: &Connection, id: &str, receipt: &MoveReceipt) -> Result<()> {
    let remote = receipt.current.as_ref().map(|m| m.remote_id.as_str());
    if let Some(remote) = remote {
        deduplicate_move(db, id, &receipt.account, &receipt.folder, remote)?;
    }
    acknowledged_mail_write(db, id, || {
        Ok(db.execute(
            "UPDATE mail SET folder=?2,remote_id=COALESCE(?3,remote_id),moved=?4 WHERE id=?1",
            params![id, receipt.folder, remote, remote.is_none()],
        )?)
    })?;
    db.execute("DELETE FROM pending_moves WHERE id=?1", [id])?;
    db.execute("INSERT INTO move_receipts(id,receipt) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET receipt=excluded.receipt",params![id,serde_json::to_string(receipt)?])?;
    Ok(())
}
pub(crate) fn resolve_cached_move(db: &mut Connection, id: &str, mail: &Mail) -> Result<()> {
    let tx = db.transaction()?;
    resolve_cached_move_in(&tx, id, mail)?;
    tx.commit()?;
    Ok(())
}
fn resolve_cached_move_in(db: &Connection, id: &str, mail: &Mail) -> Result<()> {
    deduplicate_move(db, id, &mail.account_id, &mail.folder, &mail.remote_id)?;
    acknowledged_mail_write(db, id, || {
        Ok(db.execute(
            "UPDATE mail SET folder=?2,remote_id=?3,unread=?4,starred=?5,moved=0 WHERE id=?1",
            params![id, mail.folder, mail.remote_id, mail.unread, mail.starred],
        )?)
    })?;
    db.execute("DELETE FROM move_receipts WHERE id=?1", [id])?;
    Ok(())
}
pub(crate) fn reconcile_folder(
    db: &Connection,
    account: &str,
    folder: &str,
    live: &HashSet<String>,
) -> Result<()> {
    let ids = {
        let mut statement=db.prepare("SELECT id,account_id || ':' || folder || ':' || remote_id FROM mail WHERE account_id=?1 AND folder=?2 AND moved=0 AND remote_id NOT LIKE 'local-%'")?;
        statement
            .query_map(params![account, folder], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id, remote) in ids {
        if live.contains(&remote) {
            // A complete listing has re-established the source identity. Only
            // a subsequent explicit user action may retry; sync never MOVEs.
            db.execute("DELETE FROM pending_moves WHERE id=?1", [id])?;
        } else {
            db.execute("DELETE FROM mail WHERE id=?1", [id])?;
        }
    }
    Ok(())
}
pub(crate) fn positive_revision(value: u64) -> Result<i64> {
    i64::try_from(value).context("Draft revision is invalid.")
}
async fn value<T: serde::Serialize>(result: Result<T>) -> Result<Value> {
    Ok(serde_json::to_value(result?)?)
}

pub async fn run(profile: &MobileProfile, request: Request) -> Result<Value> {
    let db = &profile.database;
    match request {
        Request::OpenProfileDiscovery { session, access_token, namespace, expected_principal } => {
            Ok(serde_json::to_value(profile.operations.profile_discovery.open(&db.path,session,access_token,namespace,expected_principal).await?)?)
        }
        Request::ProfileDiscovery { session, command } => profile.operations.profile_discovery.run(session,command).await,
        Request::ProfileEnrollment { session, command } => profile.operations.profile_discovery.enrollment(profile,session,command).await,
        Request::ProfileSync { session, command } => profile.operations.profile_discovery.sync(profile,session,command).await,
        Request::ProfileCreation { session, command } => profile.operations.profile_discovery.creation(db,session,command).await,
        Request::CloseProfileDiscovery { session } => {
            profile.operations.profile_discovery.close(session).await?;
            Ok(json!({"closed":true}))
        }
        Request::ProfileHistory { binding,command } => {
            profile.operations.profile_history.run(&db.path,binding,*command).await
        }
        Request::CloseProfileHistory { binding } => {
            profile.operations.profile_history.close(binding).await?;
            Ok(json!({"closed":true}))
        }
        Request::ValidateProfileOperation { record } => {
            let permit = profile.operations.profile_records.clone().try_acquire_owned()
                .context("Profile validation is busy. Retry shortly.")?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let operation = shep_mail_core::profiles::codec::Operation::decode(record.as_bytes())?;
                // Return encoded metadata so Dart need not reserialize unknown
                // optional values. This path never imports accounts or secrets.
                Ok(json!({"record":String::from_utf8(operation.encode()?)?}))
            }).await?
        }
        Request::FindText{blocks,query,match_case} => {
            let permit=profile.operations.search.clone().try_acquire_owned().context("Find is busy. Retry the search shortly.")?;
            tokio::task::spawn_blocking(move||{
                let _permit=permit;
                Ok(json!(shep_mail_core::find::find(&blocks,&query,match_case).context("Could not search this message. Retry Find.")?))
            }).await?
        }
        Request::CheckAccount{id} => {db.read(move|db|crate::accounts::available(db,&id)).await?;Ok(json!({"available":true}))}
        Request::AccountRemovalPreview{id} => db.read(move|db|Ok(serde_json::to_value(crate::accounts::preview(db,&id)?)?)).await,
        Request::RemoveAccount{review,discard_unresolved} => {
            // Group ownership is taken before the account lock so no owned
            // step can dispatch or record a receipt for this account meanwhile.
            let _groups=profile.operations.groups.clone().try_lock_owned()
                .context("A group action step is in progress. Pause it in History, then retry removal.")?;
            let _guard=profile.operations.try_account(&review.id).await?;
            db.write(move|db|crate::accounts::remove(db,review,discard_unresolved)).await?;
            Ok(json!({"removed":true}))
        }
        Request::PrepareAccount{account,expected} => {
            let _guard=profile.operations.try_account(&account.id).await?;
            db.write(move|db|crate::connections::prepare(db,account,expected)).await
        }
        Request::ActivateAccount{slot} => {
            let lookup=slot.clone();
            let id=db.read(move|db|crate::connections::owner(db,&lookup)).await?;
            let _guard=profile.operations.account(&id).await;
            db.write(move|db|crate::connections::activate(db,&slot)).await?;
            Ok(json!({"saved":true}))
        }
        Request::CredentialTarget{account} => db.read(move|db|Ok(json!({"slot":crate::connections::target(db,account)?}))).await,
        Request::CredentialCleanup => db.read(|db|Ok(json!(crate::connections::cleanup(db)?))).await,
        Request::CredentialCleanupDone{id} => db.write(move|db|{crate::connections::cleanup_done(db,&id)?;Ok(json!({"cleaned":true}))}).await,
        Request::Accounts => value(db.read(|db| {
            let mut statement=db.prepare("SELECT settings FROM accounts ORDER BY id")?;
            let accounts=statement.query_map([], |r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let accounts=accounts.into_iter().map(|s|serde_json::from_str::<Account>(&s)).collect::<std::result::Result<Vec<_>,_>>()?;
            let mut folders=db.prepare("SELECT account_id,names FROM folders")?;
            let folders=folders.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().map(|(id,s)|Ok((id,serde_json::from_str::<Vec<String>>(&s)?))).collect::<Result<HashMap<_,_>>>()?;
            let reconnect = db.prepare("SELECT account_id FROM profile_reconnect ORDER BY account_id")?.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(json!({"accounts":accounts,"folders":folders,"reconnect":reconnect}))
        }).await).await,
        Request::SaveSentPreferences{id,policy,folder} => {
            let _guard=profile.operations.account(&id).await;
            db.write(move|db|{
                let mut account=stored_account(db,&id)?;
                anyhow::ensure!(account.protocol==Protocol::Imap || policy==SentCopyPolicy::LocalOnly,"POP3 keeps Sent copies locally.");
                account.sent_copy=policy; account.sent_folder=folder; account.validate()?;
                db.execute("UPDATE accounts SET settings=?2 WHERE id=?1",params![id,serde_json::to_string(&account)?])?;
                Ok(json!({"saved":true}))
            }).await
        }
        Request::SaveAccount{mut account,preserve_sent} => {
            account.validate()?;
            anyhow::ensure!(!account.id.is_empty() && account.id.len()<=128,"Give the account a valid identity.");
            let _guard=profile.operations.account(&account.id).await;
            db.write(move |db| {
                crate::accounts::available(db,&account.id)?;
                let bound:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM account_credentials WHERE account_id=?1) OR EXISTS(SELECT 1 FROM credential_slots WHERE slot=?1)",[&account.id],|r|r.get(0))?;
                anyhow::ensure!(!bound,"Reconnect through Preferences to preserve the saved credential binding.");
                if let Ok(old)=stored_account(db,&account.id) {
                    if preserve_sent {account.sent_copy=old.sent_copy;account.sent_folder=old.sent_folder.clone();}
                    anyhow::ensure!(old.host==account.host && old.port==account.port && old.username==account.username && old.protocol==account.protocol,"Add changed incoming server settings as a new account to preserve cached identities.");
                }
                db.execute("INSERT INTO accounts(id,settings) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET settings=excluded.settings",params![account.id,serde_json::to_string(&account)?])?;
                Ok(())
            }).await?;
            Ok(json!({"saved":true}))
        }
        Request::Selection { command, observed } => profile.database.selection(move |db| crate::selection::run(db,command,observed)).await,
        Request::Page{folder,account,query,filter,oldest,offset,projection} => db.read(move |db| {
            crate::paging::page(db, folder, account, query, filter, oldest, offset, projection)
        }).await,
        Request::Print{id,options} => {
            let permit=profile.operations.printing.clone().try_acquire_owned().context("Print preparation is busy. Finish a preview and retry.")?;
            let (account,raw):(String,Vec<u8>)=db.read(move |db| {
                let summary=stored_mail(db,&id)?;
                Ok((summary.account_id,db.query_row("SELECT raw FROM mail WHERE id=?1",[&summary.id],|r|r.get(0))?))
            }).await?;
            tokio::task::spawn_blocking(move || {
                let _permit=permit;
                let mut value=serde_json::to_value(shep_mail_core::printing::prepare(&raw,&options).context("Could not prepare this print. Try plain text or refresh and retry.")?)?;
                value["account_id"]=account.into();
                Ok(value)
            }).await?
        }
        Request::Formatted{id,options} => {
            // One departing reader and its replacement may prepare concurrently.
            // Admission precedes the cache read, and the blocking task owns its
            // permit through cancellation. Provider and Find slots stay separate.
            let permit=profile.operations.rendering.clone().try_acquire_owned().context("The formatted reader is busy. Use plain text or retry shortly.")?;
            let raw:Vec<u8>=db.read(move |db| {
                let summary=stored_mail(db,&id)?;
                Ok(db.query_row("SELECT raw FROM mail WHERE id=?1",[&summary.id],|r|r.get(0))?)
            }).await?;
            tokio::task::spawn_blocking(move || {
                let _permit=permit;
                Ok(serde_json::to_value(shep_mail_core::document::prepare(&raw,&options).context("Could not format this cached message. Use plain text or retry.")?)?)
            }).await?
        }
        Request::Detail{id} => {
            let (summary,text,raw)=db.read(move |db| {
                let mut summary=stored_mail(db,&id)?;
                let desired:Option<(Option<String>,Option<bool>,Option<bool>)>=db.query_row(
                    "SELECT MAX(CASE WHEN mi.field='folder' AND mi.revision=a.intent_revision THEN json_extract(a.fields,'$.folder') END),MAX(CASE WHEN mi.field='unread' AND mi.revision=a.intent_revision THEN json_extract(a.fields,'$.unread') END),MAX(CASE WHEN mi.field='starred' AND mi.revision=a.intent_revision THEN json_extract(a.fields,'$.starred') END) FROM individual_mail_actions a JOIN mail_intents mi ON mi.mail=a.mail WHERE a.mail=?1 AND a.status IN ('queued','waiting','running','uncertain','repair') GROUP BY a.mail",
                    [&summary.id],
                    |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)),
                ).optional()?;
                if let Some((folder,unread,starred))=desired {
                    if let Some(folder)=folder { summary.folder=folder; }
                    if let Some(unread)=unread { summary.unread=unread; }
                    if let Some(starred)=starred { summary.starred=starred; }
                }
                let (text,raw,lineage):(String,Vec<u8>,String)=db.query_row("SELECT m.body,m.raw,l.token FROM mail m JOIN mail_lineage l ON l.id=m.id WHERE m.id=?1",[&summary.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
                let mut summary=serde_json::to_value(summary)?;
                summary["lineage"]=lineage.into();
                Ok((summary,text,raw))
            }).await?;
            tokio::task::spawn_blocking(move || {
                let (files,file_error)=match shep_mail_core::attachments::catalog(&raw) {
                    Ok(files)=>(files,None),
                    Err(_)=>(Vec::new(),Some("Could not read attachments. The cached message body is still available. Refresh the message or retry loading its attachments.")),
                };
                Ok(json!({"summary":summary,"body":text,"attachments":files.iter().map(|f|f.name.clone()).collect::<Vec<_>>(),"files":files,"file_error":file_error}))
            }).await?
        }
        Request::Attachment{id,file} => {
            let raw:Vec<u8>=db.read(move |db| {
                let summary=stored_mail(db,&id)?;
                Ok(db.query_row("SELECT raw FROM mail WHERE id=?1",[&summary.id],|r|r.get(0))?)
            }).await?;
            tokio::task::spawn_blocking(move || {
                use base64::Engine;
                let (info,bytes)=shep_mail_core::attachments::read(&raw,&file)?;
                Ok(json!({"info":info,"bytes":base64::engine::general_purpose::STANDARD.encode(bytes)}))
            }).await?
        }
        Request::Drafts => db.read(crate::drafts::list).await,
        Request::DraftFiles{id} => db.read(move|db|crate::drafts::snapshot(db,&id)).await,
        Request::AddDraftFiles{id,paths} => {
            let files=tokio::task::spawn_blocking(move||crate::drafts::read_files(paths)).await??;
            db.write(move|db|crate::drafts::add(db,&id,files)).await
        }
        Request::RemoveDraftFile{id,file} => db.write(move|db|crate::drafts::remove(db,&id,&file)).await,
        Request::Reply{id,all} => db.read(move|db|{
            let summary=stored_mail(db,&id)?;
            let raw:Vec<u8>=db.query_row("SELECT raw FROM mail WHERE id=?1",[&summary.id],|r|r.get(0))?;
            let mut accounts=db.prepare("SELECT settings FROM accounts")?;
            let accounts=accounts.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().map(|s|serde_json::from_str::<Account>(&s)).collect::<std::result::Result<Vec<_>,_>>()?;
            Ok(serde_json::to_value(shep_mail_core::compose::reply_from_raw(summary,&raw,&accounts,all)?)?)
        }).await,
        Request::Forward{id,draft_id} => {
            anyhow::ensure!(uuid::Uuid::parse_str(&draft_id).is_ok(),"Choose a new forward identity before retrying.");
            let source=id.clone(); let target=draft_id.clone();
            if let Some(saved)=db.read(move|db|crate::drafts::forwarded(db,&target,&source)).await? { return Ok(saved); }
            let permit=profile.operations.forwarding.clone().try_acquire_owned()
                .context("Forward preparation is busy. Retry shortly.")?;
            let source=id.clone();
            let (account,raw)=db.read(move|db| {
                let summary=stored_mail(db,&source)?;
                let raw:Vec<u8>=db.query_row("SELECT raw FROM mail WHERE id=?1",[summary.id],|r|r.get(0))?;
                Ok((summary.account_id,raw))
            }).await?;
            let (draft,files)=tokio::task::spawn_blocking(move||{
                let _permit=permit;
                shep_mail_core::compose::prepare_forward(draft_id,account,&raw)
            }).await??;
            db.write(move|db|crate::drafts::create_forward(db,&id,draft,files)).await
        }
        Request::SaveDraft{draft} => {
            db.write(move|db|crate::drafts::save_text(db,draft)).await?;
            Ok(json!({"saved":true}))
        }
        Request::DiscardDraft{id,revision} => {
            db.write(move|db|{
                let tx=db.transaction()?;
                let submitted:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE draft_id=?1)",[&id],|r|r.get(0))?;
                anyhow::ensure!(!submitted,"Check this draft's delivery status before discarding its recovery record.");
                tx.execute("INSERT INTO discarded_drafts VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET revision=MAX(revision,excluded.revision)",params![id,positive_revision(revision)?])?;
                tx.execute("DELETE FROM drafts WHERE id=?1",[id])?;
                tx.commit()?; Ok(())
            }).await?;
            Ok(json!({"discarded":true}))
        }
        Request::AdmitSend{attempt,id,revision,file_revision} => crate::outgoing::admit(profile,attempt,id,revision,file_revision).await,
        Request::OutgoingAccount{id} => db.read(move |db| Ok(json!({"account_id":db.query_row("SELECT account_id FROM outgoing WHERE id=?1",[id],|r|r.get::<_,String>(0))?}))).await,
        Request::RunnableOutgoing => db.read(|db| {
            let mut statement=db.prepare("SELECT id,account_id FROM outgoing WHERE state IN ('queued','waiting') ORDER BY rowid LIMIT 32")?;
            let rows=statement.query_map([],|r|Ok(json!({"id":r.get::<_,String>(0)?,"account_id":r.get::<_,String>(1)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(json!({"rows":rows}))
        }).await,
        Request::WaitOutgoing{id} => db.write(move|db| {
            let tx=db.transaction()?;
            let changed=tx.execute("UPDATE outgoing SET state='waiting' WHERE id=?1 AND state IN ('queued','waiting')",[&id])?;
            if changed>0 {
                tx.execute("UPDATE outgoing_sent SET error='Delivery is waiting. Check this account in Preferences or cancel the queued delivery to review its draft.' WHERE id=?1",[id])?;
            }
            tx.commit()?;
            Ok(json!({"waiting":changed>0}))
        }).await,
        Request::CancelOutgoing{id} => db.write(move|db| {
            let tx=db.transaction()?;
            let changed=tx.execute("DELETE FROM outgoing_sent WHERE id=?1 AND EXISTS(SELECT 1 FROM outgoing WHERE id=?1 AND state IN ('queued','waiting'))",[&id])?;
            anyhow::ensure!(changed==1,"This delivery has already started. Review Outbox before changing it.");
            tx.execute("DELETE FROM outgoing_meta WHERE id=?1",[&id])?;
            tx.execute("DELETE FROM outgoing WHERE id=?1 AND state IN ('queued','waiting')",[&id])?;
            tx.commit()?;
            Ok(json!({"id":id,"state":"cancelled"}))
        }).await,
        Request::Delivery{id} => crate::outgoing::delivery(profile, id).await,
        Request::Outbox{offset} => crate::outgoing::page(profile, offset).await,
        Request::RecoverOutgoing{id,action,confirmed} => crate::outgoing::recover(profile,id,action,confirmed).await,
        Request::Mutate{action_id,observed_lineage,require_observation,credential_slot,id,password,folder,unread,starred} => {
            let report=action_id.is_some();
            mutate(profile,Mutation{action_id:action_id.unwrap_or_else(||uuid::Uuid::new_v4().to_string()),parent_action:None,observed_lineage,require_observation,credential_slot,id,password,folder,unread,starred,intent:true,report}).await
        }
        Request::Groups{command} => crate::groups::run(profile,command).await,
        Request::MailActions{offset,runnable} => db.read(move|db| {
            let query=if runnable {
                "SELECT id,mail,account,COALESCE(accepted_fields,fields),status,error,created FROM individual_mail_actions WHERE status IN ('queued','waiting') ORDER BY created,id LIMIT 50 OFFSET ?1"
            } else {
                "SELECT id,mail,account,COALESCE(accepted_fields,fields),status,error,created FROM individual_mail_actions ORDER BY created DESC,id LIMIT 50 OFFSET ?1"
            };
            let saved=db.prepare(query)?
                .query_map([offset],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,i64>(6)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let rows=saved.into_iter().map(|(id,mail,account,fields,status,error,created)|{
                anyhow::ensure!(Status::parse(&status).is_some(),"This action status requires a newer Shep version.");
                Ok(json!({"id":id,"mail":mail,"account":account,"fields":serde_json::from_str::<Value>(&fields)?,"status":status,"error":error,"created":created}))
            }).collect::<Result<Vec<_>>>()?;
            Ok(json!({"actions":rows}))
        }).await,
        Request::CancelMailAction{id} => db.write(move|db|{
            let tx=db.transaction()?;
            let changed=tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE id=?1 AND status IN ('queued','waiting')",[&id])?;
            anyhow::ensure!(changed==1,"This action has already started. Refresh its status before Undo.");
            release_action_intent(&tx,&id)?;
            tx.commit()?;
            Ok(json!({"id":id,"status":"cancelled"}))
        }).await,
        Request::UndoMailAction{id,credential_slot,password} => {
            let lookup=id.clone();
            let undo_id=format!("{id}:undo");
            let (mail,fields,physical)=db.read(move|db|{
                let (mail,fields,physical,status):(String,String,String,String)=db.query_row("SELECT mail,COALESCE(accepted_fields,fields),physical,status FROM individual_mail_actions WHERE id=?1",[lookup],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
                anyhow::ensure!(status=="succeeded","Only a confirmed mail change can be undone.");
                let fields=serde_json::from_str::<Value>(&fields)?;
                Ok((mail,fields,serde_json::from_str::<Value>(&physical)?))
            }).await?;
            let folder=fields.get("folder").filter(|v|!v.is_null()).map(|_|physical.get("folder").and_then(Value::as_str).map(str::to_owned).context("This older move has no saved Undo folder.")).transpose()?;
            let unread=fields.get("unread").filter(|v|!v.is_null()).map(|_|physical.get("unread").and_then(Value::as_bool).context("This older flag change has no saved Undo value.")).transpose()?;
            let starred=fields.get("starred").filter(|v|!v.is_null()).map(|_|physical.get("starred").and_then(Value::as_bool).context("This older flag change has no saved Undo value.")).transpose()?;
            mutate(profile,Mutation{action_id:undo_id,parent_action:Some(id),observed_lineage:None,require_observation:false,credential_slot,id:mail,password,folder,unread,starred,intent:true,report:true}).await
        }
        Request::InspectMailAction{id,credential_slot,password} => inspect_mail_action(profile,id,credential_slot,password).await,
        request => network(profile,request).await,
    }
}

async fn inspect_mail_action(
    profile: &MobileProfile,
    id: String,
    credential_slot: Option<String>,
    password: Option<SecretString>,
) -> Result<Value> {
    let repair = id.clone();
    if profile
        .database
        .write(move |db| apply_individual_receipt(db, &repair))
        .await?
        == ReceiptApplication::Complete
    {
        return Ok(json!({"status":"succeeded","committed":true}));
    }
    let lookup = id.clone();
    let account = profile
        .database
        .read(move |db| {
            let (account, status): (String, String) = db.query_row(
                "SELECT account,status FROM individual_mail_actions WHERE id=?1",
                [lookup],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            anyhow::ensure!(
                matches!(status.as_str(), "repair" | "uncertain"),
                "This mail change no longer needs inspection."
            );
            stored_account(db, &account)
        })
        .await?;
    anyhow::ensure!(
        account.protocol == Protocol::Imap,
        "Refresh this account to inspect the current message state."
    );
    let Some(password) = password else {
        return Ok(json!({"status":"uncertain","requires_credentials":account.id}));
    };
    let _guard = profile.operations.account(&account.id).await;
    let account_id = account.id.clone();
    let lookup = id.clone();
    let (message,dispatch,destination,expected_flags,intent_revision,owns,saved_receipt,frozen_fingerprint)=profile
        .database
        .read(move |db| {
            crate::connections::check_binding(db, &account_id, credential_slot.as_deref())?;
            let (mail,fields,physical,status,intent_revision):(String,String,String,String,i64)=db.query_row("SELECT mail,COALESCE(accepted_fields,fields),physical,status,intent_revision FROM individual_mail_actions WHERE id=?1",[lookup],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
            anyhow::ensure!(matches!(status.as_str(),"repair"|"uncertain"),"This mail change no longer needs inspection.");
            let fields:Value=serde_json::from_str(&fields)?;
            let physical:Value=serde_json::from_str(&physical)?;
            let message=stored_mail(db,&mail)?;
            anyhow::ensure!(!message.remote_id.starts_with("local-"),"Refresh this account to inspect the current message state.");
            let mut dispatch=message.clone();
            dispatch.folder=physical.get("folder").and_then(Value::as_str).context("This action has no saved source folder.")?.to_owned();
            dispatch.remote_id=physical.get("remote_id").and_then(Value::as_str).context("This action has no saved source identity.")?.to_owned();
            let destination=fields.get("folder").and_then(Value::as_str).map(str::to_owned);
            let expected_flags=Flags{unread:fields.get("unread").and_then(Value::as_bool),starred:fields.get("starred").and_then(Value::as_bool)};
            let continuous=action_physical_continuity(db,&message,&fields,&physical)?;
            let owns=continuous && [("folder",destination.is_some()),("unread",expected_flags.unread.is_some()),("starred",expected_flags.starred.is_some())].into_iter().filter(|(_,changed)|*changed).all(|(field,_)|db.query_row("SELECT revision=?3 FROM mail_intents WHERE mail=?1 AND field=?2",params![message.id,field,intent_revision],|r|r.get::<_,bool>(0)).optional().ok().flatten().unwrap_or(false));
            let saved_receipt=db.query_row("SELECT receipt FROM move_receipts WHERE id=?1",[&message.id],|r|r.get::<_,String>(0)).optional()?.map(|saved|serde_json::from_str::<MoveReceipt>(&saved)).transpose()?;
            let frozen_fingerprint=physical.get("fingerprint").filter(|value|!value.is_null()).cloned().map(serde_json::from_value::<Fingerprint>).transpose()?;
            Ok((message,dispatch,destination,expected_flags,intent_revision,owns,saved_receipt,frozen_fingerprint))
        })
        .await?;
    if !owns {
        let cancelled = id.clone();
        profile.database.write(move|db|{let tx=db.transaction()?;tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE id=?1 AND status IN ('repair','uncertain')",[&cancelled])?;release_action_intent(&tx,&cancelled)?;tx.commit()?;Ok(())}).await?;
        return Ok(json!({"status":"cancelled","committed":false}));
    }
    if destination.is_none() {
        let provider = profile.operations.mail_provider(account.protocol);
        let observed = tokio::time::timeout(
            Duration::from_secs(45),
            provider.inspect_flags(&account, &password, &message),
        )
        .await
        .context("Flag inspection timed out. Retry from Mail Activity.")??;
        let applied = expected_flags
            .unread
            .is_none_or(|value| observed.unread == Some(value))
            && expected_flags
                .starred
                .is_none_or(|value| observed.starred == Some(value));
        let snapshot = (
            message.account_id.clone(),
            message.folder.clone(),
            message.remote_id.clone(),
        );
        let identity = message.id.clone();
        let saved_id = id.clone();
        let status=profile.database.write(move |db| {
            let tx=db.transaction()?;
            let action_status:String=tx.query_row("SELECT status FROM individual_mail_actions WHERE id=?1",[&saved_id],|r|r.get(0))?;
            anyhow::ensure!(matches!(action_status.as_str(),"repair"|"uncertain"),"This mail change no longer needs inspection.");
            let current=stored_mail(&tx,&identity)?;
            let unchanged=(current.account_id.clone(),current.folder.clone(),current.remote_id.clone())==snapshot;
            let owns=[("unread",expected_flags.unread.is_some()),("starred",expected_flags.starred.is_some())].into_iter().filter(|(_,changed)|*changed).all(|(field,_)|tx.query_row("SELECT revision=?3 FROM mail_intents WHERE mail=?1 AND field=?2",params![current.id,field,intent_revision],|r|r.get::<_,bool>(0)).optional().ok().flatten().unwrap_or(false));
            if !unchanged || !owns {
                tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE id=?1 AND status IN ('repair','uncertain')",[&saved_id])?;
                release_action_intent(&tx,&saved_id)?;
                tx.commit()?;
                return Ok("cancelled");
            }
            if applied {
                tx.execute("UPDATE mail SET unread=COALESCE(?2,unread),starred=COALESCE(?3,starred) WHERE id=?1",params![identity,expected_flags.unread,expected_flags.starred])?;
                tx.execute("UPDATE individual_mail_actions SET status='succeeded',error=NULL WHERE id=?1 AND status IN ('repair','uncertain')",[&saved_id])?;
            } else {
                tx.execute("UPDATE individual_mail_actions SET status='rejected',error='The provider retained its previous flags. Retry only if you still want this change.' WHERE id=?1 AND status IN ('repair','uncertain')",[&saved_id])?;
                release_action_intent(&tx,&saved_id)?;
            }
            tx.commit()?;
            Ok(if applied{"succeeded"}else{"rejected"})
        }).await?;
        return Ok(json!({"status":status,"committed":status=="succeeded"}));
    }
    let destination = destination.context("This move has no saved destination.")?;
    let receipt = saved_receipt.unwrap_or(MoveReceipt::server(
        &dispatch,
        &account.id,
        &destination,
        None,
        frozen_fingerprint.context("This action has no saved content proof.")?,
    ));
    let provider = profile.operations.mail_provider(account.protocol);
    let mut resolved = tokio::time::timeout(
        Duration::from_secs(45),
        provider.inspect_move(&account, &password, &receipt),
    )
    .await
    .context("Message inspection timed out. Retry from Mail Activity.")??;
    let snapshot = (
        message.account_id.clone(),
        message.folder.clone(),
        message.remote_id.clone(),
    );
    let identity = message.id.clone();
    resolved.id = identity.clone();
    let saved_id = id.clone();
    let status=profile
        .database
        .write(move |db| {
            let tx = db.transaction()?;
            let action_status:String=tx.query_row("SELECT status FROM individual_mail_actions WHERE id=?1",[&saved_id],|r|r.get(0))?;
            anyhow::ensure!(matches!(action_status.as_str(),"repair"|"uncertain"),"This mail change no longer needs inspection.");
            let current=stored_mail(&tx,&identity)?;
            let unchanged=(current.account_id.clone(),current.folder.clone(),current.remote_id.clone())==snapshot;
            let owns=tx.query_row("SELECT revision=?3 FROM mail_intents WHERE mail=?1 AND field='folder'",params![current.id,"folder",intent_revision],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
            if !unchanged || !owns {
                tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE id=?1 AND status IN ('repair','uncertain')",[&saved_id])?;
                release_action_intent(&tx,&saved_id)?;
                tx.commit()?;
                return Ok("cancelled");
            }
            resolved.unread=current.unread;
            resolved.starred=current.starred;
            resolve_cached_move_in(&tx, &identity, &resolved)?;
            tx.execute(
                "UPDATE individual_mail_actions SET status='succeeded',error=NULL WHERE id=?1 AND status IN ('repair','uncertain')",
                [saved_id],
            )?;
            tx.commit()?;
            Ok("succeeded")
        })
        .await?;
    Ok(json!({"status":status,"committed":status=="succeeded"}))
}
/// One message change. Individual user actions reserve per-field intent
/// revisions at input time; group steps pass `intent: false` because their
/// approval revision is older by definition.
pub(crate) struct Mutation {
    pub action_id: String,
    pub parent_action: Option<String>,
    pub observed_lineage: Option<String>,
    pub require_observation: bool,
    pub credential_slot: Option<String>,
    pub id: String,
    pub password: Option<SecretString>,
    pub folder: Option<String>,
    pub unread: Option<bool>,
    pub starred: Option<bool>,
    pub intent: bool,
    pub report: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum IndividualReceipt {
    Move { receipt: Box<MoveReceipt> },
    Flags,
}

fn save_individual_receipt(
    db: &Connection,
    action: &str,
    receipt: &IndividualReceipt,
) -> Result<()> {
    let tx = db.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO individual_mail_action_receipts(action,result) VALUES(?1,?2) ON CONFLICT(action) DO UPDATE SET result=excluded.result",
        params![action, serde_json::to_string(receipt)?],
    )?;
    tx.execute(
        "UPDATE individual_mail_actions SET status='repair',error='The provider acknowledged this change. Finish saving it locally.' WHERE id=?1 AND status='running'",
        [action],
    )?;
    tx.commit()?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReceiptApplication {
    Missing,
    NeedsInspection,
    Complete,
}

fn apply_individual_receipt(db: &Connection, action: &str) -> Result<ReceiptApplication> {
    let tx = db.unchecked_transaction()?;
    let saved: Option<(String, String, String, String, i64)> = tx
        .query_row(
            "SELECT a.mail,COALESCE(a.accepted_fields,a.fields),a.physical,r.result,a.intent_revision FROM individual_mail_actions a JOIN individual_mail_action_receipts r ON r.action=a.id WHERE a.id=?1 AND a.status IN ('running','repair')",
            [action],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
        )
        .optional()?;
    let Some((mail, fields, physical, receipt, revision)) = saved else {
        return Ok(ReceiptApplication::Missing);
    };
    let fields: Value = serde_json::from_str(&fields)?;
    let physical: Value = serde_json::from_str(&physical)?;
    let receipt: IndividualReceipt = serde_json::from_str(&receipt)?;
    let current = stored_mail(&tx, &mail)?;
    let mut needs_inspection = false;
    let owns = |field: &str| -> Result<bool> {
        Ok(tx
            .query_row(
                "SELECT revision=?3 FROM mail_intents WHERE mail=?1 AND field=?2",
                params![mail, field, revision],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false))
    };
    match receipt {
        IndividualReceipt::Move { receipt } => {
            let continuous = action_source_matches(&tx, &current, &physical)?;
            let at_source = physical["account"].as_str() == Some(current.account_id.as_str())
                && physical["folder"].as_str() == Some(current.folder.as_str())
                && physical["remote_id"].as_str() == Some(current.remote_id.as_str())
                && continuous;
            let at_receipt = receipt.current.as_ref().is_some_and(|target| {
                target.account_id == current.account_id
                    && target.folder == current.folder
                    && target.remote_id == current.remote_id
            }) && continuous;
            let at_unresolved_target = receipt.current.is_none()
                && receipt.account == current.account_id
                && receipt.folder == current.folder
                && continuous;
            anyhow::ensure!(
                at_source || at_receipt || at_unresolved_target,
                "The acknowledged move no longer matches this cached message. Refresh and review it."
            );
            if at_source {
                mark_local_sent_edit(&tx, &current)?;
                save_move(&tx, &mail, &receipt)?;
            }
            needs_inspection = receipt.current.is_none();
        }
        IndividualReceipt::Flags => {
            anyhow::ensure!(
                action_source_matches(&tx, &current, &physical)?,
                "The acknowledged flag change no longer matches this cached message. Refresh and review it."
            );
            let unread = if owns("unread")? {
                fields.get("unread").and_then(Value::as_bool)
            } else {
                None
            };
            let starred = if owns("starred")? {
                fields.get("starred").and_then(Value::as_bool)
            } else {
                None
            };
            if unread.is_none() && starred.is_none() {
                tx.execute(
                    "UPDATE individual_mail_actions SET status='succeeded',error=NULL WHERE id=?1 AND status IN ('running','repair')",
                    [action],
                )?;
                tx.commit()?;
                return Ok(ReceiptApplication::Complete);
            }
            mark_local_sent_edit(&tx, &current)?;
            acknowledged_mail_write(&tx, &mail, || {
                Ok(tx.execute(
                    "UPDATE mail SET unread=COALESCE(?2,unread),starred=COALESCE(?3,starred) WHERE id=?1",
                    params![mail, unread, starred],
                )?)
            })?;
        }
    }
    if !needs_inspection {
        tx.execute(
            "UPDATE individual_mail_actions SET status='succeeded',error=NULL WHERE id=?1 AND status IN ('running','repair')",
            [action],
        )?;
    }
    tx.commit()?;
    Ok(if needs_inspection {
        ReceiptApplication::NeedsInspection
    } else {
        ReceiptApplication::Complete
    })
}
pub(crate) fn record_intent(db: &Connection, id: &str, fields: &[&str]) -> Result<()> {
    if fields.is_empty() {
        return Ok(());
    }
    db.execute("UPDATE group_clock SET revision=revision+1 WHERE id=1", [])?;
    for field in fields {
        db.execute("INSERT INTO mail_intents(mail,field,revision) VALUES(?1,?2,(SELECT revision FROM group_clock WHERE id=1)) ON CONFLICT(mail,field) DO UPDATE SET revision=excluded.revision",params![id,field])?;
    }
    Ok(())
}
fn release_action_intent(db: &Connection, action: &str) -> Result<()> {
    let saved: Option<(String, String, i64)> = db
        .query_row(
            "SELECT mail,fields,intent_revision FROM individual_mail_actions WHERE id=?1",
            [action],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((mail, fields, revision)) = saved else {
        return Ok(());
    };
    let fields: Value = serde_json::from_str(&fields)?;
    for field in ["folder", "unread", "starred"] {
        if fields.get(field).is_some_and(|value| !value.is_null()) {
            db.execute(
                "DELETE FROM mail_intents WHERE mail=?1 AND field=?2 AND revision=?3",
                params![mail, field, revision],
            )?;
        }
    }
    Ok(())
}
fn action_physical_continuity(
    db: &Connection,
    message: &Mail,
    fields: &Value,
    physical: &Value,
) -> Result<bool> {
    if physical.get("lineage").and_then(Value::as_str).is_some() {
        return action_source_matches(db, message, physical);
    }
    let frozen = message.account_id
        == physical
            .get("account")
            .and_then(Value::as_str)
            .unwrap_or_default()
        && message.folder
            == physical
                .get("folder")
                .and_then(Value::as_str)
                .unwrap_or_default()
        && message.remote_id
            == physical
                .get("remote_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
    let Some(destination) = fields.get("folder").and_then(Value::as_str) else {
        return Ok(frozen);
    };
    if frozen {
        return Ok(true);
    }
    let account = stored_account(db, &message.account_id)?;
    if account.protocol == Protocol::Pop3
        && message.account_id
            == physical
                .get("account")
                .and_then(Value::as_str)
                .unwrap_or_default()
        && message.remote_id
            == physical
                .get("remote_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
        && message.folder == destination
    {
        return Ok(true);
    }
    let receipt = db
        .query_row(
            "SELECT receipt FROM move_receipts WHERE id=?1",
            [&message.id],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .map(|saved| serde_json::from_str::<MoveReceipt>(&saved))
        .transpose()?;
    let Some(receipt) = receipt else {
        return Ok(false);
    };
    let frozen_fingerprint = physical
        .get("fingerprint")
        .filter(|value| !value.is_null())
        .cloned()
        .map(serde_json::from_value::<Fingerprint>)
        .transpose()?;
    let raw: Vec<u8> = db.query_row("SELECT raw FROM mail WHERE id=?1", [&message.id], |r| {
        r.get(0)
    })?;
    let identity_matches = receipt.current.as_ref().map_or_else(
        || message.account_id == receipt.account && message.folder == receipt.folder,
        |current| {
            current.account_id == message.account_id
                && current.folder == message.folder
                && current.remote_id == message.remote_id
        },
    );
    Ok(receipt.account
        == physical
            .get("account")
            .and_then(Value::as_str)
            .unwrap_or_default()
        && receipt.folder == destination
        && identity_matches
        && receipt
            .fingerprint
            .as_ref()
            .zip(frozen_fingerprint.as_ref())
            .is_some_and(|(proof, frozen)| proof == frozen && proof.matches(&raw)))
}

pub(crate) fn observed_lineage_matches(db: &Connection, id: &str, observed: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM mail_lineage WHERE id=?1 AND (token=?2 OR token=(SELECT target FROM mail_lineage_aliases WHERE source=?2)))",
        params![id, observed],
        |row| row.get(0),
    )?)
}
fn action_source_matches(db: &Connection, message: &Mail, physical: &Value) -> Result<bool> {
    if physical["account"].as_str() != Some(message.account_id.as_str()) {
        return Ok(false);
    }
    if let Some(lineage) = physical.get("lineage").and_then(Value::as_str) {
        return observed_lineage_matches(db, &message.id, lineage);
    }
    Ok(physical["folder"].as_str() == Some(message.folder.as_str())
        && physical["remote_id"].as_str() == Some(message.remote_id.as_str()))
}
fn provider_definitely_refused(error: &anyhow::Error) -> bool {
    classify_move_failure(error) == MoveFailure::Refused
        || error
            .chain()
            .any(|cause| cause.is::<shep_mail_core::mail_actions::FlagsRejected>())
}
#[derive(Debug)]
struct ClaimRejected;
impl std::fmt::Display for ClaimRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("This message changed identity; no provider operation was started.")
    }
}
impl std::error::Error for ClaimRejected {}
pub(crate) async fn mutate(profile: &MobileProfile, mutation: Mutation) -> Result<Value> {
    let db = &profile.database;
    let action_id = mutation.action_id.clone();
    let parent_action = mutation.parent_action.clone();
    let observed_lineage = mutation.observed_lineage.clone();
    let require_observation = mutation.require_observation;
    let credential_slot = mutation.credential_slot.clone();
    let report = mutation.report;
    let (id, folder, unread, starred, has_password, intent) = (
        mutation.id.clone(),
        mutation.folder.clone(),
        mutation.unread,
        mutation.starred,
        mutation.password.is_some(),
        mutation.intent,
    );
    let local = db.write(move |db| {
        // A local edit and the handover eligibility marker commit
        // together, before any account lock or provider admission.
        let tx=db.transaction()?;
        let payload=serde_json::to_string(&json!({"folder":folder,"unread":unread,"starred":starred}))?;
        let saved:Option<(String,String,String,i64,String,Option<String>)>=if intent {
            tx.query_row("SELECT mail,fields,physical,intent_revision,status,error FROM individual_mail_actions WHERE id=?1",[&action_id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?
        } else { None };
        if let Some((_,saved_fields,_,_,status,error))=&saved {
            anyhow::ensure!(saved_fields==&payload,"This action identity belongs to another mail change.");
            if status == "running" {
                tx.commit()?;
                return Ok(Some(json!({"action_id":action_id,"status":"running","committed":true,"warning":"This change is still being synchronised."})));
            }
            if matches!(status.as_str(),"succeeded"|"repair"|"uncertain"|"rejected"|"cancelled") {
                tx.commit()?;
                let warning=error.clone().filter(|_|status!="succeeded");
                return Ok(Some(json!({"action_id":action_id,"status":status,"committed":matches!(status.as_str(),"succeeded"|"repair"|"uncertain"),"warning":warning})));
            }
        }
        let message=stored_mail(&tx,&id)?;
        let account=stored_account(&tx,&message.account_id)?;
        anyhow::ensure!(account.protocol==Protocol::Pop3 || folder.is_none() || (unread.is_none() && starred.is_none()),"Move and flag changes must be separate actions.");
        if intent {
            let fields:Vec<&str>=[folder.as_ref().map(|_|"folder"),unread.map(|_|"unread"),starred.map(|_|"starred")].into_iter().flatten().collect();
            let fingerprint = if folder.is_some() {
                let raw:Vec<u8>=tx.query_row("SELECT raw FROM mail WHERE id=?1",[&message.id],|r|r.get(0))?;
                Some(Fingerprint::of(&raw))
            } else {
                None
            };
            let lineage:String=tx.query_row("SELECT token FROM mail_lineage WHERE id=?1",[&message.id],|row|row.get(0))?;
            let physical=serde_json::to_string(&json!({"account":message.account_id,"folder":message.folder,"remote_id":message.remote_id,"unread":message.unread,"starred":message.starred,"fingerprint":fingerprint,"lineage":lineage}))?;
            if let Some((saved_mail,_,saved_physical,intent_revision,status,_))=saved {
                anyhow::ensure!(saved_mail==message.id,"This action identity belongs to another mail change.");
                if matches!(status.as_str(),"queued"|"waiting"|"running") {
                    let frozen:Value=serde_json::from_str(&saved_physical)?;
                    if !action_source_matches(&tx,&message,&frozen)? {
                        let warning="This message changed identity while the action was waiting. Refresh and review it before retrying.";
                        tx.execute("UPDATE individual_mail_actions SET status='rejected',error=?2 WHERE id=?1",params![&action_id,warning])?;
                        release_action_intent(&tx,&action_id)?;
                        tx.commit()?;
                        return Ok(Some(json!({"action_id":action_id,"status":"rejected","committed":false,"warning":warning})));
                    }
                    let mut superseded=false;
                    for field in &fields {
                        let owns=tx.query_row("SELECT revision=?3 FROM mail_intents WHERE mail=?1 AND field=?2",params![message.id,field,intent_revision],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
                        superseded|=!owns;
                    }
                    if superseded {
                        tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE id=?1",[&action_id])?;
                        release_action_intent(&tx,&action_id)?;
                        tx.commit()?;
                        return Ok(Some(json!({"action_id":action_id,"status":"cancelled","committed":false})));
                    }
                }
            } else {
                if require_observation {
                    let observed = observed_lineage
                        .as_deref()
                        .context("Refresh this message before changing it.")?;
                    anyhow::ensure!(
                        observed_lineage_matches(&tx, &message.id, observed)?,
                        "This message changed since it was shown. Refresh the folder and retry."
                    );
                }
                if let Some(parent)=&parent_action {
                    let (parent_mail,parent_fields,parent_revision,parent_status):(String,String,i64,String)=tx.query_row("SELECT mail,COALESCE(accepted_fields,fields),intent_revision,status FROM individual_mail_actions WHERE id=?1",[parent],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
                    anyhow::ensure!(parent_status=="succeeded" && parent_mail==message.id,"This Undo no longer matches the confirmed mail change.");
                    let parent_fields:Value=serde_json::from_str(&parent_fields)?;
                    let parent_physical:String=tx.query_row("SELECT physical FROM individual_mail_actions WHERE id=?1",[parent],|r|r.get(0))?;
                    let parent_physical:Value=serde_json::from_str(&parent_physical)?;
                    anyhow::ensure!(action_physical_continuity(&tx,&message,&parent_fields,&parent_physical)?,"This message changed identity after the original action, so it can no longer be undone.");
                    for field in ["folder","unread","starred"] {
                        if parent_fields.get(field).is_some_and(|value|!value.is_null()) {
                            let owns=tx.query_row("SELECT revision=?3 FROM mail_intents WHERE mail=?1 AND field=?2",params![message.id,field,parent_revision],|r|r.get::<_,bool>(0)).optional()?.unwrap_or(false);
                            anyhow::ensure!(owns,"A newer decision replaced this mail change, so it can no longer be undone.");
                        }
                    }
                }
                record_intent(&tx,&message.id,&fields)?;
                let intent_revision:i64=tx.query_row("SELECT revision FROM group_clock WHERE id=1",[],|r|r.get(0))?;
                tx.execute("INSERT INTO individual_mail_actions(id,mail,account,fields,physical,intent_revision,credential_slot,status,created) VALUES(?1,?2,?3,?4,?5,?6,?7,'queued',?8)",params![action_id,message.id,message.account_id,payload,physical,intent_revision,credential_slot,chrono::Utc::now().timestamp_millis()])?;
                tx.execute("DELETE FROM individual_mail_actions WHERE status IN ('succeeded','cancelled') AND id NOT IN (SELECT id FROM individual_mail_actions WHERE status IN ('succeeded','cancelled') ORDER BY created DESC,id LIMIT 100)",[])?;
            }
        }
        let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM pending_moves WHERE id=?1)",[&message.id],|r|r.get(0))?;
        if pending && intent {
            let warning="An earlier move needs checking before this change can synchronise.";
            tx.execute("UPDATE individual_mail_actions SET status='waiting',error=?2 WHERE id=?1",params![action_id,warning])?;
            tx.commit()?;
            return Ok(Some(json!({"action_id":action_id,"status":"waiting","committed":true,"warning":warning})));
        }
        anyhow::ensure!(!pending,"A previous move has no saved acknowledgment. Refresh both folders and choose the current message; it was not moved again.");
        let legacy:bool=tx.query_row("SELECT moved=1 AND NOT EXISTS(SELECT 1 FROM move_receipts WHERE id=?1) FROM mail WHERE id=?1",[&message.id],|r|r.get(0))?;
        anyhow::ensure!(!legacy,"This older move has no saved identity. Refresh its destination and choose the current message.");
        if account.protocol==Protocol::Imap && !message.remote_id.starts_with("local-") {
            if !has_password {
                tx.execute("UPDATE individual_mail_actions SET status='waiting',error='Reconnect this account to continue.' WHERE id=?1",[&action_id])?;
            }
            tx.commit()?;
            return Ok(if has_password {None} else {Some(if report {json!({"action_id":action_id,"status":"waiting","requires_credentials":account.id})} else {json!({"requires_credentials":account.id})})});
        }
        mark_local_sent_edit(&tx,&message)?;
        acknowledged_mail_write(&tx,&message.id,|| Ok(tx.execute("UPDATE mail SET folder=COALESCE(?2,folder),unread=COALESCE(?3,unread),starred=COALESCE(?4,starred) WHERE id=?1",params![message.id,folder,unread,starred])?))?;
        tx.execute("UPDATE individual_mail_actions SET status='succeeded',error=NULL WHERE id=?1",[&action_id])?;
        tx.commit()?;
        Ok(Some(if report {json!({"action_id":action_id,"status":"succeeded","committed":true})} else {json!({"committed":true})}))
    }).await?;
    if let Some(result) = local {
        return Ok(result);
    }
    if mutation.intent {
        let slot = mutation.credential_slot.clone();
        let account = mutation.id.clone();
        let binding = db
            .read(move |db| {
                let mail = stored_mail(db, &account)?;
                crate::connections::check_binding(db, &mail.account_id, slot.as_deref())
            })
            .await;
        if let Err(error) = binding {
            let waiting = mutation.action_id.clone();
            let saved_action = waiting.clone();
            let message = format!("{error:#}");
            let saved = message.clone();
            db.write(move |db| {
                db.execute(
                    "UPDATE individual_mail_actions SET status='waiting',error=?2 WHERE id=?1 AND status IN ('queued','waiting')",
                    params![&saved_action, saved],
                )?;
                Ok(())
            })
            .await?;
            return Ok(
                json!({"action_id":waiting,"status":"waiting","committed":true,"warning":message}),
            );
        }
    }
    let final_action = mutation.action_id.clone();
    let result = network(
        profile,
        Request::Mutate {
            action_id: mutation.intent.then(|| mutation.action_id.clone()),
            observed_lineage: None,
            require_observation: false,
            credential_slot: mutation.credential_slot,
            id: mutation.id,
            password: mutation.password,
            folder: mutation.folder,
            unread: mutation.unread,
            starred: mutation.starred,
        },
    )
    .await;
    if !mutation.intent {
        return result;
    }
    if let Ok(value) = &result
        && value.get("status").is_some()
    {
        return result;
    }
    let status_before_result = {
        let saved = final_action.clone();
        db.read(move |db| {
            Ok(db.query_row(
                "SELECT status FROM individual_mail_actions WHERE id=?1",
                [saved],
                |row| row.get::<_, String>(0),
            )?)
        })
        .await?
    };
    let (status, error) = match &result {
        Ok(value) if value.get("warning").is_some() => (
            "repair",
            value
                .get("warning")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
        Ok(_) => ("succeeded", None),
        Err(error) if status_before_result == "repair" => ("repair", Some(format!("{error:#}"))),
        Err(error)
            if error.chain().any(|cause| cause.is::<ClaimRejected>())
                || provider_definitely_refused(error) =>
        {
            ("rejected", Some(format!("{error:#}")))
        }
        Err(error) if matches!(status_before_result.as_str(), "queued" | "waiting") => {
            ("waiting", Some(format!("{error:#}")))
        }
        Err(error) => ("uncertain", Some(format!("{error:#}"))),
    };
    let saved_id = final_action.clone();
    let saved_error = error.clone();
    db.write(move |db| {
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE individual_mail_actions SET status=?2,error=?3 WHERE id=?1",
            params![&saved_id, status, saved_error],
        )?;
        if status == "rejected" {
            release_action_intent(&tx, &saved_id)?;
        }
        tx.commit()?;
        Ok(())
    })
    .await?;
    match result {
        Ok(mut value) => {
            if report && let Some(object) = value.as_object_mut() {
                object.insert("action_id".into(), json!(final_action));
                object.insert("status".into(), json!(status));
            }
            Ok(value)
        }
        Err(original) if status == "rejected" => Err(original),
        Err(_) => Ok(
            json!({"action_id":final_action,"status":status,"committed":true,"warning":error.expect("failed actions have an error")}),
        ),
    }
}
pub(crate) fn delivery(db: &Connection, id: &str) -> Result<Value> {
    let value = db
        .query_row(
            "SELECT id,state,message_id FROM outgoing WHERE draft_id=?1",
            [id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    Ok(match value {
        Some((id, state, message_id)) => json!({"id":id,"state":state,"message_id":message_id}),
        None => Value::Null,
    })
}
async fn network(profile: &MobileProfile, request: Request) -> Result<Value> {
    let operations = &profile.operations;
    let _admission = operations
        .admitted
        .clone()
        .try_acquire_owned()
        .context("Mail is busy. Keep browsing and retry this action shortly.")?;
    let slot = operations.slots.clone().acquire_owned().await?;
    let db = &profile.database;
    match request {
        Request::Probe {
            account,
            password,
            smtp,
        } => {
            account.validate()?;
            let result = tokio::time::timeout(Duration::from_secs(45), async {
                if smtp {
                    mail::test_smtp(&account, &password).await
                } else {
                    mail::test_incoming(&account, &password).await
                }
            })
            .await
            .unwrap_or_else(|_| Err(anyhow::anyhow!("Connection timed out")));
            result.map_err(|_|anyhow::anyhow!("Could not verify the connection. Check the hostname, TLS, username and password, then retry."))?;
            Ok(json!({"connected":true,"sent":false}))
        }
        Request::Sync {
            account,
            password,
            credential_slot,
        } => {
            let _guard = operations.account(&account).await;
            let (account, known) = db
                .read(move |db| {
                    crate::connections::check_binding(db,&account,credential_slot.as_deref())?;
                    let settings = stored_account(db, &account)?;
                    let mut statement = db.prepare(if settings.protocol == Protocol::Pop3 {"SELECT id FROM mail WHERE account_id=?1"} else {"SELECT account_id || ':' || folder || ':' || remote_id FROM mail WHERE account_id=?1 AND moved=0"})?;
                    let known = statement
                        .query_map([account], |r| r.get::<_, String>(0))?
                        .collect::<rusqlite::Result<HashSet<_>>>()?;
                    Ok((settings, known))
                })
                .await?;
            let pop = account.protocol == Protocol::Pop3;
            let account_id = account.id.clone();
            let (events, mut receive) = mpsc::channel(8);
            let provider = operations.mail_provider(account.protocol);
            let mut task =
                tokio::spawn(
                    async move { provider.sync(&account, &password, &known, events).await },
                );
            let mut reconcile = Vec::new();
            let mut skipped = 0;
            loop {
                let event =
                    match tokio::time::timeout(Duration::from_secs(45), receive.recv()).await {
                        Ok(Some(event)) => event,
                        Ok(None) => break,
                        Err(_) => {
                            task.abort();
                            let _ = task.await;
                            anyhow::bail!(
                                "Mail stopped responding. Your cache was kept; retry Refresh."
                            );
                        }
                    };
                let saved=match event {
                    MailSyncItem::Message(mail)=>db.write(move|db|insert_mail(db,mail,pop)).await,
                    MailSyncItem::Flags(flags)=>db.write(move|db|{
                        if !pop {let tx=db.transaction()?;for(id,unread,starred)in flags{tx.execute("UPDATE mail SET unread=?2,starred=?3 WHERE account_id || ':' || folder || ':' || remote_id = ?1 AND moved=0",params![id,unread,starred])?;}tx.commit()?;}Ok(())
                    }).await,
                    // Mobile caches selectable names only; the hierarchy is a desktop feature.
                    MailSyncItem::Folders(account,folders)=>db.write(move|db|{let names:Vec<&str>=folders.iter().filter(|folder|folder.selectable).map(|folder|folder.name.as_str()).collect();db.execute("INSERT INTO folders VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET names=excluded.names",params![account,serde_json::to_string(&names)?])?;Ok(())}).await,
                    MailSyncItem::InboxSyncStarted{..}|MailSyncItem::InboxSyncFinished{..}=>Ok(()),
                    MailSyncItem::Reconcile{account,folder,live_ids}=>{reconcile.push((account,folder,live_ids));Ok(())},
                    MailSyncItem::SkippedLarge=>{skipped+=1;Ok(())},
                    MailSyncItem::SentFolder(account,folder)=>db.write(move|db|{
                        db.execute("INSERT INTO discovered_sent(account_id,folder) VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET folder=excluded.folder",params![account,folder])?;Ok(())
                    }).await,
                };
                if let Err(error) = saved {
                    task.abort();
                    let _ = task.await;
                    return Err(error);
                }
            }
            let result = (&mut task).await;
            anyhow::ensure!(
                matches!(result, Ok(Ok(_))),
                "Mail sync did not finish. Cached mail was kept; check the connection and retry."
            );
            if !pop {
                db.write(move |db| {
                    let tx = db.transaction()?;
                    for (account, folder, live) in reconcile {
                        reconcile_folder(&tx, &account, &folder, &live)?;
                    }
                    tx.commit()?;
                    Ok(())
                })
                .await?;
            }
            Ok(json!({"synced":true,"account":account_id,"skipped_large":skipped}))
        }
        Request::Mutate {
            action_id,
            observed_lineage: _,
            require_observation: _,
            credential_slot,
            id,
            password,
            mut folder,
            mut unread,
            mut starred,
        } => {
            let identity = id.clone();
            let account = db
                .read(move |db| {
                    let mail = stored_mail(db, &identity)?;
                    stored_account(db, &mail.account_id)
                })
                .await?;
            #[cfg(test)]
            operations.mutation_waiting.notify_one();
            let _guard = operations.account(&account.id).await;
            let identity = id.clone();
            let (account, mut message, previous) = db
                .read(move |db| {
                    let message = stored_mail(db, &identity)?;
                    let identity = message.id.clone();
                    let pending:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM pending_moves WHERE id=?1)",[&identity],|r|r.get(0))?;
                    anyhow::ensure!(!pending,"A previous move has no saved acknowledgment. Refresh both folders and choose the current message; it was not moved again.");
                    let receipt = db
                        .query_row(
                            "SELECT receipt FROM move_receipts WHERE id=?1",
                            [&identity],
                            |r| r.get::<_, String>(0),
                        )
                        .optional()?;
                    let moved:bool=db.query_row("SELECT moved FROM mail WHERE id=?1",[&identity],|r|r.get(0))?;
                    anyhow::ensure!(!moved || receipt.is_some(),"This older move has no saved identity. Refresh its destination and choose the current message.");
                    Ok((
                        stored_account(db, &message.account_id)?,
                        message,
                        receipt
                            .map(|r| serde_json::from_str::<MoveReceipt>(&r))
                            .transpose()?,
                    ))
                })
                .await?;
            anyhow::ensure!(
                account.protocol == Protocol::Pop3
                    || folder.is_none()
                    || (unread.is_none() && starred.is_none()),
                "Move and flag changes must be separate actions."
            );
            let remote =
                account.protocol == Protocol::Imap && !message.remote_id.starts_with("local-");
            let id = message.id.clone();
            // Credential discovery is an observation, not a partial mutation.
            // Decide from the locked, current remote identity: a local Sent
            // copy can become a provider message between Flutter page loads.
            // Returning here leaves journals and flags untouched, so Flutter
            // may obtain the account's credential and make the same request.
            if remote && password.is_none() {
                return Ok(json!({"requires_credentials":account.id}));
            }
            if remote {
                let id = account.id.clone();
                let binding = credential_slot.clone();
                db.read(move |db| crate::connections::check_binding(db, &id, binding.as_deref()))
                    .await?;
            }
            if remote
                && let Some(receipt) = previous
                && receipt.current.as_ref().is_none_or(|current| {
                    current.account_id != message.account_id
                        || current.folder != message.folder
                        || current.remote_id != message.remote_id
                })
            {
                let password = password
                    .as_ref()
                    .context("Reconnect this account in Preferences.")?;
                let recovered = tokio::time::timeout(
                    Duration::from_secs(45),
                    mail::recovery::resolve(&account, password, &receipt),
                )
                .await
                .context("Message lookup timed out. Refresh its destination before retrying.")??;
                let identity = id.clone();
                let mut resolved = recovered.summary;
                resolved.id = identity.clone();
                let saved = resolved.clone();
                db.write(move |db| resolve_cached_move(db, &identity, &saved))
                    .await?;
                message = resolved;
            }
            // A queued Undo after a rejected move is already at its destination.
            if unread.is_none()
                && starred.is_none()
                && folder.as_ref().is_some_and(|f| *f == message.folder)
            {
                return Ok(json!({"committed":true}));
            }
            let fingerprint = if remote && folder.is_some() {
                let identity = id.clone();
                Some(
                    db.read(move |db| {
                        let raw: Vec<u8> =
                            db.query_row("SELECT raw FROM mail WHERE id=?1", [identity], |r| {
                                r.get(0)
                            })?;
                        Ok(Fingerprint::of(&raw))
                    })
                    .await?,
                )
            } else {
                None
            };
            let dispatch_lineage = if action_id.is_some() {
                let identity = id.clone();
                Some(
                    db.read(move |db| {
                        Ok(db.query_row(
                            "SELECT token FROM mail_lineage WHERE id=?1",
                            [identity],
                            |row| row.get::<_, String>(0),
                        )?)
                    })
                    .await?,
                )
            } else {
                None
            };
            let durable_action = action_id.clone();
            if let Some(action_id) = action_id {
                let source = message.clone();
                let slot = credential_slot.clone();
                let dispatch_physical = serde_json::to_string(
                    &json!({"account":source.account_id,"folder":source.folder,"remote_id":source.remote_id,"unread":source.unread,"starred":source.starred,"fingerprint":fingerprint.clone(),"lineage":dispatch_lineage}),
                )?;
                let claim_fingerprint = fingerprint.clone();
                let claim_lineage = dispatch_lineage.clone();
                let claimed = db.write(move |db| {
                    let tx = db.transaction()?;
                    let current = stored_mail(&tx, &source.id)?;
                    let current_lineage: String = tx.query_row(
                        "SELECT token FROM mail_lineage WHERE id=?1",
                        [&source.id],
                        |row| row.get(0),
                    )?;
                    if !(current.account_id == source.account_id
                            && current.folder == source.folder
                            && current.remote_id == source.remote_id
                            && claim_lineage.as_deref() == Some(current_lineage.as_str())) {
                        anyhow::bail!(ClaimRejected);
                    }
                    if let Some(expected) = claim_fingerprint.as_ref() {
                        let raw: Vec<u8> = tx.query_row(
                            "SELECT raw FROM mail WHERE id=?1",
                            [&source.id],
                            |row| row.get(0),
                        )?;
                        if !expected.matches(&raw) {
                            anyhow::bail!(ClaimRejected);
                        }
                    }
                    let (status, physical, fields, revision):(String,String,String,i64)=tx.query_row(
                        "SELECT status,physical,fields,intent_revision FROM individual_mail_actions WHERE id=?1",
                        [&action_id], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
                    if !matches!(status.as_str(),"queued"|"waiting") {
                        let warning = matches!(status.as_str(),"running"|"repair"|"uncertain").then_some("This action already has an execution or recovery result.");
                        return Ok((Some(json!({"action_id":action_id,"status":status,"committed":matches!(status.as_str(),"running"|"succeeded"|"repair"|"uncertain"),"warning":warning})),None));
                    }
                    let frozen:Value=serde_json::from_str(&physical)?;
                    if !action_source_matches(&tx,&source,&frozen)? {
                        anyhow::bail!(ClaimRejected);
                    }
                    let fields:Value=serde_json::from_str(&fields)?;
                    let owned = |field: &str| -> Result<bool> {
                        Ok(tx.query_row("SELECT revision=?3 FROM mail_intents WHERE mail=?1 AND field=?2",params![source.id,field,revision],|row|row.get::<_,bool>(0)).optional()?.unwrap_or(false))
                    };
                    let accepted_folder=if owned("folder")? {fields.get("folder").and_then(Value::as_str).map(str::to_owned)} else {None};
                    let accepted_unread=if owned("unread")? {fields.get("unread").and_then(Value::as_bool)} else {None};
                    let accepted_starred=if owned("starred")? {fields.get("starred").and_then(Value::as_bool)} else {None};
                    if accepted_folder.is_none() && accepted_unread.is_none() && accepted_starred.is_none() {
                        tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE id=?1",[&action_id])?;
                        release_action_intent(&tx,&action_id)?;
                        tx.commit()?;
                        return Ok((Some(json!({"action_id":action_id,"status":"cancelled","committed":false})),None));
                    }
                    let accepted=serde_json::to_string(&json!({"folder":&accepted_folder,"unread":accepted_unread,"starred":accepted_starred}))?;
                    tx.execute("UPDATE individual_mail_actions SET status='running',credential_slot=COALESCE(?2,credential_slot),physical=?3,accepted_fields=?4,error=NULL WHERE id=?1",params![action_id,slot,dispatch_physical,accepted])?;
                    tx.commit()?;
                    Ok((None,Some((accepted_folder,accepted_unread,accepted_starred))))
                }).await?;
                if let Some(result) = claimed.0 {
                    return Ok(result);
                }
                let accepted = claimed
                    .1
                    .context("The accepted mail fields were not saved.")?;
                (folder, unread, starred) = accepted;
            }
            if remote && let Some(destination) = folder.clone() {
                password
                    .as_ref()
                    .context("Reconnect this account in Preferences.")?;
                let identity = id.clone();
                db.write(move |db| {
                    db.execute(
                        "INSERT INTO pending_moves(id,destination) VALUES(?1,?2)",
                        params![identity, destination],
                    )?;
                    Ok(())
                })
                .await?;
            }
            let remote_id = if remote {
                let password = password
                    .as_ref()
                    .context("Reconnect this account in Preferences.")?;
                let provider = operations.mail_provider(account.protocol);
                let operation = async {
                    if let Some(folder) = &folder {
                        provider
                            .move_mail(&account, password, &message, folder)
                            .await
                    } else {
                        provider
                            .set_flags(&account, password, &message, Flags { unread, starred })
                            .await
                            .map(|_| None)
                    }
                };
                let result = tokio::time::timeout(Duration::from_secs(45), operation)
                    .await
                    .context("The server did not confirm this change. Refresh its folders before another action.")?;
                match result {
                    Ok(remote_id) => remote_id,
                    Err(error) => {
                        if provider_definitely_refused(&error) && folder.is_some() {
                            let identity = id.clone();
                            db.write(move |db| {
                                db.execute("DELETE FROM pending_moves WHERE id=?1", [identity])?;
                                Ok(())
                            })
                            .await?;
                        }
                        return Err(error);
                    }
                }
            } else {
                None
            };
            let receipt = fingerprint.map(|fingerprint| {
                MoveReceipt::server(
                    &message,
                    &account.id,
                    folder.as_ref().unwrap(),
                    remote_id.clone(),
                    fingerprint,
                )
            });
            let unresolved = receipt.is_some() && remote_id.is_none();
            if let Some(action) = durable_action.as_deref() {
                let acknowledged = receipt
                    .clone()
                    .map(|receipt| IndividualReceipt::Move {
                        receipt: Box::new(receipt),
                    })
                    .unwrap_or(IndividualReceipt::Flags);
                let saved_action = action.to_owned();
                db.write(move |db| save_individual_receipt(db, &saved_action, &acknowledged))
                    .await?;
            }
            let identity = id.clone();
            let pending_receipt = receipt.clone();
            let saved = if let Some(action) = durable_action {
                db.write(move |db| {
                    anyhow::ensure!(
                        apply_individual_receipt(db, &action)? != ReceiptApplication::Missing,
                        "The saved provider acknowledgment no longer matches this action."
                    );
                    Ok(())
                })
                .await
            } else {
                db.write(move |db| {
                    let tx=db.transaction()?;
                    mark_local_sent_edit(&tx,&message)?;
                    if let Some(receipt)=&pending_receipt {
                        save_move(&tx,&identity,receipt)?;
                    } else {
                        acknowledged_mail_write(&tx,&identity,|| Ok(tx.execute("UPDATE mail SET folder=COALESCE(?2,folder),unread=COALESCE(?3,unread),starred=COALESCE(?4,starred) WHERE id=?1",params![identity,folder,unread,starred])?))?;
                    }
                    tx.commit()?; Ok(())
                }).await
            };
            if remote && saved.is_err() {
                return Ok(
                    json!({"committed":true,"warning":"The server acknowledged the change, but the cache could not save it. Refresh before another action."}),
                );
            }
            saved?;
            if unresolved {
                let receipt = receipt.unwrap();
                if let Ok(Ok(recovered)) = tokio::time::timeout(
                    Duration::from_secs(45),
                    mail::recovery::resolve(&account, password.as_ref().unwrap(), &receipt),
                )
                .await
                {
                    let identity = id.clone();
                    let resolved = recovered.summary;
                    if db
                        .write(move |db| resolve_cached_move(db, &identity, &resolved))
                        .await
                        .is_ok()
                    {
                        return Ok(json!({"committed":true}));
                    }
                }
                return Ok(
                    json!({"committed":true,"warning":"The message moved, but its destination identity needs recovery. Refresh the destination before another action."}),
                );
            }
            Ok(json!({"committed":true}))
        }
        Request::SentOutgoing {
            credential_slot,
            id,
            copy,
            confirmed,
            password,
        } => crate::sent::recover(profile, id, copy, confirmed, password, credential_slot).await,
        Request::Send {
            attempt,
            credential_slot,
            password,
            incoming_password,
        } => {
            crate::outgoing::send(
                profile,
                attempt,
                (password, incoming_password, credential_slot),
                slot,
                _admission,
            )
            .await
        }
        _ => unreachable!(),
    }
}
