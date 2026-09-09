use super::*;
use crate::profiles::publication::{
    AccountRow, Phase, Review, Specification, change, integer, validate,
};
use anyhow::ensure;
use rusqlite::OptionalExtension;
use shep_profile_core::{Action, drive::catalog::Scope, history::Binding};
use uuid::Uuid;

pub(super) fn schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS profile_publications (
        seq INTEGER PRIMARY KEY, id TEXT NOT NULL UNIQUE, scope TEXT NOT NULL,
        specification TEXT NOT NULL, source_accounts TEXT NOT NULL,
        phase TEXT NOT NULL, review TEXT NOT NULL);
        CREATE UNIQUE INDEX IF NOT EXISTS profile_publication_active ON profile_publications(scope)
            WHERE phase NOT IN ('complete','cancelled');
        CREATE TABLE IF NOT EXISTS profile_publication_rows (
            publication TEXT NOT NULL REFERENCES profile_publications(id), position INTEGER NOT NULL,
            operation TEXT NOT NULL, changes TEXT NOT NULL, request TEXT,
            local_id TEXT, shared_id TEXT, account TEXT,
            PRIMARY KEY(publication,position), UNIQUE(publication,local_id));
        CREATE TABLE IF NOT EXISTS profile_account_mappings (
            profile TEXT NOT NULL, local_id TEXT NOT NULL, shared_id TEXT NOT NULL,
            PRIMARY KEY(profile,local_id), UNIQUE(profile,shared_id));")?;
    Ok(())
}
pub(crate) fn read(db: &Connection, scope: &str, id: Uuid) -> anyhow::Result<Review> {
    let data: String = db
        .query_row(
            "SELECT review FROM profile_publications WHERE scope=? AND id=?",
            params![scope, id.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .context("This publication belongs to another setup. Reopen Profiles and sync.")?;
    Ok(serde_json::from_str(&data)?)
}
pub(crate) fn write(db: &Connection, review: &Review) -> anyhow::Result<()> {
    db.execute(
        "UPDATE profile_publications SET review=?,phase=? WHERE id=?",
        params![
            serde_json::to_string(review)?,
            review.phase.as_str(),
            review.id.to_string()
        ],
    )?;
    Ok(())
}
fn settings_match(db: &Connection, spec: &Specification) -> anyhow::Result<()> {
    let prefs: Preferences = get(db, "preferences")?;
    let current = crate::profiles::preferences::export(&prefs)?;
    ensure!(
        spec.settings
            .iter()
            .all(|(key, value)| current.get(key) == Some(value)),
        "Preferences changed. Prepare a new review with the current settings."
    );
    Ok(())
}
fn insert(
    db: &Connection,
    id: Uuid,
    position: u64,
    changes: Vec<shep_profile_core::Change>,
    account: Option<(&Account, Uuid)>,
) -> anyhow::Result<()> {
    db.execute("INSERT INTO profile_publication_rows(publication,position,operation,changes,local_id,shared_id,account) VALUES(?,?,?,?,?,?,?)",
        params![id.to_string(),integer(position)?,Uuid::new_v4().to_string(),serde_json::to_string(&changes)?,
            account.map(|(a,_)| &a.id), account.map(|(_,id)| id.to_string()), account.map(|(a,_)| serde_json::to_string(a)).transpose()?])?;
    Ok(())
}
impl Store {
    pub(crate) async fn publication_current(
        &self,
        scope: String,
    ) -> anyhow::Result<Option<Review>> {
        self.run(move |db| {
            let id: Option<String> = db
                .query_row(
                    "SELECT id FROM profile_publications WHERE scope=? ORDER BY seq DESC LIMIT 1",
                    [&scope],
                    |r| r.get(0),
                )
                .optional()?;
            id.map(|id| read(db, &scope, Uuid::parse_str(&id)?))
                .transpose()
        })
        .await
    }
    pub(crate) async fn publication_review(
        &self,
        scope: String,
        id: Uuid,
    ) -> anyhow::Result<Review> {
        self.run(move |db| read(db, &scope, id)).await
    }
    pub(crate) async fn publication_prepare(
        &self,
        scope: Scope,
        id: Uuid,
        spec: Specification,
    ) -> anyhow::Result<Review> {
        let changes = validate(&spec)?;
        let scope_key = scope.storage_key()?;
        self.run(move |db| {
            ensure!(!id.is_nil(), "Choose a valid publication identity.");
            let tx = db.transaction()?;
            let encoded = serde_json::to_string(&spec)?;
            if let Some((old_scope, old_spec)) = tx.query_row("SELECT scope,specification FROM profile_publications WHERE id=?", [id.to_string()], |r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))).optional()? {
                ensure!(old_scope == scope_key && old_spec == encoded, "The saved review differs from this request. Reopen its original publication.");
                return read(&tx,&scope_key,id);
            }
            ensure!(!tx.query_row("SELECT EXISTS(SELECT 1 FROM profile_publications WHERE scope=? AND phase NOT IN ('complete','cancelled'))", [&scope_key], |r|r.get::<_,bool>(0))?, "Resume or cancel the existing review before creating another profile.");
            settings_match(&tx,&spec)?;
            // Freeze the source within the same transaction without materializing
            // the full account collection in the UI or the publication worker.
            tx.execute("INSERT INTO profile_publications(id,scope,specification,source_accounts,phase,review)
                VALUES(?,?,?,CASE WHEN ? THEN COALESCE((SELECT value FROM kv WHERE key='accounts'),'[]') ELSE '[]' END,'preparing','{}')",
                params![id.to_string(),scope_key,encoded,spec.include_accounts])?;
            ensure!(tx.query_row("SELECT json_type(source_accounts)='array' FROM profile_publications WHERE id=?",[id.to_string()],|r|r.get::<_,bool>(0))?,"The saved account list cannot be read. Repair the local configuration before publishing.");
            let accounts: u64 = tx.query_row("SELECT json_array_length(source_accounts) FROM profile_publications WHERE id=?", [id.to_string()], |r|r.get::<_,i64>(0))?.try_into()?;
            let review = Review { id, binding: Binding { namespace: scope.namespace, principal: scope.principal, profile: id, generation: Uuid::new_v4() }, name: spec.name,
                accounts, prepared: 0, settings: spec.settings, preference_revisions:Default::default(), total: accounts.checked_add(3).context("Too many profile records")?, staged:0, uploaded:0, phase:Phase::Preparing, error:None };
            insert(&tx,id,0,vec![change(Action::ProfileSetup { complete:false })],None)?;
            insert(&tx,id,1,changes,None)?;
            write(&tx,&review)?;
            tx.commit()?;
            Ok(review)
        }).await
    }
    pub(crate) async fn publication_prepare_step(
        &self,
        scope: String,
        id: Uuid,
    ) -> anyhow::Result<Review> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let mut review = read(&tx,&scope,id)?;
            ensure!(matches!(review.phase,Phase::Preparing|Phase::Review),"This profile is no longer being prepared.");
            if review.phase == Phase::Review { return Ok(review); }
            let rows = {
                let mut query = tx.prepare("SELECT a.value FROM profile_publications p,json_each(p.source_accounts) a
                    WHERE p.id=? AND CAST(a.key AS INTEGER)>=? ORDER BY CAST(a.key AS INTEGER) LIMIT 50")?;
                query.query_map(params![id.to_string(),integer(review.prepared)?], |r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for encoded in rows {
                let account: Account = serde_json::from_str(&encoded)?;
                let shared = Uuid::parse_str(&account.id).unwrap_or_else(|_| Uuid::new_v4());
                let changes = shep_mail_core::profiles::export_account(&account,shared)?;
                insert(&tx,id,review.prepared+2,changes,Some((&account,shared)))?;
                review.prepared += 1;
            }
            ensure!(review.prepared <= review.accounts, "Frozen account counts changed. Prepare a new review.");
            if review.prepared == review.accounts {
                insert(&tx,id,review.total-1,vec![change(Action::ProfileSetup { complete:true })],None)?;
                review.phase = Phase::Review;
            }
            write(&tx,&review)?;
            tx.commit()?;
            Ok(review)
        }).await
    }
    pub(crate) async fn publication_accounts(
        &self,
        scope: String,
        id: Uuid,
        after: u64,
    ) -> anyhow::Result<Vec<AccountRow>> {
        self.run(move |db| {
            read(db,&scope,id)?;
            let mut query = db.prepare("SELECT position,account FROM profile_publication_rows WHERE publication=? AND position>? AND account IS NOT NULL ORDER BY position LIMIT 50")?;
            let rows = query.query_map(params![id.to_string(),integer(after)?], |r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter().map(|(position,account)|Ok(AccountRow { position:position.try_into()?, account:serde_json::from_str(&account)? })).collect()
        }).await
    }
    pub(crate) async fn publication_approve(
        &self,
        scope: String,
        id: Uuid,
        settings: std::collections::BTreeMap<shep_profile_core::SettingKey, serde_json::Value>,
    ) -> anyhow::Result<Review> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let mut review = read(&tx,&scope,id)?;
            ensure!(matches!(review.phase,Phase::Review|Phase::Staging|Phase::Uploading|Phase::Complete),"Finish reviewing this profile before publishing.");
            if review.phase != Phase::Review { return Ok(review); }
            ensure!(settings == review.settings,"Displayed preferences changed. Prepare a new review.");
            let encoded: String = tx.query_row("SELECT specification FROM profile_publications WHERE id=?",[id.to_string()],|r|r.get(0))?;
            let spec: Specification = serde_json::from_str(&encoded)?;
            settings_match(&tx,&spec)?;
            ensure!(!spec.include_accounts || tx.query_row("SELECT source_accounts=COALESCE((SELECT value FROM kv WHERE key='accounts'),'[]') FROM profile_publications WHERE id=?",[id.to_string()],|r|r.get::<_,bool>(0))?, "Accounts changed. Cancel this review and prepare another with the current accounts.");
            tx.execute("INSERT INTO profile_account_mappings(profile,local_id,shared_id) SELECT ?,local_id,shared_id FROM profile_publication_rows WHERE publication=? AND local_id IS NOT NULL",params![review.binding.storage_key()?,id.to_string()])?;
            review.preference_revisions=profile_preferences::state(&tx)?.revisions;
            review.phase = Phase::Staging;
            write(&tx,&review)?;
            tx.commit()?;
            Ok(review)
        }).await
    }
    pub(crate) async fn publication_cancel(
        &self,
        scope: String,
        id: Uuid,
    ) -> anyhow::Result<Review> {
        self.run(move |db| {
            let tx = db.transaction()?;
            let mut review = read(&tx, &scope, id)?;
            ensure!(
                matches!(
                    review.phase,
                    Phase::Preparing | Phase::Review | Phase::Cancelled
                ),
                "This profile already has approved work. Pause it to retain its saved progress."
            );
            review.phase = Phase::Cancelled;
            write(&tx, &review)?;
            tx.commit()?;
            Ok(review)
        })
        .await
    }
}
