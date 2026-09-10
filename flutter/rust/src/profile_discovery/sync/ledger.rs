//! Durable subscription, edit, application and review records. Every row is
//! committed before the shared history or platform store is asked to act on it.
use super::*;
use shep_profile_core::history::LocalEdit;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct Edit {
    pub operation: Uuid,
    pub field: String,
    pub request: LocalEdit,
    pub native_revision: u64,
    pub review: Option<Uuid>,
    pub state: String,
}

/// Seed from a completed enrollment. Only fields whose receipt froze the
/// original device revision get a proven basis; kept, conflicting, unsupported
/// and legacy-receipt fields stay pending until a review or local edit proves them.
pub(super) async fn subscribe(
    db: &Database,
    key: &str,
    enrollment: Uuid,
    snapshot: &Preferences,
) -> Result<()> {
    let lookup = key.to_owned();
    let review = db
        .read(move |db| super::super::enrollment::read(db, &lookup, enrollment))
        .await?;
    ensure!(
        review.phase == "complete",
        "Finish applying this profile before keeping it in sync."
    );
    let lookup = key.to_owned();
    if let Some(existing) = db.read(move |db| read(db, &lookup)).await? {
        ensure!(
            existing.origin == format!("enrollment:{enrollment}")
                || existing.binding == review.binding,
            "This device already keeps another profile in sync with this Google account. Turn that sync off before applying a different profile."
        );
        return Ok(());
    }
    let receipt = review
        .settings_receipt
        .clone()
        .context("The enrollment has no preference receipt. Resume it first.")?;
    let applied: Vec<String> = serde_json::from_value(receipt["applied"].clone())?;
    let proof: Option<BTreeMap<String, u64>> = receipt
        .get("revisions")
        .filter(|v| !v.is_null())
        .cloned()
        .map(serde_json::from_value)
        .transpose()?;
    let history = worker(db, review.binding.clone()).await?;
    let result = seed_bases(db, &review, &history, &applied, proof.as_ref(), snapshot).await;
    let closed = history.close().await;
    let bases = result?;
    closed?;
    let subscription = Subscription {
        id: Uuid::new_v4(),
        binding: review.binding.clone(),
        name: review.name.clone(),
        origin: format!("enrollment:{enrollment}"),
        enabled: false,
        fields: SETTINGS.iter().map(|s| ((*s).to_owned(), true)).collect(),
        bases,
        cursor: review.cursor,
        source_device: None,
        history_revision: review.history_revision,
        revision: 0,
        last: None,
        error: None,
    };
    let key = key.to_owned();
    db.write(move |db| {
        let tx = db.transaction()?;
        ensure!(
            read(&tx, &key)?.is_none(),
            "Profile sync was already set up for this Google account. Reopen Preferences."
        );
        tx.execute(
            "INSERT INTO profile_subscriptions(scope,id,state) VALUES(?,?,?)",
            params![
                key,
                subscription.id.to_string(),
                serde_json::to_string(&subscription)?
            ],
        )?;
        tx.commit()?;
        Ok(())
    })
    .await
}
/// The enrollment cursor is only meaningful for the observation history that
/// minted it. Record that identity now when the catalog can still prove it;
/// otherwise the first cycle replays the originals idempotently from the start.
pub(super) async fn bind_source(db: &Database, key: &str, source: &dyn Source) -> Result<()> {
    let lookup = key.to_owned();
    let subscription = db.read(move |db| require(db, &lookup)).await?;
    if subscription.source_device.is_some() {
        return Ok(());
    }
    let binding = &subscription.binding;
    let device = match source.snapshot(binding.profile, binding.generation).await {
        Ok(snapshot) => source.source_device(snapshot).await.ok(),
        Err(_) => None,
    };
    let Some(device) = device else {
        return Ok(());
    };
    let key = key.to_owned();
    db.write(move |db| {
        let mut current = require(db, &key)?;
        if current.id == subscription.id && current.source_device.is_none() {
            current.source_device = Some(device);
            write(db, &current)?;
        }
        Ok(())
    })
    .await
}
async fn seed_bases(
    db: &Database,
    review: &super::super::enrollment::Review,
    history: &Worker,
    applied: &[String],
    proof: Option<&BTreeMap<String, u64>>,
    snapshot: &Preferences,
) -> Result<BTreeMap<String, Basis>> {
    let current = state(history.request(HistoryCommand::State).await?)?;
    ensure!(
        current.initialized && !current.removed,
        "The local profile history is incomplete. Retry enrollment before syncing."
    );
    // A history that moved after the review cannot prove which version the
    // device applied; keep those fields pending rather than guessing.
    let stable = current.revision == review.history_revision;
    let id = review.id;
    let rows: Vec<(String, String, String)> = db.read(move |db| {
        let mut q = db.prepare("SELECT target,details,choice FROM profile_enrollment_rows WHERE enrollment=? AND kind='setting' ORDER BY position LIMIT 9")?;
        let rows = q.query_map([id.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }).await?;
    let mut bases = BTreeMap::new();
    for field in SETTINGS {
        let row = rows
            .iter()
            .find(|(target, _, _)| target == &super::target(field));
        let proven = proof.and_then(|p| p.get(field).copied());
        // Without original proof, intent is measured from the current revision;
        // the value shown now is never treated as an acknowledged shared value.
        let observed = proven.unwrap_or(snapshot.revisions[field]);
        let Some((_, details, choice)) = row else {
            bases.insert(
                field.to_owned(),
                Basis {
                    operation: None,
                    value: Value::Null,
                    native_revision: proven,
                    observed,
                },
            );
            continue;
        };
        let details: Value = serde_json::from_str(details)?;
        let available = details["available"].as_bool().unwrap_or(false);
        let versions = if available && stable {
            versions(history, field).await?
        } else {
            vec![]
        };
        let selected = choice == "apply" && review.include_settings && available;
        let basis = match versions.as_slice() {
            [operation] => {
                let change = value(history, field, *operation).await?;
                let native_revision = if !selected || !stable {
                    None
                } else if applied.iter().any(|a| a == field) {
                    proven
                } else {
                    // Kept: the device edited this field before application. Its
                    // baseline revision precedes that intent, so the next cycle
                    // publishes the newer local value instead of acknowledging it.
                    proof.and_then(|_| review.baseline.revisions.get(field).copied())
                };
                Basis {
                    operation: Some(*operation),
                    value: scalar(field, &change)?,
                    native_revision,
                    observed: native_revision.unwrap_or(snapshot.revisions[field]),
                }
            }
            _ => Basis {
                operation: None,
                value: Value::Null,
                native_revision: None,
                observed: snapshot.revisions[field],
            },
        };
        bases.insert(field.to_owned(), basis);
    }
    Ok(bases)
}

pub(super) fn edits(db: &Connection, subscription: Uuid, state: &str) -> Result<Vec<Edit>> {
    let mut q = db.prepare("SELECT operation,field,request,native_revision,review,state FROM profile_sync_edits WHERE subscription=? AND state=? ORDER BY seq LIMIT 32")?;
    let rows = q
        .query_map(params![subscription.to_string(), state], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(
            |(operation, field, request, native_revision, review, state)| {
                Ok(Edit {
                    operation: Uuid::parse_str(&operation)?,
                    field,
                    request: serde_json::from_str(&request)?,
                    native_revision: u64::try_from(native_revision)?,
                    review: review.map(|r| Uuid::parse_str(&r)).transpose()?,
                    state,
                })
            },
        )
        .collect()
}
pub(super) fn field_edit(db: &Connection, subscription: Uuid, field: &str) -> Result<Option<Edit>> {
    let row: Option<(String, String, i64, Option<String>, String)> = db.query_row("SELECT operation,request,native_revision,review,state FROM profile_sync_edits WHERE subscription=? AND field=? AND state IN ('staged','deferred') ORDER BY seq DESC LIMIT 1", params![subscription.to_string(), field], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).optional()?;
    row.map(|(operation, request, native_revision, review, state)| {
        Ok(Edit {
            operation: Uuid::parse_str(&operation)?,
            field: field.to_owned(),
            request: serde_json::from_str(&request)?,
            native_revision: u64::try_from(native_revision)?,
            review: review.map(|r| Uuid::parse_str(&r)).transpose()?,
            state,
        })
    })
    .transpose()
}
pub(super) fn stage_edit(db: &Connection, subscription: Uuid, edit: &Edit) -> Result<()> {
    db.execute("INSERT INTO profile_sync_edits(subscription,operation,field,request,native_revision,review,state) VALUES(?,?,?,?,?,?,'staged')", params![subscription.to_string(), edit.operation.to_string(), edit.field, serde_json::to_string(&edit.request)?, integer(edit.native_revision)?, edit.review.map(|r| r.to_string())])?;
    Ok(())
}
pub(super) fn mark_edit(db: &Connection, operation: Uuid, state: &str) -> Result<()> {
    let changed = db.execute(
        "UPDATE profile_sync_edits SET state=? WHERE operation=?",
        params![state, operation.to_string()],
    )?;
    ensure!(changed == 1, "The staged preference edit is missing.");
    Ok(())
}
pub(super) fn supersede_field(db: &Connection, subscription: Uuid, field: &str) -> Result<()> {
    db.execute("UPDATE profile_sync_edits SET state='superseded' WHERE subscription=? AND field=? AND state='deferred'", params![subscription.to_string(), field])?;
    Ok(())
}

pub(super) fn stage_application(
    db: &Connection,
    subscription: Uuid,
    id: Uuid,
    field: &str,
    operation: Uuid,
    request: &Value,
) -> Result<()> {
    ensure!(
        !pending_application(db)?,
        "A preference application is still waiting for its device receipt."
    );
    ensure_no_pending_enrollment(db)?;
    db.execute("INSERT INTO profile_sync_applications(subscription,id,field,operation,request) VALUES(?,?,?,?,?)", params![subscription.to_string(), id.to_string(), field, operation.to_string(), serde_json::to_string(request)?])?;
    Ok(())
}
pub(super) fn application(db: &Connection, key: &str) -> Result<Value> {
    let subscription = require(db, key)?;
    let raw: Option<String> = db.query_row("SELECT request FROM profile_sync_applications WHERE subscription=? AND receipt IS NULL ORDER BY seq LIMIT 1", [subscription.id.to_string()], |r| r.get(0)).optional()?;
    Ok(match raw {
        Some(raw) => serde_json::from_str(&raw)?,
        None => Value::Null,
    })
}
pub(super) fn confirm(
    db: &mut Connection,
    key: &str,
    id: Uuid,
    applied: Vec<String>,
    kept: Vec<String>,
    revisions: BTreeMap<String, u64>,
) -> Result<Value> {
    let tx = db.transaction()?;
    let mut subscription = require(&tx, key)?;
    let row: Option<(String, String, String, Option<String>)> = tx.query_row("SELECT field,operation,request,receipt FROM profile_sync_applications WHERE subscription=? AND id=?", params![subscription.id.to_string(), id.to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional()?;
    let (field, operation, request, receipt) =
        row.context("This preference application is not part of the current profile sync.")?;
    let receipt_value =
        serde_json::json!({"applied": applied, "kept": kept, "revisions": revisions});
    if let Some(saved) = receipt {
        ensure!(
            serde_json::from_str::<Value>(&saved)? == receipt_value,
            "The saved preference receipt differs. Reopen Profiles and sync."
        );
        return status(&tx, key);
    }
    ensure!(
        applied.len() + kept.len() == 1
            && applied.iter().chain(&kept).all(|f| *f == field)
            && revisions.len() == SETTINGS.len()
            && SETTINGS.iter().all(|f| {
                revisions
                    .get(*f)
                    .is_some_and(|r| *r <= 9_007_199_254_740_991)
            }),
        "Invalid preference application receipt. Resume with the saved device receipt."
    );
    let request: Value = serde_json::from_str(&request)?;
    let operation = Uuid::parse_str(&operation)?;
    if applied.len() == 1 {
        let revision = revisions
            .get(&field)
            .copied()
            .context("The receipt omits the applied field revision.")?;
        subscription.bases.insert(
            field.clone(),
            Basis {
                operation: Some(operation),
                value: request["changes"][&field].clone(),
                native_revision: Some(revision),
                observed: revision,
            },
        );
    }
    // A kept field carries newer local intent; the next cycle publishes it.
    tx.execute(
        "UPDATE profile_sync_applications SET receipt=? WHERE id=?",
        params![serde_json::to_string(&receipt_value)?, id.to_string()],
    )?;
    subscription.error = None;
    write(&tx, &subscription)?;
    let value = status(&tx, key)?;
    tx.commit()?;
    Ok(value)
}

pub(super) fn review_for(
    db: &Connection,
    subscription: Uuid,
    field: &str,
) -> Result<Option<Review>> {
    let raw: Option<String> = db
        .query_row(
            "SELECT review FROM profile_sync_reviews WHERE subscription=? AND field=?",
            params![subscription.to_string(), field],
            |r| r.get(0),
        )
        .optional()?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into))
        .transpose()
}
pub(super) fn review_by_id(db: &Connection, subscription: Uuid, id: Uuid) -> Result<Review> {
    let raw: String = db
        .query_row(
            "SELECT review FROM profile_sync_reviews WHERE subscription=? AND id=?",
            params![subscription.to_string(), id.to_string()],
            |r| r.get(0),
        )
        .optional()?
        .context("This preference review is no longer open. Refresh Profiles and sync.")?;
    Ok(serde_json::from_str(&raw)?)
}
/// One open review per field. A refreshed review keeps its identity but
/// replaces the versions and local intent the next decision must match.
pub(super) fn upsert_review(db: &Connection, subscription: Uuid, review: &Review) -> Result<()> {
    db.execute("INSERT INTO profile_sync_reviews(subscription,id,field,review) VALUES(?,?,?,?) ON CONFLICT(subscription,field) DO UPDATE SET review=excluded.review", params![subscription.to_string(), review.id.to_string(), review.field, serde_json::to_string(review)?])?;
    Ok(())
}
pub(super) fn close_review(db: &Connection, id: Uuid) -> Result<()> {
    db.execute(
        "DELETE FROM profile_sync_reviews WHERE id=?",
        [id.to_string()],
    )?;
    Ok(())
}
