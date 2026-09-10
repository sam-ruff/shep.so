use super::*;
use shep_mail_core::profiles::{export_account, review_account};

pub(super) fn prepare(
    db: &mut Connection,
    key: &str,
    id: Uuid,
    source: Snapshot,
    preferences: Preferences,
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
        ensure!(
            review.baseline == preferences,
            "The saved preference review differs. Resume it or cancel and prepare another."
        );
        return Ok(review);
    }
    // There is one pending platform preference receipt across Google scopes.
    let pending: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM profile_enrollments WHERE phase NOT IN ('complete','cancelled'))", [], |r| r.get(0))?;
    ensure!(
        !pending,
        "An enrollment is already pending. Resume or cancel it with its original Google account first."
    );
    ensure!(
        !super::super::sync::pending_application(&tx)?,
        "Profile sync is still applying a preference on this device. Sync now to finish it before reviewing another profile."
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
        baseline: preferences,
        settings_receipt: None,
        error: None,
    };
    tx.execute("INSERT INTO profile_enrollments(id,scope,source,baseline,phase,review) VALUES(?,?,?,?,?,?)", params![id.to_string(), key, encoded, super::super::creation::account_fingerprint(&tx)?, review.phase, serde_json::to_string(&review)?])?;
    tx.commit()?;
    Ok(review)
}
pub(super) fn rows(db: &Connection, key: &str, id: Uuid, after: u64) -> Result<Value> {
    read(db, key, id)?;
    let mut q = db.prepare("SELECT details,choice,receipt FROM profile_enrollment_rows WHERE enrollment=? AND position>? ORDER BY position LIMIT 50")?;
    let result = q
        .query_map(params![id.to_string(), integer(after)?], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Value::Array(
        result
            .into_iter()
            .map(|(details, choice, receipt)| {
                let mut row: Value = serde_json::from_str(&details)?;
                row["selected"] = (choice == "apply").into();
                row["receipt"] = serde_json::to_value(receipt)?;
                // Device credential slots are private implementation details, not UI data.
                row.as_object_mut().unwrap().remove("slot");
                Ok(row)
            })
            .collect::<Result<Vec<_>>>()?,
    ))
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
    let existing: Option<String> = db
        .query_row(
            "SELECT settings FROM accounts WHERE id=?",
            [&existing_id],
            |r| r.get(0),
        )
        .optional()?;
    let local = existing
        .map(|raw| serde_json::from_str::<Account>(&raw))
        .transpose()?;
    let same = if let Some(account) = &local {
        export_account(account, shared)?.first().map(|c| &c.action)
            == Some(&Action::AccountConnection {
                account: connection.clone(),
            })
    } else {
        false
    };
    let removed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM removed_accounts WHERE id=?)",
        [&existing_id],
        |r| r.get(0),
    )?;
    row.local_id = Some(if same {
        existing_id
    } else {
        format!("profile-account-{}", Uuid::new_v4())
    });
    row.slot = (!same).then(|| format!("credential-{}", Uuid::new_v4()));
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
pub(super) fn finish_fields(db: &mut Connection, key: &str, id: Uuid) -> Result<Review> {
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
                slot: None,
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
                            let key = serde_json::to_value(key)?;
                            ensure!(
                                SETTINGS.contains(&key.as_str().unwrap_or("")),
                                "This preference is not available in Flutter yet. Its original value is retained in the profile."
                            );
                            row.value = value;
                        }
                        Action::SettingRemoved { key } => {
                            let key = serde_json::to_value(key)?;
                            ensure!(
                                SETTINGS.contains(&key.as_str().unwrap_or("")),
                                "This preference is not available in Flutter yet."
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
pub(super) fn choose(
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
pub(super) fn approve(
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
        baseline == super::super::creation::account_fingerprint(&tx)?,
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
