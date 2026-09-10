//! One bounded foreground pass: retry staged edits, admit new local intent,
//! pull verified remote records, apply safe fields, defer the rest to reviews,
//! then publish. Every durable step is committed before its acknowledgment.
use super::{ledger::Edit, *};
use shep_profile_core::history::{Error as HistoryError, LocalEdit, Resolution};

pub(super) struct Owner<'a> {
    pub db: &'a Database,
    pub key: String,
    pub subscription: Subscription,
    pub history: &'a Worker,
}
impl Owner<'_> {
    async fn save(&self) -> Result<()> {
        let subscription = self.subscription.clone();
        let key = self.key.clone();
        self.db
            .write(move |db| {
                let current = require(db, &key)?;
                ensure!(
                    current.id == subscription.id && current.revision == subscription.revision,
                    "Profile sync settings changed during this cycle. The next cycle continues."
                );
                write(db, &subscription)
            })
            .await
    }
    async fn edit(&self, state: &str) -> Result<Vec<Edit>> {
        let id = self.subscription.id;
        let state = state.to_owned();
        self.db.read(move |db| ledger::edits(db, id, &state)).await
    }
    async fn field_edit(&self, field: &str) -> Result<Option<Edit>> {
        let id = self.subscription.id;
        let field = field.to_owned();
        self.db
            .read(move |db| ledger::field_edit(db, id, &field))
            .await
    }
    async fn review_for(&self, field: &str) -> Result<Option<Review>> {
        let id = self.subscription.id;
        let field = field.to_owned();
        self.db
            .read(move |db| ledger::review_for(db, id, &field))
            .await
    }
    pub(super) async fn upsert_review(&self, review: Review) -> Result<()> {
        let id = self.subscription.id;
        self.db
            .write(move |db| ledger::upsert_review(db, id, &review))
            .await
    }
    /// Persist the exact request first; the shared history then sees the same
    /// UUID and bytes on every retry after a lost reply.
    pub(super) async fn stage(&self, edit: Edit) -> Result<Edit> {
        let id = self.subscription.id;
        let staged = edit.clone();
        self.db
            .write(move |db| {
                ensure_no_pending_enrollment(db)?;
                ledger::stage_edit(db, id, &staged)
            })
            .await?;
        Ok(edit)
    }
    /// Admit a staged edit. A Changed/Conflict reply defers the exact request to
    /// a review; a lost reply leaves it staged for the same retry next cycle.
    pub(super) async fn admit(&mut self, edit: &Edit, local: &Preferences) -> Result<bool> {
        let reply = self
            .history
            .request(HistoryCommand::Edit {
                edit: edit.request.clone(),
            })
            .await;
        match reply {
            Ok(reply) => {
                state(reply)?;
            }
            Err(HistoryError::Changed | HistoryError::Conflict | HistoryError::Removed) => {
                self.defer(edit, local).await?;
                return Ok(false);
            }
            Err(error) => return Err(error.into()),
        }
        let change = value(self.history, &edit.field, edit.operation).await?;
        ensure!(
            edit.request.changes.first() == Some(&change),
            "The admitted profile edit differs from its saved local request."
        );
        let versions = versions(self.history, &edit.field).await?;
        ensure!(
            versions.contains(&edit.operation),
            "The admitted profile edit is missing from shared history."
        );
        let current = state(self.history.request(HistoryCommand::State).await?)?;
        let field = edit.field.clone();
        let operation = edit.operation;
        let review = edit.review;
        let id = self.subscription.id;
        self.subscription.bases.insert(
            field.clone(),
            Basis {
                operation: Some(operation),
                value: scalar(&field, &change)?,
                native_revision: Some(edit.native_revision),
                observed: edit.native_revision,
            },
        );
        self.subscription.history_revision = current.revision;
        let subscription = self.subscription.clone();
        let key = self.key.clone();
        self.db
            .write(move |db| {
                let tx = db.transaction()?;
                let saved = require(&tx, &key)?;
                ensure!(
                    saved.id == subscription.id && saved.revision == subscription.revision,
                    "Profile sync settings changed during this cycle. The next cycle continues."
                );
                ledger::mark_edit(&tx, operation, "admitted")?;
                ledger::supersede_field(&tx, id, &field)?;
                if let Some(review) = review {
                    ledger::close_review(&tx, review)?;
                }
                write(&tx, &subscription)?;
                tx.commit()?;
                Ok(())
            })
            .await?;
        Ok(true)
    }
    async fn defer(&mut self, edit: &Edit, local: &Preferences) -> Result<()> {
        let current = state(self.history.request(HistoryCommand::State).await?)?;
        let versions = versions(self.history, &edit.field).await?;
        let review = Review {
            id: self
                .review_for(&edit.field)
                .await?
                .map_or_else(Uuid::new_v4, |r| r.id),
            field: edit.field.clone(),
            kind: "local".into(),
            local: local.values[&edit.field].clone(),
            native_revision: local.revisions[&edit.field],
            history_revision: current.revision,
            versions,
        };
        let id = self.subscription.id;
        let operation = edit.operation;
        self.db
            .write(move |db| {
                let tx = db.transaction()?;
                ledger::mark_edit(&tx, operation, "deferred")?;
                ledger::upsert_review(&tx, id, &review)?;
                tx.commit()?;
                Ok(())
            })
            .await
    }
    pub(super) async fn current_change(&self, field: &str) -> Result<Option<(Uuid, Change)>> {
        let versions = versions(self.history, field).await?;
        Ok(match versions.as_slice() {
            [operation] => Some((*operation, value(self.history, field, *operation).await?)),
            _ => None,
        })
    }
    pub(super) fn local_edit(
        &self,
        field: &str,
        change: Change,
        revision: u64,
        native_revision: u64,
        review: Option<Uuid>,
        resolutions: Vec<Uuid>,
    ) -> Edit {
        Edit {
            operation: Uuid::new_v4(),
            field: field.to_owned(),
            request: LocalEdit {
                operation: Uuid::nil(),
                expected_revision: revision,
                changes: vec![change],
                resolutions: if resolutions.len() > 1 {
                    vec![Resolution {
                        target: target(field),
                        versions: resolutions,
                    }]
                } else {
                    vec![]
                },
            },
            native_revision,
            review,
            state: "staged".into(),
        }
        .with_operation()
    }
}
impl Edit {
    fn with_operation(mut self) -> Self {
        self.request.operation = self.operation;
        self
    }
}

pub(super) async fn run(
    profile: &MobileProfile,
    key: &str,
    source: &dyn Source,
    snapshot: Preferences,
) -> Result<Report> {
    let db = &profile.database;
    let lookup = key.to_owned();
    let subscription = db
        .read(move |db| {
            ensure_no_pending_enrollment(db)?;
            require(db, &lookup)
        })
        .await?;
    ensure!(
        subscription.enabled,
        "Profile sync is turned off on this device. Turn it on in Preferences to continue."
    );
    let history = worker(db, subscription.binding.clone()).await?;
    let mut owner = Owner {
        db,
        key: key.to_owned(),
        subscription,
        history: &history,
    };
    let result = pass(&mut owner, source, &snapshot).await;
    let closed = history.close().await;
    let report = result?;
    closed?;
    Ok(report)
}
async fn pass(owner: &mut Owner<'_>, source: &dyn Source, local: &Preferences) -> Result<Report> {
    let mut report = Report::default();
    let current = state(owner.history.request(HistoryCommand::State).await?)?;
    ensure!(
        current.initialized && !current.removed,
        "This shared profile is incomplete or removed. Local preferences have been kept."
    );
    // Lost acknowledgments retry the same operation before any new intent.
    for edit in owner.edit("staged").await? {
        if owner.admit(&edit, local).await? {
            report.admitted += 1;
        } else {
            report.deferred += 1;
        }
    }
    let mut admitted = 0;
    for field in SETTINGS {
        if admitted >= BATCH || !owner.subscription.field_enabled(field) {
            continue;
        }
        if owner.field_edit(field).await?.is_some() || owner.review_for(field).await?.is_some() {
            continue;
        }
        let Some(basis) = owner.subscription.bases.get(field).cloned() else {
            continue;
        };
        if local.revisions[field] <= basis.observed {
            continue;
        }
        let versions = versions(owner.history, field).await?;
        if versions.len() > 1 {
            // Concurrent shared versions already need a review; the local intent
            // is recorded there rather than admitted over an unresolved conflict.
            continue;
        }
        let current = owner.current_change(field).await?;
        let revision = state(owner.history.request(HistoryCommand::State).await?)?.revision;
        let change = setting_change(
            field,
            &local.values[field],
            current.as_ref().map(|(_, change)| change),
        )?;
        let edit = owner.local_edit(
            field,
            change,
            revision,
            local.revisions[field],
            None,
            vec![],
        );
        let edit = owner.stage(edit).await?;
        if owner.admit(&edit, local).await? {
            report.admitted += 1;
        } else {
            report.deferred += 1;
        }
        admitted += 1;
    }
    if !source.pull().await? {
        report.remaining = true;
        owner.subscription.last = Some(report.clone());
        owner.save().await?;
        return Ok(report);
    }
    let binding = owner.subscription.binding.clone();
    let snapshot = source.snapshot(binding.profile, binding.generation).await?;
    ensure!(
        snapshot.binding == binding,
        "The shared profile belongs to another Google account or generation. Reopen Profiles and sync."
    );
    let device = source.source_device(snapshot.clone()).await?;
    if owner.subscription.source_device != Some(device) {
        // Positions are scoped to the observation history that minted them.
        owner.subscription.source_device = Some(device);
        owner.subscription.cursor = 0;
        owner.save().await?;
    }
    for _ in 0..BATCH {
        let Some(record) = source
            .export(snapshot.clone(), owner.subscription.cursor)
            .await?
        else {
            break;
        };
        owner
            .history
            .request(HistoryCommand::Import {
                record: record.record,
            })
            .await?;
        owner.subscription.cursor = record.position;
        owner.save().await?;
        report.imported += 1;
    }
    // Publishing below changes the frozen source; check for more originals now.
    report.remaining |= source
        .export(snapshot, owner.subscription.cursor)
        .await?
        .is_some();
    let mut current = state(owner.history.request(HistoryCommand::State).await?)?;
    let mut drains = 0;
    while current.ready != 0 && drains < BATCH {
        current = state(owner.history.request(HistoryCommand::Drain).await?)?;
        drains += 1;
    }
    if current.waiting != 0 || current.ready != 0 || !current.initialized || current.removed {
        report.remaining = true;
        owner.subscription.last = Some(report.clone());
        owner.save().await?;
        return Ok(report);
    }
    owner.subscription.history_revision = current.revision;
    observe(owner, local, &mut report).await?;
    if owner.subscription.enabled {
        for _ in 0..BATCH {
            let current = state(owner.history.request(HistoryCommand::State).await?)?;
            if current.queued == 0 {
                break;
            }
            source.publish(owner.history).await?;
            report.published += 1;
        }
    }
    let current = state(owner.history.request(HistoryCommand::State).await?)?;
    report.remaining |= current.queued != 0;
    owner.subscription.last = Some(report.clone());
    owner.save().await?;
    Ok(report)
}
async fn observe(owner: &mut Owner<'_>, local: &Preferences, report: &mut Report) -> Result<()> {
    let revision = owner.subscription.history_revision;
    let mut after = None;
    let mut applied_one = owner.db.read(pending_application).await?;
    loop {
        let Reply::Fields(fields) = owner
            .history
            .request(HistoryCommand::Fields {
                after: after.clone(),
            })
            .await?
        else {
            return Err(changed());
        };
        let Some(last) = fields.last() else {
            break;
        };
        after = Some(last.target.clone());
        for entry in fields {
            let Some(field) = entry.target.strip_prefix("setting:") else {
                continue;
            };
            if !SETTINGS.contains(&field) || !owner.subscription.field_enabled(field) {
                continue;
            }
            let versions = versions(owner.history, field).await?;
            if entry.conflict || versions.len() > 1 {
                review(owner, field, "conflict", local, revision, versions).await?;
                continue;
            }
            let Some((operation, change)) = owner.current_change(field).await? else {
                continue;
            };
            let basis = owner.subscription.bases.get(field).cloned();
            if basis.as_ref().and_then(|b| b.operation) == Some(operation) {
                continue;
            }
            let edit = owner.field_edit(field).await?;
            if edit.is_some() {
                review(owner, field, "local", local, revision, versions).await?;
                continue;
            }
            let value = scalar(field, &change)?;
            match basis.and_then(|b| b.native_revision) {
                Some(native_revision) if local.revisions[field] == native_revision => {
                    if applied_one {
                        report.remaining = true;
                        continue;
                    }
                    if local.values[field] == value {
                        // The device already shows this value with proven unchanged
                        // intent; record the common basis without a platform write.
                        owner.subscription.bases.insert(
                            field.to_owned(),
                            Basis {
                                operation: Some(operation),
                                value,
                                native_revision: Some(native_revision),
                                observed: native_revision,
                            },
                        );
                        owner.save().await?;
                        report.applied += 1;
                        continue;
                    }
                    let id = Uuid::new_v4();
                    let request = serde_json::json!({
                        "id": id,
                        "baseline": local,
                        "changes": {field: value},
                    });
                    let subscription = owner.subscription.id;
                    let field = field.to_owned();
                    owner
                        .db
                        .write(move |db| {
                            ledger::stage_application(
                                db,
                                subscription,
                                id,
                                &field,
                                operation,
                                &request,
                            )
                        })
                        .await?;
                    applied_one = true;
                    report.applied += 1;
                }
                Some(_) => review(owner, field, "local", local, revision, versions).await?,
                // A matching value is not proof: the field stays pending until an
                // explicit review or a newer local edit establishes its basis.
                None if local.values[field] == value => {}
                None => review(owner, field, "unproven", local, revision, versions).await?,
            }
        }
    }
    Ok(())
}
async fn review(
    owner: &mut Owner<'_>,
    field: &str,
    kind: &str,
    local: &Preferences,
    revision: u64,
    versions: Vec<Uuid>,
) -> Result<()> {
    let existing = owner.review_for(field).await?;
    // A staged decision retries its exact request; do not replace its review.
    if owner
        .field_edit(field)
        .await?
        .is_some_and(|edit| edit.state == "staged" && edit.review.is_some())
    {
        return Ok(());
    }
    owner
        .upsert_review(Review {
            id: existing.map_or_else(Uuid::new_v4, |r| r.id),
            field: field.to_owned(),
            kind: kind.into(),
            local: local.values[field].clone(),
            native_revision: local.revisions[field],
            history_revision: revision,
            versions,
        })
        .await
}
