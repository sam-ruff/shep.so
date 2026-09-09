//! One bounded background step. The caller owns provider capacity and the Google
//! lifecycle lock, and checks its active grant before each accepted step.
use super::*;
use crate::store::Store;
use anyhow::{Context, Result, ensure};
use shep_profile_core::{
    drive::{
        Drive,
        catalog::{Discovery, Phase as CatalogPhase, Scope, Snapshot},
    },
    history::{Command, Reply, Worker},
};
use std::path::PathBuf;

enum Phase {
    Local(usize),
    Refresh,
    Scan,
    Copy(Snapshot),
    Drain(Snapshot),
    Verify {
        source: Snapshot,
        after: u64,
        revision: u64,
    },
    Merge(usize),
    Upload,
    Idle,
}
pub struct Runner {
    pub binding: Binding,
    root: PathBuf,
    catalog: Discovery,
    phase: Phase,
    pub queued: u64,
    // The private catalog remains owned while this cursor is reused. Reopening
    // always starts at zero, so losing only its inventory cannot reuse proof
    // from a surviving observation journal. Normal cycles inspect new records.
    verified_after: u64,
}
pub struct Step {
    pub idle: bool,
    pub applied: Option<(SettingKey, crate::store::PreferenceSnapshot)>,
}
impl Runner {
    pub async fn open(root: PathBuf, subscription: &Subscription) -> Result<Self> {
        let path = root
            .join("histories")
            .join(format!("{}.sqlite", subscription.binding.storage_key()?));
        ensure!(
            tokio::fs::try_exists(&path).await?,
            "The enrolled history is missing. Recover it or review enrollment again before synchronizing."
        );
        let history = Worker::open(path, subscription.binding.clone()).await?;
        let observed = history.request(Command::State).await;
        history.close().await?;
        let Reply::State(state) = observed? else {
            anyhow::bail!("Could not read the enrolled history.")
        };
        ensure!(
            state.device == subscription.device
                && !state.removed
                && state.revision >= subscription.history_revision,
            "The local history was replaced, rolled back or removed. Review its recovery before syncing."
        );
        let phase = if state.initialized {
            Phase::Local(0)
        } else {
            Phase::Refresh
        };
        let scope = Scope {
            namespace: subscription.binding.namespace.clone(),
            principal: subscription.binding.principal.clone(),
        };
        // Discovery controls and background synchronization have independent
        // observation catalogs. The immutable local history is shared and opened
        // for one accepted step at a time by their common engine owner.
        let catalog = Discovery::open(
            root.join("ongoing")
                .join(format!("{}.sqlite", scope.storage_key()?)),
            scope,
        )
        .await?;
        Ok(Self {
            binding: subscription.binding.clone(),
            root,
            catalog,
            phase,
            queued: state.queued,
            verified_after: 0,
        })
    }
    pub fn needs_network(&self) -> bool {
        matches!(self.phase, Phase::Scan | Phase::Upload)
    }
    pub fn status(&self) -> &'static str {
        match self.phase {
            Phase::Local(_) => "Saving local changes",
            Phase::Refresh | Phase::Scan => "Checking shared changes",
            Phase::Copy(_) | Phase::Drain(_) | Phase::Verify { .. } => "Receiving changes",
            Phase::Merge(_) => "Applying shared preferences",
            Phase::Upload => "Uploading changes",
            Phase::Idle => "Last check finished",
        }
    }
    pub fn wake(&mut self) {
        if matches!(self.phase, Phase::Idle) {
            self.phase = Phase::Local(0);
        }
    }
    pub async fn rescan(&mut self) -> Result<()> {
        let state = self.catalog.state().await?;
        self.catalog.refresh(state.revision, true).await?;
        self.verified_after = 0;
        self.phase = if matches!(self.phase, Phase::Local(_) | Phase::Idle) {
            Phase::Local(0)
        } else {
            Phase::Scan
        };
        Ok(())
    }
    pub async fn close(self) -> Result<()> {
        Ok(self.catalog.close().await?)
    }

    pub async fn step(&mut self, store: &Store, drive: Option<&Drive>) -> Result<Step> {
        let profile = self.binding.storage_key()?;
        let subscription = store.profile_sync_subscription(profile.clone()).await?;
        ensure!(
            subscription.enabled && subscription.binding == self.binding,
            "Profile synchronization is paused or its identity changed."
        );
        let path = self
            .root
            .join("histories")
            .join(format!("{profile}.sqlite"));
        ensure!(
            tokio::fs::try_exists(&path).await?,
            "The enrolled history is missing. Recover it before continuing sync."
        );
        let history = Worker::open(path, self.binding.clone()).await?;
        let result = self.step_owned(store, &subscription, &history, drive).await;
        let closed = history.close().await;
        match result {
            Ok(step) => {
                closed?;
                store.profile_sync_error(profile, None).await?;
                Ok(step)
            }
            Err(error) => {
                let _ = store
                    .profile_sync_error(profile, Some(error.to_string()))
                    .await;
                Err(error)
            }
        }
    }
    async fn step_owned(
        &mut self,
        store: &Store,
        subscription: &Subscription,
        history: &Worker,
        drive: Option<&Drive>,
    ) -> Result<Step> {
        let profile = self.binding.storage_key()?;
        let Reply::State(state) = history.request(Command::State).await? else {
            anyhow::bail!("Could not read the shared history.")
        };
        ensure!(
            state.device == subscription.device
                && state.revision >= subscription.history_revision
                && !state.removed
                && (state.initialized
                    || matches!(
                        self.phase,
                        Phase::Refresh | Phase::Scan | Phase::Copy(_) | Phase::Drain(_)
                    )),
            "The local profile history is incomplete, replaced or removed. Keep this device's setup and review the profile before syncing."
        );
        self.queued = state.queued;
        let mut applied = None;
        match &self.phase {
            Phase::Local(index) => {
                let fields = store.profile_sync_fields(profile.clone()).await?;
                if let Some(field) = fields.get(*index) {
                    if let Some(edit) = store
                        .profile_sync_prepare_edit(profile.clone(), field.key)
                        .await?
                    {
                        match history
                            .request(Command::Edit {
                                edit: edit.request.clone(),
                            })
                            .await
                        {
                            Ok(Reply::State(saved)) => {
                                self.queued = saved.queued;
                                // An idempotent edit reply contains the *current* state,
                                // which can include later remote changes after a lost reply.
                                let revision = receipt_revision(history, &edit).await?;
                                store.profile_sync_edit_saved(edit, revision).await?;
                            }
                            Err(
                                shep_profile_core::history::Error::Changed
                                | shep_profile_core::history::Error::Conflict,
                            ) => {
                                store.profile_sync_field_error(profile,field.key,"A shared change arrived before this local edit could be recorded. Review the preference; both intents remain saved.".into()).await?;
                            }
                            Err(error) => return Err(error.into()),
                            _ => anyhow::bail!("Could not save the local profile change."),
                        }
                    }
                    self.phase = Phase::Local(index + 1);
                } else {
                    self.phase = Phase::Refresh;
                }
            }
            Phase::Refresh => {
                let state = self.catalog.state().await?;
                if state.phase == CatalogPhase::Complete && state.error.is_none() {
                    self.catalog.refresh(state.revision, false).await?;
                }
                self.phase = Phase::Scan;
            }
            Phase::Scan => {
                let state = self.catalog.state().await?;
                if state.error.is_some() {
                    self.catalog.retry(state.revision).await?;
                } else if state.phase == CatalogPhase::Complete {
                    let source = self
                        .catalog
                        .latest_snapshot(self.binding.profile, self.binding.generation)
                        .await?;
                    let device = self.catalog.source_device(source.clone()).await?;
                    store.profile_sync_source(profile, device).await?;
                    self.phase = Phase::Copy(source);
                } else {
                    self.catalog
                        .advance(drive.context(
                            "Reconnect Google with Drive access before checking shared changes.",
                        )?)
                        .await?;
                }
            }
            Phase::Copy(source) => {
                if let Some(record) = self
                    .catalog
                    .export_record(source.clone(), subscription.remote_cursor)
                    .await?
                {
                    let Reply::State(imported) = history
                        .request(Command::Import {
                            record: record.record,
                        })
                        .await?
                    else {
                        anyhow::bail!("Could not acknowledge the copied profile record.")
                    };
                    store
                        .profile_sync_copied(
                            profile,
                            subscription.remote_cursor,
                            record.position,
                            imported.revision,
                        )
                        .await?;
                } else {
                    self.phase = Phase::Drain(source.clone());
                }
            }
            Phase::Drain(source) => {
                let Reply::State(current) = history.request(Command::Drain).await? else {
                    anyhow::bail!("Could not prepare the shared changes.")
                };
                ensure!(
                    !current.removed && current.waiting == 0,
                    "Some shared changes are missing or this profile was removed. Keep local data and review the source."
                );
                if current.ready == 0 {
                    self.phase = Phase::Verify {
                        source: source.clone(),
                        after: self.verified_after,
                        revision: current.revision,
                    };
                }
            }
            Phase::Verify {
                source,
                after,
                revision,
            } => {
                if state.revision != *revision {
                    // Another accepted profile action changed ancestry or upload
                    // acknowledgments. Restart the proof over its current scope.
                    self.verified_after = 0;
                    self.phase = Phase::Local(0);
                } else {
                    let Reply::Record(record) = history
                        .request(Command::ExportAcknowledgedRecord {
                            expected_revision: *revision,
                            after: *after,
                        })
                        .await?
                    else {
                        anyhow::bail!("Could not verify the profile's acknowledged history.")
                    };
                    if let Some(record) = record {
                        let position = record.position;
                        self.catalog.verify_original(source.clone(), record).await?;
                        self.verified_after = position;
                        self.phase = Phase::Verify {
                            source: source.clone(),
                            after: position,
                            revision: *revision,
                        };
                    } else {
                        self.phase = Phase::Merge(0);
                    }
                }
            }
            Phase::Merge(index) => {
                let local = store.profile_sync_fields(profile.clone()).await?;
                if let Some(field) = local.get(*index) {
                    if field.enabled {
                        let target = shep_profile_core::history::target(
                            &shep_profile_core::Action::SettingRemoved { key: field.key },
                        );
                        let Reply::Fields(fields) = history
                            .request(Command::Fields {
                                after: Some("setting:".into()),
                            })
                            .await?
                        else {
                            anyhow::bail!("Could not read shared preferences.")
                        };
                        if let Some(remote) = fields.iter().find(|f| f.target == target) {
                            if remote.conflict || remote.versions != 1 {
                                store.profile_sync_field_error(profile.clone(),field.key,"This shared preference has concurrent versions. Review the conflict before choosing a value.".into()).await?;
                            } else {
                                let Reply::Versions(versions) = history
                                    .request(Command::Versions {
                                        target: target.clone(),
                                        after: None,
                                    })
                                    .await?
                                else {
                                    anyhow::bail!("Could not inspect the shared preference.")
                                };
                                ensure!(
                                    versions.len() == 1,
                                    "The shared preference changed. Check it again."
                                );
                                let Reply::Value(change) = history
                                    .request(Command::Value {
                                        target,
                                        operation: versions[0].operation,
                                    })
                                    .await?
                                else {
                                    anyhow::bail!("Could not read the shared preference value.")
                                };
                                match store
                                    .profile_sync_apply_setting(
                                        profile.clone(),
                                        field.key,
                                        change,
                                        remote.revision,
                                    )
                                    .await
                                {
                                    Ok(ApplyResult::Applied(snapshot)) => {
                                        applied = Some((field.key, *snapshot))
                                    }
                                    Ok(_) => {}
                                    Err(error) => {
                                        store
                                            .profile_sync_field_error(
                                                profile,
                                                field.key,
                                                error.to_string(),
                                            )
                                            .await?
                                    }
                                }
                            }
                        }
                    }
                    self.phase = Phase::Merge(index + 1);
                } else {
                    self.phase = Phase::Upload;
                }
            }
            Phase::Upload => {
                if state.queued == 0 {
                    store
                        .profile_sync_completed(profile, state.revision)
                        .await?;
                    self.phase = Phase::Idle;
                } else {
                    drive
                        .context("Reconnect Google with Drive access before uploading changes.")?
                        .upload_next_tracked(history, &self.catalog)
                        .await?;
                }
            }
            Phase::Idle => {}
        }
        Ok(Step {
            idle: matches!(self.phase, Phase::Idle),
            applied,
        })
    }
}

// When the local edit is still the sole visible version, its field revision is
// the precise receipt. Otherwise retain the known pre-edit baseline so Merge
// must inspect the later remote value/conflict; a whole-history revision could
// accidentally classify that unseen intent as already applied.
pub(super) async fn receipt_revision(history: &Worker, edit: &PendingEdit) -> Result<u64> {
    let target = shep_profile_core::history::target(&edit.request.changes[0].action);
    let Reply::Fields(fields) = history
        .request(Command::Fields {
            after: Some("setting:".into()),
        })
        .await?
    else {
        anyhow::bail!("Could not inspect the saved preference receipt.")
    };
    let field = fields
        .iter()
        .find(|field| field.target == target)
        .context("The saved preference is missing from history.")?;
    let Reply::Versions(versions) = history
        .request(Command::Versions {
            target,
            after: None,
        })
        .await?
    else {
        anyhow::bail!("Could not inspect the saved preference version.")
    };
    Ok(
        if versions.len() == 1 && versions[0].operation == edit.operation {
            field.revision
        } else {
            edit.request.expected_revision
        },
    )
}

#[cfg(all(test, feature = "test-support"))]
pub(crate) mod tests;
