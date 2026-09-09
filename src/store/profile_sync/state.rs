use super::*;
use crate::profile_sync::state::{self, Pending, State};
use shep_profile_core::history;
use std::collections::BTreeMap;
use uuid::Uuid;

fn read(c: &Connection, binding: &history::Binding) -> anyhow::Result<State> {
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
fn selected(c: &Connection) -> anyhow::Result<(Enrollment, Selection)> {
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
            let state = State::new(
                selection.binding,
                revision,
                &get::<Vec<Account>>(&tx, "accounts")?,
                &get(&tx, "preferences")?,
                mapping,
                common,
            )?;
            put(&tx, state::STORAGE_KEY, &state)?;
            tx.commit()?;
            Ok(state)
        })
        .await
    }

    /// Capture and reserve the exact intent atomically, before contacting Drive.
    /// Existing preferences/accounts are already durable even before this pass.
    pub async fn capture_profile_change(&self) -> anyhow::Result<Option<Pending>> {
        self.run(|c| {
            let tx = c.transaction()?;
            let (enrollment, selection) = selected(&tx)?;
            let mut state = read(&tx, &selection.binding)?;
            let pending = state.capture(
                &get::<Vec<Account>>(&tx, "accounts")?,
                &get(&tx, "preferences")?,
                enrollment.options,
            )?;
            put(&tx, state::STORAGE_KEY, &state)?;
            tx.commit()?;
            Ok(pending)
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
