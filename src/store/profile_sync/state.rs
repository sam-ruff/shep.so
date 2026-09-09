use super::*;
use crate::profile_sync::state::{self, Pending, State};
use shep_profile_core::history;
use std::collections::BTreeMap;
use uuid::Uuid;

pub(in crate::store) fn record_native_account_fields(
    c: &Connection,
    account: &Account,
    previous: Option<&Account>,
) -> anyhow::Result<()> {
    let name_changed = previous.is_none_or(|old| old.name != account.name);
    let connection_changed = previous.is_none_or(|old| {
        let mut before = old.clone();
        before.name.clone_from(&account.name);
        before != *account
    });
    if name_changed || connection_changed {
        let mut revisions: BTreeMap<String, u64> = get(c, state::NATIVE_EDITS_KEY)?;
        let revision = get(c, "connections_revision")?;
        if name_changed {
            revisions.insert(format!("local-account-name:{}", account.id), revision);
        }
        if connection_changed {
            revisions.insert(format!("local-account-connection:{}", account.id), revision);
        }
        put(c, state::NATIVE_EDITS_KEY, &revisions)?;
    }
    Ok(())
}

pub(super) fn native_revisions(
    c: &Connection,
    state: &State,
) -> anyhow::Result<BTreeMap<String, u64>> {
    let mut revisions: BTreeMap<String, u64> = get(c, state::NATIVE_EDITS_KEY)?;
    for (local, shared) in &state.accounts {
        if let Some(revision) = revisions.remove(&format!("local-account-connection:{local}")) {
            revisions.insert(format!("account:{shared}:connection"), revision);
        }
        if let Some(revision) = revisions.remove(&format!("local-account-name:{local}")) {
            revisions.insert(
                history::target(&shep_profile_core::Action::AccountName {
                    id: *shared,
                    name: String::new(),
                }),
                revision,
            );
        }
    }
    Ok(revisions)
}

/// Track native changes independently of enrollment, so a reversion remains an
/// edit and even malformed profile state cannot prevent local preference saves.
pub(in crate::store) fn record_native_preferences(
    c: &Connection,
    before: &Preferences,
    after: &Preferences,
    revision: u64,
) -> anyhow::Result<()> {
    use crate::profile_sync::metadata;
    let mut revisions: BTreeMap<String, u64> = get(c, state::NATIVE_EDITS_KEY)?;
    let mut changed = false;
    for &key in metadata::SETTINGS {
        if metadata::setting_value(key, before) != metadata::setting_value(key, after) {
            let target = history::target(&shep_profile_core::Action::SettingRemoved { key });
            revisions.insert(target, revision);
            changed = true;
        }
    }
    if changed {
        put(c, state::NATIVE_EDITS_KEY, &revisions)?;
    }
    Ok(())
}

pub(super) fn read(c: &Connection, binding: &history::Binding) -> anyhow::Result<State> {
    let value: Option<State> = get(c, state::STORAGE_KEY)?;
    let state =
        value.context("Finish this profile's sync setup before applying ongoing changes.")?;
    state.validate()?;
    anyhow::ensure!(
        &state.binding == binding,
        "This checkpoint belongs to another shared profile."
    );
    Ok(state)
}
pub(super) fn selected(c: &Connection) -> anyhow::Result<(Enrollment, Selection)> {
    let value = current(c)?;
    let selection = value
        .selection
        .clone()
        .context("Choose a shared profile first.")?;
    anyhow::ensure!(
        selection.ready,
        "Finish the initial profile copy before syncing later changes."
    );
    enrollment::check_google(&get(c, "preferences")?, &selection)?;
    Ok((value, selection))
}
impl Store {
    pub(crate) async fn profile_replication_optional(
        &self,
        binding: history::Binding,
    ) -> anyhow::Result<Option<State>> {
        self.run(move |c| {
            let value: Option<State> = get(c, state::STORAGE_KEY)?;
            if value.is_none() {
                return Ok(None);
            }
            read(c, &binding).map(Some)
        })
        .await
    }
    /// Called at the initial history/application checkpoint, before later pulls.
    /// Original common values may differ from newer local values; retain that
    /// difference so the next capture publishes the user's newer intent.
    pub async fn initialize_profile_replication(
        &self,
        expected: Snapshot,
        revision: u64,
        mapping: BTreeMap<String, Uuid>,
        common: Vec<shep_profile_core::Change>,
    ) -> anyhow::Result<State> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let enrollment = review_matches(&tx, &expected)?;
            let selection = enrollment
                .selection
                .context("Choose a shared profile first.")?;
            enrollment::check_google(&get(&tx, "preferences")?, &selection)?;
            if let Some(existing) = get::<Option<State>>(&tx, state::STORAGE_KEY)? {
                existing.validate()?;
                anyhow::ensure!(
                    existing.binding == selection.binding,
                    "This workspace already has another profile checkpoint."
                );
                return Ok(existing);
            }
            let mut state = State::new(
                selection.binding,
                revision,
                &get::<Vec<Account>>(&tx, "accounts")?,
                &get(&tx, "preferences")?,
                mapping,
                common,
            )?;
            // review_matches fences the current native revisions as well as
            // values; a changed/reverted form cannot silently become this basis.
            state.baseline_native(&native_revisions(&tx, &state)?);
            put(&tx, state::STORAGE_KEY, &state)?;
            tx.commit()?;
            Ok(state)
        })
        .await
    }

    /// Capture and reserve the exact intent atomically, before contacting Drive.
    /// Existing preferences/accounts are already durable even before this pass.
    pub async fn capture_profile_change(&self) -> anyhow::Result<Option<Pending>> {
        self.capture_profile_change_except(Default::default()).await
    }
    pub(crate) async fn capture_profile_change_except(
        &self,
        excluded: std::collections::BTreeSet<String>,
    ) -> anyhow::Result<Option<Pending>> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (enrollment, selection) = selected(&tx)?;
            let mut state = read(&tx, &selection.binding)?;
            let revisions = native_revisions(&tx, &state)?;
            let pending = state.capture_except(
                &get::<Vec<Account>>(&tx, "accounts")?,
                &get(&tx, "preferences")?,
                enrollment.options,
                &excluded,
                &revisions,
            )?;
            put(&tx, state::STORAGE_KEY, &state)?;
            tx.commit()?;
            Ok(pending)
        })
        .await
    }
    pub(crate) async fn defer_profile_change(
        &self,
        binding: history::Binding,
        pending: Pending,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut state = read(&tx, &binding)?;
            state.defer(pending)?;
            put(&tx, state::STORAGE_KEY, &state)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
    pub async fn profile_replication(&self, binding: history::Binding) -> anyhow::Result<State> {
        self.run(move |c| read(c, &binding)).await
    }
    pub async fn acknowledge_profile_change(
        &self,
        admitted: crate::profile_sync::state::Admitted,
    ) -> anyhow::Result<()> {
        let (binding, pending, revision) = admitted.into_parts();
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut state = read(&tx, &binding)?;
            // An admitted edit is acknowledged even after disconnect/opt-out.
            // Neither enrollment nor its newer options are written here.
            state.acknowledge(&pending, revision)?;
            put(&tx, state::STORAGE_KEY, &state)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests;
