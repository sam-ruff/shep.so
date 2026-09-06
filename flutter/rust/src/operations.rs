use crate::api::MobileProfile;
use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use shep_mail_core::{
    mail_actions::{Fingerprint, Flags, MoveReceipt},
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
    admitted: Arc<Semaphore>,
    slots: Arc<Semaphore>,
    accounts: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    pub(crate) outgoing: crate::outgoing::Runtime,
    pub(crate) sent: crate::sent::Runtime,
    #[cfg(test)]
    pub(crate) provider: std::sync::Mutex<Option<Arc<dyn shep_mail_core::providers::MailProvider>>>,
    #[cfg(test)]
    pub(crate) mutation_waiting: tokio::sync::Notify,
}
impl Operations {
    pub fn new() -> Self {
        Self {
            admitted: Arc::new(Semaphore::new(40)),
            slots: Arc::new(Semaphore::new(8)),
            accounts: Mutex::new(HashMap::new()),
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
    Accounts,
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
    },
    Detail {
        id: String,
    },
    Sync {
        account: String,
        password: SecretString,
    },
    Mutate {
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
    SaveDraft {
        draft: Draft,
    },
    DiscardDraft {
        id: String,
        revision: u64,
    },
    Send {
        id: String,
        revision: u64,
        #[serde(default)]
        file_revision: u64,
        password: SecretString,
        #[serde(default)]
        incoming_password: Option<SecretString>,
    },
    OutgoingAccount {
        id: String,
    },
    SentOutgoing {
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
fn summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<Mail> {
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
const SUMMARY: &str = "id,account_id,remote_id,folder,sender,recipient,subject,preview,timestamp,unread,starred,attachment_count";
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
        db.execute(
            "UPDATE mail_aliases SET id=?2 WHERE id=?1",
            params![other, id],
        )?;
        db.execute("DELETE FROM mail WHERE id=?1", [other])?;
    }
    Ok(())
}
pub(crate) fn save_move(db: &Connection, id: &str, receipt: &MoveReceipt) -> Result<()> {
    let remote = receipt.current.as_ref().map(|m| m.remote_id.as_str());
    if let Some(remote) = remote {
        deduplicate_move(db, id, &receipt.account, &receipt.folder, remote)?;
    }
    db.execute(
        "UPDATE mail SET folder=?2,remote_id=COALESCE(?3,remote_id),moved=?4 WHERE id=?1",
        params![id, receipt.folder, remote, remote.is_none()],
    )?;
    db.execute("DELETE FROM pending_moves WHERE id=?1", [id])?;
    db.execute("INSERT INTO move_receipts(id,receipt) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET receipt=excluded.receipt",params![id,serde_json::to_string(receipt)?])?;
    Ok(())
}
pub(crate) fn resolve_cached_move(db: &mut Connection, id: &str, mail: &Mail) -> Result<()> {
    let tx = db.transaction()?;
    deduplicate_move(&tx, id, &mail.account_id, &mail.folder, &mail.remote_id)?;
    tx.execute(
        "UPDATE mail SET folder=?2,remote_id=?3,unread=?4,starred=?5,moved=0 WHERE id=?1",
        params![id, mail.folder, mail.remote_id, mail.unread, mail.starred],
    )?;
    tx.execute("DELETE FROM move_receipts WHERE id=?1", [id])?;
    tx.commit()?;
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
fn positive_revision(value: u64) -> Result<i64> {
    i64::try_from(value).context("Draft revision is invalid.")
}
async fn value<T: serde::Serialize>(result: Result<T>) -> Result<Value> {
    Ok(serde_json::to_value(result?)?)
}

pub async fn run(profile: &MobileProfile, request: Request) -> Result<Value> {
    let db = &profile.database;
    match request {
        Request::Accounts => value(db.read(|db| {
            let mut statement=db.prepare("SELECT settings FROM accounts ORDER BY id")?;
            let accounts=statement.query_map([], |r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            let accounts=accounts.into_iter().map(|s|serde_json::from_str::<Account>(&s)).collect::<std::result::Result<Vec<_>,_>>()?;
            let mut folders=db.prepare("SELECT account_id,names FROM folders")?;
            let folders=folders.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?.into_iter().map(|(id,s)|Ok((id,serde_json::from_str::<Vec<String>>(&s)?))).collect::<Result<HashMap<_,_>>>()?;
            Ok(json!({"accounts":accounts,"folders":folders}))
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
                if let Ok(old)=stored_account(db,&account.id) {
                    if preserve_sent {account.sent_copy=old.sent_copy;account.sent_folder=old.sent_folder.clone();}
                    anyhow::ensure!(old.host==account.host && old.port==account.port && old.username==account.username && old.protocol==account.protocol,"Add changed incoming server settings as a new account to preserve cached identities.");
                }
                db.execute("INSERT INTO accounts(id,settings) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET settings=excluded.settings",params![account.id,serde_json::to_string(&account)?])?;
                Ok(())
            }).await?;
            Ok(json!({"saved":true}))
        }
        Request::Page{folder,account,query,filter,oldest,offset} => db.read(move |db| {
            let folder=if folder.eq_ignore_ascii_case("Inbox") {"INBOX".to_owned()} else {folder};
            let words=query.split_whitespace().map(|w|format!("\"{}\"*",w.replace('"',"\"\""))).collect::<Vec<_>>().join(" AND ");
            let folder_condition=if folder=="Sent" {"(folder=?1 OR (account_id,folder) IN (SELECT account_id,folder FROM sent_folder_names))"} else {"folder=?1"};
            let conditions=format!("moved=0 AND {folder_condition} AND (?2 IS NULL OR account_id=?2) AND (?3!='Unread' OR unread=1) AND (?3!='Flagged' OR starred=1) AND (?4='' OR rowid IN (SELECT rowid FROM mail_search WHERE mail_search MATCH ?4))");
            let total:i64=db.query_row(&format!("SELECT COUNT(*) FROM mail WHERE {conditions}"),params![folder,account,filter,words],|r|r.get(0))?;
            let order=if oldest {"ASC"}else{"DESC"};
            let mut rows=db.prepare(&format!("SELECT {SUMMARY} FROM mail WHERE {conditions} ORDER BY timestamp {order},id LIMIT 50 OFFSET ?5"))?;
            let mail=rows.query_map(params![folder,account,filter,words,offset],summary)?.collect::<rusqlite::Result<Vec<_>>>()?;
            let unread:i64=db.query_row("SELECT COUNT(*) FROM mail WHERE moved=0 AND folder='INBOX' AND unread=1",[],|r|r.get(0))?;
            let mut aliases=std::collections::BTreeMap::new();
            let mut alias_query=db.prepare("SELECT alias,id FROM mail_aliases WHERE id=?1")?;
            for message in &mail {
                for row in alias_query.query_map([&message.id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))? {
                    let (alias,id)=row?; aliases.insert(alias,id);
                }
            }
            let mut folder_membership:std::collections::BTreeMap<String,HashSet<String>>=Default::default();
            if folder=="Sent" { for message in &mail {folder_membership.entry(message.account_id.clone()).or_default().insert(message.folder.clone());} }
            Ok(json!({"mail":mail,"total":total,"unread":unread,"aliases":aliases,"folder_membership":folder_membership}))
        }).await,
        Request::Detail{id} => db.read(move |db| {
            let summary=stored_mail(db,&id)?;
            let (text,raw):(String,Vec<u8>)=db.query_row("SELECT body,raw FROM mail WHERE id=?1",[&summary.id],|r|Ok((r.get(0)?,r.get(1)?)))?;
            let parsed=mailparse::parse_mail(&raw)?;
            let (_,files)=content(&parsed);
            Ok(json!({"summary":summary,"body":text,"attachments":files.iter().map(|f|f.name.clone()).collect::<Vec<_>>()}))
        }).await,
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
        Request::SaveDraft{draft} => {
            db.write(move |db| {
                let revision=positive_revision(draft.revision)?;
                let delivered:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing WHERE draft_id=?1) OR EXISTS(SELECT 1 FROM discarded_drafts WHERE id=?1)",[&draft.id],|r|r.get(0))?;
                anyhow::ensure!(!delivered,"This draft has been submitted or discarded. Its delivery/discard record was preserved.");
                db.execute("INSERT INTO drafts(id,revision,content) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET revision=excluded.revision,content=excluded.content WHERE excluded.revision>=drafts.revision",params![draft.id,revision,serde_json::to_string(&draft)?])?;
                Ok(())
            }).await?;
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
        Request::OutgoingAccount{id} => db.read(move |db| Ok(json!({"account_id":db.query_row("SELECT account_id FROM outgoing WHERE id=?1",[id],|r|r.get::<_,String>(0))?}))).await,
        Request::Delivery{id} => crate::outgoing::delivery(profile, id).await,
        Request::Outbox{offset} => crate::outgoing::page(profile, offset).await,
        Request::RecoverOutgoing{id,action,confirmed} => crate::outgoing::recover(profile,id,action,confirmed).await,
        request @ Request::Mutate{..} => {
            if let Request::Mutate{id,password,folder,unread,starred} = &request {
                let (id,folder,unread,starred,has_password)=(id.clone(),folder.clone(),*unread,*starred,password.is_some());
                let local=db.write(move |db| {
                    // A local edit and the handover eligibility marker commit
                    // together, before any account lock or provider admission.
                    let tx=db.transaction()?;
                    let message=stored_mail(&tx,&id)?;
                    let account=stored_account(&tx,&message.account_id)?;
                    let pending:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM pending_moves WHERE id=?1)",[&message.id],|r|r.get(0))?;
                    anyhow::ensure!(!pending,"A previous move has no saved acknowledgment. Refresh both folders and choose the current message; it was not moved again.");
                    let legacy:bool=tx.query_row("SELECT moved=1 AND NOT EXISTS(SELECT 1 FROM move_receipts WHERE id=?1) FROM mail WHERE id=?1",[&message.id],|r|r.get(0))?;
                    anyhow::ensure!(!legacy,"This older move has no saved identity. Refresh its destination and choose the current message.");
                    if account.protocol==Protocol::Imap && !message.remote_id.starts_with("local-") {
                        return Ok(if has_password {None} else {Some(json!({"requires_credentials":account.id}))});
                    }
                    anyhow::ensure!(account.protocol==Protocol::Pop3 || folder.is_none() || (unread.is_none() && starred.is_none()),"Move and flag changes must be separate actions.");
                    mark_local_sent_edit(&tx,&message)?;
                    tx.execute("UPDATE mail SET folder=COALESCE(?2,folder),unread=COALESCE(?3,unread),starred=COALESCE(?4,starred) WHERE id=?1",params![message.id,folder,unread,starred])?;
                    tx.commit()?;
                    Ok(Some(json!({"committed":true})))
                }).await?;
                if let Some(result)=local {return Ok(result);}
            }
            network(profile,request).await
        }
        request => network(profile,request).await,
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
        Request::Sync { account, password } => {
            let _guard = operations.account(&account).await;
            let (account, known) = db
                .read(move |db| {
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
                    MailSyncItem::Folders(account,folders)=>db.write(move|db|{db.execute("INSERT INTO folders VALUES(?1,?2) ON CONFLICT(account_id) DO UPDATE SET names=excluded.names",params![account,serde_json::to_string(&folders)?])?;Ok(())}).await,
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
            id,
            password,
            folder,
            unread,
            starred,
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
            if remote && let Some(receipt) = previous {
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
                tokio::time::timeout(Duration::from_secs(45),operation).await.ok().and_then(Result::ok)
                    .context("The server did not confirm this change. Refresh its folders before another action.")?
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
            let identity = id.clone();
            let pending_receipt = receipt.clone();
            let saved=db.write(move |db| {
                let tx=db.transaction()?;
                mark_local_sent_edit(&tx,&message)?;
                if let Some(receipt)=&pending_receipt {
                    save_move(&tx,&identity,receipt)?;
                } else {
                    tx.execute("UPDATE mail SET folder=COALESCE(?2,folder),unread=COALESCE(?3,unread),starred=COALESCE(?4,starred) WHERE id=?1",params![identity,folder,unread,starred])?;
                }
                tx.commit()?; Ok(())
            }).await;
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
            id,
            copy,
            confirmed,
            password,
        } => crate::sent::recover(profile, id, copy, confirmed, password).await,
        Request::Send {
            id,
            password,
            revision,
            file_revision,
            incoming_password,
        } => {
            crate::outgoing::send(
                profile,
                id,
                revision,
                file_revision,
                (password, incoming_password),
                slot,
                _admission,
            )
            .await
        }
        _ => unreachable!(),
    }
}
