use super::*;

pub(super) async fn step(profile: &MobileProfile, key: &str, review: Review) -> Result<Review> {
    let id = review.id;
    let db = &profile.database;
    let next: Option<String> = db.read(move |db| {
        Ok(db.query_row("SELECT details FROM profile_enrollment_rows WHERE enrollment=? AND kind='account' AND receipt IS NULL ORDER BY position LIMIT 1", [id.to_string()], |r| r.get(0)).optional()?)
    }).await?;
    let row = next
        .map(|raw| serde_json::from_str::<Row>(&raw))
        .transpose()?;
    // Importing a distinct account must not wait for an unrelated mail provider.
    let _account = if let Some(id) = row.as_ref().and_then(|r| r.local_id.as_ref()) {
        Some(profile.operations.try_account(id).await?)
    } else {
        None
    };
    let key = key.to_owned();
    db.write(move |db| {
        let tx = db.transaction()?;
        let mut current = read(&tx, &key, id)?;
        ensure!(current.phase == "applying", "Enrollment progress changed. Reopen it.");
        if let Some(row) = row {
            let (choice, receipt): (String, Option<String>) = tx.query_row("SELECT choice,receipt FROM profile_enrollment_rows WHERE enrollment=? AND position=?", params![id.to_string(), integer(row.position)?], |r| Ok((r.get(0)?, r.get(1)?)))?;
            if receipt.is_some() { return Ok(current); }
            let outcome = if choice != "apply" || !current.include_accounts { "kept" } else {
                ensure!(row.available, "This account cannot be applied. Reopen its review.");
                if apply_account(&tx, &current, &row)? { "applied" } else { "kept" }
            };
            tx.execute("UPDATE profile_enrollment_rows SET receipt=? WHERE enrollment=? AND position=?", params![outcome, id.to_string(), integer(row.position)?])?;
            if outcome == "applied" { current.applied += 1; } else { current.kept += 1; }
        } else {
            current.phase = "settings".into();
        }
        current.error = None; write(&tx, &current)?; tx.commit()?; Ok(current)
    }).await
}
fn apply_account(db: &Connection, review: &Review, row: &Row) -> Result<bool> {
    let local_id = row
        .local_id
        .as_deref()
        .context("Missing reviewed account identity.")?;
    let shared = row.shared.context("Missing shared account identity.")?;
    let mut candidate = row
        .account
        .clone()
        .context("Missing reviewed account connection.")?;
    candidate.id = local_id.to_owned();
    candidate.validate()?;
    let removed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM removed_accounts WHERE id=?)",
        [local_id],
        |r| r.get(0),
    )?;
    if removed {
        return Ok(false);
    }
    if let Some(slot) = &row.slot {
        // This empty, independent slot also blocks the legacy account-ID fallback.
        db.execute(
            "INSERT INTO accounts(id,settings) VALUES(?,?)",
            params![local_id, serde_json::to_string(&candidate)?],
        )?;
        db.execute(
            "INSERT INTO credential_slots(slot,account_id,state) VALUES(?,?,'active')",
            params![slot, local_id],
        )?;
        db.execute(
            "INSERT INTO account_credentials(account_id,slot) VALUES(?,?)",
            params![local_id, slot],
        )?;
        db.execute("INSERT INTO profile_reconnect(account_id,reason) VALUES(?,'Imported profile account needs passwords on this device.')", [local_id])?;
    } else {
        let current: Option<String> = db
            .query_row(
                "SELECT settings FROM accounts WHERE id=?",
                [local_id],
                |r| r.get(0),
            )
            .optional()?;
        let current = current
            .map(|raw| serde_json::from_str::<Account>(&raw))
            .transpose()?;
        if current != row.local {
            return Ok(false);
        }
        // Matching accounts retain all their existing metadata, mail and credentials.
    }
    db.execute("INSERT INTO profile_account_mappings(profile,local_id,shared_id) VALUES(?,?,?) ON CONFLICT(profile,shared_id) DO UPDATE SET local_id=excluded.local_id", params![review.binding.storage_key()?, local_id, shared.to_string()])?;
    Ok(true)
}
pub(super) fn settings(db: &Connection, key: &str, id: Uuid) -> Result<Value> {
    let review = read(db, key, id)?;
    ensure!(
        review.phase == "settings",
        "Only the pending settings step can request an application receipt."
    );
    let mut changes = BTreeMap::new();
    if review.include_settings {
        let mut q = db.prepare("SELECT details FROM profile_enrollment_rows WHERE enrollment=? AND kind='setting' AND choice='apply' ORDER BY position LIMIT 9")?;
        let rows = q
            .query_map([id.to_string()], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ensure!(
            rows.len() <= SETTINGS.len(),
            "Too many settings in this review. Update Shep and reopen it."
        );
        for raw in rows {
            let row: Row = serde_json::from_str(&raw)?;
            ensure!(
                row.available,
                "An unsupported preference was selected. Reopen this review."
            );
            let key = row
                .target
                .strip_prefix("setting:")
                .context("Invalid preference target.")?;
            ensure!(
                SETTINGS.contains(&key),
                "This preference needs a newer Shep version."
            );
            changes.insert(key.to_owned(), row.value);
        }
    }
    Ok(serde_json::json!({"id": id, "baseline": review.baseline, "changes": changes}))
}
pub(super) fn confirm(
    db: &mut Connection,
    key: &str,
    id: Uuid,
    applied: Vec<String>,
    kept: Vec<String>,
) -> Result<Review> {
    ensure!(
        applied.len() + kept.len() <= SETTINGS.len(),
        "Invalid preference application receipt."
    );
    let tx = db.transaction()?;
    let mut review = read(&tx, key, id)?;
    let receipt = serde_json::json!({"applied": applied, "kept": kept});
    if review.phase == "complete" {
        ensure!(
            review.settings_receipt.as_ref() == Some(&receipt),
            "The saved preference receipt differs. Reopen enrollment progress."
        );
        return Ok(review);
    }
    let request = settings(&tx, key, id)?;
    let expected = request["changes"]
        .as_object()
        .context("Invalid saved preference plan.")?;
    let mut actual = std::collections::BTreeSet::new();
    for field in applied.iter().chain(&kept) {
        ensure!(
            expected.contains_key(field) && actual.insert(field),
            "Invalid or duplicate preference receipt field."
        );
    }
    ensure!(
        actual.len() == expected.len(),
        "Some preference changes have no device receipt. Resume application."
    );
    review.phase = "complete".into();
    review.settings_receipt = Some(receipt);
    review.error = None;
    write(&tx, &review)?;
    tx.commit()?;
    Ok(review)
}
