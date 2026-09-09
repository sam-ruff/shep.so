use super::*;
use crate::profiles::enrollment::{Review, Row};
use crate::profiles::publication::integer;
use anyhow::{Result, ensure};
use rusqlite::OptionalExtension;
use serde_json::Value;
use shep_profile_core::{Action, Change, drive::catalog::Snapshot};
use uuid::Uuid;

use shep_mail_core::profiles::{export_account, review_account};

pub(crate) fn prepare(
    db: &mut Connection,
    key: &str,
    id: Uuid,
    source: Snapshot,
) -> Result<Review> {
    ensure!(!id.is_nil(), "Choose a valid enrollment identity.");
    let encoded = serde_json::to_string(&source)?;
    let tx = db.transaction()?;
    let previous: Option<(String, String)> = tx
        .query_row(
            "SELECT source,scope FROM profile_enrollments WHERE id=?",
            [id.to_string()],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((saved, scope)) = previous {
        ensure!(
            saved == encoded && scope == key,
            "The saved enrollment has a different source. Reopen the profile."
        );
        let review = read(&tx, key, id)?;
        return Ok(review);
    }
    // There is one pending platform preference receipt across Google scopes.
    let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM profile_enrollments WHERE phase NOT IN ('complete','cancelled'))", [], |r| r.get(0))?;
    ensure!(
        !pending,
        "An enrollment is already pending. Resume or cancel it with its original Google account first."
    );
    let review = Review {
        id,
        binding: source.binding.clone(),
        name: source.profile.name.clone(),
        phase: "copying".into(),
        copied: 0,
        total: source.profile.operations,
        cursor: 0,
        history_revision: 0,
        field_after: None,
        rows: 0,
        applied: 0,
        kept: 0,
        include_accounts: true,
        include_settings: true,
        baseline: profile_preferences::state(&tx)?,
        settings_receipt: None,
        error: None,
    };
    tx.execute("INSERT INTO profile_enrollments(id,scope,source,baseline,phase,review) VALUES(?,?,?,?,?,?)", params![id.to_string(), key, encoded, account_fingerprint(&tx)?, review.phase, serde_json::to_string(&review)?])?;
    tx.commit()?;
    Ok(review)
}
pub(crate) fn rows(db: &Connection, key: &str, id: Uuid, after: u64) -> Result<Vec<Row>> {
    read(db, key, id)?;
    let mut q=db.prepare("SELECT details,choice,receipt FROM profile_enrollment_rows WHERE enrollment=? AND position>? ORDER BY position LIMIT 50")?;
    let mut rows = q.query(params![id.to_string(), integer(after)?])?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let mut detail: Row = serde_json::from_str(&row.get::<_, String>(0)?)?;
        detail.selected = row.get::<_, String>(1)? == "apply";
        detail.receipt = row.get(2)?;
        out.push(detail);
    }
    Ok(out)
}
fn field(db: &Connection, id: Uuid, target: &str) -> Result<Option<Change>> {
    let row: Option<(Option<String>, Option<String>)> = db
        .query_row(
            "SELECT change,error FROM profile_enrollment_fields WHERE enrollment=? AND target=?",
            params![id.to_string(), target],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    match row {
        None => Ok(None),
        Some((change, error)) => {
            ensure!(
                error.is_none(),
                "This account has concurrent or unsupported fields. Resolve them before importing it."
            );
            change
                .map(|raw| serde_json::from_str(&raw).map_err(Into::into))
                .transpose()
        }
    }
}
fn account_row(
    db: &Connection,
    review: &Review,
    connection: &shep_profile_core::account::Connection,
    row: &mut Row,
) -> Result<()> {
    let shared = connection.id;
    ensure!(
        field(db, review.id, &format!("account:{shared}:removed"))?.is_none(),
        "This account was removed from the profile. Its local data is preserved."
    );
    let name = match field(db, review.id, &format!("account:{shared}:name"))? {
        Some(Change {
            action: Action::AccountName { name, .. },
            extra,
        }) if extra.is_empty() => name,
        None => connection.email.clone(),
        _ => anyhow::bail!("This account name needs a newer Shep version."),
    };
    let candidate = review_account(connection, &name)?;
    let mapping: Option<String> = db
        .query_row(
            "SELECT local_id FROM profile_account_mappings WHERE profile=? AND shared_id=?",
            params![review.binding.storage_key()?, shared.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    let existing_id = mapping.clone().unwrap_or_else(|| shared.to_string());
    let local = baseline_account(db, review.id, &existing_id)?;
    let same = if let Some(account) = &local {
        export_account(account, shared)?.first().map(|c| &c.action)
            == Some(&Action::AccountConnection {
                account: connection.clone(),
            })
    } else {
        false
    };
    let removed = removed(db, &existing_id)?;
    row.existing_id = Some(existing_id.clone());
    row.removal_epoch = removal_epoch(db, &existing_id)?;
    row.mapping = mapping.clone();
    row.local_id = Some(if same {
        existing_id
    } else {
        format!("profile-account-{}", Uuid::new_v4())
    });
    row.new_account = !same;
    row.reason = if local.is_some() && !same {
        Some("The connection differs. Keep the local account, or select this row to add the profile connection separately. Existing mail and passwords stay with the original account.".into())
    } else if removed || (mapping.is_some() && local.is_none()) {
        Some("This account was removed here. Leave it unselected to keep that choice, or explicitly add a separate connection.".into())
    } else {
        None
    };
    row.account = Some(candidate);
    row.local = local;
    row.shared = Some(shared);
    Ok(())
}
pub(crate) fn finish_fields(db: &mut Connection, key: &str, id: Uuid) -> Result<Review> {
    let tx = db.transaction()?;
    let mut review = read(&tx, key, id)?;
    ensure!(
        review.phase == "planning",
        "The profile review changed. Reopen it."
    );
    let mut position = review.rows;
    {
        let after: String = tx.query_row(
            "SELECT COALESCE(MAX(target),'') FROM profile_enrollment_rows WHERE enrollment=?",
            [id.to_string()],
            |r| r.get(0),
        )?;
        let mut q = tx.prepare("SELECT target,change,error FROM profile_enrollment_fields WHERE enrollment=? AND target>? AND (target LIKE 'setting:%' OR target LIKE 'account:%:connection' OR target LIKE 'account:%:removed') ORDER BY target LIMIT 50")?;
        let mut fields = q.query(params![id.to_string(), after])?;
        while let Some(field) = fields.next()? {
            position += 1;
            let target: String = field.get(0)?;
            let change: Option<String> = field.get(1)?;
            let error: Option<String> = field.get(2)?;
            let mut row = Row {
                position,
                kind: if target.starts_with("account:") {
                    "account"
                } else {
                    "setting"
                }
                .into(),
                target,
                account: None,
                local: None,
                value: Value::Null,
                reason: error.clone(),
                available: error.is_none(),
                local_id: None,
                existing_id: None,
                removal_epoch: 0,
                mapping: None,
                new_account: false,
                selected: false,
                receipt: None,
                shared: None,
            };
            if error.is_none() {
                let result = (|| -> Result<()> {
                    let change: Change = serde_json::from_str(
                        change
                            .as_deref()
                            .context("Missing profile field. Retry discovery.")?,
                    )?;
                    ensure!(
                        change.extra.is_empty(),
                        "This field needs a newer Shep version."
                    );
                    match change.action {
                        Action::AccountConnection { account } => {
                            account_row(&tx, &review, &account, &mut row)?
                        }
                        Action::Setting { key, value } => {
                            ensure!(
                                crate::profiles::preferences::SUPPORTED.contains(&key),
                                "This preference is not available on desktop yet. Its original value is retained in the profile."
                            );
                            crate::profiles::preferences::apply(
                                &mut Preferences::default(),
                                key,
                                Some(&value),
                            )?;
                            row.value = value;
                        }
                        Action::SettingRemoved { key } => {
                            ensure!(
                                crate::profiles::preferences::SUPPORTED.contains(&key),
                                "This preference is not available on desktop yet."
                            );
                            row.value = Value::Null;
                        }
                        Action::AccountRemoved { .. } => anyhow::bail!(
                            "This profile account was removed. Enrollment preserves local accounts and unsent work; shared removal requires a separate review."
                        ),
                        _ => anyhow::bail!("This profile field needs a newer Shep version."),
                    }
                    Ok(())
                })();
                if let Err(error) = result {
                    row.available = false;
                    row.reason = Some(error.to_string());
                }
            }
            let choice = if row.available && row.reason.is_none() {
                "apply"
            } else {
                "keep"
            };
            tx.execute("INSERT INTO profile_enrollment_rows(enrollment,position,target,kind,details,choice) VALUES(?,?,?,?,?,?)", params![id.to_string(), integer(position)?, row.target, row.kind, serde_json::to_string(&row)?, choice])?;
        }
    }
    if position - review.rows < 50 {
        review.phase = "review".into();
    }
    review.rows = position;
    review.error = None;
    write(&tx, &review)?;
    tx.commit()?;
    Ok(review)
}
pub(crate) fn choose(
    db: &mut Connection,
    key: &str,
    id: Uuid,
    position: u64,
    selected: bool,
) -> Result<Review> {
    let tx = db.transaction()?;
    let review = read(&tx, key, id)?;
    ensure!(
        review.phase == "review",
        "Only an unapproved review can change its selections."
    );
    let raw: String = tx.query_row(
        "SELECT details FROM profile_enrollment_rows WHERE enrollment=? AND position=?",
        params![id.to_string(), integer(position)?],
        |r| r.get(0),
    )?;
    let row: Row = serde_json::from_str(&raw)?;
    ensure!(
        !selected || row.available,
        "This field cannot be applied. Keep it locally and review its explanation."
    );
    tx.execute(
        "UPDATE profile_enrollment_rows SET choice=? WHERE enrollment=? AND position=?",
        params![
            if selected { "apply" } else { "keep" },
            id.to_string(),
            integer(position)?
        ],
    )?;
    tx.commit()?;
    Ok(review)
}
pub(crate) fn approve(
    db: &mut Connection,
    key: &str,
    id: Uuid,
    accounts: bool,
    settings: bool,
) -> Result<Review> {
    let tx = db.transaction()?;
    let mut review = read(&tx, key, id)?;
    if review.phase != "review" {
        ensure!(
            matches!(review.phase.as_str(), "applying" | "settings" | "complete")
                && review.include_accounts == accounts
                && review.include_settings == settings,
            "The approved enrollment differs. Reopen its saved progress."
        );
        return Ok(review);
    }
    let baseline: String = tx.query_row(
        "SELECT baseline FROM profile_enrollments WHERE id=?",
        [id.to_string()],
        |r| r.get(0),
    )?;
    ensure!(
        baseline == account_fingerprint(&tx)?,
        "Accounts changed while this review was open. Cancel it and prepare a new review."
    );
    review.include_accounts = accounts;
    review.include_settings = settings;
    review.phase = "applying".into();
    review.error = None;
    write(&tx, &review)?;
    tx.commit()?;
    Ok(review)
}

pub(crate) fn schema(db: &Connection) -> Result<()> {
    db.execute_batch("CREATE TABLE IF NOT EXISTS profile_enrollments(seq INTEGER PRIMARY KEY,id TEXT NOT NULL UNIQUE,scope TEXT NOT NULL,source TEXT NOT NULL,baseline TEXT NOT NULL,phase TEXT NOT NULL,review TEXT NOT NULL);
 CREATE UNIQUE INDEX IF NOT EXISTS profile_enrollment_active ON profile_enrollments((1)) WHERE phase NOT IN ('complete','cancelled');
 CREATE TABLE IF NOT EXISTS profile_enrollment_fields(enrollment TEXT NOT NULL REFERENCES profile_enrollments(id),target TEXT NOT NULL,change TEXT,error TEXT,PRIMARY KEY(enrollment,target));
 CREATE TABLE IF NOT EXISTS profile_enrollment_rows(enrollment TEXT NOT NULL REFERENCES profile_enrollments(id),position INTEGER NOT NULL,target TEXT NOT NULL,kind TEXT NOT NULL,details TEXT NOT NULL,choice TEXT NOT NULL,receipt TEXT,PRIMARY KEY(enrollment,position),UNIQUE(enrollment,target));")?;
    Ok(())
}
pub(crate) fn read(db: &Connection, key: &str, id: Uuid) -> Result<Review> {
    let raw: String = db
        .query_row(
            "SELECT review FROM profile_enrollments WHERE id=? AND scope=?",
            params![id.to_string(), key],
            |r| r.get(0),
        )
        .optional()?
        .context("This enrollment belongs to another setup. Reopen Profiles and sync.")?;
    Ok(serde_json::from_str(&raw)?)
}
pub(crate) fn write(db: &Connection, review: &Review) -> Result<()> {
    db.execute(
        "UPDATE profile_enrollments SET phase=?,review=? WHERE id=?",
        params![
            review.phase,
            serde_json::to_string(review)?,
            review.id.to_string()
        ],
    )?;
    Ok(())
}
pub(crate) fn source(db: &Connection, key: &str, id: Uuid) -> Result<Snapshot> {
    read(db, key, id)?;
    let raw: String = db.query_row(
        "SELECT source FROM profile_enrollments WHERE id=?",
        [id.to_string()],
        |r| r.get(0),
    )?;
    Ok(serde_json::from_str(&raw)?)
}
pub(crate) fn current(db: &Connection, key: &str) -> Result<Option<Review>> {
    let id:Option<String>=db.query_row("SELECT id FROM profile_enrollments WHERE scope=? AND phase!='cancelled' ORDER BY seq DESC LIMIT 1",[key],|r|r.get(0)).optional()?;
    id.map(|id| read(db, key, Uuid::parse_str(&id)?))
        .transpose()
}
pub(crate) fn cancel(db: &mut Connection, key: &str, id: Uuid) -> Result<Review> {
    let mut review = read(db, key, id)?;
    ensure!(
        matches!(
            review.phase.as_str(),
            "copying" | "draining" | "fields" | "planning" | "review" | "cancelled"
        ),
        "Approved changes have saved receipts. Pause or resume this enrollment."
    );
    review.phase = "cancelled".into();
    review.error = None;
    write(db, &review)?;
    Ok(review)
}
fn account_fingerprint(db: &Connection) -> Result<String> {
    Ok(db.query_row(
        "SELECT COALESCE((SELECT value FROM kv WHERE key='accounts'),'[]')",
        [],
        |r| r.get(0),
    )?)
}
fn baseline_account(db: &Connection, id: Uuid, account: &str) -> Result<Option<Account>> {
    let raw:Option<String>=db.query_row("SELECT a.value FROM profile_enrollments e,json_each(e.baseline) a WHERE e.id=? AND json_extract(a.value,'$.id')=?",params![id.to_string(),account],|r|r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
fn local_account(db: &Connection, account: &str) -> Result<Option<Account>> {
    let raw:Option<String>=db.query_row("SELECT a.value FROM kv k,json_each(k.value) a WHERE k.key='accounts' AND json_extract(a.value,'$.id')=?",[account],|r|r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(Into::into))
        .transpose()
}
fn removed(db: &Connection, id: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM connection_tombstones WHERE kind='account' AND id=?)",
        [id],
        |r| r.get(0),
    )?)
}
fn removal_epoch(db: &Connection, id: &str) -> Result<u64> {
    Ok(db.query_row("SELECT COALESCE((SELECT revision FROM connection_removal_epochs WHERE kind='account' AND id=?),0)",[id],|r|r.get::<_,i64>(0))?.try_into()?)
}

pub(crate) fn apply_step(db: &mut Connection, key: &str, id: Uuid) -> Result<Review> {
    let tx = db.transaction()?;
    let mut review = read(&tx, key, id)?;
    if review.phase == "complete" {
        return Ok(review);
    }
    ensure!(
        matches!(review.phase.as_str(), "applying" | "settings"),
        "Review this profile before applying it."
    );
    if review.phase == "settings" {
        apply_settings(&tx, &mut review)?;
    } else {
        let raw:Option<(String,String)>=tx.query_row("SELECT details,choice FROM profile_enrollment_rows WHERE enrollment=? AND kind='account' AND receipt IS NULL ORDER BY position LIMIT 1",[id.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        if let Some((raw, choice)) = raw {
            let row: Row = serde_json::from_str(&raw)?;
            let applied = review.include_accounts
                && choice == "apply"
                && row.available
                && apply_account(&tx, &review, &row)?;
            tx.execute(
                "UPDATE profile_enrollment_rows SET receipt=? WHERE enrollment=? AND position=?",
                params![
                    if applied { "applied" } else { "kept" },
                    id.to_string(),
                    integer(row.position)?
                ],
            )?;
            if applied {
                review.applied += 1;
            } else {
                review.kept += 1;
            }
        } else {
            review.phase = "settings".into();
        }
    }
    review.error = None;
    write(&tx, &review)?;
    tx.commit()?;
    Ok(review)
}
fn apply_account(db: &Connection, review: &Review, row: &Row) -> Result<bool> {
    let existing = row
        .existing_id
        .as_deref()
        .context("Missing reviewed account identity.")?;
    let local_id = row
        .local_id
        .as_deref()
        .context("Missing destination account identity.")?;
    let shared = row.shared.context("Missing shared identity.")?;
    let mapping: Option<String> = db
        .query_row(
            "SELECT local_id FROM profile_account_mappings WHERE profile=? AND shared_id=?",
            params![review.binding.storage_key()?, shared.to_string()],
            |r| r.get(0),
        )
        .optional()?;
    // Re-check original identity even when approval adds a separate connection.
    if mapping != row.mapping
        || local_account(db, existing)? != row.local
        || removal_epoch(db, existing)? != row.removal_epoch
    {
        return Ok(false);
    }
    if removed(db, local_id)? {
        return Ok(false);
    }
    if row.new_account {
        ensure!(
            local_account(db, local_id)?.is_none(),
            "The destination identity is already occupied. Review the account again."
        );
        let mut account = row
            .account
            .clone()
            .context("Missing reviewed connection.")?;
        account.id = local_id.to_owned();
        account.validate()?;
        let raw = serde_json::to_string(&account)?;
        db.execute(
            "INSERT INTO kv(key,value) VALUES('accounts','[]') ON CONFLICT(key) DO NOTHING",
            [],
        )?;
        db.execute(
            "UPDATE kv SET value=json_insert(value,'$[#]',json(?)) WHERE key='accounts'",
            [raw],
        )?;
        // Metadata, reconnect guard, mapping and receipt share one transaction.
        profile_reconnect::mark(db, local_id)?;
        if !account.sent_folder.is_empty() {
            db.execute(
                "INSERT INTO sent_folders(account,folder) VALUES(?,?)",
                params![local_id, account.sent_folder],
            )?;
        }
        connections::changed(db)?;
    }
    db.execute("INSERT INTO profile_account_mappings(profile,local_id,shared_id) VALUES(?,?,?) ON CONFLICT(profile,shared_id) DO UPDATE SET local_id=excluded.local_id",params![review.binding.storage_key()?,local_id,shared.to_string()])?;
    Ok(true)
}
fn apply_settings(db: &Connection, review: &mut Review) -> Result<()> {
    use shep_profile_core::SettingKey;
    let current = profile_preferences::state(db)?;
    let mut preferences: Preferences = get(db, "preferences")?;
    let mut applied = std::collections::BTreeMap::new();
    let mut kept = Vec::new();
    let mut q=db.prepare("SELECT details,choice FROM profile_enrollment_rows WHERE enrollment=? AND kind='setting' ORDER BY position")?;
    let mut rows = q.query([review.id.to_string()])?;
    while let Some(raw) = rows.next()? {
        let row: Row = serde_json::from_str(&raw.get::<_, String>(0)?)?;
        let selected =
            review.include_settings && row.available && raw.get::<_, String>(1)? == "apply";
        let key: Option<SettingKey> = row
            .target
            .strip_prefix("setting:")
            .and_then(|s| serde_json::from_value(Value::String(s.into())).ok());
        let can_apply = selected
            && key.is_some_and(|k| {
                crate::profiles::preferences::SUPPORTED.contains(&k)
                    && current.revisions.get(&k) == review.baseline.revisions.get(&k)
                    && current.values.get(&k) == review.baseline.values.get(&k)
            });
        if can_apply {
            let key = key.unwrap();
            crate::profiles::preferences::apply(
                &mut preferences,
                key,
                (!row.value.is_null()).then_some(&row.value),
            )?;
            applied.insert(
                key,
                crate::profiles::preferences::export(&preferences)?[&key].clone(),
            );
        } else {
            kept.push(row.target.clone());
        }
        db.execute(
            "UPDATE profile_enrollment_rows SET receipt=? WHERE enrollment=? AND position=?",
            params![
                if can_apply { "applied" } else { "kept" },
                review.id.to_string(),
                integer(row.position)?
            ],
        )?;
    }
    if !applied.is_empty() {
        preferences.validate()?;
        put(db, "preferences", &preferences)?;
    }
    review.settings_receipt = Some(serde_json::json!({"applied":applied,"kept":kept}));
    review.phase = "complete".into();
    Ok(())
}
pub(crate) fn local(db: &Connection) -> Result<crate::profiles::enrollment::Local> {
    let mut q = db.prepare("SELECT account_id FROM profile_reconnect")?;
    let reconnect = q
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(crate::profiles::enrollment::Local {
        preferences: PreferenceSnapshot {
            revision: get(db, "preferences_revision")?,
            value: get(db, "preferences")?,
        },
        accounts: get(db, "accounts")?,
        connections_revision: get(db, "connections_revision")?,
        reconnect,
    })
}
