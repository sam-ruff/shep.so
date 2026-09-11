//! Device-local password sync state. Only revisions and flags live here; the
//! passwords stay in the OS keychain and the Drive vault.
use super::*;
use crate::profile_sync::{
    join::{RECONNECT_KEY, Reconnect},
    state::State,
    vault::{self, Field, Local, Plan, Synced},
};
use shep_profile_core::history;

fn local(c: &Connection, binding: &history::Binding) -> anyhow::Result<Local> {
    let saved: Local = get(c, vault::STORAGE_KEY)?;
    Ok(if saved.binding.as_ref() == Some(binding) {
        saved
    } else {
        Local {
            binding: Some(binding.clone()),
            // Staged slots belong to this device whichever profile they came from.
            staged: saved.staged,
            ..Default::default()
        }
    })
}

fn replication(c: &Connection, binding: &history::Binding) -> anyhow::Result<Option<State>> {
    Ok(
        get::<Option<State>>(c, crate::profile_sync::state::STORAGE_KEY)?
            .filter(|state| &state.binding == binding),
    )
}

/// Record a toggle change in the same transaction as the enrollment options.
/// Turning passwords off asks the next online pass to remove this device's
/// entries; turning them back on cancels that request.
pub(super) fn toggled(c: &Connection, before: Options, after: Options) -> anyhow::Result<()> {
    if before.passwords == after.passwords {
        return Ok(());
    }
    let Some(selection) = current(c)?.selection else {
        return Ok(());
    };
    let mut state = local(c, &selection.binding)?;
    state.withdraw = !after.passwords;
    state.revision += 1;
    put(c, vault::STORAGE_KEY, &state)
}

/// The account a password import was tested for must still be this mapped,
/// unsuppressed native account with the same connection.
fn check_import(
    c: &Connection,
    id: &str,
    shared: uuid::Uuid,
    tested: &Account,
) -> anyhow::Result<history::Binding> {
    let (enrollment, selection) = state::selected(c)?;
    anyhow::ensure!(
        enrollment.options.sync_passwords(),
        "Password sync was turned off before the synced password was saved."
    );
    let state = replication(c, &selection.binding)?
        .context("Finish this profile's sync setup before syncing passwords.")?;
    anyhow::ensure!(
        state.accounts.get(id) == Some(&shared) && !state.suppressed.contains(&shared),
        "This account is no longer shared. Its password was not changed."
    );
    let accounts: Vec<Account> = get(c, "accounts")?;
    anyhow::ensure!(
        accounts.iter().any(|a| a.id == id && a == tested),
        "This account changed while its synced password was tested. The previous password was kept."
    );
    Ok(selection.binding)
}

impl Store {
    pub(crate) async fn credential_plan(&self) -> anyhow::Result<Option<Plan>> {
        self.run(|c| {
            let tx = c.transaction()?;
            let enrollment = current(&tx)?;
            let Some(selection) = enrollment.selection.clone().filter(|s| s.ready) else {
                return Ok(None);
            };
            if enrollment::check_google(&get(&tx, "preferences")?, &selection).is_err() {
                return Ok(None);
            }
            let local = local(&tx, &selection.binding)?;
            let publish = enrollment.options.sync_passwords();
            if !publish && !local.withdraw && local.staged.is_empty() {
                return Ok(None);
            }
            let Some(state) = replication(&tx, &selection.binding)? else {
                return Ok(None);
            };
            Ok(Some(Plan {
                device: local.device.unwrap_or_else(uuid::Uuid::new_v4),
                binding: selection.binding,
                publish,
                local,
                accounts: get(&tx, "accounts")?,
                mapping: state.accounts,
                suppressed: state.suppressed,
                reconnect: get::<Reconnect>(&tx, RECONNECT_KEY)?,
            }))
        })
        .await
    }

    /// Revision-checked, so a toggle saved during a pass is never overwritten.
    pub(crate) async fn commit_credentials(
        &self,
        expected: u64,
        next: Local,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let binding = next
                .binding
                .clone()
                .context("The password sync state has no profile.")?;
            let saved = local(&tx, &binding)?;
            anyhow::ensure!(
                saved.revision == expected,
                "Password sync settings changed during this check. It will run again."
            );
            let next = Local {
                revision: expected + 1,
                staged: saved.staged,
                ..next
            };
            put(&tx, vault::STORAGE_KEY, &next)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }

    /// Persisted before staging so a crash cannot leave staged secrets behind.
    pub(crate) async fn mark_credentials_staged(
        &self,
        id: String,
        staged: bool,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let mut state: Local = get(&tx, vault::STORAGE_KEY)?;
            let changed = if staged {
                state.staged.insert(id)
            } else {
                state.staged.remove(&id)
            };
            if changed {
                put(&tx, vault::STORAGE_KEY, &state)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn check_credential_import(
        &self,
        id: String,
        shared: uuid::Uuid,
        tested: Account,
    ) -> anyhow::Result<()> {
        self.run(move |c| check_import(c, &id, shared, &tested).map(|_| ()))
            .await
    }

    /// Clears the Reconnect marker and records the imported revisions together.
    pub(crate) async fn activate_credentials(
        &self,
        id: String,
        shared: uuid::Uuid,
        tested: Account,
        revisions: Vec<(Field, u64)>,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let binding = check_import(&tx, &id, shared, &tested)?;
            let mut state = local(&tx, &binding)?;
            for (field, revision) in revisions {
                state.slots.insert(
                    vault::slot_key(shared, field),
                    Synced {
                        revision,
                        failed: 0,
                    },
                );
            }
            put(&tx, vault::STORAGE_KEY, &state)?;
            let pending: Reconnect = get(&tx, RECONNECT_KEY)?;
            if pending.contains(&id) {
                super::join::reconnected(&tx, &id)?;
                connections::changed(&tx)?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
    }

    pub(crate) async fn fail_credential_import(
        &self,
        shared: uuid::Uuid,
        revisions: Vec<(Field, u64)>,
    ) -> anyhow::Result<()> {
        self.run(move |c| {
            let tx = c.transaction()?;
            let (_, selection) = state::selected(&tx)?;
            let mut state = local(&tx, &selection.binding)?;
            for (field, revision) in revisions {
                state
                    .slots
                    .entry(vault::slot_key(shared, field))
                    .or_default()
                    .failed = revision;
            }
            put(&tx, vault::STORAGE_KEY, &state)?;
            tx.commit()?;
            Ok(())
        })
        .await
    }
}
