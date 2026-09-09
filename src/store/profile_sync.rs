//! The existing Store connection owns synchronization receipts. In particular,
//! changing a local preference never depends on opening a cloud/history worker.
use super::*;
use crate::profiles::{
    preferences::{SUPPORTED, apply, export},
    sync::{ApplyResult, Field, PendingEdit, Seed, Subscription},
};
use anyhow::ensure;
use rusqlite::OptionalExtension;
use shep_profile_core::{Action, Change, SettingKey, history::LocalEdit};
use uuid::Uuid;

pub(super) fn schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS profile_sync (
        profile TEXT PRIMARY KEY, binding TEXT NOT NULL, device TEXT NOT NULL, name TEXT NOT NULL,
        enabled INTEGER NOT NULL, revision INTEGER NOT NULL DEFAULT 1,
        history_revision INTEGER NOT NULL, error TEXT, remote_device TEXT, remote_cursor INTEGER NOT NULL DEFAULT 0, last_synced INTEGER);
        CREATE UNIQUE INDEX IF NOT EXISTS one_enabled_profile_sync ON profile_sync(enabled) WHERE enabled=1;
        CREATE TABLE IF NOT EXISTS profile_sync_fields (
            profile TEXT NOT NULL REFERENCES profile_sync(profile), field TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1, shared TEXT,
            shared_revision INTEGER NOT NULL, local_revision INTEGER NOT NULL,
            operation TEXT, edit_revision INTEGER, request TEXT, error TEXT,
            dirty INTEGER NOT NULL DEFAULT 0, incoming TEXT, incoming_revision INTEGER,
            PRIMARY KEY(profile,field));
        CREATE INDEX IF NOT EXISTS profile_sync_pending ON profile_sync_fields(profile,field) WHERE operation IS NOT NULL;")?;
    Ok(())
}

fn field_name(key: SettingKey) -> anyhow::Result<String> {
    Ok(serde_json::to_string(&key)?)
}
fn integer(value: u64) -> anyhow::Result<i64> {
    value
        .try_into()
        .context("Profile revision is invalid. Reopen sync settings.")
}
fn read(db: &Connection, profile: &str) -> anyhow::Result<Subscription> {
    let (binding,name,enabled,revision,history_revision,error,remote_cursor,last_synced,device,remote_device) = db.query_row(
        "SELECT binding,name,enabled,revision,history_revision,error,remote_cursor,last_synced,device,remote_device FROM profile_sync WHERE profile=?",
        [profile], |r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,bool>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?,r.get::<_,Option<String>>(5)?,r.get::<_,i64>(6)?,r.get::<_,Option<i64>>(7)?,r.get::<_,String>(8)?,r.get::<_,Option<String>>(9)?)))
        .optional()?.context("Review this shared profile before enabling synchronization.")?;
    let mut value = Subscription {
        binding: serde_json::from_str(&binding)?,
        device: Uuid::parse_str(&device)?,
        name,
        enabled,
        revision: revision.try_into()?,
        history_revision: history_revision.try_into()?,
        remote_cursor: remote_cursor.try_into()?,
        remote_device: remote_device.map(|v| Uuid::parse_str(&v)).transpose()?,
        last_synced,
        pending: 0,
        conflicts: 0,
        error,
    };
    value.pending = db
        .query_row(
            "SELECT count(*) FROM profile_sync_fields f LEFT JOIN profile_preference_revisions r ON r.field=f.field WHERE f.profile=? AND (f.operation IS NOT NULL OR f.dirty=1 OR COALESCE(r.revision,0)!=f.local_revision)",
            [profile],
            |r| r.get::<_, i64>(0),
        )?
        .try_into()?;
    value.conflicts = db
        .query_row(
            "SELECT count(*) FROM profile_sync_fields WHERE profile=? AND error IS NOT NULL",
            [profile],
            |r| r.get::<_, i64>(0),
        )?
        .try_into()?;
    Ok(value)
}

fn checked_change(key: SettingKey, change: &Change) -> anyhow::Result<()> {
    ensure!(
        change.extra.is_empty(),
        "This shared preference needs a newer Shep version before it can be changed."
    );
    ensure!(
        matches!(&change.action, Action::Setting { key:k,.. } | Action::SettingRemoved { key:k } if *k==key),
        "The profile field changed. Reopen its review."
    );
    let mut preferences = Preferences::default();
    match &change.action {
        Action::Setting { value, .. } => apply(&mut preferences, key, Some(value))?,
        Action::SettingRemoved { .. } => apply(&mut preferences, key, None)?,
        _ => unreachable!(),
    }
    Ok(())
}

impl Store {
    pub async fn profile_sync_active(&self) -> anyhow::Result<Option<Subscription>> {
        self.run(|db| {
            let key: Option<String> = db
                .query_row(
                    "SELECT profile FROM profile_sync WHERE enabled=1 LIMIT 1",
                    [],
                    |r| r.get(0),
                )
                .optional()?;
            key.map(|key| read(db, &key)).transpose()
        })
        .await
    }
    pub async fn profile_sync_error(
        &self,
        profile: String,
        error: Option<String>,
    ) -> anyhow::Result<()> {
        self.run(move |db| {
            db.execute(
                "UPDATE profile_sync SET error=? WHERE profile=?",
                params![error, profile],
            )?;
            Ok(())
        })
        .await
    }
    pub async fn profile_sync_field_error(
        &self,
        profile: String,
        key: SettingKey,
        error: String,
    ) -> anyhow::Result<()> {
        self.run(move |db| {
            db.execute(
                "UPDATE profile_sync_fields SET error=? WHERE profile=? AND field=?",
                params![error, profile, field_name(key)?],
            )?;
            Ok(())
        })
        .await
    }
    pub async fn profile_sync_source(&self, profile: String, device: Uuid) -> anyhow::Result<()> {
        ensure!(
            !device.is_nil(),
            "The remote observation identity is invalid."
        );
        self.run(move |db| {
            db.execute("UPDATE profile_sync SET remote_device=?,remote_cursor=0 WHERE profile=? AND (remote_device IS NULL OR remote_device!=?)", params![device.to_string(),profile,device.to_string()])?;
            Ok(())
        }).await
    }
    pub async fn profile_sync_copied(
        &self,
        profile: String,
        before: u64,
        after: u64,
    ) -> anyhow::Result<()> {
        ensure!(after > before, "The remote history cursor did not advance.");
        self.run(move |db| {
            ensure!(
                db.execute(
                    "UPDATE profile_sync SET remote_cursor=? WHERE profile=? AND remote_cursor=?",
                    params![integer(after)?, profile, integer(before)?]
                )? == 1,
                "Profile copy progress changed. Reopen its saved state."
            );
            Ok(())
        })
        .await
    }
    pub async fn profile_sync_completed(&self, profile: String) -> anyhow::Result<()> {
        self.run(move |db| {
            db.execute(
                "UPDATE profile_sync SET last_synced=?,error=NULL WHERE profile=?",
                params![chrono::Utc::now().timestamp(), profile],
            )?;
            Ok(())
        })
        .await
    }

    /// Called only with a completed, checked publication/enrollment review.
    /// Reusing an existing subscription preserves queued work and its baseline.
    pub async fn profile_sync_seed(&self, seed: Seed) -> anyhow::Result<Subscription> {
        let profile = seed.binding.storage_key()?;
        ensure!(
            !seed.device.is_nil(),
            "A local history identity is required before enabling sync."
        );
        ensure!(
            !seed.fields.is_empty() && seed.fields.len() <= SUPPORTED.len(),
            "Select supported preferences to synchronize."
        );
        ensure!(seed.name.len() <= 256, "The profile name is too long.");
        for (key, change) in &seed.fields {
            ensure!(
                SUPPORTED.contains(key),
                "This preference is not supported on this device."
            );
            if let Some(change) = change {
                checked_change(*key, change)?;
            }
        }
        self.run(move |db| {
            let tx=db.transaction()?;
            if tx.query_row("SELECT EXISTS(SELECT 1 FROM profile_sync WHERE profile=?)",[&profile],|r|r.get::<_,bool>(0))? {
                return read(&tx,&profile);
            }
            let local=profile_preferences::state(&tx)?;
            tx.execute("INSERT INTO profile_sync(profile,binding,device,name,enabled,history_revision) VALUES(?,?,?,?,0,?)",
                params![profile,serde_json::to_string(&seed.binding)?,seed.device.to_string(),seed.name,integer(seed.history_revision)?])?;
            for (key,shared) in seed.fields {
                let mut saved=Preferences::default();
                match shared.as_ref().map(|c|&c.action) {
                    Some(Action::Setting {value,..})=>apply(&mut saved,key,Some(value))?,
                    Some(Action::SettingRemoved {..})=>apply(&mut saved,key,None)?,
                    _=>{},
                }
                let dirty=shared.is_none() || export(&saved)?[&key]!=local.values[&key];
                tx.execute("INSERT INTO profile_sync_fields(profile,field,shared,shared_revision,local_revision,dirty) VALUES(?,?,?,?,?,?)",
                    params![profile,field_name(key)?,shared.map(|c|serde_json::to_string(&c)).transpose()?,integer(seed.history_revision)?,integer(local.revisions[&key])?,dirty])?;
            }
            let result=read(&tx,&profile)?; tx.commit()?; Ok(result)
        }).await
    }

    pub async fn profile_sync_subscription(&self, profile: String) -> anyhow::Result<Subscription> {
        self.run(move |db| read(db, &profile)).await
    }

    /// A stale toggle cannot restore an earlier enabled state after a later
    /// pause. Other profile queues remain durable and require a reviewed switch.
    pub async fn profile_sync_enable(
        &self,
        profile: String,
        revision: u64,
        enabled: bool,
    ) -> anyhow::Result<Subscription> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let old = read(&tx, &profile)?;
            ensure!(
                old.revision == revision,
                "Sync settings changed. Reopen them before changing this profile."
            );
            if enabled {
                let context: String = get(&tx, "profile_sync_context")?;
                ensure!(context.is_empty() || context == profile,
                    "This workspace belongs to another shared profile. Review a profile switch before synchronizing a different setup.");
                put(&tx, "profile_sync_context", &profile)?;
                ensure!(
                    !tx.query_row(
                        "SELECT EXISTS(SELECT 1 FROM profile_sync WHERE enabled=1 AND profile!=?)",
                        [&profile],
                        |r| r.get::<_, bool>(0)
                    )?,
                    "Pause the current profile before reviewing a switch to another setup."
                );
            }
            tx.execute(
                "UPDATE profile_sync SET enabled=?,revision=revision+1,error=NULL WHERE profile=?",
                params![enabled, profile],
            )?;
            let next = read(&tx, &profile)?;
            tx.commit()?;
            Ok(next)
        })
        .await
    }

    pub async fn profile_sync_fields(&self, profile: String) -> anyhow::Result<Vec<Field>> {
        self.run(move |db| {
            read(db,&profile)?;
            let local=profile_preferences::state(db)?;
            let mut query=db.prepare("SELECT field,enabled,shared,shared_revision,local_revision,operation IS NOT NULL,error,dirty,incoming,incoming_revision FROM profile_sync_fields WHERE profile=? ORDER BY field LIMIT 50")?;
            let mut rows=query.query([profile])?;
            let mut result=Vec::new();
            while let Some(row)=rows.next()? {
                let key:SettingKey=serde_json::from_str(&row.get::<_,String>(0)?)?;
                let local_revision:u64=row.get::<_,i64>(4)?.try_into()?;
                result.push(Field {key,enabled:row.get(1)?,shared:row.get::<_,Option<String>>(2)?.map(|v|serde_json::from_str(&v)).transpose()?,shared_revision:row.get::<_,i64>(3)?.try_into()?,local_revision,
                    pending:row.get::<_,bool>(5)? || row.get::<_,bool>(7)? || local.revisions[&key]!=local_revision,
                    local:local.values[&key].clone(),error:row.get(6)?,incoming:row.get::<_,Option<String>>(8)?.map(|v|serde_json::from_str(&v)).transpose()?,incoming_revision:row.get::<_,Option<i64>>(9)?.map(u64::try_from).transpose()?});
            }
            Ok(result)
        }).await
    }

    pub async fn profile_sync_field_enable(
        &self,
        profile: String,
        revision: u64,
        key: SettingKey,
        enabled: bool,
    ) -> anyhow::Result<Subscription> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let current = read(&tx, &profile)?;
            ensure!(
                current.revision == revision,
                "Sync settings changed. Reopen the current choices."
            );
            ensure!(
                tx.execute(
                    "UPDATE profile_sync_fields SET enabled=? WHERE profile=? AND field=?",
                    params![enabled, profile, field_name(key)?]
                )? == 1,
                "Review this preference before synchronizing it."
            );
            tx.execute(
                "UPDATE profile_sync SET revision=revision+1 WHERE profile=?",
                [&profile],
            )?;
            let result = read(&tx, &profile)?;
            tx.commit()?;
            Ok(result)
        })
        .await
    }

    /// Save an exact immutable request before crossing into the history DB.
    /// Later local edits never alter it; they are captured after its receipt.
    pub async fn profile_sync_prepare_edit(
        &self,
        profile: String,
        key: SettingKey,
    ) -> anyhow::Result<Option<PendingEdit>> {
        self.run(move |db| {
            let tx=db.transaction()?;
            let subscription=read(&tx,&profile)?;
            ensure!(subscription.enabled,"Profile synchronization is paused.");
            let name=field_name(key)?;
            let (enabled,shared_revision,local_revision,request,error,dirty):(bool,i64,i64,Option<String>,Option<String>,bool)=tx.query_row(
                "SELECT enabled,shared_revision,local_revision,request,error,dirty FROM profile_sync_fields WHERE profile=? AND field=?",params![profile,name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?)))?;
            if !enabled || error.is_some() {return Ok(None);}
            if let Some(request)=request { return Ok(Some(serde_json::from_str(&request)?)); }
            let current=profile_preferences::state(&tx)?;
            if !dirty && integer(current.revisions[&key])?==local_revision {return Ok(None);}
            let operation=Uuid::new_v4();
            let edit=PendingEdit {binding:subscription.binding,key,operation,local_revision:current.revisions[&key],request:LocalEdit {operation,expected_revision:shared_revision.try_into()?,changes:vec![Change{action:Action::Setting {key,value:current.values[&key].clone()},extra:Default::default()}],resolutions:vec![]}};
            tx.execute("UPDATE profile_sync_fields SET operation=?,edit_revision=?,request=? WHERE profile=? AND field=? AND operation IS NULL",params![operation.to_string(),integer(edit.local_revision)?,serde_json::to_string(&edit)?,profile,name])?;
            tx.commit()?;Ok(Some(edit))
        }).await
    }

    /// A history receipt is acknowledged using the staged local generation, not
    /// the latest preferences. A newer edit remains detectable after restart.
    pub async fn profile_sync_edit_saved(
        &self,
        edit: PendingEdit,
        history_revision: u64,
    ) -> anyhow::Result<()> {
        let profile = edit.binding.storage_key()?;
        self.run(move |db| {
            let tx=db.transaction()?;
            let request:Option<String>=tx.query_row("SELECT request FROM profile_sync_fields WHERE profile=? AND field=?",params![profile,field_name(edit.key)?],|r|r.get(0))?;
            if request.is_none() {return Ok(());}
            ensure!(request.as_deref()==Some(&serde_json::to_string(&edit)?),"The saved profile change belongs to a different request. Reopen its progress.");
            tx.execute("UPDATE profile_sync_fields SET shared=?,shared_revision=?,local_revision=?,operation=NULL,edit_revision=NULL,request=NULL,error=NULL,dirty=0 WHERE profile=? AND field=?",params![serde_json::to_string(&edit.request.changes[0])?,integer(history_revision)?,integer(edit.local_revision)?,profile,field_name(edit.key)?])?;
            tx.execute("UPDATE profile_sync SET history_revision=max(history_revision,?) WHERE profile=?",params![integer(history_revision)?,profile])?;
            tx.commit()?;Ok(())
        }).await
    }

    /// Recheck the local field inside the same transaction that applies remote
    /// data and advances its receipt. No delayed read can replace newer intent.
    pub async fn profile_sync_apply_setting(
        &self,
        profile: String,
        key: SettingKey,
        change: Change,
        history_revision: u64,
    ) -> anyhow::Result<ApplyResult> {
        checked_change(key, &change)?;
        self.run(move |db| {
            let tx=db.transaction()?;
            let subscription=read(&tx,&profile)?;
            ensure!(subscription.enabled,"Profile synchronization is paused.");
            let name=field_name(key)?;
            let (enabled,shared_revision,local_revision,pending,dirty):(bool,i64,i64,bool,bool)=tx.query_row("SELECT enabled,shared_revision,local_revision,operation IS NOT NULL,dirty FROM profile_sync_fields WHERE profile=? AND field=?",params![profile,name],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
            if !enabled || integer(history_revision)?<=shared_revision {return Ok(ApplyResult::Unchanged);}
            let encoded=serde_json::to_string(&change)?;
            let current=profile_preferences::state(&tx)?;
            if pending || dirty || integer(current.revisions[&key])?!=local_revision {
                tx.execute("UPDATE profile_sync_fields SET error=?,incoming=?,incoming_revision=? WHERE profile=? AND field=?",params!["This preference changed both here and in the shared profile. Review both versions before replacing either.",encoded,integer(history_revision)?,profile,name])?;
                tx.commit()?;return Ok(ApplyResult::ReviewRequired);
            }
            let mut preferences:Preferences=get(&tx,"preferences")?;
            match &change.action {Action::Setting{value,..}=>apply(&mut preferences,key,Some(value))?,Action::SettingRemoved{..}=>apply(&mut preferences,key,None)?,_=>unreachable!()}
            preferences.validate()?;
            put(&tx,"preferences",&preferences)?;
            let local=profile_preferences::state(&tx)?;
            tx.execute("UPDATE profile_sync_fields SET shared=?,shared_revision=?,local_revision=?,error=NULL,dirty=0,incoming=NULL,incoming_revision=NULL WHERE profile=? AND field=?",params![encoded,integer(history_revision)?,integer(local.revisions[&key])?,profile,name])?;
            tx.execute("UPDATE profile_sync SET history_revision=max(history_revision,?) WHERE profile=?",params![integer(history_revision)?,profile])?;
            let snapshot=PreferenceSnapshot {revision:get(&tx,"preferences_revision")?,value:preferences};
            tx.commit()?;Ok(ApplyResult::Applied(Box::new(snapshot)))
        }).await
    }
}

#[cfg(test)]
mod tests;
