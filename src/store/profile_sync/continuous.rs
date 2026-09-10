use super::*;
use crate::profile_sync::{
    account_reviews::Match,
    continuous::{Observed, Report},
    join::{Reconnect, links},
    metadata,
    state::{Field, State as Replication},
};
use shep_profile_core::{Action, Change, history};
use std::collections::BTreeMap;

/// Unmapped native accounts resembling a new shared definition. Exact matches
/// may reuse their credentials; same-address matches may only be added or kept.
pub(super) fn link_matches(
    state: &Replication,
    accounts: &[Account],
    shared: &Account,
) -> Vec<Match> {
    link_matches_with(state, accounts, shared, &BTreeMap::new())
}
pub(super) fn link_matches_with(
    state: &Replication,
    accounts: &[Account],
    shared: &Account,
    native: &BTreeMap<String, u64>,
) -> Vec<Match> {
    accounts
        .iter()
        .filter(|local| !state.accounts.contains_key(&local.id))
        .filter_map(|local| {
            let exact = links::compatible(local, shared).unwrap_or(false);
            (exact || links::same_address(local, shared)).then(|| Match {
                account: local.clone(),
                exact,
                native_revision: native
                    .get(&format!("local-account-connection:{}", local.id))
                    .copied()
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// Create a fresh reconnecting native identity for a shared definition. The
/// initial name is a real basis so a later shared name applies cleanly.
pub(super) fn add_shared_account(
    tx: &Connection,
    state: &mut Replication,
    accounts: &mut Vec<Account>,
    reconnect: &mut Reconnect,
    mut imported: Account,
    shared: uuid::Uuid,
) -> anyhow::Result<Change> {
    let id = uuid::Uuid::new_v4().to_string();
    connections::allow(tx, ConnectionKind::Account, &id)?;
    imported.id = id.clone();
    state.accounts.insert(id.clone(), shared);
    reconnect.insert(id.clone());
    if !imported.sent_folder.is_empty() {
        tx.execute(
            "INSERT INTO sent_folders(account,folder) VALUES(?,?)",
            params![id, imported.sent_folder],
        )?;
    }
    let name = Change {
        action: Action::AccountName {
            id: shared,
            name: imported.name.clone(),
        },
        extra: Default::default(),
    };
    state
        .fields
        .entry(history::target(&name.action))
        .or_insert(Field {
            local: Some(name.clone()),
            remote: None,
            revision: 0,
            native_revision: 0,
        });
    accounts.push(imported);
    Ok(name)
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
                // Suppressed accounts are a resolved device-local choice, even
                // when a concurrent connection/name value still needs review
                // elsewhere. Do not keep reporting their hidden fields here.
                if field
                    .target
                    .strip_prefix("account:")
                    .and_then(|target| target.split_once(':'))
                    .and_then(|(id, _)| uuid::Uuid::parse_str(id).ok())
                    .is_some_and(|id| state.suppressed.contains(&id))
                {
                    continue;
                }
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
                let local = crate::profile_sync::state::normalized(change.clone());
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
                            let Ok(imported) = metadata::review_account(account, &account.email)
                            else {
                                report.review += 1;
                                continue;
                            };
                            // A native account with the same connection or
                            // address needs an explicit link/add/keep choice.
                            if !link_matches(&state, &accounts, &imported).is_empty() {
                                report.review += 1;
                                continue;
                            }
                            let name = add_shared_account(
                                &tx,
                                &mut state,
                                &mut accounts,
                                &mut reconnect,
                                imported,
                                account.id,
                            )?;
                            values.insert(history::target(&name.action), name);
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
