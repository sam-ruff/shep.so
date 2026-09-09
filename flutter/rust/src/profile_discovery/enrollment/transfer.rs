use super::*;
#[async_trait::async_trait]
pub(super) trait Records: Send + Sync {
    async fn export(
        &self,
        source: Snapshot,
        after: u64,
    ) -> Result<Option<shep_profile_core::history::Record>>;
}
#[async_trait::async_trait]
impl Records for Discovery {
    async fn export(
        &self,
        source: Snapshot,
        after: u64,
    ) -> Result<Option<shep_profile_core::history::Record>> {
        Ok(self.export_record(source, after).await?)
    }
}
pub(super) async fn step(
    db: &Database,
    key: &str,
    review: Review,
    source: Snapshot,
    catalog: &dyn Records,
    worker: &Worker,
) -> Result<Review> {
    let id = review.id;
    let mut next = review.clone();
    match review.phase.as_str() {
        "copying" => {
            if let Some(record) = catalog.export(source, review.cursor).await? {
                // An accepted import commits even if our mail-cache receipt is lost.
                // Retrying this cursor imports the same immutable operation once.
                worker
                    .request(HistoryCommand::Import {
                        record: record.record,
                    })
                    .await?;
                next.cursor = record.position;
                next.copied += 1;
                ensure!(
                    next.copied <= review.total,
                    "The remote profile changed. Prepare another review."
                );
            } else {
                ensure!(
                    review.copied == review.total,
                    "Some profile records are missing. Retry discovery before importing."
                );
                next.phase = "draining".into();
            }
        }
        "draining" => {
            let Reply::State(state) = worker.request(HistoryCommand::Drain).await? else {
                return Err(changed());
            };
            if state.ready == 0 {
                ensure!(
                    state.initialized && state.waiting == 0 && !state.removed,
                    "The combined history is incomplete or removed. Preserve this device's setup and review the profile."
                );
                next.history_revision = state.revision;
                next.phase = "fields".into();
            }
        }
        "fields" | "planning" => {
            let Reply::State(state) = worker.request(HistoryCommand::State).await? else {
                return Err(changed());
            };
            ensure!(
                state.revision == review.history_revision && state.initialized && !state.removed,
                "Local profile history changed. Prepare a fresh review."
            );
            if review.phase == "planning" {
                let key = key.to_owned();
                return db.write(move |db| store::finish_fields(db, &key, id)).await;
            }
            let Reply::Fields(fields) = worker
                .request(HistoryCommand::Fields {
                    after: review.field_after.clone(),
                })
                .await?
            else {
                return Err(changed());
            };
            if let Some(field) = fields.first() {
                let (change, error) = if field.conflict || field.versions != 1 {
                    (None, Some("This field has concurrent versions. Resolve the conflict before applying it.".to_string()))
                } else {
                    let Reply::Versions(versions) = worker
                        .request(HistoryCommand::Versions {
                            target: field.target.clone(),
                            after: None,
                        })
                        .await?
                    else {
                        return Err(changed());
                    };
                    ensure!(
                        versions.len() == 1,
                        "The field review changed. Prepare it again."
                    );
                    let Reply::Value(change) = worker
                        .request(HistoryCommand::Value {
                            target: field.target.clone(),
                            operation: versions[0].operation,
                        })
                        .await?
                    else {
                        return Err(changed());
                    };
                    (Some(serde_json::to_string(&change)?), None)
                };
                let target = field.target.clone();
                next.field_after = Some(target.clone());
                next.error = None;
                let key = key.to_owned();
                return db.write(move |db| {
                    let tx = db.transaction()?;
                    let current = read(&tx, &key, id)?;
                    ensure!(current.phase == review.phase && current.field_after == review.field_after, "The profile field cursor changed. Reopen its progress.");
                    tx.execute("INSERT INTO profile_enrollment_fields(enrollment,target,change,error) VALUES(?,?,?,?)", params![id.to_string(), target, change, error])?;
                    write(&tx, &next)?; tx.commit()?; Ok(next)
                }).await;
            }
            next.phase = "planning".into();
        }
        _ => anyhow::bail!("This review is no longer preparing. Reopen its progress."),
    }
    next.error = None;
    let key = key.to_owned();
    db.write(move |db| {
        let current = read(db, &key, id)?;
        ensure!(
            current.phase == review.phase && current.cursor == review.cursor,
            "Enrollment progress changed. Reopen it before continuing."
        );
        write(db, &next)?;
        Ok(next)
    })
    .await
}
