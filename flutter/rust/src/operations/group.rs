//! Subordinate attempts use the existing mail action and receipt rows. Only
//! the group owner can dispatch, repair or retire these rows.
use super::*;

pub(crate) struct Admission<'a> {
    pub job: &'a str,
    pub position: i64,
    pub inverse: bool,
    pub attempt: &'a str,
    pub mail: &'a str,
    pub lineage: &'a str,
    pub approved: i64,
    pub fields: &'a Value,
    pub credential_slot: Option<&'a str>,
}

pub(crate) fn connection(db: &Connection, account: &str) -> Result<String> {
    let saved = stored_account(db, account)?;
    let slot = crate::connections::stored_slot(db, account)?;
    Ok(serde_json::to_string(
        &json!({"slot":slot,"protocol":saved.protocol,"host":saved.host,"port":saved.port,"username":saved.username,"security":saved.incoming_security,"auth":saved.incoming_auth}),
    )?)
}

pub(crate) fn check_connection(db: &Connection, action: &str) -> Result<()> {
    let saved:Option<(String,Option<String>)>=db.query_row("SELECT a.account,i.connection FROM individual_mail_actions a JOIN group_items i ON i.job=a.group_job AND i.position=a.group_position WHERE a.id=?1",[action],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    if let Some((account, expected)) = saved {
        anyhow::ensure!(
            expected.as_deref() == Some(connection(db, &account)?.as_str()),
            "The connection changed since the group review. No provider operation was started."
        );
    }
    Ok(())
}

pub(crate) fn public_action(db: &Connection, id: &str) -> Result<bool> {
    Ok(!db.query_row("SELECT EXISTS(SELECT 1 FROM individual_mail_actions WHERE id=?1 AND group_job IS NOT NULL)", [id], |row| row.get::<_, bool>(0))?)
}
pub(crate) fn require_public(db: &Connection, id: &str) -> Result<()> {
    anyhow::ensure!(
        public_action(db, id)?,
        "This attempt belongs to a group action. Open its History recovery instead."
    );
    Ok(())
}

pub(crate) fn owns_field(
    db: &Connection,
    action: &str,
    mail: &str,
    field: &str,
    revision: i64,
) -> Result<bool> {
    let own: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM mail_intents WHERE mail=?1 AND field=?2 AND revision=?3)",
        params![mail, field, revision],
        |row| row.get(0),
    )?;
    if !own {
        return Ok(false);
    }
    let owner: Option<(String,i64)> = db.query_row("SELECT a.group_job,j.approved FROM individual_mail_actions a JOIN group_jobs j ON j.id=a.group_job WHERE a.id=?1",[action],|row|Ok((row.get(0)?,row.get(1)?))).optional()?;
    match owner {
        Some((job, approved)) => crate::groups::no_newer_group(db, &job, mail, field, approved),
        None => Ok(true),
    }
}

pub(crate) struct Snapshot {
    pub job: String,
    pub position: i64,
    pub inverse: bool,
    pub status: String,
    pub dispatch: crate::groups::Identity,
    pub fields: crate::groups::Fields,
    pub after: Option<crate::groups::Identity>,
    pub acknowledged: bool,
}

pub(crate) fn snapshot(db: &Connection, id: &str) -> Result<Snapshot> {
    let (job,position,inverse,status,physical,fields,receipt): (String,i64,bool,String,String,String,Option<String>) = db.query_row(
        "SELECT a.group_job,a.group_position,a.group_inverse,a.status,a.physical,COALESCE(a.accepted_fields,a.fields),r.result FROM individual_mail_actions a LEFT JOIN individual_mail_action_receipts r ON r.action=a.id WHERE a.id=?1 AND a.group_job IS NOT NULL",
        [id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?,row.get(6)?)))?;
    let physical: Value = serde_json::from_str(&physical)?;
    let fields: crate::groups::Fields = serde_json::from_str(&fields)?;
    let dispatch = crate::groups::Identity {
        folder: physical["folder"]
            .as_str()
            .context("Saved group folder")?
            .into(),
        remote_id: physical["remote_id"]
            .as_str()
            .context("Saved group UID")?
            .into(),
        unread: physical["unread"].as_bool().context("Saved unread value")?,
        starred: physical["starred"].as_bool().context("Saved star value")?,
        lineage: physical["lineage"].as_str().map(str::to_owned),
    };
    let receipt: Option<IndividualReceipt> = receipt
        .map(|value| serde_json::from_str(&value))
        .transpose()?;
    let after = match receipt.as_ref() {
        Some(IndividualReceipt::Move { receipt }) => {
            receipt
                .current
                .as_ref()
                .map(|current| crate::groups::Identity {
                    folder: current.folder.clone(),
                    remote_id: current.remote_id.clone(),
                    ..dispatch.clone()
                })
        }
        Some(IndividualReceipt::Flags) => Some(crate::groups::Identity {
            unread: fields.unread.unwrap_or(dispatch.unread),
            starred: fields.starred.unwrap_or(dispatch.starred),
            ..dispatch.clone()
        }),
        None => None,
    };
    Ok(Snapshot {
        job,
        position,
        inverse,
        status,
        dispatch,
        fields,
        after,
        acknowledged: receipt.is_some(),
    })
}

pub(crate) fn repair(db: &Connection, id: &str) -> Result<ReceiptApplication> {
    anyhow::ensure!(
        !public_action(db, id)?,
        "The group receipt is no longer available."
    );
    apply_individual_receipt(db, id)
}

pub(crate) fn pending(db: &Connection) -> Result<Option<String>> {
    Ok(db.query_row("SELECT i.attempt FROM group_jobs j INDEXED BY group_job_state CROSS JOIN group_items i INDEXED BY group_item_state WHERE j.state IN ('running','undoing','paused') AND i.job=j.id AND i.state IN ('sending','reversing','repair','undo_repair') AND EXISTS(SELECT 1 FROM individual_mail_actions a WHERE a.id=i.attempt AND a.group_job=j.id AND a.status IN ('repair','succeeded')) ORDER BY j.seq,i.position LIMIT 1",[],|row|row.get(0)).optional()?)
}

pub(crate) async fn inspect(
    profile: &MobileProfile,
    id: String,
    password: SecretString,
    supplied_slot: Option<String>,
) -> Result<()> {
    let lookup = id.clone();
    let (account,mail,slot,receipt,original_receipt) = profile.database.read(move |db| {
        let (account,mail,slot,raw): (String,String,Option<String>,String) = db.query_row("SELECT a.account,a.mail,a.credential_slot,r.result FROM individual_mail_actions a JOIN individual_mail_action_receipts r ON r.action=a.id WHERE a.id=?1 AND a.group_job IS NOT NULL AND a.status='repair'",[lookup],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
        anyhow::ensure!(slot==supplied_slot, "The original group connection changed. Reconnect and review its saved receipt.");
        crate::connections::check_binding(db,&account,slot.as_deref())?;
        Ok((stored_account(db,&account)?,mail,slot,serde_json::from_str::<IndividualReceipt>(&raw)?,raw))
    }).await?;
    let IndividualReceipt::Move { mut receipt } = receipt else {
        return Ok(());
    };
    if receipt.current.is_some() {
        return Ok(());
    }
    let _admission = profile
        .operations
        .admitted
        .clone()
        .try_acquire_owned()
        .context("Mail is busy. The saved group acknowledgement was retained.")?;
    let _slot = profile.operations.slots.clone().acquire_owned().await?;
    let _owner = profile.operations.account(&account.id).await;
    let owner_account = account.clone();
    let owner_mail = mail.clone();
    let owner_slot = slot.clone();
    let owner_id = id.clone();
    let owner_receipt = original_receipt.clone();
    profile.database.read(move |db| {
        check_connection(db,&owner_id)?;
        crate::connections::check_binding(db,&owner_account.id,owner_slot.as_deref())?;
        anyhow::ensure!(serde_json::to_value(stored_account(db,&owner_account.id)?)?==serde_json::to_value(owner_account)?, "The original group connection changed during inspection admission.");
        let (physical,status,original_slot):(String,String,Option<String>)=db.query_row("SELECT physical,status,credential_slot FROM individual_mail_actions WHERE id=?1 AND group_job IS NOT NULL",[&owner_id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        anyhow::ensure!(status=="repair" && original_slot==owner_slot && action_source_matches(db,&stored_mail(db,&owner_mail)?,&serde_json::from_str(&physical)?)?, "This group receipt changed before inspection could start.");
        let same_receipt: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM individual_mail_action_receipts WHERE action=?1 AND result=?2)",params![owner_id,owner_receipt],|row|row.get(0))?;
        anyhow::ensure!(same_receipt,"This group acknowledgement changed before inspection could start.");
        Ok(())
    }).await?;
    let provider = profile.operations.mail_provider(account.protocol);
    let resolved = tokio::time::timeout(
        Duration::from_secs(45),
        provider.inspect_move(&account, &password, &receipt),
    )
    .await
    .context("Group identity inspection timed out. Its saved acknowledgement was retained.")??;
    anyhow::ensure!(
        resolved.account_id == account.id
            && resolved.folder == receipt.folder
            && !resolved.remote_id.is_empty(),
        "The inspected message does not match the acknowledged group destination."
    );
    receipt.current = Some(resolved);
    profile
        .database
        .write(move |db| {
            check_connection(db, &id)?;
            crate::connections::check_binding(db, &account.id, slot.as_deref())?;
            let same_receipt: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM individual_mail_actions a JOIN individual_mail_action_receipts r ON r.action=a.id WHERE a.id=?1 AND a.group_job IS NOT NULL AND a.status='repair' AND r.result=?2)",params![id,original_receipt],|row|row.get(0))?;
            anyhow::ensure!(same_receipt,"This group acknowledgement changed during inspection.");
            let current = stored_mail(db, &mail)?;
            let physical: String = db.query_row(
                "SELECT physical FROM individual_mail_actions WHERE id=?1",
                [&id],
                |row| row.get(0),
            )?;
            anyhow::ensure!(
                action_source_matches(db, &current, &serde_json::from_str(&physical)?)?,
                "The cached group source changed during inspection."
            );
            save_individual_receipt(db, &id, &IndividualReceipt::Move { receipt })
        })
        .await
}

pub(crate) fn admit(db: &Connection, request: Admission<'_>) -> Result<Value> {
    let message = stored_mail(db, request.mail)?;
    stored_account(db, &message.account_id)?;
    let expected: Option<String> = db.query_row(
        "SELECT connection FROM group_items WHERE job=?1 AND position=?2",
        params![request.job, request.position],
        |row| row.get(0),
    )?;
    anyhow::ensure!(
        expected.as_deref() == Some(connection(db, &message.account_id)?.as_str()),
        "The connection changed since the group review. No provider operation was started."
    );
    anyhow::ensure!(
        observed_lineage_matches(db, &message.id, request.lineage)?,
        "This message changed since the group review. No provider operation was started."
    );
    let mut accepted = serde_json::Map::new();
    let revision: i64 = db.query_row(
        "SELECT CASE WHEN ?2 THEN undone ELSE approved END FROM group_jobs WHERE id=?1",
        params![request.job, request.inverse],
        |row| row.get(0),
    )?;
    for field in ["folder", "unread", "starred"] {
        let Some(value) = request.fields.get(field).filter(|value| !value.is_null()) else {
            continue;
        };
        if !crate::groups::field_owned(db, request.job, &message.id, field, request.approved)? {
            continue;
        }
        db.execute("INSERT INTO mail_intents(mail,field,revision) VALUES(?1,?2,?3) ON CONFLICT(mail,field) DO UPDATE SET revision=excluded.revision WHERE mail_intents.revision<=?4", params![message.id,field,revision,request.approved])?;
        accepted.insert(field.into(), value.clone());
    }
    let accepted = Value::Object(accepted);
    let fingerprint = if accepted.get("folder").is_some() {
        let raw: Vec<u8> =
            db.query_row("SELECT raw FROM mail WHERE id=?1", [&message.id], |row| {
                row.get(0)
            })?;
        Some(Fingerprint::of(&raw))
    } else {
        None
    };
    let physical = json!({"account":message.account_id,"folder":message.folder,"remote_id":message.remote_id,"unread":message.unread,"starred":message.starred,"lineage":request.lineage,"fingerprint":fingerprint});
    db.execute("INSERT INTO individual_mail_actions(id,mail,account,fields,accepted_fields,physical,intent_revision,credential_slot,status,created,group_job,group_position,group_inverse) VALUES(?1,?2,?3,?4,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![request.attempt,message.id,message.account_id,serde_json::to_string(&accepted)?,serde_json::to_string(&physical)?,revision,request.credential_slot,if accepted.as_object().is_none_or(|fields| fields.is_empty()) {"cancelled"} else {"queued"},chrono::Utc::now().timestamp_millis(),request.job,request.position,request.inverse])?;
    Ok(accepted)
}

pub(crate) async fn dispatch(
    profile: &MobileProfile,
    id: String,
    password: Option<SecretString>,
) -> Result<Value> {
    let lookup = id.clone();
    let (mail, fields, slot, status) = profile.database.read(move |db| {
        Ok(db.query_row("SELECT mail,accepted_fields,credential_slot,status FROM individual_mail_actions WHERE id=?1 AND group_job IS NOT NULL", [lookup], |row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,Option<String>>(2)?,row.get::<_,String>(3)?)))?)
    }).await?;
    if status == "cancelled" {
        return Ok(json!({"status":"cancelled","committed":false}));
    }
    anyhow::ensure!(
        status == "queued" || status == "waiting",
        "This group attempt already has an execution or recovery result."
    );
    let fields: Value = serde_json::from_str(&fields)?;
    let local_id = id.clone();
    let local = profile.database.write(move |db| {
        let tx = db.transaction()?;
        let saved = snapshot(&tx,&local_id)?;
        if saved.status=="cancelled" { return Ok(Some(json!({"status":"cancelled","committed":false}))); }
        anyhow::ensure!(matches!(saved.status.as_str(),"queued"|"waiting"), "The local group attempt already has a result.");
        check_connection(&tx,&local_id)?;
        let message = stored_mail(&tx,&mail)?;
        let account = stored_account(&tx,&message.account_id)?;
        if account.protocol == Protocol::Imap && !message.remote_id.starts_with("local-") {
            return Ok(None);
        }
        let lineage=saved.dispatch.lineage.as_deref().context("This group has no captured source proof.")?;
        anyhow::ensure!(observed_lineage_matches(&tx,&message.id,lineage)?, "This group source changed before its local action.");
        let revision: i64 = tx.query_row("SELECT intent_revision FROM individual_mail_actions WHERE id=?1",[&local_id],|row|row.get(0))?;
        let folder = if owns_field(&tx,&local_id,&message.id,"folder",revision)? { saved.fields.folder.clone() } else { None };
        let unread = if owns_field(&tx,&local_id,&message.id,"unread",revision)? { saved.fields.unread } else { None };
        let starred = if owns_field(&tx,&local_id,&message.id,"starred",revision)? { saved.fields.starred } else { None };
        if folder.is_none() && unread.is_none() && starred.is_none() {
            tx.execute("UPDATE individual_mail_actions SET status='cancelled',error=NULL WHERE id=?1",[&local_id])?;
            tx.commit()?;
            return Ok(Some(json!({"status":"cancelled","committed":false})));
        }
        let receipt = if let Some(folder) = folder.as_deref() {
            let raw: Vec<u8> = tx.query_row("SELECT raw FROM mail WHERE id=?1",[&message.id],|row|row.get(0))?;
            IndividualReceipt::Move { receipt: Box::new(MoveReceipt::server(&message,&account.id,folder,Some(message.remote_id.clone()),Fingerprint::of(&raw))) }
        } else { IndividualReceipt::Flags };
        mark_local_sent_edit(&tx,&message)?;
        if let IndividualReceipt::Move { receipt } = &receipt { save_move(&tx,&message.id,receipt)?; }
        acknowledged_mail_write(&tx,&message.id,||Ok(tx.execute("UPDATE mail SET unread=COALESCE(?2,unread),starred=COALESCE(?3,starred) WHERE id=?1",params![message.id,unread,starred])?))?;
        for field in [folder.as_ref().map(|_|"folder"),unread.map(|_|"unread"),starred.map(|_|"starred")].into_iter().flatten() {
            record_applied(&tx,&message.id,field,revision)?;
        }
        tx.execute("INSERT INTO individual_mail_action_receipts(action,result) VALUES(?1,?2)",params![local_id,serde_json::to_string(&receipt)?])?;
        tx.execute("UPDATE individual_mail_actions SET status='succeeded',accepted_fields=?2,error=NULL WHERE id=?1",params![local_id,serde_json::to_string(&json!({"folder":folder,"unread":unread,"starred":starred}))?])?;
        tx.commit()?;
        Ok(Some(json!({"status":"succeeded","committed":true})))
    }).await?;
    if let Some(result) = local {
        return Ok(result);
    }
    let lookup = id.clone();
    let mail: String = profile
        .database
        .read(move |db| {
            Ok(db.query_row(
                "SELECT mail FROM individual_mail_actions WHERE id=?1",
                [lookup],
                |row| row.get(0),
            )?)
        })
        .await?;
    let result = network(
        profile,
        Request::Mutate {
            action_id: Some(id.clone()),
            observed_lineage: None,
            require_observation: false,
            credential_slot: slot,
            id: mail,
            password,
            folder: fields
                .get("folder")
                .and_then(Value::as_str)
                .map(str::to_owned),
            unread: fields.get("unread").and_then(Value::as_bool),
            starred: fields.get("starred").and_then(Value::as_bool),
        },
    )
    .await;
    finish_durable_mutation(profile, id, result, true).await
}

pub(crate) fn retire(db: &Connection, job: &str) -> Result<bool> {
    let attempts=db.prepare("SELECT id FROM individual_mail_actions INDEXED BY individual_mail_action_group WHERE group_job=?1 ORDER BY group_position,group_inverse,created,id LIMIT 50")?
        .query_map([job],|row|row.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for attempt in &attempts {
        db.execute(
            "DELETE FROM individual_mail_action_receipts WHERE action=?1",
            [attempt],
        )?;
        db.execute("DELETE FROM individual_mail_actions WHERE id=?1", [attempt])?;
    }
    Ok(!attempts.is_empty())
}
