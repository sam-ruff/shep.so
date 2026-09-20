//! Exact local submission ownership and explicit delivery recovery.
use crate::{api::MobileProfile, database::Database, operations};
use anyhow::{Context, Result};
use lettre::address::Envelope;
use rusqlite::{Connection, OptionalExtension, params};
use secrecy::SecretString;
use serde::Deserialize;
use serde_json::{Value, json};
use shep_mail_core::{
    model::*,
    providers::mail::{self, DeliveryFailure},
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit};

type SendFuture = Pin<Box<dyn Future<Output = Result<(), DeliveryFailure>> + Send>>;
pub(crate) trait Smtp: Send + Sync {
    fn send(
        &self,
        account: Account,
        password: SecretString,
        envelope: Envelope,
        raw: Vec<u8>,
    ) -> SendFuture;
}
struct Server;
impl Smtp for Server {
    fn send(
        &self,
        account: Account,
        password: SecretString,
        envelope: Envelope,
        raw: Vec<u8>,
    ) -> SendFuture {
        Box::pin(async move { mail::send_raw(&account, &password, &envelope, &raw).await })
    }
}
pub(crate) struct Runtime {
    phases: AsyncMutex<HashMap<String, &'static str>>,
    smtp: Mutex<Arc<dyn Smtp>>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            phases: AsyncMutex::new(HashMap::new()),
            smtp: Mutex::new(Arc::new(Server)),
        }
    }
}
impl Runtime {
    #[cfg(test)]
    pub(crate) fn set_smtp(&self, smtp: Arc<dyn Smtp>) {
        *self.smtp.lock().unwrap() = smtp;
    }
}
#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Recovery {
    Check,
    Return,
    Mark,
    Local,
}

pub(crate) async fn delivery(profile: &MobileProfile, id: String) -> Result<Value> {
    let mut result = profile.database.read(move |db| {
        let mut value = operations::delivery(db,&id)?;
        if let Some(attempt) = value["id"].as_str() {
            let reviewed:bool = db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing_meta WHERE id=?1 AND recovery IS NOT NULL)",[attempt],|r|r.get(0))?;
            if reviewed {value["state"]=json!("reviewed");}
        }
        Ok(value)
    }).await?;
    if let Some(id) = result["id"].as_str()
        && result["state"] != "reviewed"
        && let Some(phase) = profile.operations.outgoing.phases.lock().await.get(id)
    {
        result["state"] = json!(phase);
    }
    Ok(result)
}
pub(crate) async fn page(profile: &MobileProfile, offset: u32) -> Result<Value> {
    let mut page = profile.database.read(move |db| {
        let total:u32=db.query_row("SELECT COUNT(*) FROM outgoing o LEFT JOIN outgoing_meta m ON m.id=o.id LEFT JOIN outgoing_sent s ON s.id=o.id LEFT JOIN accounts a ON a.id=o.account_id WHERE (m.recovery IS NULL OR (m.recovery='marked' AND s.id IS NOT NULL)) AND COALESCE(s.complete,0)=0",[],|r|r.get(0))?;
        let offset=offset.min(total.saturating_sub(1)/20*20)/20*20;
        let mut query=db.prepare("SELECT o.id,o.draft_id,o.account_id,o.message_id,o.state,json_extract(o.draft,'$.subject'),json_extract(o.draft,'$.to'),json_extract(o.draft,'$.cc'),m.created,COALESCE(m.from_address,''),s.state,s.error,json_extract(s.account,'$.protocol'),COALESCE(json_extract(a.settings,'$.sent_copy'),json_extract(s.account,'$.sent_copy')),m.recovery FROM outgoing o LEFT JOIN outgoing_meta m ON m.id=o.id LEFT JOIN outgoing_sent s ON s.id=o.id LEFT JOIN accounts a ON a.id=o.account_id WHERE (m.recovery IS NULL OR (m.recovery='marked' AND s.id IS NOT NULL)) AND COALESCE(s.complete,0)=0 ORDER BY o.rowid DESC LIMIT 20 OFFSET ?1")?;
        let rows=query.query_map([offset],|r| {
            let to:String=r.get(6)?; let cc:String=r.get(7)?;
            Ok(json!({"id":r.get::<_,String>(0)?,"draft_id":r.get::<_,String>(1)?,"account_id":r.get::<_,String>(2)?,"message_id":r.get::<_,String>(3)?,"state":r.get::<_,String>(4)?,"subject":r.get::<_,String>(5)?,"to":if !to.is_empty(){to}else if !cc.is_empty(){cc}else{"Recipients in Bcc".into()},"created":r.get::<_,Option<i64>>(8)?,"from":r.get::<_,String>(9)?,"sent":r.get::<_,Option<String>>(10)?,"sent_error":r.get::<_,Option<String>>(11)?,"protocol":r.get::<_,Option<String>>(12)?,"sent_policy":r.get::<_,Option<String>>(13)?,"marked":r.get::<_,Option<String>>(14)?.as_deref()==Some("marked")}))
        })?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(json!({"rows":rows,"total":total,"offset":offset}))
    }).await?;
    let phases = profile.operations.outgoing.phases.lock().await;
    for row in page["rows"].as_array_mut().unwrap() {
        if let Some(phase) = phases.get(row["id"].as_str().unwrap()) {
            row["state"] = json!(phase);
        }
    }
    drop(phases);
    let receipts = profile.operations.sent.receipts.lock().await;
    for row in page["rows"].as_array_mut().unwrap() {
        if receipts.contains_key(row["id"].as_str().unwrap()) {
            row["sent"] = json!("saved");
        }
    }
    Ok(page)
}
fn saved_draft(draft: &Draft) -> Result<String> {
    let mut value = serde_json::to_value(draft)?;
    // Text autosave excludes file associations. The immutable submission must
    // keep its own association snapshot for recovery validation.
    value["attachments"] = serde_json::to_value(&draft.attachments)?;
    Ok(serde_json::to_string(&value)?)
}
fn saved_submission_draft(
    draft: &Draft,
    envelope: &Envelope,
    credential_slot: &str,
) -> Result<String> {
    let mut value: Value = serde_json::from_str(&saved_draft(draft)?)?;
    value["_delivery_credential_slot"] = json!(credential_slot);
    value["_delivery_envelope"] = serde_json::to_value(shep_mail_core::outgoing::EnvelopeData {
        from: envelope
            .from()
            .context("The sending identity is missing.")?
            .to_string(),
        to: envelope.to().iter().map(ToString::to_string).collect(),
    })?;
    Ok(serde_json::to_string(&value)?)
}
pub(crate) fn local_sent(db: &mut Connection, id: &str, recovery: Option<&str>) -> Result<()> {
    let (account, draft, raw): (String, String, Vec<u8>) = db.query_row(
        "SELECT account_id,draft_id,raw FROM outgoing WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let identity = format!("{account}:Sent:local-sent-{id}");
    let tx = db.transaction()?;
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM mail WHERE id=?1)",
        [&identity],
        |r| r.get(0),
    )?;
    if !exists {
        let local = parse_mail(
            &account,
            &format!("local-sent-{id}"),
            "Sent",
            raw,
            false,
            false,
        )?;
        operations::insert_mail(&tx, local, false)?;
    }
    tx.execute("DELETE FROM drafts WHERE id=?1", [draft])?;
    if let Some(action) = recovery {
        tx.execute("INSERT INTO outgoing_meta(id,recovery) VALUES(?1,?2) ON CONFLICT(id) DO UPDATE SET recovery=excluded.recovery",params![id,action])?;
    }
    tx.commit()?;
    Ok(())
}
async fn persist_phase(db: &Arc<Database>, id: String, phase: &'static str) -> Result<()> {
    db.write(move |db| {
        db.execute(
            "UPDATE outgoing SET state=?2 WHERE id=?1 AND state='submitting'",
            params![id, phase],
        )?;
        let saved: String =
            db.query_row("SELECT state FROM outgoing WHERE id=?1", [id], |r| r.get(0))?;
        anyhow::ensure!(
            saved == phase,
            "This delivery record changed. Reopen Outbox before making a decision."
        );
        Ok(())
    })
    .await
}
pub(crate) async fn recover(
    profile: &MobileProfile,
    id: String,
    action: Recovery,
    confirmed: bool,
) -> Result<Value> {
    let identity = id.clone();
    let account = profile
        .database
        .read(move |db| {
            Ok(db.query_row(
                "SELECT account_id FROM outgoing WHERE id=?1",
                [identity],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await?;
    // Never queue a recovery decision behind SMTP. Navigation remains usable.
    let _guard = profile.operations.try_account(&account).await?;
    let phase = profile
        .operations
        .outgoing
        .phases
        .lock()
        .await
        .get(&id)
        .copied();
    if let Some(phase) = phase {
        anyhow::ensure!(
            phase != "submitting",
            "SMTP is still running. Check Outbox after it finishes."
        );
        persist_phase(&profile.database, id.clone(), phase).await?;
        profile.operations.outgoing.phases.lock().await.remove(&id);
    }
    let known_copy = profile
        .operations
        .sent
        .receipts
        .lock()
        .await
        .contains_key(&id);
    profile.database.write(move |db| {
        let known_copy=known_copy || db.query_row("SELECT EXISTS(SELECT 1 FROM outgoing_sent WHERE id=?1 AND state='saved')",[&id],|r|r.get::<_,bool>(0))?;
        anyhow::ensure!(!known_copy || !matches!(action,Recovery::Return|Recovery::Mark),"A matching Sent copy is already confirmed. Finish saving it in Outbox; do not create another send.");
        if action==Recovery::Local {db.execute("UPDATE outgoing_sent SET local_edited=1 WHERE id=?1",[&id])?;}
        let (state,text):(String,String)=db.query_row("SELECT state,draft FROM outgoing WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?)))?;
        let recovered:Option<(Option<String>,Option<String>)>=db.query_row("SELECT recovery,recovered_draft FROM outgoing_meta WHERE id=?1",[&id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((Some(previous),draft_id))=recovered {
            if previous=="marked" && action==Recovery::Local {local_sent(db,&id,Some("local"))?; return Ok(json!({"recovery":"local"}));}
            return Ok(json!({"recovery":previous,"draft_id":draft_id}));
        }
        anyhow::ensure!(state!="submitting","SMTP is still running. Check Outbox after it finishes.");
        if action==Recovery::Check {
            if state=="delivered" {local_sent(db,&id,None)?;}
            return Ok(json!({"state":state}));
        }
        if action==Recovery::Return {
            anyhow::ensure!(state=="rejected" || state=="uncertain","This message was delivered. Keep its Sent copy instead.");
            anyhow::ensure!(state!="uncertain" || confirmed,"Review delivery first; another send could create a duplicate.");
            let value:Value=serde_json::from_str(&text)?;
            let mut draft:Draft=serde_json::from_value(value.clone())?;
            let original=draft.id.clone();
            let expected=draft.attachments.clone();
            let parts=crate::drafts::files(db,&mut draft)?;
            anyhow::ensure!(value.get("attachments").is_none() || draft.attachments==expected,"The saved attachments changed. Keep this delivery record and review the original message.");
            draft.id=uuid::Uuid::new_v4().to_string(); draft.revision=0;
            let parts:Vec<_>=parts.into_iter().map(|mut file| { file.attachment.id=uuid::Uuid::new_v4().to_string(); file }).collect();
            draft.attachments=parts.iter().map(|f|f.attachment.clone()).collect();
            let tx=db.transaction()?;
            tx.execute("INSERT INTO drafts VALUES(?1,0,?2)",params![draft.id,serde_json::to_string(&draft)?])?;
            for file in parts { crate::drafts::insert_file(&tx,&draft.id,file)?; }
            tx.execute("DELETE FROM drafts WHERE id=?1",[original])?;
            tx.execute("INSERT INTO outgoing_meta(id,recovery,recovered_draft) VALUES(?1,'returned',?2) ON CONFLICT(id) DO UPDATE SET recovery='returned',recovered_draft=excluded.recovered_draft",params![id,draft.id])?;
            tx.commit()?;
            return Ok(json!({"recovery":"returned","draft_id":draft.id}));
        }
        if action==Recovery::Mark {
            anyhow::ensure!((state=="uncertain" || state=="delivered") && confirmed,"Confirm your delivery review before recording this message as sent.");
            local_sent(db,&id,Some("marked"))?;
        } else {
            anyhow::ensure!(state=="delivered" || known_copy,"Delivery is not confirmed. Review it before keeping a Sent copy.");
            local_sent(db,&id,Some("local"))?;
        }
        Ok(json!({"state":state,"recovery":if action==Recovery::Mark{"marked"}else{"local"}}))
    }).await
}
pub(crate) async fn admit(
    profile: &MobileProfile,
    attempt: String,
    id: String,
    revision: u64,
    file_revision: u64,
) -> Result<Value> {
    anyhow::ensure!(
        uuid::Uuid::parse_str(&attempt).is_ok(),
        "Choose a new delivery identity before retrying."
    );
    let saved_attempt = attempt.clone();
    profile.database.write(move |db| {
        if let Some((saved,reviewed)) = db.query_row("SELECT o.state,m.recovery IS NOT NULL FROM outgoing o LEFT JOIN outgoing_meta m ON m.id=o.id WHERE o.id=?1",[&saved_attempt],|r|Ok((r.get::<_,String>(0)?,r.get::<_,bool>(1)?))).optional()? {
            return Ok(json!({"id":saved_attempt,"state":if reviewed{"reviewed"}else{&saved}}));
        }
        let prior = operations::delivery(db, &id)?;
        if !prior.is_null() {
            return Ok(prior);
        }
        let mut draft = crate::drafts::editable(db, &id)?;
        let account = operations::stored_account(db, &draft.account_id)?;
        anyhow::ensure!(draft.revision == revision, "This draft changed before sending. Reopen it and review the latest text.");
        let snapshot = crate::drafts::snapshot(db, &id)?;
        anyhow::ensure!(snapshot["file_revision"].as_u64() == Some(file_revision), "The attachments changed before sending. Reopen the draft and review its files.");
        let files = crate::drafts::files(db, &mut draft)?;
        let message_id = format!("<{}@shep.so>", uuid::Uuid::new_v4());
        let message = shep_mail_core::compose::build_with_message_id(&account,&draft,files,&message_id)?;
        let raw = message.formatted();
        let tx = db.transaction()?;
        let credential_slot=crate::connections::stored_slot(&tx,&account.id)?;
        tx.execute("INSERT INTO outgoing VALUES(?1,?2,'queued',?3,?4,?5,?6)",params![attempt,draft.id,account.id,message_id,raw,saved_submission_draft(&draft,message.envelope(),&credential_slot)?])?;
        tx.execute("INSERT INTO outgoing_meta(id,created,from_address) VALUES(?1,?2,?3)",params![attempt,chrono::Utc::now().timestamp(),account.email])?;
        tx.execute("INSERT INTO outgoing_sent(id,account,state) VALUES(?1,?2,'pending')",params![attempt,serde_json::to_string(&account)?])?;
        tx.commit()?;
        Ok(json!({"id":attempt,"state":"queued","message_id":message_id}))
    }).await
}

pub(crate) async fn send(
    profile: &MobileProfile,
    attempt: String,
    passwords: (SecretString, Option<SecretString>, Option<String>),
    slot: OwnedSemaphorePermit,
    admission: OwnedSemaphorePermit,
) -> Result<Value> {
    let (password, incoming_password, credential_slot) = passwords;
    let identity = attempt.clone();
    let account = profile
        .database
        .read(move |db| {
            let account: String = db.query_row(
                "SELECT account_id FROM outgoing WHERE id=?1",
                [identity],
                |r| r.get(0),
            )?;
            operations::stored_account(db, &account)
        })
        .await?;
    let guard = profile.operations.account(&account.id).await;
    let id_for_binding = account.id.clone();
    let account = profile
        .database
        .read(move |db| {
            crate::connections::check_binding(db, &id_for_binding, credential_slot.as_deref())?;
            operations::stored_account(db, &id_for_binding)
        })
        .await?;
    let dispatch = attempt.clone();
    let account_copy = account.clone();
    let (envelope,raw,message_id,state)=profile.database.write(move|db|{
        let tx=db.transaction()?;
        let (state,raw,message_id,account,draft):(String,Vec<u8>,String,String,String)=tx.query_row("SELECT state,raw,message_id,account_id,draft FROM outgoing WHERE id=?1",[&dispatch],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        if state != "queued" && state != "waiting" {
            let reviewed:bool=tx.query_row("SELECT recovery IS NOT NULL FROM outgoing_meta WHERE id=?1",[&dispatch],|r|r.get(0))?;
            return Ok((None,raw,message_id,if reviewed{"reviewed".to_owned()}else{state}));
        }
        anyhow::ensure!(account==account_copy.id,"This delivery account changed. Review Outbox before retrying.");
        let frozen:String=tx.query_row("SELECT account FROM outgoing_sent WHERE id=?1",[&dispatch],|row|row.get(0))?;
        let frozen:Account=serde_json::from_str(&frozen)?;
        anyhow::ensure!(frozen==account_copy,"This delivery account changed. Cancel the queued delivery and review its draft.");
        let saved:Value=serde_json::from_str(&draft)?;
        let active_slot=crate::connections::stored_slot(&tx,&account_copy.id)?;
        anyhow::ensure!(saved["_delivery_credential_slot"].as_str()==Some(active_slot.as_str()),"This delivery's credentials changed. Cancel the queued delivery and review its draft.");
        let envelope: shep_mail_core::outgoing::EnvelopeData=serde_json::from_value(saved.get("_delivery_envelope").cloned().context("This older queued delivery has no saved recipient proof.")?)?;
        let envelope=envelope.envelope()?;
        tx.execute("UPDATE outgoing SET state='submitting' WHERE id=?1 AND state IN ('queued','waiting')",[&dispatch])?;
        tx.execute("UPDATE outgoing_sent SET error=NULL WHERE id=?1",[&dispatch])?;
        tx.commit()?;
        Ok((Some(envelope),raw,message_id,"submitting".to_owned()))
    }).await?;
    let Some(envelope) = envelope else {
        return Ok(json!({"id":attempt,"state":state,"message_id":message_id}));
    };
    let database = profile.database.clone();
    let operations = profile.operations.clone();
    let smtp = operations
        .outgoing
        .smtp
        .lock()
        .map_err(|_| anyhow::anyhow!("SMTP setup failed. Review Outbox before retrying."))?
        .clone();
    operations
        .outgoing
        .phases
        .lock()
        .await
        .insert(attempt.clone(), "submitting");
    let message_id = json!(message_id);
    let task = tokio::spawn(async move {
        let (_slot, _admission, _guard) = (slot, admission, guard);
        let operation = tokio::spawn(async move {
            tokio::time::timeout(
                Duration::from_secs(90),
                smtp.send(account, password, envelope, raw),
            )
            .await
        });
        let phase = match operation.await {
            Ok(Ok(Ok(()))) => "delivered",
            Ok(Ok(Err(DeliveryFailure::Rejected(_)))) => "rejected",
            _ => "uncertain",
        };
        operations
            .outgoing
            .phases
            .lock()
            .await
            .insert(attempt.clone(), phase);
        let saved = persist_phase(&database, attempt.clone(), phase).await;
        let mut warning = None;
        if saved.is_ok() {
            operations.outgoing.phases.lock().await.remove(&attempt);
            if phase == "delivered" {
                let identity = attempt.clone();
                if database
                    .write(move |db| local_sent(db, &identity, None))
                    .await
                    .is_err()
                {
                    warning = Some(
                        "SMTP accepted this message, but the local Sent copy could not be saved. Check Outbox to finish saving it; do not send again.",
                    );
                }
            }
        } else {
            warning = Some(
                "SMTP finished, but its delivery record could not be saved. Check Outbox to retry saving the result; do not send again.",
            );
        }
        if phase == "delivered" && saved.is_ok() && warning.is_none() {
            let profile = MobileProfile {
                database: database.clone(),
                operations: operations.clone(),
            };
            let id = attempt.clone();
            // A confirmed send can close the editor while its Sent copy runs.
            // This owned continuation retains the same account lock/capacity.
            tokio::spawn(async move {
                let _ownership = (_slot, _admission, _guard);
                let _ =
                    crate::sent::run_locked(&profile, &id, true, false, incoming_password, true)
                        .await;
            });
        }
        Ok::<_, anyhow::Error>(
            json!({"id":attempt,"state":phase,"message_id":message_id,"warning":warning}),
        )
    });
    task.await
        .context("Delivery is unconfirmed. Check Outbox before another send.")?
}
