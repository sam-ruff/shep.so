use super::*;
use crate::profiles::sync::resolution::{Choice, Page, Review, Version};
use shep_profile_core::history::Resolution;

pub(super) fn schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS profile_sync_reviews(
        id TEXT PRIMARY KEY,profile TEXT NOT NULL,field TEXT NOT NULL,phase TEXT NOT NULL,
        review TEXT NOT NULL);
        CREATE UNIQUE INDEX IF NOT EXISTS profile_sync_open_review ON profile_sync_reviews(profile)
            WHERE phase IN ('collecting','review','staged');
        CREATE TABLE IF NOT EXISTS profile_sync_review_versions(
            review TEXT NOT NULL REFERENCES profile_sync_reviews(id),operation TEXT NOT NULL,
            value TEXT NOT NULL,seen INTEGER NOT NULL DEFAULT 0,PRIMARY KEY(review,operation));
        CREATE TABLE IF NOT EXISTS profile_sync_rejected_edits(operation TEXT PRIMARY KEY,request TEXT NOT NULL);")?;
    Ok(())
}
fn read_review(db: &Connection, profile: &str, id: Uuid) -> anyhow::Result<Review> {
    let raw: String = db
        .query_row(
            "SELECT review FROM profile_sync_reviews WHERE id=? AND profile=?",
            params![id.to_string(), profile],
            |r| r.get(0),
        )
        .optional()?
        .context("Reopen the current preference review.")?;
    Ok(serde_json::from_str(&raw)?)
}
fn write(db: &Connection, review: &Review) -> anyhow::Result<()> {
    db.execute(
        "UPDATE profile_sync_reviews SET phase=?,review=? WHERE id=? AND profile=?",
        params![
            review.phase,
            serde_json::to_string(review)?,
            review.id.to_string(),
            review.profile
        ],
    )?;
    Ok(())
}
fn current(db: &Connection, review: &Review) -> anyhow::Result<Subscription> {
    let sub = read(db, &review.profile)?;
    let context: String = get(db, "profile_sync_context")?;
    ensure!(
        context.is_empty() || context == review.profile,
        "This workspace belongs to another profile. Review a profile switch before applying these preferences."
    );
    ensure!(
        sub.device == review.device
            && sub.revision == review.subscription_revision
            && sub.history_revision <= review.history_revision,
        "Sync settings or history changed. Reopen this preference review."
    );
    let local = profile_preferences::state(db)?;
    ensure!(
        local.revisions[&review.key] == review.local_revision,
        "This device's preference changed after the review. Reopen it to keep your latest choice."
    );
    let pending: bool = db.query_row(
        "SELECT operation IS NOT NULL FROM profile_sync_fields WHERE profile=? AND field=?",
        params![review.profile, field_name(review.key)?],
        |r| r.get(0),
    )?;
    ensure!(
        !pending,
        "Finish inspecting the pending preference change before resolving it."
    );
    Ok(sub)
}
impl Store {
    pub async fn profile_sync_pending_edit(
        &self,
        profile: String,
        key: SettingKey,
    ) -> anyhow::Result<Option<PendingEdit>> {
        self.run(move |db| {
            let raw: Option<String> = db.query_row(
                "SELECT request FROM profile_sync_fields WHERE profile=? AND field=?",
                params![profile, field_name(key)?],
                |r| r.get(0),
            )?;
            raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
                .transpose()
        })
        .await
    }
    /// Only the owning history's definitive Changed/Conflict reply authorizes
    /// this transition. Keep the rejected request as audit, never rewrite it.
    pub(crate) async fn profile_sync_reject_edit(&self, edit: PendingEdit) -> anyhow::Result<()> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let raw = serde_json::to_string(&edit)?;
            ensure!(tx.execute("UPDATE profile_sync_fields SET operation=NULL,request=NULL,edit_revision=NULL,dirty=1 WHERE profile=? AND field=? AND request=?",params![edit.binding.storage_key()?,field_name(edit.key)?,raw])? == 1,"The pending preference changed. Inspect its saved request again.");
            tx.execute("INSERT INTO profile_sync_rejected_edits VALUES(?,?)",params![edit.operation.to_string(),raw])?;
            tx.commit()?; Ok(())
        }).await
    }
    pub async fn profile_resolution_can_begin(&self, profile: String) -> anyhow::Result<()> {
        self.run(move |db| {
            ensure!(!db.query_row("SELECT EXISTS(SELECT 1 FROM profile_sync_reviews WHERE profile=? AND phase='staged')",[profile],|r|r.get::<_,bool>(0))?,"Retry the saved preference decision before opening another review.");
            Ok(())
        }).await
    }
    pub(crate) async fn profile_resolution_begin(
        &self,
        profile: String,
        key: SettingKey,
        history_revision: u64,
    ) -> anyhow::Result<Uuid> {
        ensure!(
            SUPPORTED.contains(&key),
            "This preference is not supported on this device."
        );
        self.run(move |db| {
            let tx = db.transaction()?;
            let sub = read(&tx, &profile)?;
            ensure!(history_revision >= sub.history_revision,"The history moved backwards. Recover it before reviewing preferences.");
            ensure!(!tx.query_row("SELECT EXISTS(SELECT 1 FROM profile_sync_reviews WHERE profile=? AND phase='staged')",[&profile],|r|r.get::<_,bool>(0))?,"Retry the saved preference decision first.");
            let local = profile_preferences::state(&tx)?;
            let review = Review { id: Uuid::new_v4(), profile: profile.clone(), key, device: sub.device, subscription_revision: sub.revision, history_revision, local_revision: local.revisions[&key],local:local.values[&key].clone(),total:0,seen:0,phase:"collecting".into(),request:None,error:None };
            current(&tx,&review)?;
            tx.execute("UPDATE profile_sync_reviews SET phase='cancelled',review=json_set(review,'$.phase','cancelled') WHERE profile=? AND phase IN ('collecting','review')",[&profile])?;
            tx.execute("INSERT INTO profile_sync_reviews VALUES(?,?,?,?,?)",params![review.id.to_string(),profile,field_name(key)?,review.phase,serde_json::to_string(&review)?])?;
            tx.commit()?; Ok(review.id)
        }).await
    }
    pub(crate) async fn profile_resolution_append(
        &self,
        id: Uuid,
        versions: Vec<Version>,
    ) -> anyhow::Result<()> {
        ensure!(
            !versions.is_empty() && versions.len() <= 50,
            "Invalid preference review page."
        );
        self.run(move |db| {
            let tx=db.transaction()?;
            let profile:String=tx.query_row("SELECT profile FROM profile_sync_reviews WHERE id=?",[id.to_string()],|r|r.get(0))?;
            let mut review=read_review(&tx,&profile,id)?;
            ensure!(review.phase=="collecting", "This preference review is already frozen.");
            ensure!(review.total + versions.len() as u64 <= shep_profile_core::MAX_PARENTS as u64, "Too many independent preference versions need reconciliation. Update Shep before resolving this preference.");
            for version in &versions {
                checked_change(review.key,&version.change)?;
                tx.execute("INSERT INTO profile_sync_review_versions(review,operation,value) VALUES(?,?,?)",params![id.to_string(),version.operation.to_string(),serde_json::to_string(version)?])?;
            }
            review.total += versions.len() as u64;
            write(&tx,&review)?; tx.commit()?; Ok(())
        }).await
    }
    pub(crate) async fn profile_resolution_ready(&self, id: Uuid) -> anyhow::Result<()> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let profile: String = tx.query_row(
                "SELECT profile FROM profile_sync_reviews WHERE id=?",
                [id.to_string()],
                |r| r.get(0),
            )?;
            let mut review = read_review(&tx, &profile, id)?;
            ensure!(
                review.phase == "collecting" && review.total > 0,
                "The shared preference is missing. Check its history before resolving it."
            );
            current(&tx, &review)?;
            review.phase = "review".into();
            write(&tx, &review)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn profile_resolution_current(&self, profile: String) -> anyhow::Result<Page> {
        let query_profile = profile.clone();
        let id=self.run(move |db| {
            let id:Option<String>=db.query_row("SELECT id FROM profile_sync_reviews WHERE profile=? AND phase IN ('collecting','review','staged')",[query_profile],|r|r.get(0)).optional()?;
            id.map(|id|Uuid::parse_str(&id).map_err(Into::into)).transpose()
        }).await?;
        match id {
            Some(id) => self.profile_resolution_page(profile, id, None).await,
            None => Ok(Page::default()),
        }
    }
    pub async fn profile_resolution_page(
        &self,
        profile: String,
        id: Uuid,
        after: Option<Uuid>,
    ) -> anyhow::Result<Page> {
        self.run(move |db| {
            let tx=db.transaction()?;
            let mut review=read_review(&tx,&profile,id)?;
            let mut page=Page {review:None,versions:vec![],after,more:false};
            {
                let mut query=tx.prepare("SELECT value FROM profile_sync_review_versions WHERE review=? AND operation>? ORDER BY operation LIMIT 50")?;
                let mut rows=query.query(params![id.to_string(),after.map(|v|v.to_string()).unwrap_or_default()])?;
                while let Some(row)=rows.next()? {page.versions.push(serde_json::from_str::<Version>(&row.get::<_,String>(0)?)?);}
            }
            if review.phase=="review" {
                for version in &page.versions {tx.execute("UPDATE profile_sync_review_versions SET seen=1 WHERE review=? AND operation=?",params![id.to_string(),version.operation.to_string()])?;}
                review.seen=tx.query_row("SELECT count(*) FROM profile_sync_review_versions WHERE review=? AND seen=1",[id.to_string()],|r|r.get::<_,i64>(0))?.try_into()?;
                write(&tx,&review)?;
            }
            if let Some(last)=page.versions.last() {
                page.more=tx.query_row("SELECT EXISTS(SELECT 1 FROM profile_sync_review_versions WHERE review=? AND operation>?)",params![id.to_string(),last.operation.to_string()],|r|r.get(0))?;
            }
            page.review=Some(review); tx.commit()?; Ok(page)
        }).await
    }
    pub async fn profile_resolution_cancel(&self, profile: String, id: Uuid) -> anyhow::Result<()> {
        self.run(move |db| {
            let tx=db.transaction()?;
            let mut review=read_review(&tx,&profile,id)?;
            ensure!(review.phase!="staged", "Retry the saved decision before closing this review; its result may already be recorded.");
            if review.phase!="complete" {review.phase="cancelled".into(); write(&tx,&review)?;}
            tx.commit()?; Ok(())
        }).await
    }
    pub(crate) async fn profile_resolution_stage(
        &self,
        profile: String,
        id: Uuid,
        choice: Option<Choice>,
    ) -> anyhow::Result<Review> {
        self.run(move |db| {
            let tx=db.transaction()?;
            let mut review=read_review(&tx,&profile,id)?;
            // Retries never replace even a completed decision with a new choice.
            if matches!(review.phase.as_str(),"staged"|"complete") {ensure!(choice.is_none(),"This decision is already saved. Retry its existing request.");return Ok(review);}
            ensure!(review.phase=="review" && review.seen==review.total,"Review every version before choosing which preference to keep.");
            let sub=current(&tx,&review)?;
            let change=match choice.context("Choose the value to keep before saving.")? {
                Choice::Local=>Change{action:Action::Setting{key:review.key,value:review.local.clone()},extra:Default::default()},
                Choice::Version(operation)=> {
                    let raw:String=tx.query_row("SELECT value FROM profile_sync_review_versions WHERE review=? AND operation=?",params![id.to_string(),operation.to_string()],|r|r.get(0)).optional()?.context("Choose a version from this review.")?;
                    serde_json::from_str::<Version>(&raw)?.change
                }
            };
            checked_change(review.key,&change)?;
            // IDs only; values remain paged in SQLite. The causal wire protocol
            // already bounds one resolution to MAX_PARENTS independent heads.
            let versions=tx.prepare("SELECT operation FROM profile_sync_review_versions WHERE review=? ORDER BY operation")?.query_map([id.to_string()],|r|r.get::<_,String>(0))?.map(|r|Ok(Uuid::parse_str(&r?)?)).collect::<anyhow::Result<Vec<_>>>()?;
            let operation=Uuid::new_v4();
            review.request=Some(PendingEdit {binding:sub.binding,key:review.key,operation,local_revision:review.local_revision,request:LocalEdit {operation,expected_revision:review.history_revision,changes:vec![change.clone()],resolutions:if versions.len()>1 {vec![Resolution{target:shep_profile_core::history::target(&change.action),versions}]} else {vec![]}}});
            review.phase="staged".into(); write(&tx,&review)?; tx.commit()?; Ok(review)
        }).await
    }
    pub(crate) async fn profile_resolution_stale(
        &self,
        profile: String,
        id: Uuid,
        error: String,
    ) -> anyhow::Result<()> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let mut review = read_review(&tx, &profile, id)?;
            ensure!(
                review.phase == "staged",
                "The saved review changed. Reopen its progress."
            );
            review.phase = "stale".into();
            review.error = Some(error);
            write(&tx, &review)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub(crate) async fn profile_resolution_finish(
        &self,
        profile: String,
        id: Uuid,
        edit: PendingEdit,
        receipt: u64,
        history_revision: u64,
    ) -> anyhow::Result<PreferenceSnapshot> {
        self.run(move |db| {
            let tx=db.transaction()?;
            let mut review=read_review(&tx,&profile,id)?;
            ensure!(matches!(review.phase.as_str(),"staged"|"complete") && serde_json::to_string(&review.request)?==serde_json::to_string(&Some(&edit))?,"The saved decision changed. Reopen its exact request.");
            if review.phase!="complete" {
                let sub=read(&tx,&profile)?;
                ensure!(sub.device==review.device,"The enrolled history was replaced. Recover its saved decision.");
                let current=profile_preferences::state(&tx)?;
                let mut local_revision=review.local_revision;
                if current.revisions[&review.key]==review.local_revision {
                    let mut preferences:Preferences=get(&tx,"preferences")?;
                    match &edit.request.changes[0].action {
                        Action::Setting{value,..}=>apply(&mut preferences,review.key,Some(value))?,
                        Action::SettingRemoved{..}=>apply(&mut preferences,review.key,None)?,
                        _=>anyhow::bail!("The saved decision is not a preference."),
                    }
                    preferences.validate()?; put(&tx,"preferences",&preferences)?;
                    local_revision=profile_preferences::state(&tx)?.revisions[&review.key];
                }
                ensure!(tx.execute("UPDATE profile_sync_fields SET shared=?,shared_revision=?,local_revision=?,dirty=0,error=NULL,incoming=NULL,incoming_revision=NULL WHERE profile=? AND field=? AND request IS NULL",params![serde_json::to_string(&edit.request.changes[0])?,integer(receipt)?,integer(local_revision)?,profile,field_name(review.key)?])?==1,"Another preference edit is pending. Keep this decision and recover its receipt.");
                tx.execute("UPDATE profile_sync SET history_revision=max(history_revision,?) WHERE profile=?",params![integer(history_revision)?,profile])?;
                review.phase="complete".into();write(&tx,&review)?;
            }
            let snapshot=PreferenceSnapshot{revision:get(&tx,"preferences_revision")?,value:get(&tx,"preferences")?};
            tx.commit()?; Ok(snapshot)
        }).await
    }
}
