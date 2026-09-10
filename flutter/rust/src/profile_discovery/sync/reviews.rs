//! Checked preference decisions. A review freezes the local intent and the exact
//! shared versions; the decision is staged as one durable operation whose retry
//! replays the same request after a lost receipt.
use super::{cycle::Owner, ledger::Edit, *};

pub(super) fn list(db: &Connection, key: &str) -> Result<Value> {
    let subscription = require(db, key)?;
    let mut q = db.prepare(
        "SELECT review FROM profile_sync_reviews WHERE subscription=? ORDER BY seq LIMIT 50",
    )?;
    let rows = q
        .query_map([subscription.id.to_string()], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let reviews = rows
        .into_iter()
        .map(|raw| {
            let review: Review = serde_json::from_str(&raw)?;
            let mut value = serde_json::to_value(&review)?;
            value["total"] = (review.versions.len() as u64).into();
            value["deciding"] = ledger::field_edit(db, subscription.id, &review.field)?
                .is_some_and(|edit| edit.state == "staged" && edit.review == Some(review.id))
                .into();
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Value::Array(reviews))
}
async fn open(profile: &MobileProfile, key: &str, id: Uuid) -> Result<(Subscription, Review)> {
    let lookup = key.to_owned();
    profile
        .database
        .read(move |db| {
            let subscription = require(db, &lookup)?;
            let review = ledger::review_by_id(db, subscription.id, id)?;
            Ok((subscription, review))
        })
        .await
}
/// Fifty exact versions per page, read from the owned history, never cached UI state.
pub(super) async fn versions_page(
    profile: &MobileProfile,
    key: &str,
    id: Uuid,
    after: Option<Uuid>,
) -> Result<Value> {
    let (subscription, review) = open(profile, key, id).await?;
    let start = match after {
        None => 0,
        Some(after) => {
            review
                .versions
                .iter()
                .position(|v| *v == after)
                .context("This version page is no longer part of the review. Refresh it.")?
                + 1
        }
    };
    let history = worker(&profile.database, subscription.binding.clone()).await?;
    let result = async {
        ensure!(
            versions(&history, &review.field).await? == review.versions,
            "Shared preferences changed while this review was open. Refresh it before choosing."
        );
        let mut page = Vec::new();
        for operation in review.versions.iter().skip(start).take(50) {
            let change = value(&history, &review.field, *operation).await?;
            page.push(serde_json::json!({
                "operation": operation,
                "value": scalar(&review.field, &change)?,
                "reset": matches!(change.action, Action::SettingRemoved { .. }),
            }));
        }
        Ok(Value::Array(page))
    }
    .await;
    let closed = history.close().await;
    let page = result?;
    closed?;
    Ok(page)
}

pub(super) async fn decide(
    profile: &MobileProfile,
    key: &str,
    id: Uuid,
    choice: Choice,
    seen: u64,
    local: Preferences,
) -> Result<Value> {
    let (subscription, review) = open(profile, key, id).await?;
    ensure!(
        seen == review.versions.len() as u64,
        "Open every version page before choosing."
    );
    ensure!(
        local.revisions[&review.field] == review.native_revision
            && local.values[&review.field] == review.local,
        "This preference changed on the device. Refresh the review before choosing."
    );
    let db = &profile.database;
    let history = worker(db, subscription.binding.clone()).await?;
    let mut owner = Owner {
        db,
        key: key.to_owned(),
        subscription,
        history: &history,
    };
    let result = choose(&mut owner, &review, choice, &local).await;
    let closed = history.close().await;
    let value = result?;
    closed?;
    Ok(value)
}
async fn choose(
    owner: &mut Owner<'_>,
    review: &Review,
    choice: Choice,
    local: &Preferences,
) -> Result<Value> {
    let field = review.field.clone();
    let id = owner.subscription.id;
    // A staged decision is retried exactly, never re-chosen.
    let staged = owner
        .db
        .read({
            let field = field.clone();
            move |db| ledger::field_edit(db, id, &field)
        })
        .await?
        .filter(|e| e.review == Some(review.id) && e.state == "staged");
    if let Some(edit) = staged {
        ensure!(
            owner.admit(&edit, local).await?,
            "Shared preferences changed while this decision was saved. Refresh the review before choosing again."
        );
        return finish(owner, review, &edit, local).await;
    }
    // The exact reviewed versions, not the whole history revision, fence the
    // decision: unrelated fields may have been published since the review.
    let current = state(owner.history.request(HistoryCommand::State).await?)?;
    let actual = versions(owner.history, &field).await?;
    ensure!(
        current.initialized
            && !current.removed
            && current.waiting == 0
            && current.ready == 0
            && actual == review.versions,
        "Shared preferences changed while this review was open. Refresh it before choosing."
    );
    let (operation, change) = match choice {
        Choice::Local => {
            let source = review
                .versions
                .first()
                .context("No shared preference version remains.")?;
            let source = value(owner.history, &field, *source).await?;
            (None, setting_change(&field, &review.local, Some(&source))?)
        }
        Choice::Shared { operation } => {
            ensure!(
                review.versions.contains(&operation),
                "Choose a version from this review."
            );
            (
                Some(operation),
                value(owner.history, &field, operation).await?,
            )
        }
    };
    scalar(&field, &change)?;
    if operation.is_some() && review.versions.len() == 1 {
        // Using the only shared version needs no new operation: the basis is that
        // version, and the device applies its value through the platform receipt.
        let operation = operation.context("Missing shared version.")?;
        let edit = Edit {
            operation,
            field: field.clone(),
            request: shep_profile_core::history::LocalEdit {
                operation,
                expected_revision: current.revision,
                changes: vec![change],
                resolutions: vec![],
            },
            native_revision: review.native_revision,
            review: Some(review.id),
            state: "admitted".into(),
        };
        return finish(owner, review, &edit, local).await;
    }
    let edit = owner.local_edit(
        &field,
        change,
        current.revision,
        review.native_revision,
        Some(review.id),
        review.versions.clone(),
    );
    let edit = owner.stage(edit).await?;
    ensure!(
        owner.admit(&edit, local).await?,
        "Shared preferences changed while this review was open. Refresh it before choosing."
    );
    finish(owner, review, &edit, local).await
}
async fn finish(
    owner: &mut Owner<'_>,
    review: &Review,
    edit: &Edit,
    local: &Preferences,
) -> Result<Value> {
    let field = review.field.clone();
    let change = edit
        .request
        .changes
        .first()
        .context("The saved decision has no preference change.")?;
    let value = scalar(&field, change)?;
    let id = owner.subscription.id;
    let review_id = review.id;
    let operation = edit.operation;
    if local.values[&field] == value {
        owner.subscription.bases.insert(
            field.clone(),
            Basis {
                operation: Some(operation),
                value,
                native_revision: Some(review.native_revision),
                observed: review.native_revision,
            },
        );
        let subscription = owner.subscription.clone();
        let key = owner.key.clone();
        owner
            .db
            .write(move |db| {
                let tx = db.transaction()?;
                ledger::supersede_field(&tx, id, &field)?;
                ledger::close_review(&tx, review_id)?;
                let current = require(&tx, &key)?;
                ensure!(
                    current.revision == subscription.revision,
                    "Profile sync settings changed. Reopen Preferences."
                );
                write(&tx, &subscription)?;
                tx.commit()?;
                Ok(())
            })
            .await?;
    } else {
        let application = Uuid::new_v4();
        let request = serde_json::json!({
            "id": application,
            "baseline": local,
            "changes": {field.clone(): value},
        });
        let staged_field = field.clone();
        owner
            .db
            .write(move |db| {
                let tx = db.transaction()?;
                ledger::supersede_field(&tx, id, &staged_field)?;
                ledger::close_review(&tx, review_id)?;
                ledger::stage_application(
                    &tx,
                    id,
                    application,
                    &staged_field,
                    operation,
                    &request,
                )?;
                tx.commit()?;
                Ok(())
            })
            .await?;
    }
    let key = owner.key.clone();
    owner.db.read(move |db| status(db, &key)).await
}
