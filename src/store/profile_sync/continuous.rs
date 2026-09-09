use super::*;
use crate::profile_sync::{
    continuous::{Observed, Report},
    metadata,
    state::Field,
};
use shep_profile_core::{Action, Change, history};

fn normalized(mut change: Change) -> anyhow::Result<Change> {
    change.extra.clear();
    if let Action::AccountConnection { account } = &mut change.action {
        account.extra.clear();
    }
    if let Action::SettingRemoved { key } = change.action {
        if let Some(value) = metadata::setting_value(key, &Preferences::default()) {
            change.action = Action::Setting { key, value };
        } else {
            change.action = Action::SettingRemoved { key };
        }
    }
    Ok(change)
}

impl Store {
    pub(crate) async fn apply_profile_observation(
        &self,
        observed: Observed,
    ) -> anyhow::Result<Report> {
        let (binding, revision, fields) = observed.into_parts();
        self.run(move |c| {
            let tx = c.transaction()?;
            let (mut enrollment, selection) = state::selected(&tx)?;
            anyhow::ensure!(
                selection.binding == binding,
                "The selected shared profile changed during sync."
            );
            if !enrollment.options.enabled {
                return Ok(Report::default());
            }
            let mut state = state::read(&tx, &binding)?;
            let mut prefs: Preferences = get(&tx, "preferences")?;
            let mut accounts: Vec<Account> = get(&tx, "accounts")?;
            let mut reconnect: crate::profile_sync::join::Reconnect =
                get(&tx, crate::profile_sync::join::RECONNECT_KEY)?;
            let mut values = state.values(&accounts, &prefs)?;
            let native_revisions = state::native_revisions(&tx, &state)?;
            let mut report = Report::default();
            let mut account_changed = false;
            let mut prefs_changed = false;
            let mut enrollment_changed = false;
            for (field, change) in fields {
                let Some(change) = change else {
                    report.review += 1;
                    continue;
                };
                if !crate::profile_sync::state::allowed(&change, enrollment.options) {
                    continue;
                }
                let target = history::target(&change.action);
                anyhow::ensure!(
                    target == field.target && field.revision <= revision,
                    "The shared field observation has inconsistent identity."
                );
                if state
                    .fields
                    .get(&target)
                    .is_some_and(|old| old.revision > field.revision)
                {
                    continue;
                }
                if state.pending.as_ref().is_some_and(|p| p.target() == target)
                    || state.deferred.contains_key(&target)
                {
                    report.review += 1;
                    continue;
                }
                let local = normalized(change.clone())?;
                let before = values.get(&target);
                let basis = state.fields.get(&target);
                let base = basis.and_then(|f| f.local.as_ref());
                let native_revision = native_revisions.get(&target).copied().unwrap_or_default();
                if !matches!(change.action, Action::ProfileName { .. })
                    && (before != base || native_revision > basis.map_or(0, |f| f.native_revision))
                    && before != Some(&local)
                {
                    report.review += 1;
                    continue;
                }
                let applied = match &change.action {
                    Action::Setting { key, .. } | Action::SettingRemoved { key } => {
                        if !metadata::SETTINGS.contains(key) {
                            continue;
                        }
                        let changed = metadata::apply_setting(&mut prefs, &change)?;
                        prefs_changed |= changed;
                        changed
                    }
                    Action::AccountConnection { account } => {
                        if state.suppressed.contains(&account.id) {
                            continue;
                        }
                        if let Some(id) = state
                            .accounts
                            .iter()
                            .find_map(|(id, shared)| (*shared == account.id).then_some(id))
                        {
                            // Never direct an existing keychain credential at a
                            // downloaded endpoint. This requires explicit review.
                            if before != Some(&local) {
                                report.review += 1;
                                continue;
                            }
                            if !accounts.iter().any(|a| &a.id == id) {
                                continue;
                            }
                            false
                        } else {
                            let Ok(mut imported) =
                                metadata::review_account(account, &account.email)
                            else {
                                report.review += 1;
                                continue;
                            };
                            let id = uuid::Uuid::new_v4().to_string();
                            connections::allow(&tx, ConnectionKind::Account, &id)?;
                            imported.id = id.clone();
                            state.accounts.insert(id.clone(), account.id);
                            reconnect.insert(id.clone());
                            if !imported.sent_folder.is_empty() {
                                tx.execute(
                                    "INSERT INTO sent_folders(account,folder) VALUES(?,?)",
                                    params![id, imported.sent_folder],
                                )?;
                            }
                            // A following name may arrive on another page. Its
                            // initial native value is a real basis for late edits.
                            let name = Change {
                                action: Action::AccountName {
                                    id: account.id,
                                    name: imported.name.clone(),
                                },
                                extra: Default::default(),
                            };
                            let name_target = history::target(&name.action);
                            values.insert(name_target.clone(), name.clone());
                            state.fields.entry(name_target).or_insert(Field {
                                local: Some(name),
                                remote: None,
                                revision: 0,
                                native_revision: 0,
                            });
                            accounts.push(imported);
                            account_changed = true;
                            true
                        }
                    }
                    Action::AccountName { id, name } => {
                        if state.suppressed.contains(id) {
                            continue;
                        }
                        let Some(local) = state
                            .accounts
                            .iter()
                            .find_map(|(local, shared)| (*shared == *id).then_some(local))
                        else {
                            continue;
                        };
                        let Some(account) = accounts.iter_mut().find(|a| &a.id == local) else {
                            continue;
                        };
                        let changed = account.name != *name;
                        account.name = name.clone();
                        account.validate()?;
                        account_changed |= changed;
                        changed
                    }
                    Action::AccountRemoved { .. } => {
                        report.review += 1;
                        continue;
                    }
                    Action::ProfileName { name } => {
                        let selected = enrollment.selection.as_mut().expect("checked selection");
                        let changed = selected.name != *name;
                        selected.name = name.clone();
                        enrollment_changed |= changed;
                        changed
                    }
                    Action::ProfileRemoved => anyhow::bail!(
                        "The shared profile was removed. Local accounts have been kept for review."
                    ),
                    Action::ProfileSetup { .. } => continue,
                };
                report.applied += usize::from(applied);
                values.insert(target.clone(), local.clone());
                state.fields.insert(
                    target,
                    Field {
                        local: Some(local),
                        remote: Some(change),
                        revision: field.revision,
                        native_revision,
                    },
                );
            }
            state.revision = state.revision.max(revision);
            state.validate()?;
            prefs.validate()?;
            put(&tx, crate::profile_sync::state::STORAGE_KEY, &state)?;
            if prefs_changed {
                put(&tx, "preferences", &prefs)?;
            }
            if account_changed {
                put(&tx, "accounts", &accounts)?;
                put(&tx, crate::profile_sync::join::RECONNECT_KEY, &reconnect)?;
                connections::changed(&tx)?;
            }
            if enrollment_changed {
                enrollment.advance()?;
                enrollment.validate()?;
                put(&tx, STORAGE_KEY, &enrollment)?;
            }
            tx.commit()?;
            Ok(report)
        })
        .await
    }
}
